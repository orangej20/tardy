use crate::ads::{
    AdPaymentProcessor, AdsError, CampaignReport, FundingIntent, NewCampaign, PaymentRequired,
    PaymentRequirements, PgAdsStore, ResourceInfo, X402_VERSION,
};
use crate::domain::{
    AgentCapabilities, AgentHandoff, EngagementKind, LiveEventPayload, ProfilePrivacy,
    ShareSubject, Visibility,
};
use crate::media::{MediaError, MediaService, UploadIntent};
use crate::metrics::Metrics;
use crate::onboarding::{AccountRegistry, OnboardingError};
use crate::push::{NotificationPreference, PgPushStore, PushDevice, PushError, RegisterPushDevice};
use crate::ranking::LuaRanker;
use crate::search::{SearchError, SearchService};
use crate::store::{MemoryStore, NewLive, NewProfile, NewReel, Store, StoreError};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router, middleware};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use utoipa::ToSchema;
use uuid::Uuid;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub ranker: LuaRanker,
    pub public_base_url: String,
    pub accounts: Arc<AccountRegistry>,
    pub metrics: Arc<Metrics>,
    pub media: Arc<MediaService>,
    pub search: Arc<SearchService>,
    pub push: Option<Arc<PgPushStore>>,
    pub ads: Option<Arc<AdsRuntime>>,
}

pub struct AdsRuntime {
    pub store: PgAdsStore,
    pub processor: Arc<dyn AdPaymentProcessor>,
    pub requirement: PaymentRequirements,
    pub atomic_per_budget_micro: u64,
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
            search: Arc::new(SearchService::disabled()),
            push: None,
            ads: None,
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
            search: Arc::new(SearchService::from_env()?),
            push: None,
            ads: None,
        })
    }

    pub fn with_push_store(mut self, push: PgPushStore) -> Self {
        self.push = Some(Arc::new(push));
        self
    }

    pub fn with_ads(mut self, ads: AdsRuntime) -> Self {
        self.ads = Some(Arc::new(ads));
        self
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let metrics = state.metrics.clone();
    Router::new()
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/metrics", get(metrics_endpoint))
        .route("/openapi.json", get(openapi_endpoint))
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
        .route("/v1/reels/{id}/engagements", post(record_engagement))
        .route("/v1/saved-posts", get(list_saved_posts))
        .route("/v1/saved-posts/{id}", put(save_post).delete(unsave_post))
        .route(
            "/v1/ai-consents/search",
            post(grant_search_consent).delete(revoke_search_consent),
        )
        .route("/v1/search", post(search_posts))
        .route("/v1/explore", post(explore_posts))
        .route("/v1/lives", post(start_live))
        .route("/v1/lives/{id}/events", post(append_event).get(list_events))
        .route("/v1/lives/{id}/end", post(end_live))
        .route("/v1/feed", get(feed))
        .route("/v1/feed/hyper-tardy", get(hyper_tardy_feed))
        .route("/v1/agent-handoffs", post(agent_handoff))
        .route("/v1/push/devices", post(register_push_device))
        .route(
            "/v1/push/devices/{id}",
            axum::routing::delete(unregister_push_device),
        )
        .route("/v1/push/preferences", put(set_notification_preference))
        .route("/v1/ad-campaigns", post(create_ad_campaign))
        .route(
            "/v1/ad-campaigns/{id}/funding-intents",
            post(create_ad_funding_intent),
        )
        .route("/v1/ad-campaigns/{id}/report", get(ad_campaign_report))
        .route(
            "/v1/ad-funding-intents/{id}/settle",
            post(settle_ad_funding),
        )
        .with_state(state)
        .layer(middleware::from_fn(move |request, next| {
            crate::metrics::track(metrics.clone(), request, next)
        }))
}

async fn create_ad_campaign(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NewCampaign>,
) -> Result<(StatusCode, Json<Uuid>), ApiError> {
    let actor = authenticated_actor(&state, &headers)?;
    if actor != body.advertiser_profile_id {
        return Err(ApiError::forbidden(
            "advertiser profile must match selected profile",
        ));
    }
    let id = ads_runtime(&state)?.store.create_campaign(body).await?;
    Ok((StatusCode::CREATED, Json(id)))
}

