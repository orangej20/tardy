use crate::domain::{
    AgentCapabilities, AgentHandoff, LiveEventPayload, ProfilePrivacy, ShareSubject, Visibility,
};
use crate::media::{MediaError, MediaService, UploadIntent};
use crate::metrics::Metrics;
use crate::onboarding::{AccountRegistry, OnboardingError};
use crate::ranking::LuaRanker;
use crate::store::{MemoryStore, NewLive, NewProfile, NewReel, Store, StoreError};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router, middleware};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub ranker: LuaRanker,
    pub public_base_url: String,
    pub accounts: Arc<AccountRegistry>,
    pub metrics: Arc<Metrics>,
    pub media: Arc<MediaService>,
}

impl AppState {
    pub fn in_memory(
        public_base_url: impl Into<String>,
    ) -> Result<Self, crate::ranking::RankingError> {
        Ok(Self {
            store: Arc::new(MemoryStore::default()),
            ranker: LuaRanker::default_policy()?,
            public_base_url: public_base_url.into().trim_end_matches('/').to_owned(),
            accounts: Arc::new(AccountRegistry::in_memory().expect("in-memory account registry")),
            metrics: Arc::new(Metrics::new()),
            media: Arc::new(MediaService::new(None)),
        })
    }

    pub fn with_account_db(
        public_base_url: impl Into<String>,
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            store: Arc::new(MemoryStore::default()),
            ranker: LuaRanker::default_policy()?,
            public_base_url: public_base_url.into().trim_end_matches('/').to_owned(),
            accounts: Arc::new(AccountRegistry::open(path)?),
            metrics: Arc::new(Metrics::new()),
            media: Arc::new(MediaService::from_env()?),
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let metrics = state.metrics.clone();
    Router::new()
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/metrics", get(metrics_endpoint))
        .route("/llms.txt", get(llms_txt))
        .route("/v1/profiles", post(create_profile))
        .route("/v1/profiles/{handle}", get(get_profile))
        .route("/v1/profile/privacy", post(update_privacy))
        .route("/v1/blocks/{profile_id}", post(block_profile))
        .route("/v1/dm-threads", post(create_thread))
        .route(
            "/v1/dm-threads/{id}/messages",
            post(send_message).get(list_messages),
        )
        .route("/v1/shares", post(create_share))
        .route("/v1/shares/{id}/revoke", post(revoke_share))
        .route("/v1/shared/{token}", get(resolve_share))
        .route("/v1/onboarding/agent-codes", post(issue_agent_code))
        .route("/v1/onboarding/claims", post(claim_agent_code))
        .route("/v1/uploads", post(authorize_upload))
        .route("/v1/uploads/{id}/complete", post(complete_upload))
        .route("/v1/reels", post(publish_reel))
        .route("/v1/lives", post(start_live))
        .route("/v1/lives/{id}/events", post(append_event).get(list_events))
        .route("/v1/lives/{id}/end", post(end_live))
        .route("/v1/feed", get(feed))
        .route("/v1/agent-handoffs", post(agent_handoff))
        .with_state(state)
        .layer(middleware::from_fn(move |request, next| {
            crate::metrics::track(metrics.clone(), request, next)
        }))
}

async fn metrics_endpoint(State(state): State<Arc<AppState>>) -> Response {
    crate::metrics::response(&state.metrics)
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
    headers: HeaderMap,
    Json(body): Json<CreateProfile>,
) -> Result<impl IntoResponse, ApiError> {
    let account_id = authenticated_account(&state, &headers)?;
    validate_handle(&body.handle)?;
    let value = state.store.create_profile(NewProfile {
        handle: body.handle,
        display_name: body.display_name,
        bio: body.bio,
        privacy: ProfilePrivacy::default(),
        created_at_ms: now_ms()?,
    })?;
    state.accounts.bind_profile(account_id, value.id)?;
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
    headers: HeaderMap,
    Json(body): Json<PublishReel>,
) -> Result<impl IntoResponse, ApiError> {
    require_http_url(&body.media_url, "media_url")?;
    let value = state.store.publish_reel(
        authenticated_actor(&state, &headers)?,
        NewReel {
            profile_id: body.profile_id,
            caption: body.caption,
            media_url: body.media_url,
            poster_url: body.poster_url,
            duration_ms: body.duration_ms,
            visibility: body.visibility,
            published_at_ms: now_ms()?,
        },
    )?;
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
    headers: HeaderMap,
    Json(body): Json<StartLive>,
) -> Result<impl IntoResponse, ApiError> {
    require_http_url(&body.repository_url, "repository_url")?;
    require_http_url(&body.playback_url, "playback_url")?;
    let value = state.store.start_live(
        authenticated_actor(&state, &headers)?,
        NewLive {
            profile_id: body.profile_id,
            title: body.title,
            repository_url: body.repository_url,
            playback_url: body.playback_url,
            visibility: body.visibility,
            started_at_ms: now_ms()?,
        },
    )?;
    Ok((StatusCode::CREATED, Json(value)))
}

async fn append_event(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<LiveEventPayload>,
) -> Result<impl IntoResponse, ApiError> {
    let value = state.store.append_live_event(
        authenticated_actor(&state, &headers)?,
        id,
        now_ms()?,
        payload,
    )?;
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
    headers: HeaderMap,
    Query(query): Query<EventQuery>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.live_events(
        optional_authenticated_actor(&state, &headers)?,
        id,
        query.after,
    )?))
}

