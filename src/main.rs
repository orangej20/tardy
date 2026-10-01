use std::sync::Arc;
use tardy::ads::{HttpX402Facilitator, PaymentRequirements, PgAdsStore};
use tardy::api::AdsRuntime;
use tardy::push::PgPushStore;
use tardy::subscriptions::PgSubscriptionStore;
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
    let mut state = AppState::with_account_db(public_base_url, database_path)?;
    if let Ok(database_url) = std::env::var("DATABASE_URL") {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect(&database_url)
            .await?;
        state = state.with_push_store(PgPushStore::new(pool.clone()));
        let subscription_base_url = state.public_base_url.clone();
        state = state.with_subscriptions(PgSubscriptionStore::new(
            pool.clone(),
            subscription_base_url,
            std::env::var("WEBHOOK_SIGNING_KEY")
                .ok()
                .map(String::into_bytes),
        ));
        if let Ok(facilitator_url) = std::env::var("X402_FACILITATOR_URL") {
            let processor = HttpX402Facilitator::new(
                facilitator_url,
                std::env::var("X402_FACILITATOR_BEARER_TOKEN").ok(),
            )?;
            let atomic_per_budget_micro = required("X402_ATOMIC_PER_BUDGET_MICRO")?.parse()?;
            state = state.with_ads(AdsRuntime {
                store: PgAdsStore::new(pool.clone()),
                processor: Arc::new(processor),
                requirement: PaymentRequirements {
                    scheme: std::env::var("X402_SCHEME").unwrap_or_else(|_| "exact".into()),
                    network: required("X402_NETWORK")?,
                    amount: String::new(),
                    asset: required("X402_ASSET")?,
                    pay_to: required("X402_PAY_TO")?,
                    max_timeout_seconds: 300,
                    extra: serde_json::Map::new(),
                },
                atomic_per_budget_micro,
            });
        }
    }
    axum::serve(listener, router(Arc::new(state)))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| format!("{name} is required").into())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
