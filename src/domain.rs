use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub type TimestampMs = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Unlisted,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Profile {
    pub id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub bio: String,
    pub privacy: ProfilePrivacy,
    pub created_at_ms: TimestampMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileVisibility {
    Public,
    Unlisted,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirectMessagePolicy {
    Everyone,
    Nobody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResharePolicy {
    OwnerOnly,
    AnyoneWhoCanView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProfilePrivacy {
    pub profile_visibility: ProfileVisibility,
    pub direct_messages: DirectMessagePolicy,
    pub resharing: ResharePolicy,
    pub default_content_visibility: Visibility,
}

impl Default for ProfilePrivacy {
    fn default() -> Self {
        Self {
            profile_visibility: ProfileVisibility::Private,
            direct_messages: DirectMessagePolicy::Nobody,
            resharing: ResharePolicy::OwnerOnly,
            default_content_visibility: Visibility::Private,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PublicProfile {
    pub id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub bio: String,
}

impl From<&Profile> for PublicProfile {
    fn from(value: &Profile) -> Self {
        Self {
            id: value.id,
            handle: value.handle.clone(),
            display_name: value.display_name.clone(),
            bio: value.bio.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Reel {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub caption: String,
    /// Immutable output from Hyperframes or another renderer.
    pub media_url: String,
    pub poster_url: Option<String>,
    pub duration_ms: u64,
    pub visibility: Visibility,
    pub published_at_ms: TimestampMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EngagementKind {
    View,
    CompletedView,
    Like,
    Share,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EngagementReceipt {
    pub event_id: Uuid,
    pub reel_id: Uuid,
    pub profile_id: Uuid,
    pub kind: EngagementKind,
    pub occurred_at_ms: TimestampMs,
    /// False when this profile already contributed this signal for the reel.
    pub counted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HyperTardyItem {
    pub reel: Reel,
    /// Deterministic weighted velocity score, intended for ordering rather than display.
    pub score: u64,
    pub unique_views: u64,
    pub unique_completed_views: u64,
    pub unique_likes: u64,
    pub unique_shares: u64,
    pub window_started_at_ms: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SavedPost {
    pub reel: Reel,
    pub saved_at_ms: TimestampMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveStatus {
    Live,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LiveSession {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub title: String,
    pub repository_url: String,
    pub playback_url: String,
    pub visibility: Visibility,
    pub status: LiveStatus,
    pub started_at_ms: TimestampMs,
    pub ended_at_ms: Option<TimestampMs>,
    pub latest_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LiveEvent {
    pub session_id: Uuid,
    /// Monotonic within a session. Assigned by Tardy, never the producer.
    pub sequence: u64,
    pub occurred_at_ms: TimestampMs,
    #[serde(flatten)]
    pub payload: LiveEventPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEventPayload {
    Status { message: String },
    Tool { name: String, summary: String },
    Commit { sha: String, message: String },
    ViewerCount { count: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedItem {
    Reel(Reel),
    Live(LiveSession),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShareSubject {
    Profile { id: Uuid },
    Reel { id: Uuid },
    Live { id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AgentHandoff {
    pub schema_version: String,
    pub target: String,
    pub subject: ShareSubject,
    pub prompt: String,
    pub capabilities: AgentCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AgentCapabilities {
    pub publish_reel_url: String,
    pub start_live_url: String,
    pub append_live_event_url_template: String,
    pub end_live_url_template: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DirectThread {
    pub id: Uuid,
    pub participants: [Uuid; 2],
    pub created_at_ms: TimestampMs,
    pub latest_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DirectMessage {
    pub thread_id: Uuid,
    pub sequence: u64,
    pub sender_id: Uuid,
    pub body: String,
    pub sent_at_ms: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ShareGrant {
    pub id: Uuid,
    pub token: Uuid,
    pub created_by: Uuid,
    pub subject: ShareSubject,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: Option<TimestampMs>,
    pub revoked_at_ms: Option<TimestampMs>,
}

impl FeedItem {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Reel(value) => value.id,
            Self::Live(value) => value.id,
        }
    }

    pub fn published_at_ms(&self) -> TimestampMs {
        match self {
            Self::Reel(value) => value.published_at_ms,
            Self::Live(value) => value.started_at_ms,
        }
    }

    pub fn profile_id(&self) -> Uuid {
        match self {
            Self::Reel(value) => value.profile_id,
            Self::Live(value) => value.profile_id,
        }
    }

    pub fn visibility(&self) -> Visibility {
        match self {
            Self::Reel(value) => value.visibility,
            Self::Live(value) => value.visibility,
        }
    }
}
