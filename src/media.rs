use async_trait::async_trait;
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

const UPLOAD_TTL_MS: u64 = 15 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Scene,
    Voiceover,
    Poster,
    VideoOriginal,
}

impl MediaKind {
    fn max_bytes(self) -> u64 {
        match self {
            Self::Scene => 1 << 20,
            Self::Poster => 10 << 20,
            Self::Voiceover => 25 << 20,
            Self::VideoOriginal => 250 << 20,
        }
    }
    fn allows(self, mime: &str) -> bool {
        match self {
            Self::Scene => mime == "application/json",
            Self::Poster => matches!(mime, "image/jpeg" | "image/png" | "image/webp"),
            Self::Voiceover => matches!(mime, "audio/mp4" | "audio/mpeg" | "audio/ogg"),
            Self::VideoOriginal => matches!(mime, "video/mp4" | "video/quicktime" | "video/webm"),
        }
    }
    fn key_segment(self) -> &'static str {
        match self {
            Self::Scene => "structured",
            Self::Voiceover => "audio",
            Self::Poster => "poster",
            Self::VideoOriginal => "video",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UploadIntent {
    pub profile_id: Uuid,
    pub kind: MediaKind,
    pub content_type: String,
    pub byte_length: u64,
    pub sha256_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UploadAuthorization {
    pub id: Uuid,
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub expires_at_ms: u64,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MediaAsset {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub kind: MediaKind,
    pub object_key: String,
    pub content_type: String,
    pub byte_length: u64,
    pub sha256_base64: Option<String>,
    pub status: MediaStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaStatus {
    Quarantined,
    Ready,
}

#[derive(Debug, Clone)]
struct UploadSession {
    id: Uuid,
    profile_id: Uuid,
    kind: MediaKind,
    object_key: String,
    content_type: String,
    byte_length: u64,
    sha256_base64: Option<String>,
    expires_at_ms: u64,
    completed: Option<MediaAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub content_type: Option<String>,
    pub byte_length: u64,
    pub sha256_base64: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("R2 is not configured")]
    Unconfigured,
    #[error("unsupported media type for this upload kind")]
    UnsupportedType,
    #[error("upload size must be between 1 and {0} bytes")]
    InvalidSize(u64),
    #[error("upload not found or expired")]
    NotFound,
    #[error("access denied")]
    Forbidden,
    #[error("uploaded object metadata does not match authorization")]
    MetadataMismatch,
    #[error("object store: {0}")]
    ObjectStore(String),
    #[error("media state lock poisoned")]
    Poisoned,
    #[error("timestamp overflow")]
    TimestampOverflow,
}

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        byte_length: u64,
        sha256_base64: Option<&str>,
        expires: Duration,
    ) -> Result<(String, String, BTreeMap<String, String>), MediaError>;
    async fn head(&self, key: &str) -> Result<ObjectMetadata, MediaError>;
}

pub struct R2ObjectStore {
    bucket: Bucket,
    credentials: Credentials,
    client: reqwest::Client,
}

impl R2ObjectStore {
    pub fn from_env() -> Result<Option<Self>, MediaError> {
        let Some(account_id) = std::env::var("R2_ACCOUNT_ID").ok() else {
            return Ok(None);
        };
        let access_key = std::env::var("R2_ACCESS_KEY_ID").map_err(|_| MediaError::Unconfigured)?;
        let secret = std::env::var("R2_SECRET_ACCESS_KEY").map_err(|_| MediaError::Unconfigured)?;
        let bucket_name = std::env::var("R2_BUCKET").map_err(|_| MediaError::Unconfigured)?;
        let endpoint = format!("https://{account_id}.r2.cloudflarestorage.com")
            .parse()
            .map_err(|error| MediaError::ObjectStore(format!("invalid R2 endpoint: {error}")))?;
        let bucket = Bucket::new(endpoint, UrlStyle::Path, bucket_name, "auto")
            .map_err(|error| MediaError::ObjectStore(format!("invalid R2 bucket: {error}")))?;
        Ok(Some(Self {
            bucket,
            credentials: Credentials::new(access_key, secret),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|error| MediaError::ObjectStore(error.to_string()))?,
        }))
    }
}

#[async_trait]
impl ObjectStore for R2ObjectStore {
    async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        byte_length: u64,
        sha256_base64: Option<&str>,
        expires: Duration,
    ) -> Result<(String, String, BTreeMap<String, String>), MediaError> {
        let mut action = self.bucket.put_object(Some(&self.credentials), key);
        action.headers_mut().insert("content-type", content_type);
        let length = byte_length.to_string();
        action.headers_mut().insert("content-length", &length);
        if let Some(checksum) = sha256_base64 {
            action
                .headers_mut()
                .insert("x-amz-checksum-sha256", checksum);
        }
        let headers = action
            .headers_mut()
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        Ok(("PUT".into(), action.sign(expires).into(), headers))
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, MediaError> {
        let mut action = self.bucket.head_object(Some(&self.credentials), key);
        action
            .headers_mut()
            .insert("x-amz-checksum-mode", "ENABLED");
        let url = action.sign(Duration::from_secs(60));
        let value = self
            .client
            .head(url)
            .header("x-amz-checksum-mode", "ENABLED")
            .send()
            .await
            .map_err(|error| MediaError::ObjectStore(error.to_string()))?;
        if !value.status().is_success() {
            return Err(MediaError::ObjectStore(format!("HTTP {}", value.status())));
        }
        let headers = value.headers();
        Ok(ObjectMetadata {
            content_type: headers
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            byte_length: headers
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .ok_or(MediaError::MetadataMismatch)?
                .parse()
                .map_err(|_| MediaError::MetadataMismatch)?,
            sha256_base64: headers
                .get("x-amz-checksum-sha256")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        })
    }
}

pub struct MediaService {
    object_store: Option<Arc<dyn ObjectStore>>,
    sessions: RwLock<HashMap<Uuid, UploadSession>>,
}

impl MediaService {
    pub fn new(object_store: Option<Arc<dyn ObjectStore>>) -> Self {
        Self {
            object_store,
            sessions: RwLock::new(HashMap::new()),
        }
    }
    pub fn from_env() -> Result<Self, MediaError> {
        Ok(Self::new(
            R2ObjectStore::from_env()?.map(|store| Arc::new(store) as Arc<dyn ObjectStore>),
        ))
    }

