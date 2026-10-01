use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

const APNS_PRODUCTION: &str = "https://api.push.apple.com";
const APNS_SANDBOX: &str = "https://api.sandbox.push.apple.com";
const TOKEN_MAX_AGE_SECONDS: i64 = 50 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApnsEnvironment {
    Sandbox,
    Production,
}

impl ApnsEnvironment {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Production => "production",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisterPushDevice {
    pub token: String,
    pub environment: ApnsEnvironment,
    pub topic: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PushDevice {
    pub id: Uuid,
    pub environment: ApnsEnvironment,
    pub topic: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NotificationPreference {
    pub category: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct NewNotification {
    pub source_event_id: Uuid,
    pub account_id: Uuid,
    pub category: String,
    pub title: String,
    pub body: String,
    pub deep_link: Option<String>,
    pub data: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRequest {
    pub account_id: Uuid,
    pub category: String,
    pub title: String,
    pub body: String,
    pub deep_link: Option<String>,
    #[serde(default)]
    pub data: serde_json::Map<String, Value>,
}

impl NotificationRequest {
    pub fn from_event(self, source_event_id: Uuid) -> NewNotification {
        NewNotification {
            source_event_id,
            account_id: self.account_id,
            category: self.category,
            title: self.title,
            body: self.body,
            deep_link: self.deep_link,
            data: self.data,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaimedDelivery {
    pub id: Uuid,
    pub device_id: Uuid,
    pub token: String,
    pub environment: ApnsEnvironment,
    pub topic: String,
    pub notification_id: Uuid,
    pub title: String,
    pub body: String,
    pub deep_link: Option<String>,
    pub data: serde_json::Map<String, Value>,
    pub attempts: i32,
}

#[derive(Debug, Clone)]
pub struct ApnsConfig {
    pub key_id: String,
    pub team_id: String,
    pub topic: String,
    pub private_key_pem: String,
    pub environment: ApnsEnvironment,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertPayload {
    pub aps: Aps,
    #[serde(flatten)]
    pub data: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Aps {
    pub alert: Alert,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApnsReceipt {
    pub apns_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("invalid APNs signing key: {0}")]
    Key(#[from] jsonwebtoken::errors::Error),
    #[error("invalid APNs device token")]
    DeviceToken,
    #[error("APNs payload exceeds 4096 bytes")]
    PayloadTooLarge,
    #[error("APNs request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("APNs rejected notification with HTTP {status}: {reason}")]
    Rejected { status: u16, reason: String },
    #[error("APNs authentication token cache is unavailable")]
    TokenCache,
    #[error("PostgreSQL push store: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid push category or topic")]
    InvalidName,
    #[error("push delivery lease was lost")]
    LeaseLost,
}

#[derive(Clone)]
pub struct PgPushStore {
    pool: PgPool,
}

impl PgPushStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, PushError> {
        Ok(Self {
            pool: PgPoolOptions::new()
                .max_connections(max_connections)
                .acquire_timeout(Duration::from_secs(5))
                .connect(database_url)
                .await?,
        })
    }

    pub async fn register_device(
        &self,
        account_id: Uuid,
        input: RegisterPushDevice,
    ) -> Result<PushDevice, PushError> {
        validate_name(&input.topic)?;
        let digest = device_token_digest(&input.token)?;
        let id = Uuid::new_v4();
        let row = sqlx::query(
            "INSERT INTO push_devices
               (id,account_id,environment,topic,token,token_digest)
             VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (environment,topic,token_digest) DO UPDATE SET
               account_id=excluded.account_id, token=excluded.token, active=true,
               invalidated_at=NULL, updated_at=now()
             RETURNING id,environment,topic,active",
        )
        .bind(id)
        .bind(account_id)
        .bind(input.environment.as_str())
        .bind(&input.topic)
        .bind(&input.token)
        .bind(digest)
        .fetch_one(&self.pool)
        .await?;
        Ok(PushDevice {
            id: row.try_get("id")?,
            environment: parse_environment(row.try_get("environment")?)?,
            topic: row.try_get("topic")?,
            active: row.try_get("active")?,
        })
    }

    pub async fn unregister_device(
        &self,
        account_id: Uuid,
        device_id: Uuid,
    ) -> Result<(), PushError> {
        require_one(
            sqlx::query(
                "UPDATE push_devices SET active=false,invalidated_at=now(),updated_at=now()
                 WHERE id=$1 AND account_id=$2 AND active",
            )
            .bind(device_id)
            .bind(account_id)
            .execute(&self.pool)
            .await?
            .rows_affected(),
        )
    }

    pub async fn set_preference(
        &self,
        account_id: Uuid,
        preference: NotificationPreference,
    ) -> Result<NotificationPreference, PushError> {
        validate_name(&preference.category)?;
        sqlx::query(
            "INSERT INTO notification_preferences (account_id,category,enabled)
             VALUES ($1,$2,$3) ON CONFLICT (account_id,category) DO UPDATE SET
             enabled=excluded.enabled,updated_at=now()",
        )
        .bind(account_id)
        .bind(&preference.category)
        .bind(preference.enabled)
        .execute(&self.pool)
        .await?;
        Ok(preference)
    }

    pub async fn enqueue(&self, notification: NewNotification) -> Result<Uuid, PushError> {
        validate_name(&notification.category)?;
        let mut transaction = self.pool.begin().await?;
        let notification_id = notification_id_for_event(notification.source_event_id);
        sqlx::query(
            "INSERT INTO push_notifications
               (id,source_event_id,account_id,category,title,body,deep_link,data)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (source_event_id) DO NOTHING",
        )
        .bind(notification_id)
        .bind(notification.source_event_id)
        .bind(notification.account_id)
        .bind(&notification.category)
        .bind(&notification.title)
        .bind(&notification.body)
        .bind(&notification.deep_link)
        .bind(Value::Object(notification.data))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO push_deliveries (id,notification_id,device_id)
             SELECT gen_random_uuid(),n.id,d.id FROM push_notifications n
             JOIN push_devices d ON d.account_id=n.account_id
             LEFT JOIN notification_preferences p ON p.account_id=n.account_id AND p.category=n.category
             WHERE n.id=$1 AND d.active AND COALESCE(p.enabled,true)
             ON CONFLICT (notification_id,device_id) DO NOTHING",
        )
        .bind(notification_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(notification_id)
    }

    pub async fn claim_deliveries(
        &self,
        worker: &str,
        environment: ApnsEnvironment,
        topic: &str,
        limit: i64,
    ) -> Result<Vec<ClaimedDelivery>, PushError> {
        let rows = sqlx::query(
            "WITH claimed AS (
               SELECT d.id FROM push_deliveries d JOIN push_devices filter_device ON filter_device.id=d.device_id
               WHERE d.status IN ('queued','retry') AND filter_device.active
                 AND filter_device.environment=$3 AND filter_device.topic=$4
                 AND d.available_at<=now() AND (d.lease_until IS NULL OR d.lease_until<now())
               ORDER BY d.available_at,d.id FOR UPDATE OF d SKIP LOCKED LIMIT $1
             )
             UPDATE push_deliveries d SET status='sending',lease_owner=$2,
               lease_until=now()+interval '60 seconds',attempts=d.attempts+1
             FROM claimed c,push_notifications n,push_devices pd
             WHERE d.id=c.id AND n.id=d.notification_id AND pd.id=d.device_id
             RETURNING d.id,d.device_id,pd.token,pd.environment,pd.topic,d.notification_id,n.title,n.body,
               n.deep_link,n.data,d.attempts",
        )
        .bind(limit)
        .bind(worker)
        .bind(environment.as_str())
        .bind(topic)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(delivery_from_row).collect()
    }

    pub async fn mark_delivered(
        &self,
        worker: &str,
        delivery_id: Uuid,
        apns_id: Option<Uuid>,
    ) -> Result<(), PushError> {
        require_one(
            sqlx::query(
                "UPDATE push_deliveries SET status='delivered',delivered_at=now(),apns_id=$1,
               response_status=200,response_reason=NULL,lease_owner=NULL,lease_until=NULL
             WHERE id=$2 AND lease_owner=$3 AND status='sending'",
            )
            .bind(apns_id)
            .bind(delivery_id)
            .bind(worker)
            .execute(&self.pool)
            .await?
            .rows_affected(),
        )
    }

    pub async fn mark_failed(
        &self,
        worker: &str,
        delivery: &ClaimedDelivery,
        status: Option<u16>,
        reason: &str,
        permanent: bool,
    ) -> Result<(), PushError> {
        let permanent = permanent || delivery.attempts >= 10;
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "UPDATE push_deliveries SET status=CASE WHEN $1 THEN 'permanent_failure' ELSE 'retry' END,
               available_at=CASE WHEN $1 THEN available_at ELSE now()+make_interval(secs => LEAST(3600,power(2,LEAST(attempts,10)))::double precision) END,
               response_status=$2,response_reason=left($3,500),lease_owner=NULL,lease_until=NULL
             WHERE id=$4 AND lease_owner=$5 AND status='sending'",
        )
        .bind(permanent)
        .bind(status.map(i32::from))
        .bind(reason)
        .bind(delivery.id)
        .bind(worker)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        require_one(rows)?;
        if matches!(
            reason,
            "BadDeviceToken" | "Unregistered" | "DeviceTokenNotForTopic"
        ) {
            sqlx::query(
                "UPDATE push_devices SET active=false,invalidated_at=now(),updated_at=now() WHERE id=$1",
            )
            .bind(delivery.device_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct ApnsClient {
    client: reqwest::Client,
    config: Arc<ApnsConfig>,
    encoding_key: Arc<EncodingKey>,
    token: Arc<Mutex<Option<CachedToken>>>,
}

#[derive(Clone)]
struct CachedToken {
    value: String,
    issued_at: i64,
}

#[derive(Serialize)]
struct ProviderClaims<'a> {
    iss: &'a str,
    iat: i64,
}

#[derive(Deserialize)]
struct ApnsErrorBody {
    reason: String,
}

impl ApnsClient {
    pub fn new(config: ApnsConfig) -> Result<Self, PushError> {
        let encoding_key = EncodingKey::from_ec_pem(config.private_key_pem.as_bytes())?;
        Ok(Self {
            client: reqwest::Client::builder()
                .http2_adaptive_window(true)
                .build()?,
            config: Arc::new(config),
            encoding_key: Arc::new(encoding_key),
            token: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn send_alert(
        &self,
        device_token: &str,
        payload: &AlertPayload,
        collapse_id: Option<&str>,
    ) -> Result<ApnsReceipt, PushError> {
        validate_device_token(device_token)?;
        let body = serde_json::to_vec(payload).expect("serializable APNs payload");
        if body.len() > 4096 {
            return Err(PushError::PayloadTooLarge);
        }
        let base = match self.config.environment {
            ApnsEnvironment::Sandbox => APNS_SANDBOX,
            ApnsEnvironment::Production => APNS_PRODUCTION,
        };
        let mut request = self
            .client
            .post(format!("{base}/3/device/{device_token}"))
            .bearer_auth(self.provider_token()?)
            .header("apns-topic", &self.config.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header("apns-expiration", "0")
            .header("content-type", "application/json")
            .body(body);
        if let Some(collapse_id) = collapse_id {
            request = request.header("apns-collapse-id", collapse_id);
        }
        let response = request.send().await?;
        let status = response.status();
        let apns_id = response
            .headers()
            .get("apns-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if status.is_success() {
            return Ok(ApnsReceipt { apns_id });
        }
        let reason = response
            .json::<ApnsErrorBody>()
            .await
            .map(|body| body.reason)
            .unwrap_or_else(|_| "Unknown".into());
        Err(PushError::Rejected {
            status: status.as_u16(),
            reason,
        })
    }

    fn provider_token(&self) -> Result<String, PushError> {
        let now = Utc::now().timestamp();
        let mut cached = self.token.lock().map_err(|_| PushError::TokenCache)?;
        if let Some(token) = cached.as_ref()
            && now - token.issued_at < TOKEN_MAX_AGE_SECONDS
        {
            return Ok(token.value.clone());
        }
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());
        let value = encode(
            &header,
            &ProviderClaims {
                iss: &self.config.team_id,
                iat: now,
            },
            &self.encoding_key,
        )?;
        *cached = Some(CachedToken {
            value: value.clone(),
            issued_at: now,
        });
        Ok(value)
    }
}

pub fn device_token_digest(token: &str) -> Result<Vec<u8>, PushError> {
    validate_device_token(token)?;
    Ok(Sha256::digest(token.as_bytes()).to_vec())
}

fn validate_device_token(token: &str) -> Result<(), PushError> {
    if token.len() < 32 || token.len() > 400 || !token.len().is_multiple_of(2) {
        return Err(PushError::DeviceToken);
    }
    if token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(PushError::DeviceToken)
    }
}

pub fn notification_id_for_event(event_id: Uuid) -> Uuid {
    event_id
}

fn validate_name(value: &str) -> Result<(), PushError> {
    if (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(PushError::InvalidName)
    }
}

fn parse_environment(value: &str) -> Result<ApnsEnvironment, PushError> {
    match value {
        "sandbox" => Ok(ApnsEnvironment::Sandbox),
        "production" => Ok(ApnsEnvironment::Production),
        _ => Err(PushError::InvalidName),
    }
}

fn delivery_from_row(row: sqlx::postgres::PgRow) -> Result<ClaimedDelivery, PushError> {
    let data: Value = row.try_get("data")?;
    Ok(ClaimedDelivery {
        id: row.try_get("id")?,
        device_id: row.try_get("device_id")?,
        token: row.try_get("token")?,
        environment: parse_environment(row.try_get("environment")?)?,
        topic: row.try_get("topic")?,
        notification_id: row.try_get("notification_id")?,
        title: row.try_get("title")?,
        body: row.try_get("body")?,
        deep_link: row.try_get("deep_link")?,
        data: data.as_object().cloned().unwrap_or_default(),
        attempts: row.try_get("attempts")?,
    })
}

fn require_one(rows: u64) -> Result<(), PushError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(PushError::LeaseLost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_tokens_are_strictly_validated_and_stably_hashed() {
        let token = "ab".repeat(32);
        assert_eq!(
            device_token_digest(&token).unwrap(),
            device_token_digest(&token).unwrap()
        );
        assert!(device_token_digest("not-a-device-token").is_err());
    }

    #[test]
    fn payload_uses_apple_alert_shape() {
        let payload = AlertPayload {
            aps: Aps {
                alert: Alert {
                    title: "Hyper-Tardy".into(),
                    body: "Something is breaking.".into(),
                },
                sound: Some("default".into()),
            },
            data: serde_json::Map::from_iter([("post_id".into(), Value::String("post-1".into()))]),
        };
        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value["aps"]["alert"]["title"], "Hyper-Tardy");
        assert_eq!(value["post_id"], "post-1");
    }
}
