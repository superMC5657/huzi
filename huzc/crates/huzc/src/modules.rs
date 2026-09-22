//! 模块加载:处理 import 语句,加载被导入模块的源码,做路径解析、
//! 去重与循环导入检测。加载结果交给 codegen 的 add_module 注册。
//!
//! 出错策略分两层,供不同调用方选用:
//! - `load_modules`:CLI 兼容入口,出错打印并 `die`(退出码 1),行为与旧版一致。
//! - `load_modules_result` / `load_modules_from_memory`:纯函数入口,返回
//!   `Err(String)` 而不退出进程,供 LSP 等常驻进程直接调用;内存版不读盘。

use crate::die;
use huzi_ast::{Program, Stmt};
use huzi_codegen::BUILTIN_MODULES;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;
use std::collections::{HashMap, HashSet};
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

/// 与 `huzi_codegen::BUILTIN_MODULES` 同步的本地内置表。
/// 内存版加载路径只依赖 lexer/parser/ast,不触 codegen,
/// 未来 huzi-lsp 直接复用时不会把 LLVM 拖进依赖图。
/// 同步由单测 `memory_builtin_table_matches_codegen` 锁定。
const MEMORY_BUILTIN_MODULES: &[&str] = &["math"];

/// 全加载过程共享的状态:已解析模块(防循环导入)与加载结果。
/// `loaded` 必须跨递归共享,否则循环导入会无限递归。
struct LoadState {
    loaded: HashMap<String, PathBuf>,
    modules: Vec<LoadedModule>,
}

