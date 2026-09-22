//! huzi-lsp:Huzi 语言服务器(FULL 全量诊断版,stdio 传输)。
//!
//! stdout 由 LSP 传输独占,日志只走 stderr。

mod analysis;
mod backend;
mod completion;
mod imports;
mod mapping;
mod references;
mod semantic;

use tower_lsp_server::{LspService, Server};

use backend::Backend;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
