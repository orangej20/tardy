use std::time::Duration;
use tardy::pg_ingest::PgIngestStore;
use tardy::push::{
    Alert, AlertPayload, ApnsClient, ApnsConfig, ApnsEnvironment, Aps, ClaimedDelivery,
    NotificationRequest, PgPushStore, PushError,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let database_url = required("DATABASE_URL")?;
    let environment = match required("APNS_ENVIRONMENT")?.as_str() {
        "sandbox" => ApnsEnvironment::Sandbox,
        "production" => ApnsEnvironment::Production,
        _ => return Err("APNS_ENVIRONMENT must be sandbox or production".into()),
    };
    let topic = required("APNS_TOPIC")?;
    let client = ApnsClient::new(ApnsConfig {
        key_id: required("APNS_KEY_ID")?,
        team_id: required("APNS_TEAM_ID")?,
        topic: topic.clone(),
        private_key_pem: required("APNS_PRIVATE_KEY_PEM")?.replace("\\n", "\n"),
        environment,
    })?;
    let store = PgPushStore::connect(&database_url, 4).await?;
    let outbox = PgIngestStore::connect(&database_url, 2).await?;
    let worker = format!(
        "{}:{}",
        std::env::var("HOSTNAME").unwrap_or_else(|_| "tardy-push".into()),
        Uuid::new_v4()
    );

    loop {
        materialize_notifications(&outbox, &store, &worker).await?;
        let deliveries = store
            .claim_deliveries(&worker, environment, &topic, 100)
            .await?;
        if deliveries.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        for delivery in deliveries {
            deliver(&store, &client, &worker, delivery).await?;
        }
    }
}

async fn materialize_notifications(
    outbox: &PgIngestStore,
    push: &PgPushStore,
    worker: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for event in outbox
        .claim_outbox_topic(worker, "notification.requested.v1", 100)
        .await?
    {
        match serde_json::from_value::<NotificationRequest>(event.payload) {
            Ok(request) => {
                push.enqueue(request.from_event(event.id)).await?;
                outbox.mark_outbox_delivered(worker, event.id).await?;
            }
            Err(error) => {
                outbox
                    .fail_outbox(worker, event.id, &error.to_string())
                    .await?;
                tracing::error!(event_id=%event.id, %error, "notification outbox payload moved to failed state");
            }
        }
    }
    Ok(())
}

async fn deliver(
    store: &PgPushStore,
    client: &ApnsClient,
    worker: &str,
    delivery: ClaimedDelivery,
) -> Result<(), PushError> {
    let mut data = delivery.data.clone();
    if let Some(deep_link) = &delivery.deep_link {
        data.insert("deep_link".into(), deep_link.clone().into());
    }
    let payload = AlertPayload {
        aps: Aps {
            alert: Alert {
                title: delivery.title.clone(),
                body: delivery.body.clone(),
            },
            sound: Some("default".into()),
        },
        data,
    };
    match client
        .send_alert(
            &delivery.token,
            &payload,
            Some(&delivery.notification_id.to_string()),
        )
        .await
    {
        Ok(receipt) => {
            let apns_id = receipt
                .apns_id
                .as_deref()
                .and_then(|value| Uuid::parse_str(value).ok());
            store.mark_delivered(worker, delivery.id, apns_id).await
        }
        Err(PushError::Rejected { status, reason }) => {
            let permanent = status == 400 || status == 410;
            tracing::warn!(delivery_id=%delivery.id, status, %reason, permanent, "APNs rejected notification");
            store
                .mark_failed(worker, &delivery, Some(status), &reason, permanent)
                .await
        }
        Err(error) => {
            tracing::warn!(delivery_id=%delivery.id, %error, "APNs transport failed; rescheduling");
            store
                .mark_failed(worker, &delivery, None, &error.to_string(), false)
                .await
        }
    }
}

fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| format!("{name} is required").into())
}