async fn create_ad_funding_intent(
    State(state): State<Arc<AppState>>,
    Path(campaign_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<FundingIntent>), ApiError> {
    let actor = authenticated_actor(&state, &headers)?;
    let ads = ads_runtime(&state)?;
    let (owner, budget_micros) = ads.store.campaign_owner_budget(campaign_id).await?;
    if owner != actor {
        return Err(ApiError::forbidden("campaign is owned by another profile"));
    }
    let amount = u128::try_from(budget_micros)
        .ok()
        .and_then(|budget| budget.checked_mul(u128::from(ads.atomic_per_budget_micro)))
        .ok_or_else(|| {
            ApiError::bad_request("campaign budget cannot be represented by payment rate")
        })?;
    let mut requirement = ads.requirement.clone();
    requirement.amount = amount.to_string();
    let intent = ads
        .store
        .create_funding_intent(campaign_id, requirement, budget_micros)
        .await?;
    Ok((StatusCode::CREATED, Json(intent)))
}

async fn settle_ad_funding(
    State(state): State<Arc<AppState>>,
    Path(intent_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    use base64::Engine as _;
    let actor = authenticated_actor(&state, &headers)?;
    let ads = ads_runtime(&state)?;
    let intent = ads.store.funding_intent(intent_id).await?;
    let (owner, _) = ads.store.campaign_owner_budget(intent.campaign_id).await?;
    if owner != actor {
        return Err(ApiError::forbidden("campaign is owned by another profile"));
    }
    let Some(signature) = headers
        .get("payment-signature")
        .and_then(|value| value.to_str().ok())
    else {
        let required = PaymentRequired {
            x402_version: X402_VERSION,
            error: "PAYMENT-SIGNATURE header is required".into(),
            resource: ResourceInfo {
                url: format!(
                    "{}/v1/ad-funding-intents/{intent_id}/settle",
                    state.public_base_url
                ),
                description: format!("Fund Tardy ad campaign {}", intent.campaign_id),
                mime_type: "application/json".into(),
            },
            accepts: vec![intent.requirement],
            extensions: serde_json::Map::new(),
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            serde_json::to_vec(&required).map_err(|error| ApiError::internal(error.to_string()))?,
        );
        return Ok((
            StatusCode::PAYMENT_REQUIRED,
            [("payment-required", encoded)],
            Json(required),
        )
            .into_response());
    };
    let receipt = ads
        .store
        .settle_funding(intent_id, signature, ads.processor.as_ref())
        .await?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        serde_json::to_vec(&receipt).map_err(|error| ApiError::internal(error.to_string()))?,
    );
    Ok((
        StatusCode::OK,
        [("payment-response", encoded)],
        Json(receipt),
    )
        .into_response())
}

async fn ad_campaign_report(
    State(state): State<Arc<AppState>>,
    Path(campaign_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<CampaignReport>, ApiError> {
    let actor = authenticated_actor(&state, &headers)?;
    let ads = ads_runtime(&state)?;
    let (owner, _) = ads.store.campaign_owner_budget(campaign_id).await?;
    if owner != actor {
        return Err(ApiError::forbidden("campaign is owned by another profile"));
    }
    Ok(Json(ads.store.campaign_report(campaign_id).await?))
}

fn ads_runtime(state: &AppState) -> Result<&AdsRuntime, ApiError> {
    state.ads.as_deref().ok_or_else(|| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "ads payments are not configured".into(),
    })
}

async fn register_push_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RegisterPushDevice>,
) -> Result<(StatusCode, Json<PushDevice>), ApiError> {
    let account_id = authenticated_account(&state, &headers)?;
    let device = push_store(&state)?
        .register_device(account_id, body)
        .await?;
    Ok((StatusCode::CREATED, Json(device)))
}

async fn unregister_push_device(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    push_store(&state)?
        .unregister_device(authenticated_account(&state, &headers)?, id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_notification_preference(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NotificationPreference>,
) -> Result<Json<NotificationPreference>, ApiError> {
    let account_id = authenticated_account(&state, &headers)?;
    Ok(Json(
        push_store(&state)?.set_preference(account_id, body).await?,
    ))
}

fn push_store(state: &AppState) -> Result<&PgPushStore, ApiError> {
    state.push.as_deref().ok_or_else(|| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "push notifications are not configured".into(),
    })
}

async fn metrics_endpoint(State(state): State<Arc<AppState>>) -> Response {
    crate::metrics::response(&state.metrics)
}

async fn openapi_endpoint() -> Json<serde_json::Value> {
    Json(crate::openapi::document())
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateProfile {
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct PublishReel {
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct RecordEngagement {
    /// Stable client-generated UUID used to make retries idempotent.
    event_id: Uuid,
    kind: EngagementKind,
}

async fn record_engagement(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RecordEngagement>,
) -> Result<impl IntoResponse, ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(state.store.record_engagement(
            authenticated_actor(&state, &headers)?,
            id,
            body.event_id,
            body.kind,
            now_ms()?,
        )?),
    ))
}

async fn save_post(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let account_id = authenticated_account(&state, &headers)?;
    let viewer_id = authenticated_actor(&state, &headers)?;
    Ok(Json(state.store.save_post(
        account_id,
        viewer_id,
        id,
        now_ms()?,
    )?))
}

async fn unsave_post(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .unsave_post(authenticated_account(&state, &headers)?, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_saved_posts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let account_id = authenticated_account(&state, &headers)?;
    let viewer_id = authenticated_actor(&state, &headers)?;
    Ok(Json(state.store.saved_posts(account_id, viewer_id)?))
}

const SEARCH_CONSENT_PURPOSE: &str = "search_reranking";
const SEARCH_CONSENT_POLICY: &str = "search-v1";

async fn grant_search_consent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let account_id = authenticated_account(&state, &headers)?;
    let provider = state.search.provider().ok_or(SearchError::Unavailable)?;
    Ok(Json(state.accounts.grant_ai_consent(
        account_id,
        provider,
        SEARCH_CONSENT_PURPOSE,
        SEARCH_CONSENT_POLICY,
        now_ms()?,
    )?))
}

async fn revoke_search_consent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let account_id = authenticated_account(&state, &headers)?;
    if let Some(provider) = state.search.provider() {
        state.accounts.revoke_ai_consent(
            account_id,
            provider,
            SEARCH_CONSENT_PURPOSE,
            now_ms()?,
        )?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct SearchRequest {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

async fn search_posts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SearchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    run_search(&state, &headers, &body.query, body.limit).await
}

async fn explore_posts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SearchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let query = format!(
        "Find timely, substantive open-source and AI project updates about: {}",
        body.query
    );
    run_search(&state, &headers, &query, body.limit).await
}

async fn run_search(
    state: &AppState,
    headers: &HeaderMap,
    query: &str,
    limit: usize,
) -> Result<Json<Vec<crate::search::SearchResult>>, ApiError> {
    if !(1..=50).contains(&limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 50"));
    }
    let account_id = authenticated_account(state, headers)?;
    let provider = state.search.provider().ok_or(SearchError::Unavailable)?;
    if !state.accounts.has_ai_consent(
        account_id,
        provider,
        SEARCH_CONSENT_PURPOSE,
        SEARCH_CONSENT_POLICY,
    )? {
        return Err(ApiError::forbidden(
            "explicit search AI consent is required",
        ));
    }
    let candidates = state.store.feed_candidates(None)?;
    Ok(Json(state.search.search(query, candidates, limit).await?))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct StartLive {
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

async fn hyper_tardy_feed(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<FeedQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !(1..=100).contains(&query.limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 100"));
    }
    Ok(Json(state.store.hyper_tardy(
        optional_authenticated_actor(&state, &headers)?,
        now_ms()?,
        query.limit,
    )?))
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct HandoffRequest {
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateThread {
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct SendMessage {
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateShare {
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

#[derive(Deserialize, ToSchema)]
pub(crate) struct ClaimAgentCode {
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

#[derive(Serialize, ToSchema)]
pub(crate) struct ErrorBody {
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
            | StoreError::ReelNotFound
            | StoreError::LiveNotFound
            | StoreError::ThreadNotFound
            | StoreError::ShareNotFound => Self::not_found(value.to_string()),
            StoreError::Forbidden | StoreError::DirectMessagesClosed => {
                Self::forbidden(value.to_string())
            }
            StoreError::EmptyMessage => Self::bad_request(value.to_string()),
            StoreError::IdempotencyConflict => Self {
                status: StatusCode::CONFLICT,
                message: value.to_string(),
            },
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

impl From<SearchError> for ApiError {
    fn from(value: SearchError) -> Self {
        match value {
            SearchError::Unavailable => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: value.to_string(),
            },
            SearchError::EmptyQuery => Self::bad_request(value.to_string()),
            SearchError::Provider(_) | SearchError::InvalidResult => Self {
                status: StatusCode::BAD_GATEWAY,
                message: value.to_string(),
            },
        }
    }
}

impl From<PushError> for ApiError {
    fn from(value: PushError) -> Self {
        match value {
            PushError::DeviceToken | PushError::InvalidName => Self::bad_request(value.to_string()),
            PushError::LeaseLost => Self::not_found(value.to_string()),
            PushError::Database(_)
            | PushError::Key(_)
            | PushError::PayloadTooLarge
            | PushError::Transport(_)
            | PushError::Rejected { .. }
            | PushError::TokenCache => Self::internal(value.to_string()),
        }
    }
}

impl From<AdsError> for ApiError {
    fn from(value: AdsError) -> Self {
        match value {
            AdsError::Validation(_) => Self::bad_request(value.to_string()),
            AdsError::Conflict(_) => Self {
                status: StatusCode::CONFLICT,
                message: value.to_string(),
            },
            AdsError::Payment(_) => Self {
                status: StatusCode::PAYMENT_REQUIRED,
                message: value.to_string(),
            },
            AdsError::Database(sqlx::Error::RowNotFound) => {
                Self::not_found("ads resource not found")
            }
            AdsError::Database(_) => Self::internal(value.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{RankedDocument, Reranker, SearchDocument};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, header::CONTENT_TYPE};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    struct TestReranker;

    #[async_trait::async_trait]
    impl Reranker for TestReranker {
        fn provider(&self) -> &'static str {
            "test-provider"
        }

        async fn rerank(
            &self,
            _query: &str,
            documents: &[SearchDocument],
            limit: usize,
        ) -> Result<Vec<RankedDocument>, SearchError> {
            Ok((0..documents.len().min(limit))
                .map(|index| RankedDocument {
                    index,
                    score: 1.0 - index as f64 / 100.0,
                })
                .collect())
        }
    }

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
    async fn serves_the_generated_openapi_contract() {
        let app = router(Arc::new(AppState::in_memory("https://tardy.test").unwrap()));
        let (status, document) =
            request(&app, "GET", "/openapi.json", Value::Null, None, None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(document["openapi"], "3.1.0");
        assert_eq!(
            document["paths"]["/v1/uploads"]["post"]["operationId"],
            "authorizeUpload"
        );
    }

    #[tokio::test]
    async fn saved_posts_are_private_and_search_requires_revocable_consent() {
        let mut state = AppState::in_memory("https://tardy.test").unwrap();
        state.search = Arc::new(SearchService::with_reranker(Arc::new(TestReranker)));
        let state = Arc::new(state);
        let ticket = state.accounts.issue_claim(1).unwrap();
        let claimed = state
            .accounts
            .claim(&ticket.code, "reader@example.com", 2)
            .unwrap();
        let token = claimed.api_token;
        let profile = state
            .store
            .create_profile(NewProfile {
                handle: "reader".into(),
                display_name: "Reader".into(),
                bio: String::new(),
                privacy: ProfilePrivacy::default(),
                created_at_ms: 3,
            })
            .unwrap();
        state
            .accounts
            .bind_profile(claimed.account.id, profile.id)
            .unwrap();
        let reel = state
            .store
            .publish_reel(
                profile.id,
                NewReel {
                    profile_id: profile.id,
                    caption: "Rust search".into(),
                    media_url: "https://media.test/reel.mp4".into(),
                    poster_url: None,
                    duration_ms: 10,
                    visibility: Visibility::Public,
                    published_at_ms: 4,
                },
            )
            .unwrap();
        let app = router(state);
        let profile_id = profile.id.to_string();

        let (status, saved) = request(
            &app,
            "PUT",
            &format!("/v1/saved-posts/{}", reel.id),
            Value::Null,
            Some(&profile_id),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(saved["reel"]["id"], reel.id.to_string());

        let (status, _) = request(
            &app,
            "POST",
            "/v1/search",
            json!({"query":"Rust", "limit":10}),
            None,
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = request(
            &app,
            "POST",
            "/v1/ai-consents/search",
            Value::Null,
            None,
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, results) = request(
            &app,
            "POST",
            "/v1/search",
            json!({"query":"Rust", "limit":10}),
            None,
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(results[0]["item"]["id"], reel.id.to_string());

        let (status, _) = request(
            &app,
            "DELETE",
            "/v1/ai-consents/search",
            Value::Null,
            None,
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = request(
            &app,
            "POST",
            "/v1/search",
            json!({"query":"Rust", "limit":10}),
            None,
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
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
