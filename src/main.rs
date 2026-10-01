use std::sync::Arc;
use tardy::{AppState, router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let bind = std::env::var("TARDY_BIND").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let public_base_url =
        std::env::var("TARDY_PUBLIC_BASE_URL").unwrap_or_else(|_| format!("http://{bind}"));
    let database_path = std::env::var("TARDY_DB_PATH").unwrap_or_else(|_| "tardy.sqlite".into());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, %public_base_url, "tardy listening");
    axum::serve(
        listener,
        router(Arc::new(AppState::with_account_db(
            public_base_url,
            database_path,
        )?)),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
