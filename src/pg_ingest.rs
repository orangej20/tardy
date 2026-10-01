use crate::ingest::{NormalizedItem, SourceDefinition, TransformPlan};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum PgIngestError {
    #[error("PostgreSQL ingestion store: {0}")]
    Database(#[from] sqlx::Error),
    #[error("PostgreSQL migration: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("invalid persisted source configuration: {0}")]
    Configuration(#[from] serde_json::Error),
    #[error("timestamp is outside the supported range")]
    Timestamp,
    #[error("source item limit is outside the PostgreSQL integer range")]
    ItemLimit,
}

#[derive(Debug, Clone)]
pub struct ClaimedSource {
    pub definition: SourceDefinition,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEvent {
    pub id: Uuid,
    pub topic: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub payload: serde_json::Value,
    pub attempts: i32,
}

#[derive(Clone)]
pub struct PgIngestStore {
    pool: PgPool,
}

impl PgIngestStore {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, PgIngestError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<(), PgIngestError> {
        sqlx::migrate!().run(&self.pool).await?;
        Ok(())
    }

    pub async fn sync_sources(&self, sources: &[SourceDefinition]) -> Result<(), PgIngestError> {
        for source in sources {
            sqlx::query(
                "INSERT INTO source_channels
                   (id, display_name, enabled, transport, rights_policy, transform_plugin, item_limit)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (id) DO UPDATE SET
                   display_name = excluded.display_name,
                   enabled = excluded.enabled,
                   transport = excluded.transport,
                   rights_policy = excluded.rights_policy,
                   transform_plugin = excluded.transform_plugin,
                   item_limit = excluded.item_limit,
                   updated_at = now()",
            )
            .bind(&source.id)
            .bind(&source.display_name)
            .bind(source.enabled)
            .bind(serde_json::to_value(&source.transport)?)
            .bind(serde_json::to_value(&source.rights)?)
            .bind(&source.transform)
            .bind(i32::try_from(source.limit).map_err(|_| PgIngestError::ItemLimit)?)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn claim_due_source(
        &self,
        worker: &str,
        lease_seconds: i64,
    ) -> Result<Option<ClaimedSource>, PgIngestError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT id, display_name, enabled, transport, rights_policy, transform_plugin,
                    item_limit, etag, last_modified
             FROM source_channels
             WHERE enabled AND next_poll_at <= now()
               AND (lease_until IS NULL OR lease_until < now())
             ORDER BY next_poll_at, id
             FOR UPDATE SKIP LOCKED
             LIMIT 1",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.commit().await?;
            return Ok(None);
        };
        let id: String = row.try_get("id")?;
        sqlx::query(
            "UPDATE source_channels
             SET lease_owner = $1, lease_until = now() + make_interval(secs => $2::double precision), updated_at = now()
             WHERE id = $3",
        )
        .bind(worker)
        .bind(lease_seconds)
        .bind(&id)
        .execute(&mut *transaction)
        .await?;
        let definition = SourceDefinition {
            id,
            display_name: row.try_get("display_name")?,
            enabled: row.try_get("enabled")?,
            limit: usize::try_from(row.try_get::<i32, _>("item_limit")?)
                .map_err(|_| PgIngestError::Timestamp)?,
            transport: serde_json::from_value(row.try_get("transport")?)?,
            rights: serde_json::from_value(row.try_get("rights_policy")?)?,
            transform: row.try_get("transform_plugin")?,
        };
        let claimed = ClaimedSource {
            definition,
            etag: row.try_get("etag")?,
            last_modified: row.try_get("last_modified")?,
        };
        transaction.commit().await?;
        Ok(Some(claimed))
    }

    pub async fn record_success(
        &self,
        worker: &str,
        source: &SourceDefinition,
        items: &[(NormalizedItem, TransformPlan)],
        etag: Option<&str>,
        last_modified: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<usize, PgIngestError> {
        let mut transaction = self.pool.begin().await?;
        let mut inserted = 0;
        for (item, plan) in items {
            let item_id = Uuid::new_v4();
            let serialized = serde_json::to_vec(item)?;
            let published = item.published_at_ms.map(timestamp).transpose()?;
            let created = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO source_items
                   (id, source_id, external_id, canonical_url, title, author, source_published_at,
                    summary, facts, content_digest, first_seen_at, last_seen_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11)
                 ON CONFLICT (source_id, external_id) DO UPDATE SET last_seen_at = excluded.last_seen_at
                 RETURNING id",
            )
            .bind(item_id)
            .bind(&source.id)
            .bind(&item.external_id)
            .bind(&item.canonical_url)
            .bind(&item.title)
            .bind(&item.author)
            .bind(published)
            .bind(&item.summary)
            .bind(serde_json::to_value(&item.facts)?)
            .bind(Sha256::digest(serialized).to_vec())
            .bind(now)
            .fetch_one(&mut *transaction)
            .await?;
            if created != item_id {
                continue;
            }
            inserted += 1;
            let run_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO transformation_runs
                   (id, source_item_id, plugin, status, plan, available_at, created_at)
                 VALUES ($1,$2,$3,'planned',$4,$5,$5)",
            )
            .bind(run_id)
            .bind(item_id)
            .bind(&source.transform)
            .bind(serde_json::to_value(plan)?)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO outbox
                   (id, topic, aggregate_type, aggregate_id, payload, available_at, created_at)
                 VALUES ($1,'source.item_ingested.v1','source_item',$2,$3,$4,$4)",
            )
            .bind(Uuid::new_v4())
            .bind(item_id.to_string())
            .bind(serde_json::json!({"source_id": source.id, "source_item_id": item_id, "run_id": run_id}))
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        let result = sqlx::query(
            "UPDATE source_channels SET
               etag = COALESCE($1, etag), last_modified = COALESCE($2, last_modified),
               consecutive_failures = 0, last_error = NULL,
               next_poll_at = now() + make_interval(secs => poll_interval_seconds),
               lease_owner = NULL, lease_until = NULL, updated_at = now()
             WHERE id = $3 AND lease_owner = $4",
        )
        .bind(etag)
        .bind(last_modified)
        .bind(&source.id)
        .bind(worker)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(PgIngestError::Database(sqlx::Error::RowNotFound));
        }
        transaction.commit().await?;
        Ok(inserted)
    }

    pub async fn record_failure(
        &self,
        worker: &str,
        source_id: &str,
        error: &str,
    ) -> Result<(), PgIngestError> {
        sqlx::query(
            "UPDATE source_channels SET
               consecutive_failures = consecutive_failures + 1,
               last_error = left($1, 2000),
               next_poll_at = now() + make_interval(secs => LEAST(3600, 30 * power(2, LEAST(consecutive_failures, 6)))::double precision),
               lease_owner = NULL, lease_until = NULL, updated_at = now()
             WHERE id = $2 AND lease_owner = $3",
        )
        .bind(error)
        .bind(source_id)
        .bind(worker)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn claim_outbox(
        &self,
        worker: &str,
        limit: i64,
    ) -> Result<Vec<OutboxEvent>, PgIngestError> {
        let rows = sqlx::query(
            "WITH claimed AS (
               SELECT id FROM outbox
               WHERE delivered_at IS NULL AND failed_at IS NULL AND available_at <= now()
                 AND (lease_until IS NULL OR lease_until < now())
               ORDER BY available_at, id FOR UPDATE SKIP LOCKED LIMIT $1
             )
             UPDATE outbox o SET lease_owner=$2, lease_until=now()+interval '60 seconds', attempts=o.attempts+1
             FROM claimed WHERE o.id=claimed.id
             RETURNING o.id,o.topic,o.aggregate_type,o.aggregate_id,o.payload,o.attempts",
        )
        .bind(limit)
        .bind(worker)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(OutboxEvent {
                    id: row.try_get("id")?,
                    topic: row.try_get("topic")?,
                    aggregate_type: row.try_get("aggregate_type")?,
                    aggregate_id: row.try_get("aggregate_id")?,
                    payload: row.try_get("payload")?,
                    attempts: row.try_get("attempts")?,
                })
            })
            .collect()
    }

    pub async fn claim_outbox_topic(
        &self,
        worker: &str,
        topic: &str,
        limit: i64,
    ) -> Result<Vec<OutboxEvent>, PgIngestError> {
        let rows = sqlx::query(
            "WITH claimed AS (
               SELECT id FROM outbox
               WHERE topic=$1 AND delivered_at IS NULL AND failed_at IS NULL AND available_at<=now()
                 AND (lease_until IS NULL OR lease_until<now())
               ORDER BY available_at,id FOR UPDATE SKIP LOCKED LIMIT $2
             )
             UPDATE outbox o SET lease_owner=$3,lease_until=now()+interval '60 seconds',attempts=o.attempts+1
             FROM claimed WHERE o.id=claimed.id
             RETURNING o.id,o.topic,o.aggregate_type,o.aggregate_id,o.payload,o.attempts",
        )
        .bind(topic)
        .bind(limit)
        .bind(worker)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(OutboxEvent {
                    id: row.try_get("id")?,
                    topic: row.try_get("topic")?,
                    aggregate_type: row.try_get("aggregate_type")?,
                    aggregate_id: row.try_get("aggregate_id")?,
                    payload: row.try_get("payload")?,
                    attempts: row.try_get("attempts")?,
                })
            })
            .collect()
    }

    pub async fn mark_outbox_delivered(
        &self,
        worker: &str,
        event_id: Uuid,
    ) -> Result<(), PgIngestError> {
        require_one(
            sqlx::query(
                "UPDATE outbox SET delivered_at=now(), lease_owner=NULL, lease_until=NULL, last_error=NULL
                 WHERE id=$1 AND delivered_at IS NULL AND lease_owner=$2",
            )
            .bind(event_id)
            .bind(worker)
            .execute(&self.pool)
            .await?
            .rows_affected(),
        )
    }

    pub async fn reschedule_outbox(
        &self,
        worker: &str,
        event_id: Uuid,
        error: &str,
        delay_seconds: i64,
    ) -> Result<(), PgIngestError> {
        require_one(
            sqlx::query(
                "UPDATE outbox SET available_at=now()+make_interval(secs => $1::double precision),
                   lease_owner=NULL, lease_until=NULL, last_error=left($2, 2000)
                 WHERE id=$3 AND delivered_at IS NULL AND lease_owner=$4",
            )
            .bind(delay_seconds)
            .bind(error)
            .bind(event_id)
            .bind(worker)
            .execute(&self.pool)
            .await?
            .rows_affected(),
        )
    }

    pub async fn fail_outbox(
        &self,
        worker: &str,
        event_id: Uuid,
        error: &str,
    ) -> Result<(), PgIngestError> {
        require_one(
            sqlx::query(
                "UPDATE outbox SET failed_at=now(),lease_owner=NULL,lease_until=NULL,last_error=left($1,2000)
                 WHERE id=$2 AND delivered_at IS NULL AND failed_at IS NULL AND lease_owner=$3",
            )
            .bind(error)
            .bind(event_id)
            .bind(worker)
            .execute(&self.pool)
            .await?
            .rows_affected(),
        )
    }
}

fn require_one(rows_affected: u64) -> Result<(), PgIngestError> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(PgIngestError::Database(sqlx::Error::RowNotFound))
    }
}

fn timestamp(value: u64) -> Result<DateTime<Utc>, PgIngestError> {
    let value = i64::try_from(value).map_err(|_| PgIngestError::Timestamp)?;
    DateTime::from_timestamp_millis(value).ok_or(PgIngestError::Timestamp)
}
