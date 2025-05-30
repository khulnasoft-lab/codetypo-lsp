//! Binary entry point for Codetypo-LSP. Initializes tracing and runs the LSP server.

use codetypo_lsp::lsp;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(lsp::Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