async fn end_live(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.end_live(
        authenticated_actor(&state, &headers)?,
        id,
        now_ms()?,
    )?))
}

#[derive(Deserialize)]
struct FeedQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    20
}

async fn feed(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FeedQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !(1..=100).contains(&query.limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 100"));
    }
    let mut items = state.ranker.rank(
        state
            .store
            .feed_candidates(optional_authenticated_actor(&state, &headers)?)?,
        now_ms()?,
    )?;
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
    headers: HeaderMap,
    Json(body): Json<HandoffRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = authenticated_actor(&state, &headers)?;
    if !state.store.can_share_subject(actor, &body.subject)? {
        return Err(ApiError::forbidden(
            "subject cannot be shared by this profile",
        ));
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

async fn llms_txt(State(state): State<Arc<AppState>>) -> String {
    format!(
        r#"# Tardy agent onboarding

Tardy turns agent project updates into private feeds, live sessions, and Hyperframes reels.

API base: {base}

1. POST {base}/v1/onboarding/agent-codes with an empty JSON object.
2. Store the returned one-time code securely. It expires after 24 hours and is never recoverable.
3. Ask the human for the email they want attached to the account.
4. POST the code and email to {base}/v1/onboarding/claims.
5. Create a profile. New profiles, DMs, and content default to private/closed.
6. Do not publish, live-stream, message, or share until the human explicitly changes the relevant privacy setting.

Never send secrets, environment variables, hidden prompts, or raw command output to Tardy.
Email delivery through AgentMail is a planned adapter; the code flow is the currently supported onboarding path.
"#,
        base = state.public_base_url
    )
}

async fn get_profile(
    State(state): State<Arc<AppState>>,
    Path(handle): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.public_profile(
        &handle,
        optional_authenticated_actor(&state, &headers)?,
    )?))
}

async fn update_privacy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(privacy): Json<ProfilePrivacy>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.update_privacy(
        authenticated_actor(&state, &headers)?,
        privacy,
    )?))
}

async fn block_profile(
    State(state): State<Arc<AppState>>,
    Path(profile_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .block_profile(authenticated_actor(&state, &headers)?, profile_id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct CreateThread {
    recipient_id: Uuid,
}

async fn create_thread(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateThread>,
) -> Result<impl IntoResponse, ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(state.store.create_thread(
            authenticated_actor(&state, &headers)?,
            body.recipient_id,
            now_ms()?,
        )?),
    ))
}

#[derive(Deserialize)]
struct SendMessage {
    body: String,
}

async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<SendMessage>,
) -> Result<impl IntoResponse, ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(state.store.send_message(
            authenticated_actor(&state, &headers)?,
            id,
            body.body,
            now_ms()?,
        )?),
    ))
}

async fn list_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<EventQuery>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.messages(
        authenticated_actor(&state, &headers)?,
        id,
        query.after,
    )?))
}

