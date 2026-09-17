//! LSP 后端:文档快照管理 + 全量诊断发布 + hover/跳转/Outline。
//!
//! 同步模式为 FULL:每次变更取最后一份全文做快照,
//! 快照转 `String` 后立即释放锁,再做词法/语法分析,
//! 最后携带文档 version 发布诊断。
//!
//! hover/定义/Outline 均为同文件符号级能力:快照 Rope 后释放锁,
//! 经 [`crate::mapping`] 做 UTF-16 折算取词,再用
//! [`crate::analysis::parse_and_collect`] 查同名符号;失败一律
//! `Ok(None)`/空表,不抛错。

use std::sync::Arc;

use dashmap::DashMap;
use huzi_ast::symbols::{Symbol, SymbolKind as HuziSymbolKind};
use ropey::Rope;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer};

use crate::analysis::{RawDiagnostic, check_text, parse_and_collect};
use crate::completion::completion_for_text;
use crate::imports::{import_jump_location, module_fn_location};
use crate::mapping::{huzi_range_to_lsp, lsp_pos_to_huzi, word_at_position};
use crate::references::references_for_text;
use crate::semantic::{legend, tokens_for_text};

/// 有状态的 LSP 后端:客户端句柄 + 打开文档的 [`Rope`] 快照。
#[derive(Debug)]
pub struct Backend {
    client: Client,
    docs: Arc<DashMap<Uri, Rope>>,
}

impl Backend {
    /// 供 `LspService::new` 的构造闭包使用。
    pub fn new(client: Client) -> Self {
        Self {
            client,
            docs: Arc::new(DashMap::new()),
        }
    }

    /// 快照 -> 解析 -> 发布,全链路唯一入口。
    async fn lint_and_publish(&self, uri: Uri, version: Option<i32>) {
        let text = self.snapshot(&uri);
        let diagnostics = to_lsp_diagnostics(&text);
        self.client
            .publish_diagnostics(uri, diagnostics, version)
            .await;
    }

    /// 取文档快照(无文档时按空文本处理),锁内只做拷贝。
    fn snapshot(&self, uri: &Uri) -> String {
        self.docs
            .get(uri)
            .map(|rope| rope.to_string())
            .unwrap_or_default()
    }
}

impl LanguageServer for Backend {
    async fn initialize(
        &self,
        _: InitializeParams,
    ) -> tower_lsp_server::jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: server_capabilities(),
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        tracing::info!("huzi-lsp initialized");
    }

    async fn shutdown(&self) -> tower_lsp_server::jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = Some(params.text_document.version);
        self.docs
            .insert(uri.clone(), Rope::from_str(&params.text_document.text));
        self.lint_and_publish(uri, version).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = Some(params.text_document.version);
        if let Some(change) = params.content_changes.into_iter().last() {
            self.docs.insert(uri.clone(), Rope::from_str(&change.text));
        }
        self.lint_and_publish(uri, version).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.docs.remove(&uri);
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        self.lint_and_publish(uri, None).await;
    }

    async fn did_change_watched_files(&self, _params: DidChangeWatchedFilesParams) {
    }

    async fn hover(
        &self,
        params: HoverParams,
    ) -> tower_lsp_server::jsonrpc::Result<Option<Hover>> {
        let pos = params.text_document_position_params.position;
        let text =
            self.snapshot(&params.text_document_position_params.text_document.uri);
        Ok(hover_for_text(&text, pos))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> tower_lsp_server::jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let pos = params.text_document_position_params.position;
        let text = self.snapshot(&uri);
        if let Some(loc) = import_jump_location(&text, &uri, pos) {
            return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
        }
        if let Some(loc) = module_fn_location(&text, &uri, pos) {
            return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
        }
        Ok(definition_range_for_text(&text, pos).map(|range| {
            GotoDefinitionResponse::Scalar(Location { uri, range })
        }))
    }

    async fn references(
        &self,
        params: ReferenceParams,
    ) -> tower_lsp_server::jsonrpc::Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.clone();
        let pos = params.text_document_position.position;
        let text = self.snapshot(&uri);
        Ok(Some(references_for_text(&text, &uri, pos)))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> tower_lsp_server::jsonrpc::Result<Option<SemanticTokensResult>> {
        let text = self.snapshot(&params.text_document.uri);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens_for_text(&text),
        })))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> tower_lsp_server::jsonrpc::Result<Option<DocumentSymbolResponse>> {
        let text = self.snapshot(&params.text_document.uri);
        Ok(Some(DocumentSymbolResponse::Nested(symbols_for_text(&text))))
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> tower_lsp_server::jsonrpc::Result<Option<CompletionResponse>> {
        let pos = params.text_document_position.position;
        let text = self.snapshot(
            &params.text_document_position.text_document.uri,
        );
        Ok(Some(CompletionResponse::Array(completion_for_text(&text, pos))))
    }
}