/// 内存版加载的状态:按绑定名去重(截断循环导入)与加载结果。
struct MemState {
    done: HashSet<String>,
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

/// 纯内存版入口:不读盘,供 huzi-lsp 直接调用。
/// `files` 的 key 为 import 名(`mods.helpers`)或 Url/路径字符串
/// (`file:///proj/mods/helpers.hz`),value 为源码文本。
/// 每个模块走 `Lexer::tokenize` + `Parser::parse_recoverable`,
/// 全部失败收集为一个 `\n` 连接的 `Err` 字符串。
pub fn load_modules_from_memory(
    files: HashMap<String, String>,
) -> Result<Vec<LoadedModule>, String> {
    let mut state = MemState {
        done: HashSet::new(),
        modules: Vec::new(),
    };
    let mut errors = Vec::new();
    let mut keys: Vec<&String> = files.keys().collect();
    keys.sort();
    for key in keys {
        let import_name = memory_key_to_import_name(key);
        load_memory_module(&import_name, &files, &mut state, &mut errors);
    }
    if errors.is_empty() {
        Ok(state.modules)
    } else {
        Err(errors.join("\n"))
    }
}

/// 取出主程序中的 import 语句并从语句列表中移除，同时收集 export 依赖的子模块名(保留 export 供 codegen 做重导出)。
fn extract_imports(program: &mut Program) -> Vec<String> {
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
fn module_bind_name(import_name: &str) -> &str {
    import_name.rsplit('.').next().unwrap_or(import_name)
}

/// 内存 key 归一化为点分 import 名:Url/路径取后缀段转点分,
/// 纯 import 名原样保留(`file:///p/mods/helpers.hz` -> `…mods.helpers`)。
fn memory_key_to_import_name(key: &str) -> String {
    let no_scheme = key.rsplit("://").next().unwrap_or(key);
    let normalized = no_scheme.trim_start_matches('/').replace('\\', "/");
    let stripped = normalized.strip_suffix(".hz").unwrap_or(&normalized);
    stripped
        .split('/')
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

/// 内存中按 import 名找源码:先精确命中,再按相对路径后缀命中 Url/路径 key,
/// 最后回退到绑定名短 key(`helpers`)。
fn lookup_memory_source<'a>(
    files: &'a HashMap<String, String>,
    import_name: &str,
) -> Option<&'a String> {
    if let Some(source) = files.get(import_name) {
        return Some(source);
    }
    let rel = format!("{}.hz", import_name.replace('.', "/"));
    for (key, source) in files {
        let normalized = key.replace('\\', "/");
        let path_part = normalized
            .rsplit("://")
            .next()
            .unwrap_or(&normalized)
            .trim_start_matches('/');
        if path_part == rel || path_part.ends_with(&format!("/{rel}")) {
            return Some(source);
        }
    }
    let rel_stem = import_name.replace('.', "/");
    for candidate in &["src/lib.hz", "lib.hz", "src/mod.hz", "mod.hz"] {
        let rel_cand = format!("{}/{}", rel_stem, candidate);
        for (key, source) in files {
            let normalized = key.replace('\\', "/");
            let path_part = normalized
                .rsplit("://")
                .next()
                .unwrap_or(&normalized)
                .trim_start_matches('/');
            if path_part == rel_cand || path_part.ends_with(&format!("/{rel_cand}")) {
                return Some(source);
            }
        }
    }
    files.get(module_bind_name(import_name))
}

/// 解析并加载单个内存模块(递归处理其自身依赖,错误收集后继续)。
/// 按绑定名先占位去重,循环 import 在此截断,不无限递归。
fn load_memory_module(
    import_name: &str,
    files: &HashMap<String, String>,
    state: &mut MemState,
    errors: &mut Vec<String>,
) {
    let name = module_bind_name(import_name).to_string();
    if !state.done.insert(name.clone()) {
        return;
    }

    // 内置模块:不解析源码,由调用方走 builtin 调度。
    if MEMORY_BUILTIN_MODULES.contains(&name.as_str()) {
        state.modules.push(LoadedModule {
            name,
            program: None,
            path: None,
        });
        return;
    }

    let Some(source) = lookup_memory_source(files, import_name) else {
        errors.push(format!("Cannot find module '{import_name}' in memory"));
        return;
    };
    let source = source.clone();
    let mut module_program = match parse_memory_source(&name, &source) {
        Ok(program) => program,
        Err(err) => {
            errors.push(err);
            return;
        }
    };

    if let Err(err) = validate_module_program_result(&name, &module_program) {
        errors.push(err);
        return;
    }

    let nested = extract_imports(&mut module_program);
    for nested_name in nested {
        load_memory_module(&nested_name, files, state, errors);
    }
    state.modules.push(LoadedModule {
        name,
        program: Some(module_program),
        path: None,
    });
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

    let path = resolve_module_file_result(import_name, base_dir)?;
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

/// 内存版源码解析:语句级错误恢复,全部错误渲染后收集。
fn parse_memory_source(name: &str, source: &str) -> Result<Program, String> {
    let tokens = Lexer::new(source.to_string())
        .tokenize()
        .map_err(|e| huzi_error::render(&e, source, &format!("Module '{name}' error")))?;
    let (program, errors) = HuziParser::new(tokens).parse_recoverable();
    if errors.is_empty() {
        Ok(program)
    } else {
        Err(errors
            .iter()
            .map(|e| huzi_error::render(e, source, &format!("Module '{name}' error")))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// 探查目录下的模块入口文件:
/// 1. `<root>/<file>` 直接存在 (如 `core/assert.hz`)
/// 2. `<root>/<file_stem>/huzi.toml` 中的 `lib_entry`
/// 3. 候选入口 `<root>/<file_stem>/{src/lib.hz, lib.hz, src/mod.hz, mod.hz}`
fn probe_entry_file(root: &Path, file: &Path) -> Option<PathBuf> {
    let p = root.join(file);
    if p.is_file() {
        return Some(p);
    }
    let stem_path = file.with_extension("");
    let stem_dir = root.join(&stem_path);
    if stem_dir.is_dir() {
        let toml_path = stem_dir.join("huzi.toml");
        if toml_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&toml_path) {
                if let Ok(m) = crate::pkg::parse_manifest(&content) {
                    if let Some(lib_entry) = m.lib_entry {
                        let p = stem_dir.join(lib_entry);
                        if p.is_file() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        for candidate in &["src/lib.hz", "lib.hz", "src/mod.hz", "mod.hz"] {
            let p = stem_dir.join(candidate);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 模块名 -> 文件:点分段转子路径加 `.hz`,先找导入文件同目录,再找当前工作目录。
/// 纯函数版:找不到返回 `Err`,不退出进程。
fn resolve_module_file_result(import_name: &str, base_dir: &Path) -> Result<PathBuf, String> {
    let segments: PathBuf = import_name.split('.').collect();
    let file = segments.with_extension("hz");
    for dir in [base_dir, Path::new(".")] {
        if let Some(hit) = probe_entry_file(dir, &file) {
            return Ok(hit);
        }
    }

    // 标准库根解析 (HUZI_LIB -> 可执行文件旁 ../huzi-src -> 相对路径 -> ~/.huzi)
    if let Some(hit) = resolve_std_module(&file, base_dir) {
        return Ok(hit);
    }

    // 尝试在 vendor/ 或 ~/.huzi/packages/ 中按包解析
    let segs: Vec<&str> = import_name.split('.').collect();
    if !segs.is_empty() {
        let pkg_name = segs[0];
        let sub_segs = &segs[1..];
        if let Some(hit) = crate::pkg::resolve_package_module(pkg_name, sub_segs, base_dir) {
            return Ok(hit);
        }
    }

    Err(format!(
        "Cannot find module '{}': tried {} and {}",
        import_name,
        base_dir.join(&file).display(),
        Path::new(".").join(&file).display()
    ))
}

/// 探查标准库模块文件:
/// 优先级: HUZI_LIB 环境变量 -> 可执行文件相对路径 -> base_dir/工作目录相对路径 -> ~/.huzi/
fn resolve_std_module(file: &Path, base_dir: &Path) -> Option<PathBuf> {
    if let Ok(lib) = std::env::var("HUZI_LIB") {
        if let Some(hit) = probe_entry_file(&PathBuf::from(lib), file) {
            return Some(hit);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for sub in ["../huzi-src", "../../huzi-src", "../../../huzi-src", "../lib/huzi-src", "huzi-src"] {
                if let Some(hit) = probe_entry_file(&exe_dir.join(sub), file) {
                    return Some(hit);
                }
            }
        }
    }
    for parent in [base_dir, Path::new(".")] {
        for sub in ["huzi-src", "../huzi-src", "../../huzi-src", "../../../huzi-src"] {
            if let Some(hit) = probe_entry_file(&parent.join(sub), file) {
                return Some(hit);
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        for sub in ["huzi-src", "std"] {
            if let Some(hit) = probe_entry_file(&PathBuf::from(&home).join(".huzi").join(sub), file) {
                return Some(hit);
            }
        }
    }
    None
}

/// 模块文件只允许定义(fn/struct/enum/trait/impl)与 import,不允许顶层级语句。
/// 纯函数版:违规返回 `Err`,不退出进程。
fn validate_module_program_result(name: &str, program: &Program) -> Result<(), String> {
    for stmt in &program.statements {
        if !matches!(
            &stmt.node,
            Stmt::Fn(_) | Stmt::Struct(_) | Stmt::Enum(_) | Stmt::Import(_) | Stmt::Export(_) | Stmt::Trait(_) | Stmt::Impl(_)
        ) {
            return Err(format!(
                "Module '{name}' may only contain fn/struct/enum/trait/impl definitions, imports, and exports"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 测试夹具源码走真实 Lexer + Parser,保证 import 语法有效。
    fn parse_program(source: &str) -> Program {
        let tokens = Lexer::new(source.to_string())
            .tokenize()
            .expect("test fixture must lex");
        HuziParser::new(tokens)
            .parse()
            .expect("test fixture must parse")
    }

    /// 并行安全的唯一临时目录,避免多 checkout/多用例互相干扰。
    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("huzc_mod_{tag}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn bad_module_name_returns_err_without_exiting() {
        let mut program =
            parse_program("import no_such_module_xyz\nfn main() -> i32 {\n return 0\n}\n");
        let err = load_modules_result(&mut program, &unique_dir("missing"))
            .expect_err("missing module must be Err, not exit");
        assert!(
            err.contains("Cannot find module 'no_such_module_xyz'"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn circular_import_truncates() {
        let dir = unique_dir("cycle");
        std::fs::write(
            dir.join("ca.hz"),
            "import cb\nfn fa() -> i32 {\n return 1\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("cb.hz"),
            "import ca\nfn fb() -> i32 {\n return 2\n}\n",
        )
        .unwrap();
        let mut program = parse_program("import ca\nfn main() -> i32 {\n return 0\n}\n");
        let modules = load_modules_result(&mut program, &dir).expect("cycle must truncate");
        let mut names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["ca", "cb"]);
    }

    #[test]
    fn memory_loader_collects_two_errors() {
        let mut files = HashMap::new();
        files.insert("bad_lex".to_string(), "\"unterminated\n".to_string());
        files.insert("bad_top".to_string(), "let x = 1\n".to_string());
        let err = load_modules_from_memory(files).expect_err("two bad modules must be Err");
        assert!(err.contains("bad_lex"), "missing lex error: {err}");
        assert!(err.contains("bad_top"), "missing validate error: {err}");
    }

    #[test]
    fn memory_loader_ok_with_url_key_and_builtin_skip() {
        let mut files = HashMap::new();
        files.insert(
            "file:///proj/mods/greeter.hz".to_string(),
            "fn greet() -> i32 {\n return 1\n}\n".to_string(),
        );
        // 内置模块直接跳过:内容为垃圾也能成功,证明未尝试解析。
        files.insert("math".to_string(), "garbage ((( ".to_string());
        files.insert(
            "app".to_string(),
            "import mods.greeter\nfn run() -> i32 {\n return 1\n}\n".to_string(),
        );
        let modules = load_modules_from_memory(files).expect("memory load must succeed");
        assert!(modules.iter().any(|m| m.name == "app" && m.program.is_some()));
        assert!(modules
            .iter()
            .any(|m| m.name == "greeter" && m.program.is_some() && m.path.is_none()));
        assert!(modules
            .iter()
            .any(|m| m.name == "math" && m.program.is_none() && m.path.is_none()));
    }

    #[test]
    fn memory_builtin_table_matches_codegen() {
        assert_eq!(MEMORY_BUILTIN_MODULES, BUILTIN_MODULES);
    }

    #[test]
    fn std_module_resolution_works() {
        let mut program = parse_program("import core.result\nfn main() -> i32 {\n return 0\n}\n");
        let dir = unique_dir("std_test");
        let modules = load_modules_result(&mut program, &dir).expect("std core.result must resolve");
        assert!(modules.iter().any(|m| m.name == "result"));
    }
}
