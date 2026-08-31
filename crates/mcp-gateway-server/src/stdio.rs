use crate::handler::GatewayHandler;
use rmcp::service::ServiceExt;
use rmcp::transport::io::stdio;

/// Serve JSON-RPC on stdio. All human-readable logs must go to stderr.
pub async fn serve_stdio(handler: GatewayHandler) -> Result<(), std::io::Error> {
    let running = handler
        .serve(stdio())
        .await
        .map_err(|e| std::io::Error::other(format!("stdio transport: {e}")))?;
    running
        .waiting()
        .await
        .map_err(|e| std::io::Error::other(format!("stdio session: {e}")))?;
    Ok(())
}