/// 首版能力:FULL 全量同步 + 推送诊断 + hover/定义/Outline(同文件符号)。
/// 注:只走 push publishDiagnostics，不开 pull diagnostic，避免 VSCode 发
/// textDocument/diagnostic 拉取造成 Method not found。
fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        semantic_tokens_provider: Some(
            SemanticTokensServerCapabilities::SemanticTokensOptions(
                SemanticTokensOptions {
                    legend: legend(),
                    full: Some(SemanticTokensFullOptions::Bool(true)),
                    ..Default::default()
                },
            ),
        ),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![
                ".".to_string(),
                ":".to_string(),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 全文 -> LSP 诊断列表(纯函数,便于单测复用)。
fn to_lsp_diagnostics(text: &str) -> Vec<Diagnostic> {
    let rope = Rope::from_str(text);
    check_text(text)
        .into_iter()
        .map(|raw| raw_to_diagnostic(&rope, &raw))
        .collect()
}

/// 单条原始诊断 -> LSP [`Diagnostic`](Error 级别)。
fn raw_to_diagnostic(rope: &Rope, raw: &RawDiagnostic) -> Diagnostic {
    let range = huzi_range_to_lsp(rope, raw.0, raw.1, raw.2, raw.3);
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("huzi".to_string()),
        message: raw.4.clone(),
        ..Default::default()
    }
}

/// 全文 + 光标 -> hover(纯函数:建 Rope 快照,取词,查同名符号)。
///
/// 找词/解析/查符号任一步失败返回 `None`。
fn hover_for_text(text: &str, pos: Position) -> Option<Hover> {
    let rope = Rope::from_str(text);
    let (word, range) = word_at_position(&rope, pos)?;
    let (line, col) = lsp_pos_to_huzi(&rope, pos)?;
    let (_program, symbols) = parse_and_collect(text);
    let sym = find_symbol(&symbols, &word, line, col)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!(
                "```huzi\n{}\n```\n\n{:?} `{}`",
                sym.detail, sym.kind, sym.name
            ),
        }),
        range: Some(range),
    })
}

/// 全文 + 光标 -> 定义处 [`Range`](纯函数,调用方配 `uri` 组装 `Location`)。
///
/// 区间经 [`huzi_range_to_lsp`] 换算,永不倒置;失败返回 `None`。
fn definition_range_for_text(text: &str, pos: Position) -> Option<Range> {
    let rope = Rope::from_str(text);
    let (word, _) = word_at_position(&rope, pos)?;
    let (line, col) = lsp_pos_to_huzi(&rope, pos)?;
    let (_program, symbols) = parse_and_collect(text);
    let sym = find_symbol(&symbols, &word, line, col)?;
    Some(huzi_range_to_lsp(
        &rope,
        sym.span.line,
        sym.span.column,
        sym.span.end_line,
        sym.span.end_column,
    ))
}

/// 全文 -> Outline 符号表(纯函数;坏文件返回空表,不抛错)。
fn symbols_for_text(text: &str) -> Vec<DocumentSymbol> {
    let rope = Rope::from_str(text);
    let (_program, symbols) = parse_and_collect(text);
    symbols
        .iter()
        .filter_map(|sym| to_document_symbol(&rope, sym))
        .collect()
}

/// 单个 Huzi 符号 -> LSP [`DocumentSymbol`](`range` 与 `selectionRange` 同取语句区间)。
///
/// 经 `serde_json` 反序列化构造:`deprecated` 字段已废弃,直接写结构体
/// 字面量会引入废弃警告,此处不命名该字段(缺省为 `None`)。
fn to_document_symbol(rope: &Rope, sym: &Symbol) -> Option<DocumentSymbol> {
    let range = huzi_range_to_lsp(
        rope,
        sym.span.line,
        sym.span.column,
        sym.span.end_line,
        sym.span.end_column,
    );
    serde_json::from_value(serde_json::json!({
        "name": sym.name,
        "detail": sym.detail,
        "kind": to_symbol_kind(sym.kind),
        "range": range,
        "selectionRange": range,
    }))
    .ok()
}

/// Huzi 符号种类 -> LSP [`SymbolKind`]。
fn to_symbol_kind(kind: HuziSymbolKind) -> SymbolKind {
    match kind {
        HuziSymbolKind::Function => SymbolKind::FUNCTION,
        HuziSymbolKind::Struct => SymbolKind::STRUCT,
        HuziSymbolKind::Enum => SymbolKind::ENUM,
        HuziSymbolKind::Variant => SymbolKind::ENUM_MEMBER,
        HuziSymbolKind::Module => SymbolKind::MODULE,
        HuziSymbolKind::Trait => SymbolKind::INTERFACE,
        HuziSymbolKind::Variable | HuziSymbolKind::Param => SymbolKind::VARIABLE,
    }
}

