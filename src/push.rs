use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const APNS_PRODUCTION: &str = "https://api.push.apple.com";
const APNS_SANDBOX: &str = "https://api.sandbox.push.apple.com";
const TOKEN_MAX_AGE_SECONDS: i64 = 50 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApnsEnvironment {
    Sandbox,
    Production,
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
