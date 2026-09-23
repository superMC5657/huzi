//! 模块加载:处理 import 语句,加载被导入模块的源码,做路径解析、
//! 去重与循环导入检测。加载结果交给 codegen 的 add_module 注册。
//!
//! 出错策略分两层,供不同调用方选用:
//! - `load_modules`:CLI 兼容入口,出错打印并 `die`(退出码 1),行为与旧版一致。
//! - `load_modules_result` / `load_modules_from_memory`:纯函数入口,返回
//!   `Err(String)` 而不退出进程,供 LSP 等常驻进程直接调用;内存版不读盘。

mod memory;
mod resolve;
#[cfg(test)]
mod tests;

pub use memory::load_modules_from_memory;

use crate::die;
use huzi_ast::{Program, Stmt};
use huzi_codegen::BUILTIN_MODULES;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 一个已加载的模块。
#[derive(Debug)]
pub struct LoadedModule {
    /// 符号绑定名(点分 import 名的末段)。
    pub name: String,
    /// 解析后的 AST;内置模块(如 math)为 None。
    pub program: Option<Program>,
    /// 模块源文件路径;内置模块与内存模块为 None,供调试信息生成 DIFile。
    pub path: Option<PathBuf>,
}

/// 全加载过程共享的状态:已解析模块(防循环导入)与加载结果。
/// `loaded` 必须跨递归共享,否则循环导入会无限递归。
struct LoadState {
    loaded: HashMap<String, PathBuf>,
    modules: Vec<LoadedModule>,
}

/// 加载主程序的全部 import,并把 Import 语句从程序中移除。
/// 返回模块列表;内置模块的 AST 与路径均为 None。
/// CLI 兼容入口:内部走 `load_modules_result`,出错仍 `die`,
/// 成功路径的模块解析语义与旧版逐字节一致。
pub fn load_modules(program: &mut Program, base_dir: &Path) -> Vec<LoadedModule> {
    match load_modules_result(program, base_dir) {
        Ok(modules) => modules,
        Err(msg) => die(msg),
    }
}

/// 可恢复的磁盘版入口:坏 import 返回 `Err` 而不退出进程。
/// 解析顺序、路径优先级与去重语义和 `load_modules` 完全一致。
pub fn load_modules_result(
    program: &mut Program,
    base_dir: &Path,
) -> Result<Vec<LoadedModule>, String> {
    let import_names = extract_imports(program);
    let mut state = LoadState {
        loaded: HashMap::new(),
        modules: Vec::new(),
    };
    for name in import_names {
        load_module_result(&name, base_dir, &mut state)?;
    }
    Ok(state.modules)
}

/// 取出主程序中的 import 语句并从语句列表中移除，同时收集 export 依赖的子模块名(保留 export 供 codegen 做重导出)。
pub(super) fn extract_imports(program: &mut Program) -> Vec<String> {
    let mut names = Vec::new();
    program.statements.retain(|stmt| match &stmt.node {
        Stmt::Import(imp) => {
            names.push(imp.name.clone());
            false
        }
        _ => true,
    });
    for stmt in &program.statements {
        if let Stmt::Export(exp) = &stmt.node {
            let root_mod = exp.path.split("::").next().unwrap_or(&exp.path);
            let root_mod = root_mod.split('.').next().unwrap_or(root_mod);
            if !names.contains(&root_mod.to_string()) {
                names.push(root_mod.to_string());
            }
        }
    }
    names
}

/// import 名的符号绑定名:点分路径取末段(`mods.helpers` -> `helpers`)。
pub(super) fn module_bind_name(import_name: &str) -> &str {
    import_name.rsplit('.').next().unwrap_or(import_name)
}

/// 解析并加载单个磁盘模块(递归处理其自身依赖),失败返回 `Err`。
fn load_module_result(
    import_name: &str,
    base_dir: &Path,
    state: &mut LoadState,
) -> Result<(), String> {
    let name = module_bind_name(import_name).to_string();
    if state.modules.iter().any(|m| m.name == name) {
        return Ok(());
    }

    // 内置模块:不解析文件,由 codegen 走 builtin 调度。
    if BUILTIN_MODULES.contains(&name.as_str()) {
        state.modules.push(LoadedModule {
            name,
            program: None,
            path: None,
        });
        return Ok(());
    }

    let path = resolve::resolve_module_file_result(import_name, base_dir)?;
    if let Some(prev) = state.loaded.get(&name) {
        if prev != &path {
            return Err(format!(
                "Module '{}' resolves to different files: {} and {}",
                name,
                prev.display(),
                path.display()
            ));
        }
        return Ok(());
    }
    state.loaded.insert(name.clone(), path.clone());

    let source = std::fs::read_to_string(&path)
        .map_err(|e| format!("Error reading module '{}': {}", path.display(), e))?;

    let mut module_program = parse_disk_source(&name, &source)?;
    validate_module_program_result(&name, &module_program)?;

    // 模块自身的 import 相对模块文件所在目录解析,与主程序共享状态。
    let module_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let nested = extract_imports(&mut module_program);
    for nested_name in nested {
        load_module_result(&nested_name, &module_dir, state)?;
    }
    state.modules.push(LoadedModule {
        name,
        program: Some(module_program),
        path: Some(path),
    });
    Ok(())
}

/// 磁盘版源码解析:首错即停,渲染文案与旧内联逻辑逐字一致。
fn parse_disk_source(name: &str, source: &str) -> Result<Program, String> {
    let tokens = Lexer::new(source.to_string())
        .tokenize()
        .map_err(|e| huzi_error::render(&e, source, &format!("Module '{name}' error")))?;
    HuziParser::new(tokens)
        .parse()
        .map_err(|e| huzi_error::render(&e, source, &format!("Module '{name}' error")))
}

/// 模块文件只允许定义(fn/struct/enum/trait/impl)与 import,不允许顶层级语句。
/// 纯函数版:违规返回 `Err`,不退出进程。
pub(super) fn validate_module_program_result(
    name: &str,
    program: &Program,
) -> Result<(), String> {
    for stmt in &program.statements {
        if !matches!(
            &stmt.node,
            Stmt::Fn(_)
                | Stmt::Struct(_)
                | Stmt::Enum(_)
                | Stmt::Import(_)
                | Stmt::Export(_)
                | Stmt::Trait(_)
                | Stmt::Impl(_)
        ) {
            return Err(format!(
                "Module '{name}' may only contain fn/struct/enum/trait/impl definitions, imports, and exports"
            ));
        }
    }
    Ok(())
}
