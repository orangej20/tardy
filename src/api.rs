use crate::domain::{AgentCapabilities, AgentHandoff, LiveEventPayload, ShareSubject, Visibility};
use crate::ranking::LuaRanker;
use crate::store::{MemoryStore, NewLive, NewProfile, NewReel, Store, StoreError};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub ranker: LuaRanker,
    pub public_base_url: String,
}

impl AppState {
    pub fn in_memory(
        public_base_url: impl Into<String>,
    ) -> Result<Self, crate::ranking::RankingError> {
        Ok(Self {
            store: Arc::new(MemoryStore::default()),
            ranker: LuaRanker::default_policy()?,
            public_base_url: public_base_url.into().trim_end_matches('/').to_owned(),
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/v1/profiles", post(create_profile))
        .route("/v1/reels", post(publish_reel))
        .route("/v1/lives", post(start_live))
        .route("/v1/lives/{id}/events", post(append_event).get(list_events))
        .route("/v1/lives/{id}/end", post(end_live))
        .route("/v1/feed", get(feed))
        .route("/v1/agent-handoffs", post(agent_handoff))
        .with_state(state)
}

#[derive(Deserialize)]
struct CreateProfile {
    handle: String,
    display_name: String,
    #[serde(default)]
    bio: String,
}

async fn create_profile(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProfile>,
) -> Result<impl IntoResponse, ApiError> {
    validate_handle(&body.handle)?;
    let value = state.store.create_profile(NewProfile {
        handle: body.handle,
        display_name: body.display_name,
        bio: body.bio,
        created_at_ms: now_ms()?,
    })?;
    Ok((StatusCode::CREATED, Json(value)))
}

#[derive(Deserialize)]
struct PublishReel {
    profile_id: Uuid,
    caption: String,
    media_url: String,
    poster_url: Option<String>,
    duration_ms: u64,
    visibility: Visibility,
}

async fn publish_reel(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PublishReel>,
) -> Result<impl IntoResponse, ApiError> {
    require_http_url(&body.media_url, "media_url")?;
    let value = state.store.publish_reel(NewReel {
        profile_id: body.profile_id,
        caption: body.caption,
        media_url: body.media_url,
        poster_url: body.poster_url,
        duration_ms: body.duration_ms,
        visibility: body.visibility,
        published_at_ms: now_ms()?,
    })?;
    Ok((StatusCode::CREATED, Json(value)))
}

#[derive(Deserialize)]
struct StartLive {
    profile_id: Uuid,
    title: String,
    repository_url: String,
    playback_url: String,
    visibility: Visibility,
}

async fn start_live(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartLive>,
) -> Result<impl IntoResponse, ApiError> {
    require_http_url(&body.repository_url, "repository_url")?;
    require_http_url(&body.playback_url, "playback_url")?;
    let value = state.store.start_live(NewLive {
        profile_id: body.profile_id,
        title: body.title,
        repository_url: body.repository_url,
        playback_url: body.playback_url,
        visibility: body.visibility,
        started_at_ms: now_ms()?,
    })?;
    Ok((StatusCode::CREATED, Json(value)))
}

async fn append_event(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<LiveEventPayload>,
) -> Result<impl IntoResponse, ApiError> {
    let value = state.store.append_live_event(id, now_ms()?, payload)?;
    Ok((StatusCode::CREATED, Json(value)))
}

#[derive(Deserialize)]
struct EventQuery {
    #[serde(default)]
    after: u64,
}

async fn list_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(query): Query<EventQuery>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.live_events(id, query.after)?))
}

async fn end_live(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.end_live(id, now_ms()?)?))
}

#[derive(Deserialize)]
struct FeedQuery {
    viewer_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    20
}

async fn feed(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FeedQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !(1..=100).contains(&query.limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 100"));
    }
    let mut items = state
        .ranker
        .rank(state.store.feed_candidates(query.viewer_id)?, now_ms()?)?;
    items.truncate(query.limit);
    Ok(Json(items))
}

#[derive(Deserialize)]
struct HandoffRequest {
    target: String,
    subject: ShareSubject,
}

