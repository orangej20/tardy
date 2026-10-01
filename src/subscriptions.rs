use crate::domain::{HyperTardyItem, Reel};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::{PgPool, Row};
use std::collections::BTreeSet;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionKind {
    Hashtag,
    HyperTardy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Poll,
    Webhook,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct NewSubscription {
    pub kind: SubscriptionKind,
    pub hashtag: Option<String>,
    pub delivery: DeliveryMode,
    pub webhook_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Subscription {
    pub id: Uuid,
    pub kind: SubscriptionKind,
    pub hashtag: Option<String>,
    pub delivery: DeliveryMode,
    pub webhook_url: Option<String>,
    pub poll_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeedEvent {
    pub id: i64,
    pub kind: String,
    pub reel_id: Uuid,
    pub hashtags: Vec<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ClaimedWebhook {
    pub delivery_id: Uuid,
    pub subscription_id: Uuid,
    pub event_id: i64,
    pub url: String,
    pub event: FeedEvent,
    pub attempts: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum SubscriptionError {
    #[error("invalid hashtag, subscription, or webhook URL")]
    Invalid,
    #[error("webhook signing is not configured")]
    SigningUnavailable,
    #[error("subscription not found")]
    NotFound,
    #[error("subscription database: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct PgSubscriptionStore {
    pool: PgPool,
    public_base_url: String,
    signing_key: Option<Vec<u8>>,
}

impl PgSubscriptionStore {
    pub fn new(pool: PgPool, public_base_url: String, signing_key: Option<Vec<u8>>) -> Self {
        Self {
            pool,
            public_base_url,
            signing_key,
        }
    }

    pub async fn create(
        &self,
        account_id: Uuid,
        mut input: NewSubscription,
    ) -> Result<Subscription, SubscriptionError> {
        input.hashtag = input
            .hashtag
            .map(|value| normalize_hashtag(&value))
            .transpose()?;
        if matches!(input.kind, SubscriptionKind::Hashtag) != input.hashtag.is_some() {
            return Err(SubscriptionError::Invalid);
        }
        if matches!(input.delivery, DeliveryMode::Webhook) {
            let url = input
                .webhook_url
                .as_deref()
                .ok_or(SubscriptionError::Invalid)?;
            validate_webhook_url(url)?;
            if self.signing_key.is_none() {
                return Err(SubscriptionError::SigningUnavailable);
            }
        } else if input.webhook_url.is_some() {
            return Err(SubscriptionError::Invalid);
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO feed_subscriptions (id,account_id,kind,hashtag,delivery,webhook_url) VALUES ($1,$2,$3,$4,$5,$6)")
            .bind(id).bind(account_id).bind(kind_name(input.kind)).bind(&input.hashtag)
            .bind(delivery_name(input.delivery)).bind(&input.webhook_url).execute(&self.pool).await?;
        Ok(Subscription {
            id,
            kind: input.kind,
            hashtag: input.hashtag,
            delivery: input.delivery,
            webhook_url: input.webhook_url,
            poll_url: format!("{}/v1/feed-subscriptions/{id}/events", self.public_base_url),
            webhook_secret: matches!(input.delivery, DeliveryMode::Webhook)
                .then(|| self.secret(id))
                .transpose()?,
        })
    }

    pub async fn delete(&self, account_id: Uuid, id: Uuid) -> Result<(), SubscriptionError> {
        let result = sqlx::query(
            "UPDATE feed_subscriptions SET active=false WHERE id=$1 AND account_id=$2 AND active",
        )
        .bind(id)
        .bind(account_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            Err(SubscriptionError::NotFound)
        }
    }

    pub async fn poll(
        &self,
        account_id: Uuid,
        id: Uuid,
        after: i64,
        limit: i64,
    ) -> Result<Vec<FeedEvent>, SubscriptionError> {
        let rows = sqlx::query("SELECT e.id,e.kind,e.reel_id,e.hashtags,e.payload FROM feed_subscriptions s JOIN feed_events e ON ((s.kind='hyper_tardy' AND e.kind='hyper_tardy') OR (s.kind='hashtag' AND e.kind='post_published' AND s.hashtag=ANY(e.hashtags))) WHERE s.id=$1 AND s.account_id=$2 AND s.active AND e.id>$3 ORDER BY e.id LIMIT $4")
            .bind(id).bind(account_id).bind(after).bind(limit).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| event_from_row(&row))
            .collect::<Result<_, _>>()?)
    }

    pub async fn publish_reel(&self, reel: &Reel) -> Result<(), SubscriptionError> {
        let tags = extract_hashtags(&reel.caption);
        self.emit(
            "post_published",
            reel.id,
            tags,
            serde_json::to_value(reel).map_err(|_| SubscriptionError::Invalid)?,
        )
        .await
    }

    pub async fn publish_hyper_tardy(
        &self,
        item: &HyperTardyItem,
    ) -> Result<(), SubscriptionError> {
        self.emit(
            "hyper_tardy",
            item.reel.id,
            extract_hashtags(&item.reel.caption),
            serde_json::to_value(item).map_err(|_| SubscriptionError::Invalid)?,
        )
        .await
    }

    async fn emit(
        &self,
        kind: &str,
        reel_id: Uuid,
        hashtags: Vec<String>,
        payload: serde_json::Value,
    ) -> Result<(), SubscriptionError> {
        let mut tx = self.pool.begin().await?;
        let event_id = sqlx::query_scalar::<_,i64>("INSERT INTO feed_events (kind,reel_id,hashtags,payload) VALUES ($1,$2,$3,$4) ON CONFLICT (kind,reel_id) DO NOTHING RETURNING id")
            .bind(kind).bind(reel_id).bind(&hashtags).bind(payload).fetch_optional(&mut *tx).await?;
        if let Some(event_id) = event_id {
            sqlx::query("INSERT INTO webhook_deliveries (id,subscription_id,event_id) SELECT gen_random_uuid(),s.id,$1 FROM feed_subscriptions s JOIN feed_events e ON e.id=$1 WHERE s.active AND s.delivery='webhook' AND ((s.kind='hyper_tardy' AND e.kind='hyper_tardy') OR (s.kind='hashtag' AND e.kind='post_published' AND s.hashtag=ANY(e.hashtags))) ON CONFLICT DO NOTHING")
                .bind(event_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn claim_webhooks(
        &self,
        worker: &str,
        limit: i64,
    ) -> Result<Vec<ClaimedWebhook>, SubscriptionError> {
        let rows=sqlx::query("WITH claimed AS (SELECT id FROM webhook_deliveries WHERE status IN ('queued','retry') AND available_at<=now() AND (lease_until IS NULL OR lease_until<now()) ORDER BY available_at,id FOR UPDATE SKIP LOCKED LIMIT $1) UPDATE webhook_deliveries d SET status='sending',lease_owner=$2,lease_until=now()+interval '60 seconds',attempts=d.attempts+1 FROM claimed c,feed_subscriptions s,feed_events e WHERE d.id=c.id AND s.id=d.subscription_id AND e.id=d.event_id RETURNING d.id,d.subscription_id,d.event_id,s.webhook_url,e.kind,e.reel_id,e.hashtags,e.payload,d.attempts")
            .bind(limit).bind(worker).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(ClaimedWebhook {
                    delivery_id: row.try_get("id")?,
                    subscription_id: row.try_get("subscription_id")?,
                    event_id: row.try_get("event_id")?,
                    url: row.try_get("webhook_url")?,
                    attempts: row.try_get("attempts")?,
                    event: FeedEvent {
                        id: row.try_get("event_id")?,
                        kind: row.try_get("kind")?,
                        reel_id: row.try_get("reel_id")?,
                        hashtags: row.try_get("hashtags")?,
                        payload: row.try_get("payload")?,
                    },
                })
            })
            .collect()
    }

    pub async fn finish_webhook(
        &self,
        worker: &str,
        delivery: &ClaimedWebhook,
        status: Option<u16>,
        error: Option<&str>,
    ) -> Result<(), SubscriptionError> {
        let success = status.is_some_and(|value| (200..300).contains(&value));
        let permanent = success
            || delivery.attempts >= 10
            || status
                .is_some_and(|value| (400..500).contains(&value) && value != 408 && value != 429);
        sqlx::query("UPDATE webhook_deliveries SET status=CASE WHEN $1 THEN 'delivered' WHEN $2 THEN 'failed' ELSE 'retry' END, delivered_at=CASE WHEN $1 THEN now() ELSE NULL END, available_at=CASE WHEN $1 OR $2 THEN available_at ELSE now()+make_interval(secs=>LEAST(3600,power(2,LEAST(attempts,10)))::double precision) END,response_status=$3,last_error=left($4,1000),lease_owner=NULL,lease_until=NULL WHERE id=$5 AND lease_owner=$6 AND status='sending'")
            .bind(success).bind(permanent && !success).bind(status.map(i32::from)).bind(error).bind(delivery.delivery_id).bind(worker).execute(&self.pool).await?;
        Ok(())
    }

    pub fn sign(&self, subscription_id: Uuid, body: &[u8]) -> Result<String, SubscriptionError> {
        let secret = self.secret_bytes(subscription_id)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret)
            .map_err(|_| SubscriptionError::SigningUnavailable)?;
        mac.update(body);
        Ok(format!(
            "sha256={}",
            hex(mac.finalize().into_bytes().as_slice())
        ))
    }
    fn secret(&self, id: Uuid) -> Result<String, SubscriptionError> {
        use base64::Engine as _;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.secret_bytes(id)?))
    }
    fn secret_bytes(&self, id: Uuid) -> Result<Vec<u8>, SubscriptionError> {
        let key = self
            .signing_key
            .as_ref()
            .ok_or(SubscriptionError::SigningUnavailable)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(key)
            .map_err(|_| SubscriptionError::SigningUnavailable)?;
        mac.update(id.as_bytes());
        Ok(mac.finalize().into_bytes().to_vec())
    }
}

pub fn extract_hashtags(caption: &str) -> Vec<String> {
    caption
        .split_whitespace()
        .filter_map(|word| word.strip_prefix('#'))
        .filter_map(|tag| normalize_hashtag(tag).ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(20)
        .collect()
}
fn normalize_hashtag(value: &str) -> Result<String, SubscriptionError> {
    let value = value.trim_start_matches('#').to_ascii_lowercase();
    if (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        Ok(value)
    } else {
        Err(SubscriptionError::Invalid)
    }
}
fn kind_name(value: SubscriptionKind) -> &'static str {
    match value {
        SubscriptionKind::Hashtag => "hashtag",
        SubscriptionKind::HyperTardy => "hyper_tardy",
    }
}
fn delivery_name(value: DeliveryMode) -> &'static str {
    match value {
        DeliveryMode::Poll => "poll",
        DeliveryMode::Webhook => "webhook",
    }
}
fn validate_webhook_url(value: &str) -> Result<(), SubscriptionError> {
    let url = url::Url::parse(value).map_err(|_| SubscriptionError::Invalid)?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(SubscriptionError::Invalid);
    }
    match url.host() {
        Some(url::Host::Domain(host)) if host != "localhost" && !host.ends_with(".local") => Ok(()),
        Some(url::Host::Ipv4(ip))
            if !(ip.is_private() || ip.is_loopback() || ip.is_link_local()) =>
        {
            Ok(())
        }
        Some(url::Host::Ipv6(ip)) if !(ip.is_loopback() || ip.is_unspecified()) => Ok(()),
        _ => Err(SubscriptionError::Invalid),
    }
}
fn event_from_row(row: &sqlx::postgres::PgRow) -> Result<FeedEvent, sqlx::Error> {
    Ok(FeedEvent {
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        reel_id: row.try_get("reel_id")?,
        hashtags: row.try_get("hashtags")?,
        payload: row.try_get("payload")?,
    })
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hashtags_are_normalized_deduplicated_and_bounded() {
        assert_eq!(
            extract_hashtags("#Rust #rust #AI! #agents_are_here"),
            vec!["agents_are_here", "rust"]
        );
    }
}