    pub async fn authorize(
        &self,
        actor: Uuid,
        intent: UploadIntent,
        now_ms: u64,
    ) -> Result<UploadAuthorization, MediaError> {
        if actor != intent.profile_id {
            return Err(MediaError::Forbidden);
        }
        if !intent.kind.allows(&intent.content_type) {
            return Err(MediaError::UnsupportedType);
        }
        let max_bytes = intent.kind.max_bytes();
        if intent.byte_length == 0 || intent.byte_length > max_bytes {
            return Err(MediaError::InvalidSize(max_bytes));
        }
        let store = self.object_store.as_ref().ok_or(MediaError::Unconfigured)?;
        let id = Uuid::new_v4();
        let key = format!("quarantine/{}/{}/{}", actor, intent.kind.key_segment(), id);
        let expires_at_ms = now_ms
            .checked_add(UPLOAD_TTL_MS)
            .ok_or(MediaError::TimestampOverflow)?;
        let (method, url, headers) = store
            .presign_put(
                &key,
                &intent.content_type,
                intent.byte_length,
                intent.sha256_base64.as_deref(),
                Duration::from_millis(UPLOAD_TTL_MS),
            )
            .await?;
        self.sessions
            .write()
            .map_err(|_| MediaError::Poisoned)?
            .insert(
                id,
                UploadSession {
                    id,
                    profile_id: actor,
                    kind: intent.kind,
                    object_key: key,
                    content_type: intent.content_type,
                    byte_length: intent.byte_length,
                    sha256_base64: intent.sha256_base64,
                    expires_at_ms,
                    completed: None,
                },
            );
        Ok(UploadAuthorization {
            id,
            method,
            url,
            headers,
            expires_at_ms,
            max_bytes,
        })
    }

