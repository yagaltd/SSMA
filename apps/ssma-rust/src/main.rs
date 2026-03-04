use ssma_rust::gateway;
use ssma_rust::runtime::Config;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let config = Config::from_env();
    tracing::info!(subprotocol = %config.subprotocol, "ssma-rust boot");
    if let Err(error) = gateway::run(config).await {
        tracing::error!(%error, "ssma-rust failed");
    }
}
