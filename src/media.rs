use async_trait::async_trait;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::types::ChecksumMode;
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
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl R2ObjectStore {
    pub fn from_env() -> Result<Option<Self>, MediaError> {
        let Some(account_id) = std::env::var("R2_ACCOUNT_ID").ok() else {
            return Ok(None);
        };
        let access_key = std::env::var("R2_ACCESS_KEY_ID").map_err(|_| MediaError::Unconfigured)?;
        let secret = std::env::var("R2_SECRET_ACCESS_KEY").map_err(|_| MediaError::Unconfigured)?;
        let bucket = std::env::var("R2_BUCKET").map_err(|_| MediaError::Unconfigured)?;
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .credentials_provider(Credentials::new(access_key, secret, None, None, "tardy-r2"))
            .region(Region::new("auto"))
            .endpoint_url(format!("https://{account_id}.r2.cloudflarestorage.com"))
            .force_path_style(true)
            .build();
        Ok(Some(Self {
            client: aws_sdk_s3::Client::from_conf(config),
            bucket,
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
        let length: i64 = byte_length
            .try_into()
            .map_err(|_| MediaError::InvalidSize(byte_length))?;
        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .content_length(length);
        if let Some(checksum) = sha256_base64 {
            request = request.checksum_sha256(checksum);
        }
        let signed = request
            .presigned(
                PresigningConfig::expires_in(expires)
                    .map_err(|error| MediaError::ObjectStore(error.to_string()))?,
            )
            .await
            .map_err(|error| MediaError::ObjectStore(error.to_string()))?;
        Ok((
            signed.method().into(),
            signed.uri().into(),
            signed
                .headers()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        ))
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, MediaError> {
        let value = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await
            .map_err(|error| MediaError::ObjectStore(error.to_string()))?;
        Ok(ObjectMetadata {
            content_type: value.content_type().map(str::to_owned),
            byte_length: value
                .content_length()
                .unwrap_or_default()
                .try_into()
                .map_err(|_| MediaError::MetadataMismatch)?,
            sha256_base64: value.checksum_sha256().map(str::to_owned),
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
    async fn aws_adapter_presigns_an_r2_put_without_network_io() {
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .credentials_provider(Credentials::new("access", "secret", None, None, "test"))
            .region(Region::new("auto"))
            .endpoint_url("https://account.r2.cloudflarestorage.com")
            .force_path_style(true)
            .build();
        let store = R2ObjectStore {
            client: aws_sdk_s3::Client::from_conf(config),
            bucket: "media".into(),
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
