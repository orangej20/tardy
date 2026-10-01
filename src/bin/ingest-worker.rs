use chrono::Utc;
use std::time::Duration;
use tardy::ingest::{FetchCursor, Ingestor};
use tardy::pg_ingest::PgIngestStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let database_url = std::env::var("DATABASE_URL")?;
    let store = PgIngestStore::connect(&database_url, 4).await?;
    if std::env::args().nth(1).as_deref() == Some("migrate") {
        store.migrate().await?;
        return Ok(());
    }
    let ingestor = Ingestor::bundled()?;
    store.sync_sources(&ingestor.sources()?).await?;
    let worker = format!(
        "{}:{}",
        std::env::var("HOSTNAME").unwrap_or_else(|_| "tardy".into()),
        uuid::Uuid::new_v4()
    );
    loop {
        let Some(claimed) = store.claim_due_source(&worker, 60).await? else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let source = claimed.definition;
        let result = ingestor
            .fetch_source_conditional(
                &source,
                &FetchCursor {
                    etag: claimed.etag,
                    last_modified: claimed.last_modified,
                },
            )
            .await;
        match result {
            Ok(batch) => {
                let planned = batch
                    .items
                    .into_iter()
                    .map(|item| ingestor.transform(&source, &item).map(|plan| (item, plan)))
                    .collect::<Result<Vec<_>, _>>();
                match planned {
                    Ok(planned) => {
                        let inserted = store
                            .record_success(
                                &worker,
                                &source,
                                &planned,
                                batch.etag.as_deref(),
                                batch.last_modified.as_deref(),
                                Utc::now(),
                            )
                            .await?;
                        tracing::info!(source_id = %source.id, inserted, not_modified = batch.not_modified, "source poll completed");
                    }
                    Err(error) => {
                        store
                            .record_failure(&worker, &source.id, &error.to_string())
                            .await?;
                        tracing::error!(source_id = %source.id, %error, "source transformation failed");
                    }
                }
            }
            Err(error) => {
                store
                    .record_failure(&worker, &source.id, &error.to_string())
                    .await?;
                tracing::error!(source_id = %source.id, %error, "source poll failed");
            }
        }
    }
}
