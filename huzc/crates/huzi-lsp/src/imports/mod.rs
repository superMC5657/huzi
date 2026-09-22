//! 多文件跳转:import 行 -> 文件 Uri 与 `模块::函数` -> 模块文件内符号。
//!
//! 纯函数层,不触 codegen/LLVM:磁盘解析走 `Lexer` + `parse_recoverable`
//! 全量解析(M3 仍全量,无 salsa)。任一步失败返回 `None`,
//! 调用方回退同文件定义,不抛错。

use std::path::PathBuf;

use ropey::Rope;
use tower_lsp_server::ls_types::*;

use crate::analysis::parse_and_collect;
use crate::mapping::{huzi_range_to_lsp, word_at_position};

/// import 行跳转:光标落在 `import X` 行且 `X` 解析到磁盘文件时,
///
/// 返回目标文件头 [`Location`]。
pub fn import_jump_location(
    text: &str,
    current_uri: &Uri,
    pos: Position,
) -> Option<Location> {
    let line_idx = pos.line as usize;
    let line = line_text(text, line_idx)?;
    let import_name = import_path_on_line(&line)?;
    // 光标须落在词上(空白处不跳)。
    let rope = Rope::from_str(text);
    word_at_position(&rope, pos)?;
    let target = resolve_import_uri(current_uri, &import_name)?;
    Some(file_head_location(target))
}

/// `模块::函数` 跳转:光标处 `A::b` 经 import 表找到模块文件,
///
/// 读盘全量解析后定位其 fn 符号;失败返回 `None`(调用方回退同文件)。
pub fn module_fn_location(
    text: &str,
    current_uri: &Uri,
    pos: Position,
) -> Option<Location> {
    let line_idx = pos.line as usize;
    let line = line_text(text, line_idx)?;
    let char_col = char_col_on_line(&line, pos)?;
    let (module, func) = double_colon_at(&line, char_col)?;
    let import_name = find_import_for_bind(text, &module)?;
    let target_uri = resolve_import_uri(current_uri, &import_name)?;
    let target_path = target_uri.to_file_path()?.into_owned();
    let target_text = std::fs::read_to_string(&target_path).ok()?;
    let target_rope = Rope::from_str(&target_text);
    let (_program, symbols) = parse_and_collect(&target_text);
    let sym = symbols
        .iter()
        .find(|s| s.name == func && is_fn_like(s.kind))
        .or_else(|| {
            symbols.iter().find(|s| s.name == func && is_type_like(s.kind))
        })
        .or_else(|| symbols.iter().find(|s| s.name == func))?;
    let range = huzi_range_to_lsp(
        &target_rope,
        sym.span.line,
        sym.span.column,
        sym.span.end_line,
        sym.span.end_column,
    );
    Some(Location {
        uri: target_uri,
        range,
    })
}

/// 当前文件 Uri + 点分 import 名 -> 目标文件 Uri。
///
/// 经 [`crate::stditems`] 统一探查:直连 `<root>/<a/b.hz>` →
/// `<stem>/huzi.toml` 的 `lib_entry` → 常规入口候选,失败为 `None`。
pub fn resolve_import_uri(
    current_uri: &Uri,
    import_name: &str,
) -> Option<Uri> {
    let path = resolve_module_path(import_name, Some(current_uri))?;
    Uri::from_file_path(path)
}

/// 点分 import 名 -> 模块文件路径(经共享候选根逐个探查)。
fn resolve_module_path(
    import_name: &str,
    current_uri: Option<&Uri>,
) -> Option<PathBuf> {
    let rel: PathBuf = import_name.split('.').collect();
    let file = rel.with_extension("hz");
    for root in crate::stditems::module_file_roots(current_uri) {
        if let Some(hit) = crate::stditems::probe_entry_file(&root, &file) {
            return Some(hit);
        }
    }
    None
}

/// 行文本是 `import X` 声明时返回点分 import 名(去首尾空格)。
fn import_path_on_line(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("import")?;
    let after = rest.strip_prefix(|c: char| c.is_whitespace())?;
    let name = after.split_whitespace().next()?;
    if valid_dotted(name) { Some(name.to_string()) } else { None }
}

/// 点分名合法性:非空段全由词字符组成。
fn valid_dotted(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|seg| {
            !seg.is_empty()
                && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// 取文档指定行(0-based)的无换行文本。
fn line_text(text: &str, line_idx: usize) -> Option<String> {
    let rope = Rope::from_str(text);
    if line_idx >= rope.len_lines() {
        return None;
    }
    Some(
        rope.line(line_idx)
            .chars()
            .take_while(|c| *c != '\n' && *c != '\r')
            .collect(),
    )
}

/// LSP 列(UTF-16)折算为行内字符列(含中文对齐,代理对中间返回 `None`)。
fn char_col_on_line(line: &str, pos: Position) -> Option<usize> {
    let target = pos.character as usize;
    let mut used = 0usize;
    for (idx, c) in line.chars().enumerate() {
        if used == target {
            return Some(idx);
        }
        used += c.len_utf16();
    }
    if used == target {
        return Some(line.chars().count());
    }
    None
}

/// 行内含 `A::b` 且字符列落在其中一处时返回 `(模块, 函数)`。
fn double_colon_at(line: &str, char_col: usize) -> Option<(String, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == ':' && chars[i + 1] == ':' {
            let start = word_start_before(&chars, i);
            let end = word_end_after(&chars, i + 2);
            if start < i && i + 2 < end {
                let module: String = chars[start..i].iter().collect();
                let func: String = chars[i + 2..end].iter().collect();
                if start <= char_col && char_col <= end {
                    return Some((module, func));
                }
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    None
}

/// 全文 import 表中找绑定名为 `bind` 的点分名(末段匹配)。
pub(crate) fn find_import_for_bind(text: &str, bind: &str) -> Option<String> {
    let rope = Rope::from_str(text);
    for idx in 0..rope.len_lines() {
        let line: String = rope
            .line(idx)
            .chars()
            .take_while(|c| *c != '\n' && *c != '\r')
            .collect();
        if let Some(name) = import_path_on_line(&line) {
            if name.rsplit('.').next() == Some(bind) || name == bind {
                return Some(name);
            }
        }
    }
    None
}

/// fn/方法类符号优先(跨模块调用只跳函数定义)。
fn is_fn_like(kind: huzi_ast::symbols::SymbolKind) -> bool {
    kind == huzi_ast::symbols::SymbolKind::Function
}

/// 具名类型次之(`json::JsonField` 跳结构体定义,优于同名变量)。
fn is_type_like(kind: huzi_ast::symbols::SymbolKind) -> bool {
    matches!(
        kind,
        huzi_ast::symbols::SymbolKind::Struct
            | huzi_ast::symbols::SymbolKind::Enum
            | huzi_ast::symbols::SymbolKind::Trait
    )
}

/// 目标文件头 Location(0 行 0 列点区间)。
fn file_head_location(uri: Uri) -> Location {
    let point = Position {
        line: 0,
        character: 0,
    };
    Location {
        uri,
        range: Range {
            start: point,
            end: point,
        },
    }
}

/// 向前取连续词字符起点。
fn word_start_before(chars: &[char], col: usize) -> usize {
    let mut start = col.min(chars.len());
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    start
}

/// 向后取连续词字符终点(开区间)。
fn word_end_after(chars: &[char], col: usize) -> usize {
    let mut end = col.min(chars.len());
    while end < chars.len() && is_word_char(chars[end]) {
        end += 1;
    }
    end
}

/// 词字符:字母/数字/下划线(与 [`crate::mapping`] 一致)。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests;