    pub async fn complete(
        &self,
        actor: Uuid,
        id: Uuid,
        now_ms: u64,
    ) -> Result<MediaAsset, MediaError> {
        let session = self
            .sessions
            .read()
            .map_err(|_| MediaError::Poisoned)?
            .get(&id)
            .cloned()
            .ok_or(MediaError::NotFound)?;
        if session.profile_id != actor {
            return Err(MediaError::Forbidden);
        }
        if let Some(asset) = session.completed {
            return Ok(asset);
        }
        if session.expires_at_ms <= now_ms {
            return Err(MediaError::NotFound);
        }
        let actual = self
            .object_store
            .as_ref()
            .ok_or(MediaError::Unconfigured)?
            .head(&session.object_key)
            .await?;
        if actual.byte_length != session.byte_length
            || actual.content_type.as_deref() != Some(&session.content_type)
            || (session.sha256_base64.is_some() && actual.sha256_base64 != session.sha256_base64)
        {
            return Err(MediaError::MetadataMismatch);
        }
        let asset = MediaAsset {
            id: session.id,
            profile_id: actor,
            kind: session.kind,
            object_key: session.object_key,
            content_type: session.content_type,
            byte_length: session.byte_length,
            sha256_base64: session.sha256_base64,
            status: MediaStatus::Quarantined,
        };
        self.sessions
            .write()
            .map_err(|_| MediaError::Poisoned)?
            .get_mut(&id)
            .ok_or(MediaError::NotFound)?
            .completed = Some(asset.clone());
        Ok(asset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        metadata: Mutex<Option<ObjectMetadata>>,
    }
    #[async_trait]
    impl ObjectStore for Fake {
        async fn presign_put(
            &self,
            _: &str,
            content_type: &str,
            byte_length: u64,
            _: Option<&str>,
            _: Duration,
        ) -> Result<(String, String, BTreeMap<String, String>), MediaError> {
            *self.metadata.lock().unwrap() = Some(ObjectMetadata {
                content_type: Some(content_type.into()),
                byte_length,
                sha256_base64: None,
            });
            Ok((
                "PUT".into(),
                "https://r2.test/upload".into(),
                BTreeMap::new(),
            ))
        }
        async fn head(&self, _: &str) -> Result<ObjectMetadata, MediaError> {
            Ok(self.metadata.lock().unwrap().clone().unwrap())
        }
    }

    #[tokio::test]
    async fn authorizes_and_head_verifies_an_upload() {
        let service = MediaService::new(Some(Arc::new(Fake {
            metadata: Mutex::new(None),
        })));
        let profile = Uuid::new_v4();
        let auth = service
            .authorize(
                profile,
                UploadIntent {
                    profile_id: profile,
                    kind: MediaKind::Scene,
                    content_type: "application/json".into(),
                    byte_length: 42,
                    sha256_base64: None,
                },
                1,
            )
            .await
            .unwrap();
        assert_eq!(auth.method, "PUT");
        let asset = service.complete(profile, auth.id, 2).await.unwrap();
        assert_eq!(asset.status, MediaStatus::Quarantined);
        assert_eq!(service.complete(profile, auth.id, 3).await.unwrap(), asset);
    }

    #[tokio::test]
    async fn rejects_oversized_or_cross_profile_uploads() {
        let service = MediaService::new(Some(Arc::new(Fake {
            metadata: Mutex::new(None),
        })));
        let profile = Uuid::new_v4();
        let other = Uuid::new_v4();
        let intent = UploadIntent {
            profile_id: profile,
            kind: MediaKind::Scene,
            content_type: "application/json".into(),
            byte_length: 2 << 20,
            sha256_base64: None,
        };
        assert!(matches!(
            service.authorize(profile, intent.clone(), 1).await,
            Err(MediaError::InvalidSize(_))
        ));
        assert!(matches!(
            service.authorize(other, intent, 1).await,
            Err(MediaError::Forbidden)
        ));
    }

    #[tokio::test]
    async fn r2_adapter_presigns_a_put_without_network_io() {
        let store = R2ObjectStore {
            bucket: Bucket::new(
                "https://account.r2.cloudflarestorage.com".parse().unwrap(),
                UrlStyle::Path,
                "media",
                "auto",
            )
            .unwrap(),
            credentials: Credentials::new("access", "secret"),
            client: reqwest::Client::new(),
        };
        let (method, url, headers) = store
            .presign_put(
                "quarantine/profile/scene/upload",
                "application/json",
                42,
                None,
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        assert_eq!(method, "PUT");
        assert!(url.starts_with("https://account.r2.cloudflarestorage.com/media/quarantine/"));
        assert!(url.contains("X-Amz-Signature="));
        assert_eq!(
            headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
    }
}