/// 同名符号中优先取包含光标的最小区间;都不包含时退回首个同名符号。
fn find_symbol<'a>(
    symbols: &'a [Symbol],
    name: &str,
    line: usize,
    col: usize,
) -> Option<&'a Symbol> {
    let mut first: Option<&'a Symbol> = None;
    let mut best: Option<&'a Symbol> = None;
    let mut best_area = usize::MAX;
    for sym in symbols.iter().filter(|sym| sym.name == name) {
        if first.is_none() {
            first = Some(sym);
        }
        if span_contains(sym, line, col) {
            let area = span_area(sym);
            if area < best_area {
                best_area = area;
                best = Some(sym);
            }
        }
    }
    best.or(first)
}

/// 光标是否落在符号语句区间内(含边界)。
fn span_contains(sym: &Symbol, line: usize, col: usize) -> bool {
    let span = sym.span;
    (span.line, span.column) <= (line, col)
        && (line, col) <= (span.end_line, span.end_column)
}

/// 语句区间面积(行主序):同名嵌套时小区间优先。
fn span_area(sym: &Symbol) -> usize {
    let span = sym.span;
    span.end_line.saturating_sub(span.line) * 4096
        + span.end_column.saturating_sub(span.column)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACT: &str =
        "fn factorial(n: i32) -> i32 {\n    n\n}\nfn main() -> i32 {\n    factorial(1)\n}\n";

    #[test]
    fn hover_on_fn_name_shows_signature() {
        // Given: 递归函数定义的全文
        // When: hover 首行 fn 名(0-based 行 0,列 3)
        let hover = hover_for_text(FACT, Position { line: 0, character: 3 });
        // Then: 返回 markdown 签名(含 detail 与 kind)
        let hover = hover.expect("hover hits fn name");
        match hover.contents {
            HoverContents::Markup(markup) => {
                assert!(markup.value.contains("fn factorial"), "{markup:?}");
                assert!(markup.value.contains("Function"), "{markup:?}");
            }
            other => panic!("expected markup, got {other:?}"),
        }
    }

    #[test]
    fn hover_miss_returns_none() {
        // Given: 同上全文与空文件
        // When: 落在缩进空白 / 关键字 `fn` 无符号处 / 空文件
        // Then: 一律 None,不抛错
        assert!(hover_for_text(FACT, Position { line: 1, character: 0 }).is_none());
        assert!(hover_for_text(FACT, Position { line: 3, character: 0 }).is_none());
        assert!(hover_for_text("", Position { line: 0, character: 0 }).is_none());
        assert!(hover_for_text("let = \n", Position { line: 0, character: 0 }).is_none());
    }

    #[test]
    fn definition_range_is_never_inverted() {
        // Given: main 内调用 factorial 的全文
        // When: 在调用点(0-based 行 4,列 5)请求定义
        let range = definition_range_for_text(FACT, Position { line: 4, character: 5 })
            .expect("definition found");
        // Then: 区间非倒置,且落在首行 fn 定义处
        assert!(
            (range.end.line, range.end.character)
                >= (range.start.line, range.start.character),
            "{range:?}"
        );
        assert_eq!(range.start.line, 0);
    }

    #[test]
    fn document_symbol_lists_outline() {
        // Given: 含 fn 与顶层 let 的全文
        let text = "fn factorial(n: i32) -> i32 {\n    n\n}\nlet x = 1\n";
        // When: 取 Outline
        let syms = symbols_for_text(text);
        // Then: 非空,fn 映射为 Function,变量映射为 Variable
        assert!(!syms.is_empty());
        let fun = syms.iter().find(|s| s.name == "factorial").expect("fn symbol");
        assert_eq!(fun.kind, SymbolKind::FUNCTION);
        assert!(fun.detail.as_deref().unwrap_or_default().contains("fn factorial"));
        let var = syms.iter().find(|s| s.name == "x").expect("let symbol");
        assert_eq!(var.kind, SymbolKind::VARIABLE);
        for s in &syms {
            assert!(
                (s.range.end.line, s.range.end.character)
                    >= (s.range.start.line, s.range.start.character),
                "inverted range for {}",
                s.name
            );
        }
    }

    #[test]
    fn hover_after_chinese_line_stays_aligned() {
        // Given: 首行为中文注释,次行定义函数
        let text = "# 递归函数示例中文行\nfn factorial(n: i32) -> i32 {\n    n\n}\n";
        // When: hover 次行 fn 名(0-based 行 1,列 3)
        let hover = hover_for_text(text, Position { line: 1, character: 3 });
        // Then: 命中签名,不因中文行错位
        let hover = hover.expect("hover aligned after chinese line");
        match hover.contents {
            HoverContents::Markup(markup) => {
                assert!(markup.value.contains("fn factorial"), "{markup:?}");
            }
            other => panic!("expected markup, got {other:?}"),
        }
        // 同行中文标识符同样可取词命中
        let text2 = "let 变量 = 1\n变量\n";
        let hover2 = hover_for_text(text2, Position { line: 1, character: 1 })
            .expect("chinese ident hover");
        match hover2.contents {
            HoverContents::Markup(markup) => {
                assert!(markup.value.contains("变量"), "{markup:?}");
            }
            other => panic!("expected markup, got {other:?}"),
        }
    }
}
