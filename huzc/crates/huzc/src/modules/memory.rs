use super::{extract_imports, module_bind_name, validate_module_program_result, LoadedModule};
use huzi_ast::Program;
use huzi_lexer::Lexer;
use huzi_parser::Parser as HuziParser;
use std::collections::{HashMap, HashSet};

/// 与 `huzi_codegen::BUILTIN_MODULES` 同步的本地内置表。
/// 内存版加载路径只依赖 lexer/parser/ast,不触 codegen,
/// 未来 huzi-lsp 直接复用时不会把 LLVM 拖进依赖图。
/// 同步由单测 `memory_builtin_table_matches_codegen` 锁定。
#[cfg(test)]
pub(super) const MEMORY_BUILTIN_MODULES: &[&str] = &["math"];
#[cfg(not(test))]
const MEMORY_BUILTIN_MODULES: &[&str] = &["math"];

/// 内存版加载的状态:按绑定名去重(截断循环导入)与加载结果。
struct MemState {
    done: HashSet<String>,
    modules: Vec<LoadedModule>,
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