async fn agent_handoff(
    State(state): State<Arc<AppState>>,
    Json(body): Json<HandoffRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.store.share_subject_exists(&body.subject)? {
        return Err(ApiError::not_found("share subject not found"));
    }
    if body.target.trim().is_empty() {
        return Err(ApiError::bad_request("target is required"));
    }
    let base = &state.public_base_url;
    let capabilities = AgentCapabilities {
        publish_reel_url: format!("{base}/v1/reels"),
        start_live_url: format!("{base}/v1/lives"),
        append_live_event_url_template: format!("{base}/v1/lives/{{live_id}}/events"),
        end_live_url_template: format!("{base}/v1/lives/{{live_id}}/end"),
    };
    let prompt = build_handoff_prompt(&body.target, &body.subject, &capabilities);
    Ok((
        StatusCode::CREATED,
        Json(AgentHandoff {
            schema_version: "tardy.agent-handoff.v1".into(),
            target: body.target,
            subject: body.subject,
            prompt,
            capabilities,
        }),
    ))
}

fn build_handoff_prompt(
    target: &str,
    subject: &ShareSubject,
    capabilities: &AgentCapabilities,
) -> String {
    format!(
        r#"Integrate this project with Tardy using the {target} agent system.

Shared subject: {subject:?}

Goals:
1. Add this project to the agent's known projects without changing its existing build workflow.
2. When a coding session begins, POST metadata to {start_live} and retain the returned live id.
3. Publish concise status, tool, and commit events to {events}. Never send secrets, environment values, full prompts, or raw command output.
4. When the session finishes or fails, POST to {end_live}.
5. When Hyperframes produces a final vertical video, POST its immutable media URL and metadata to {reels}.

Requirements:
- Ask for a Tardy API credential through the agent system's secret store; credentials are intentionally absent here.
- Treat HTTP non-2xx responses as visible failures. Retry only idempotent reads; buffer writes with stable local ordering.
- Keep orchestration deterministic. Use AI only to summarize an already-observed event.
- Do not begin publishing until the user confirms the target profile and visibility.

First report the files and hooks you intend to change, then implement the smallest adapter that calls these APIs."#,
        start_live = capabilities.start_live_url,
        events = capabilities.append_live_event_url_template,
        end_live = capabilities.end_live_url_template,
        reels = capabilities.publish_reel_url,
    )
}

fn validate_handle(value: &str) -> Result<(), ApiError> {
    let valid = (3..=32).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    valid.then_some(()).ok_or_else(|| {
        ApiError::bad_request("handle must be 3-32 lowercase letters, digits, or underscores")
    })
}

fn require_http_url(value: &str, field: &str) -> Result<(), ApiError> {
    (value.starts_with("https://") || value.starts_with("http://"))
        .then_some(())
        .ok_or_else(|| ApiError::bad_request(format!("{field} must be an http(s) URL")))
}

fn now_ms() -> Result<u64, ApiError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::internal("system clock is before Unix epoch"))?
        .as_millis()
        .try_into()
        .map_err(|_| ApiError::internal("timestamp overflow"))?)
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::ProfileNotFound | StoreError::LiveNotFound => {
                Self::not_found(value.to_string())
            }
            StoreError::LiveEnded | StoreError::HandleConflict => Self {
                status: StatusCode::CONFLICT,
                message: value.to_string(),
            },
            StoreError::Poisoned => Self::internal(value.to_string()),
        }
    }
}

impl From<crate::ranking::RankingError> for ApiError {
    fn from(value: crate::ranking::RankingError) -> Self {
        Self::internal(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, header::CONTENT_TYPE};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn request(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }

    #[tokio::test]
    async fn creates_a_hermes_handoff_with_a_complete_prompt() {
        let app = router(Arc::new(AppState::in_memory("https://tardy.test").unwrap()));
        let (_, profile) = request(
            &app,
            "POST",
            "/v1/profiles",
            json!({"handle":"build_agent","display_name":"Build Agent"}),
        )
        .await;
        let (status, handoff) = request(
            &app,
            "POST",
            "/v1/agent-handoffs",
            json!({"target":"hermes","subject":{"kind":"profile","id":profile["id"]}}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(handoff["schema_version"], "tardy.agent-handoff.v1");
        assert!(
            handoff["prompt"]
                .as_str()
                .unwrap()
                .contains("Never send secrets")
        );
        assert_eq!(
            handoff["capabilities"]["start_live_url"],
            "https://tardy.test/v1/lives"
        );
    }
}
