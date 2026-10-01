use std::time::Duration;
use tardy::subscriptions::PgSubscriptionStore;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let database_url = required("DATABASE_URL")?;
    let key = required("WEBHOOK_SIGNING_KEY")?.into_bytes();
    let store = PgSubscriptionStore::new(
        sqlx::PgPool::connect(&database_url).await?,
        String::new(),
        Some(key),
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let worker = format!(
        "{}:{}",
        std::env::var("HOSTNAME").unwrap_or_else(|_| "tardy-webhook".into()),
        Uuid::new_v4()
    );
    loop {
        let deliveries = store.claim_webhooks(&worker, 100).await?;
        if deliveries.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        for delivery in deliveries {
            let body = serde_json::to_vec(&delivery.event)?;
            let signature = store.sign(delivery.subscription_id, &body)?;
            let result = client
                .post(&delivery.url)
                .header("content-type", "application/json")
                .header("x-tardy-delivery", delivery.delivery_id.to_string())
                .header("x-tardy-event", delivery.event_id.to_string())
                .header("x-tardy-signature", signature)
                .body(body)
                .send()
                .await;
            match result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    store
                        .finish_webhook(
                            &worker,
                            &delivery,
                            Some(status),
                            (!response.status().is_success())
                                .then_some("non-success webhook response"),
                        )
                        .await?;
                }
                Err(error) => {
                    store
                        .finish_webhook(&worker, &delivery, None, Some(&error.to_string()))
                        .await?
                }
            }
        }
    }
}
fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| format!("{name} is required").into())
}