#[derive(Deserialize)]
struct CreateShare {
    subject: ShareSubject,
    expires_at_ms: Option<u64>,
}

async fn create_share(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateShare>,
) -> Result<impl IntoResponse, ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(state.store.create_share(
            authenticated_actor(&state, &headers)?,
            body.subject,
            now_ms()?,
            body.expires_at_ms,
        )?),
    ))
}

async fn resolve_share(
    State(state): State<Arc<AppState>>,
    Path(token): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.resolve_share(token, now_ms()?)?))
}

async fn revoke_share(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.revoke_share(
        authenticated_actor(&state, &headers)?,
        id,
        now_ms()?,
    )?))
}

async fn issue_agent_code(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    let claim = state.accounts.issue_claim(now_ms()?)?;
    state.metrics.note_claim_issued();
    Ok((StatusCode::CREATED, Json(claim)))
}

#[derive(Deserialize)]
struct ClaimAgentCode {
    code: String,
    email: String,
}

async fn claim_agent_code(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ClaimAgentCode>,
) -> Result<impl IntoResponse, ApiError> {
    let account = state.accounts.claim(&body.code, &body.email, now_ms()?)?;
    state.metrics.note_account_claimed();
    Ok((StatusCode::CREATED, Json(account)))
}

async fn authorize_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(intent): Json<UploadIntent>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = authenticated_actor(&state, &headers)?;
    Ok((
        StatusCode::CREATED,
        Json(state.media.authorize(actor, intent, now_ms()?).await?),
    ))
}

async fn complete_upload(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let actor = authenticated_actor(&state, &headers)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(state.media.complete(actor, id, now_ms()?).await?),
    ))
}

fn selected_profile(headers: &HeaderMap) -> Result<Option<Uuid>, ApiError> {
    headers
        .get("x-tardy-profile-id")
        .map(|value| {
            value
                .to_str()
                .map_err(|_| ApiError::bad_request("invalid x-tardy-profile-id header"))
                .and_then(|value| {
                    Uuid::parse_str(value)
                        .map_err(|_| ApiError::bad_request("invalid x-tardy-profile-id header"))
                })
        })
        .transpose()
}

fn bearer_token(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
    headers
        .get("authorization")
        .map(|value| {
            value
                .to_str()
                .map_err(|_| ApiError::unauthorized("invalid authorization header"))
                .and_then(|value| {
                    value
                        .strip_prefix("Bearer ")
                        .filter(|token| !token.is_empty())
                        .ok_or_else(|| ApiError::unauthorized("bearer token is required"))
                })
        })
        .transpose()
}

fn authenticated_account(state: &AppState, headers: &HeaderMap) -> Result<Uuid, ApiError> {
    let token =
        bearer_token(headers)?.ok_or_else(|| ApiError::unauthorized("bearer token is required"))?;
    state
        .accounts
        .authenticate(token)
        .map_err(|_| ApiError::unauthorized("invalid bearer token"))
}

fn authenticated_actor(state: &AppState, headers: &HeaderMap) -> Result<Uuid, ApiError> {
    let account = authenticated_account(state, headers)?;
    let profile = selected_profile(headers)?
        .ok_or_else(|| ApiError::unauthorized("x-tardy-profile-id is required"))?;
    if !state.accounts.owns_profile(account, profile)? {
        return Err(ApiError::forbidden("account does not own selected profile"));
    }
    Ok(profile)
}

fn optional_authenticated_actor(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<Uuid>, ApiError> {
    match (bearer_token(headers)?, selected_profile(headers)?) {
        (None, None) => Ok(None),
        (Some(_), Some(_)) => authenticated_actor(state, headers).map(Some),
        _ => Err(ApiError::unauthorized(
            "both bearer token and x-tardy-profile-id are required",
        )),
    }
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
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }
    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
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
            StoreError::ProfileNotFound
            | StoreError::LiveNotFound
            | StoreError::ThreadNotFound
            | StoreError::ShareNotFound => Self::not_found(value.to_string()),
            StoreError::Forbidden | StoreError::DirectMessagesClosed => {
                Self::forbidden(value.to_string())
            }
            StoreError::EmptyMessage => Self::bad_request(value.to_string()),
            StoreError::LiveEnded | StoreError::HandleConflict => Self {
                status: StatusCode::CONFLICT,
                message: value.to_string(),
            },
            StoreError::Poisoned => Self::internal(value.to_string()),
        }
    }
}

impl From<OnboardingError> for ApiError {
    fn from(value: OnboardingError) -> Self {
        match value {
            OnboardingError::InvalidClaim => Self::not_found(value.to_string()),
            OnboardingError::EmailConflict => Self {
                status: StatusCode::CONFLICT,
                message: value.to_string(),
            },
            OnboardingError::InvalidEmail => Self::bad_request(value.to_string()),
            OnboardingError::Database(_)
            | OnboardingError::Poisoned
            | OnboardingError::TimestampOverflow => Self::internal(value.to_string()),
        }
    }
}

impl From<MediaError> for ApiError {
    fn from(value: MediaError) -> Self {
        match value {
            MediaError::Unconfigured => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: value.to_string(),
            },
            MediaError::UnsupportedType
            | MediaError::InvalidSize(_)
            | MediaError::MetadataMismatch => Self::bad_request(value.to_string()),
            MediaError::NotFound => Self::not_found(value.to_string()),
            MediaError::Forbidden => Self::forbidden(value.to_string()),
            MediaError::ObjectStore(_) | MediaError::Poisoned | MediaError::TimestampOverflow => {
                Self::internal(value.to_string())
            }
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

    async fn request(
        app: &Router,
        method: &str,
        uri: &str,
        body: Value,
        actor: Option<&str>,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(CONTENT_TYPE, "application/json");
        if let Some(actor) = actor {
            request = request.header("x-tardy-profile-id", actor);
        }
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
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
        let state = Arc::new(AppState::in_memory("https://tardy.test").unwrap());
        let ticket = state.accounts.issue_claim(1).unwrap();
        let claimed = state
            .accounts
            .claim(&ticket.code, "agent@example.com", 2)
            .unwrap();
        let token = claimed.api_token;
        let app = router(state);
        let (_, profile) = request(
            &app,
            "POST",
            "/v1/profiles",
            json!({"handle":"build_agent","display_name":"Build Agent"}),
            None,
            Some(&token),
        )
        .await;
        let profile_id = profile["id"].as_str().unwrap();
        let (status, handoff) = request(
            &app,
            "POST",
            "/v1/agent-handoffs",
            json!({"target":"hermes","subject":{"kind":"profile","id":profile["id"]}}),
            Some(profile_id),
            Some(&token),
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

    #[tokio::test]
    async fn llms_txt_explains_private_agent_claim_flow() {
        let app = router(Arc::new(AppState::in_memory("https://tardy.test").unwrap()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/llms.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let text = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(text.contains("one-time code"));
        assert!(text.contains("default to private"));
        assert!(text.contains("https://tardy.test/v1/onboarding/claims"));
    }

    #[tokio::test]
    async fn one_account_cannot_act_as_another_accounts_profile() {
        let state = Arc::new(AppState::in_memory("https://tardy.test").unwrap());
        let first_ticket = state.accounts.issue_claim(1).unwrap();
        let first = state
            .accounts
            .claim(&first_ticket.code, "first@example.com", 2)
            .unwrap();
        let second_ticket = state.accounts.issue_claim(3).unwrap();
        let second = state
            .accounts
            .claim(&second_ticket.code, "second@example.com", 4)
            .unwrap();
        let app = router(state);
        let (_, profile) = request(
            &app,
            "POST",
            "/v1/profiles",
            json!({"handle":"first_agent","display_name":"First"}),
            None,
            Some(&first.api_token),
        )
        .await;
        let profile_id = profile["id"].as_str().unwrap();
        let (status, _) = request(
            &app,
            "POST",
            "/v1/agent-handoffs",
            json!({"target":"hermes","subject":{"kind":"profile","id":profile["id"]}}),
            Some(profile_id),
            Some(&second.api_token),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn metrics_use_bounded_route_templates() {
        let app = router(Arc::new(AppState::in_memory("https://tardy.test").unwrap()));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/profiles/not-a-real-profile")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let text = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(text.contains("route=\"/v1/profiles/{handle}\""));
        assert!(!text.contains("not-a-real-profile"));
    }
}
