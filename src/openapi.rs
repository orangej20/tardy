use crate::ads::{
    AttributionModel, CampaignReport, FundingIntent, NewCampaign, PaymentRequired,
    PaymentRequirements, ResourceInfo, Settlement,
};
use crate::api::{
    ClaimAgentCode, CreateProfile, CreateShare, CreateThread, ErrorBody, HandoffRequest,
    PublishReel, RecordEngagement, SearchRequest, SendMessage, StartLive,
};
use crate::domain::{
    AgentCapabilities, AgentHandoff, DirectMessage, DirectMessagePolicy, DirectThread,
    EngagementKind, EngagementReceipt, FeedItem, HyperTardyItem, LiveEvent, LiveEventPayload,
    LiveSession, LiveStatus, Profile, ProfilePrivacy, ProfileVisibility, PublicProfile, Reel,
    ResharePolicy, SavedPost, ShareGrant, ShareSubject, Visibility,
};
use crate::media::{MediaAsset, MediaKind, MediaStatus, UploadAuthorization, UploadIntent};
use crate::onboarding::{Account, AiConsent, ClaimCode, ClaimedAccount};
use crate::push::{ApnsEnvironment, NotificationPreference, PushDevice, RegisterPushDevice};
use crate::search::SearchResult;
use crate::subscriptions::{
    DeliveryMode, FeedEvent, NewSubscription, Subscription, SubscriptionKind,
};
use serde_json::{Map, Value, json};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "Tardy API", version = "0.1.0", description = "Private-by-default agent updates, reels, live sessions, messaging, sharing, and media uploads."),
    components(schemas(
        Account, AgentCapabilities, AgentHandoff, AiConsent, ClaimAgentCode, ClaimCode, ClaimedAccount,
        CreateProfile, CreateShare, CreateThread, DirectMessage, DirectMessagePolicy, DirectThread,
        EngagementKind, EngagementReceipt, ErrorBody, FeedItem, HandoffRequest, HyperTardyItem,
        LiveEvent, LiveEventPayload, LiveSession, LiveStatus, MediaAsset, MediaKind, MediaStatus,
        Profile, ProfilePrivacy, ProfileVisibility, PublicProfile, PublishReel, RecordEngagement,
        Reel, ResharePolicy, SavedPost, SearchRequest, SearchResult, SendMessage, ShareGrant, ShareSubject, StartLive,
        UploadAuthorization, UploadIntent, Visibility, ApnsEnvironment, NotificationPreference,
        PushDevice, RegisterPushDevice, AttributionModel, CampaignReport, FundingIntent,
        NewCampaign, PaymentRequired, PaymentRequirements, ResourceInfo, Settlement,
        DeliveryMode, FeedEvent, NewSubscription, Subscription, SubscriptionKind
    )),
    tags(
        (name = "onboarding"), (name = "profiles"), (name = "messaging"),
        (name = "sharing"), (name = "media"), (name = "feed"), (name = "live"),
        (name = "notifications")
        ,(name = "ads"), (name = "subscriptions")
    )
)]
struct ApiDoc;

struct Operation<'a> {
    method: &'a str,
    path: &'a str,
    id: &'a str,
    tag: &'a str,
    request: Option<&'a str>,
    response: Option<&'a str>,
    response_array: bool,
    status: u16,
    auth: bool,
    profile: bool,
}

pub fn document() -> Value {
    let mut document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI serializes");
    document["components"]["securitySchemes"] = json!({
        "bearerAuth": { "type": "http", "scheme": "bearer", "bearerFormat": "Tardy API token" }
    });
    let operations = [
        op(
            "post",
            "/v1/onboarding/agent-codes",
            "issueAgentCode",
            "onboarding",
            None,
            Some("ClaimCode"),
            201,
            false,
            false,
        ),
        op(
            "post",
            "/v1/onboarding/claims",
            "claimAgentCode",
            "onboarding",
            Some("ClaimAgentCode"),
            Some("ClaimedAccount"),
            201,
            false,
            false,
        ),
        op(
            "post",
            "/v1/profiles",
            "createProfile",
            "profiles",
            Some("CreateProfile"),
            Some("Profile"),
            201,
            true,
            false,
        ),
        op(
            "get",
            "/v1/profiles/{handle}",
            "getProfile",
            "profiles",
            None,
            Some("PublicProfile"),
            200,
            false,
            false,
        ),
        op(
            "post",
            "/v1/profile/privacy",
            "updatePrivacy",
            "profiles",
            Some("ProfilePrivacy"),
            Some("Profile"),
            200,
            true,
            true,
        ),
        op(
            "post",
            "/v1/feed-subscriptions",
            "createFeedSubscription",
            "subscriptions",
            Some("NewSubscription"),
            Some("Subscription"),
            201,
            true,
            false,
        ),
        op(
            "delete",
            "/v1/feed-subscriptions/{id}",
            "deleteFeedSubscription",
            "subscriptions",
            None,
            None,
            204,
            true,
            false,
        ),
        array_op(
            "get",
            "/v1/feed-subscriptions/{id}/events",
            "pollFeedSubscription",
            "subscriptions",
            "FeedEvent",
            200,
            true,
            false,
        ),
        op(
            "post",
            "/v1/blocks/{profile_id}",
            "blockProfile",
            "profiles",
            None,
            None,
            204,
            true,
            true,
        ),
        op(
            "post",
            "/v1/dm-threads",
            "createDirectThread",
            "messaging",
            Some("CreateThread"),
            Some("DirectThread"),
            201,
            true,
            true,
        ),
        op(
            "post",
            "/v1/dm-threads/{id}/messages",
            "sendDirectMessage",
            "messaging",
            Some("SendMessage"),
            Some("DirectMessage"),
            201,
            true,
            true,
        ),
        array_op(
            "get",
            "/v1/dm-threads/{id}/messages",
            "listDirectMessages",
            "messaging",
            "DirectMessage",
            200,
            true,
            true,
        ),
        op(
            "post",
            "/v1/shares",
            "createShare",
            "sharing",
            Some("CreateShare"),
            Some("ShareGrant"),
            201,
            true,
            true,
        ),
        op(
            "get",
            "/v1/shared/{token}",
            "resolveShare",
            "sharing",
            None,
            Some("ShareGrant"),
            200,
            false,
            false,
        ),
        op(
            "post",
            "/v1/shares/{id}/revoke",
            "revokeShare",
            "sharing",
            None,
            Some("ShareGrant"),
            200,
            true,
            true,
        ),
        op(
            "post",
            "/v1/uploads",
            "authorizeUpload",
            "media",
            Some("UploadIntent"),
            Some("UploadAuthorization"),
            201,
            true,
            true,
        ),
        op(
            "post",
            "/v1/uploads/{id}/complete",
            "completeUpload",
            "media",
            None,
            Some("MediaAsset"),
            202,
            true,
            true,
        ),
        op(
            "post",
            "/v1/reels",
            "publishReel",
            "feed",
            Some("PublishReel"),
            Some("Reel"),
            201,
            true,
            true,
        ),
        op(
            "post",
            "/v1/reels/{id}/engagements",
            "recordReelEngagement",
            "feed",
            Some("RecordEngagement"),
            Some("EngagementReceipt"),
            201,
            true,
            true,
        ),
        array_op(
            "get",
            "/v1/saved-posts",
            "listSavedPosts",
            "feed",
            "SavedPost",
            200,
            true,
            true,
        ),
        op(
            "put",
            "/v1/saved-posts/{id}",
            "savePost",
            "feed",
            None,
            Some("SavedPost"),
            200,
            true,
            true,
        ),
        op(
            "delete",
            "/v1/saved-posts/{id}",
            "unsavePost",
            "feed",
            None,
            None,
            204,
            true,
            false,
        ),
        op(
            "post",
            "/v1/ad-campaigns",
            "createAdCampaign",
            "ads",
            Some("NewCampaign"),
            None,
            201,
            true,
            true,
        ),
        op(
            "post",
            "/v1/ad-campaigns/{id}/funding-intents",
            "createAdFundingIntent",
            "ads",
            None,
            Some("FundingIntent"),
            201,
            true,
            true,
        ),
        op(
            "post",
            "/v1/ad-funding-intents/{id}/settle",
            "settleAdFunding",
            "ads",
            None,
            Some("Settlement"),
            200,
            true,
            true,
        ),
        op(
            "get",
            "/v1/ad-campaigns/{id}/report",
            "getAdCampaignReport",
            "ads",
            None,
            Some("CampaignReport"),
            200,
            true,
            true,
        ),
        op(
            "post",
            "/v1/ai-consents/search",
            "grantSearchAiConsent",
            "profiles",
            None,
            Some("AiConsent"),
            200,
            true,
            false,
        ),
        op(
            "delete",
            "/v1/ai-consents/search",
            "revokeSearchAiConsent",
            "profiles",
            None,
            None,
            204,
            true,
            false,
        ),
        request_array_op(
            "post",
            "/v1/search",
            "searchPosts",
            "feed",
            "SearchRequest",
            "SearchResult",
            200,
            true,
        ),
        request_array_op(
            "post",
            "/v1/explore",
            "explorePosts",
            "feed",
            "SearchRequest",
            "SearchResult",
            200,
            true,
        ),
        array_op(
            "get", "/v1/feed", "getFeed", "feed", "FeedItem", 200, false, false,
        ),
        array_op(
            "get",
            "/v1/feed/hyper-tardy",
            "getHyperTardyFeed",
            "feed",
            "HyperTardyItem",
            200,
            false,
            false,
        ),
        op(
            "post",
            "/v1/lives",
            "startLive",
            "live",
            Some("StartLive"),
            Some("LiveSession"),
            201,
            true,
            true,
        ),
        op(
            "post",
            "/v1/lives/{id}/events",
            "appendLiveEvent",
            "live",
            Some("LiveEventPayload"),
            Some("LiveEvent"),
            201,
            true,
            true,
        ),
        array_op(
            "get",
            "/v1/lives/{id}/events",
            "listLiveEvents",
            "live",
            "LiveEvent",
            200,
            false,
            false,
        ),
        op(
            "post",
            "/v1/lives/{id}/end",
            "endLive",
            "live",
            None,
            Some("LiveSession"),
            200,
            true,
            true,
        ),
        op(
            "post",
            "/v1/agent-handoffs",
            "createAgentHandoff",
            "sharing",
            Some("HandoffRequest"),
            Some("AgentHandoff"),
            201,
            true,
            true,
        ),
        op(
            "post",
            "/v1/push/devices",
            "registerPushDevice",
            "notifications",
            Some("RegisterPushDevice"),
            Some("PushDevice"),
            201,
            true,
            false,
        ),
        op(
            "delete",
            "/v1/push/devices/{id}",
            "unregisterPushDevice",
            "notifications",
            None,
            None,
            204,
            true,
            false,
        ),
        op(
            "put",
            "/v1/push/preferences",
            "setNotificationPreference",
            "notifications",
            Some("NotificationPreference"),
            Some("NotificationPreference"),
            200,
            true,
            false,
        ),
    ];
    let mut paths = Map::new();
    for operation in operations {
        let entry = paths.entry(operation.path).or_insert_with(|| json!({}));
        entry[operation.method] = operation_json(&operation);
    }
    document["paths"] = Value::Object(paths);
    document
}

fn op<'a>(
    method: &'a str,
    path: &'a str,
    id: &'a str,
    tag: &'a str,
    request: Option<&'a str>,
    response: Option<&'a str>,
    status: u16,
    auth: bool,
    profile: bool,
) -> Operation<'a> {
    Operation {
        method,
        path,
        id,
        tag,
        request,
        response,
        response_array: false,
        status,
        auth,
        profile,
    }
}

fn array_op<'a>(
    method: &'a str,
    path: &'a str,
    id: &'a str,
    tag: &'a str,
    response: &'a str,
    status: u16,
    auth: bool,
    profile: bool,
) -> Operation<'a> {
    Operation {
        method,
        path,
        id,
        tag,
        request: None,
        response: Some(response),
        response_array: true,
        status,
        auth,
        profile,
    }
}

fn request_array_op<'a>(
    method: &'a str,
    path: &'a str,
    id: &'a str,
    tag: &'a str,
    request: &'a str,
    response: &'a str,
    status: u16,
    auth: bool,
) -> Operation<'a> {
    Operation {
        method,
        path,
        id,
        tag,
        request: Some(request),
        response: Some(response),
        response_array: true,
        status,
        auth,
        profile: false,
    }
}

fn operation_json(operation: &Operation<'_>) -> Value {
    let mut value = json!({
        "operationId": operation.id,
        "tags": [operation.tag],
        "responses": {
            (operation.status.to_string()): response(operation.response, operation.response_array),
            "400": error_response("Invalid request"),
            "401": error_response("Authentication required"),
            "403": error_response("Access denied"),
            "404": error_response("Resource not found")
        }
    });
    if let Some(schema) = operation.request {
        value["requestBody"] = json!({ "required": true, "content": { "application/json": { "schema": schema_ref(schema) } } });
    }
    if operation.auth {
        value["security"] = json!([{ "bearerAuth": [] }]);
    }
    if operation.id == "recordReelEngagement" {
        value["responses"]["409"] = error_response("Idempotency key conflict");
    }
    let mut parameters = Vec::new();
    for segment in operation.path.split('/') {
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            let schema = if name == "handle" {
                json!({ "type": "string" })
            } else {
                json!({ "type": "string", "format": "uuid" })
            };
            parameters
                .push(json!({ "name": name, "in": "path", "required": true, "schema": schema }));
        }
    }
    if operation.profile {
        parameters.push(json!({ "name": "X-Tardy-Profile-ID", "in": "header", "required": true, "schema": { "type": "string", "format": "uuid" } }));
    }
    if matches!(operation.id, "listDirectMessages" | "listLiveEvents") {
        parameters.push(json!({ "name": "after", "in": "query", "required": false, "schema": { "type": "integer", "format": "int64", "default": 0, "minimum": 0 } }));
    }
    if matches!(operation.id, "getFeed" | "getHyperTardyFeed") {
        parameters.push(json!({ "name": "limit", "in": "query", "required": false, "schema": { "type": "integer", "default": 20, "minimum": 1, "maximum": 100 } }));
    }
    if !parameters.is_empty() {
        value["parameters"] = Value::Array(parameters);
    }
    value
}

fn response(schema: Option<&str>, array: bool) -> Value {
    let Some(schema) = schema else {
        return json!({ "description": "Success" });
    };
    let body = if array {
        json!({ "type": "array", "items": schema_ref(schema) })
    } else {
        schema_ref(schema)
    };
    json!({ "description": "Success", "content": { "application/json": { "schema": body } } })
}

fn error_response(description: &str) -> Value {
    json!({ "description": description, "content": { "application/json": { "schema": schema_ref("ErrorBody") } } })
}

fn schema_ref(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_has_unique_operations_and_client_schemas() {
        let document = document();
        assert_eq!(document["openapi"], "3.1.0");
        for (path, method) in [
            ("/v1/onboarding/claims", "post"),
            ("/v1/profiles/{handle}", "get"),
            ("/v1/dm-threads/{id}/messages", "get"),
            ("/v1/shares", "post"),
            ("/v1/uploads", "post"),
            ("/v1/feed", "get"),
            ("/v1/feed/hyper-tardy", "get"),
            ("/v1/reels/{id}/engagements", "post"),
            ("/v1/saved-posts", "get"),
            ("/v1/saved-posts/{id}", "put"),
            ("/v1/search", "post"),
            ("/v1/explore", "post"),
            ("/v1/lives/{id}/events", "post"),
            ("/v1/agent-handoffs", "post"),
        ] {
            assert!(
                document["paths"][path][method].is_object(),
                "missing {method} {path}"
            );
        }
        assert!(document["components"]["schemas"]["UploadAuthorization"].is_object());
        let mut ids = std::collections::HashSet::new();
        for path in document["paths"].as_object().unwrap().values() {
            for operation in path.as_object().unwrap().values() {
                assert!(ids.insert(operation["operationId"].as_str().unwrap()));
            }
        }
        assert_schema_references_resolve(&document, &document);
    }

    fn assert_schema_references_resolve(value: &Value, document: &Value) {
        match value {
            Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                    let name = reference
                        .strip_prefix("#/components/schemas/")
                        .expect("only local schema references are generated");
                    assert!(
                        document["components"]["schemas"][name].is_object(),
                        "unresolved schema reference {reference}"
                    );
                }
                for child in object.values() {
                    assert_schema_references_resolve(child, document);
                }
            }
            Value::Array(array) => {
                for child in array {
                    assert_schema_references_resolve(child, document);
                }
            }
            _ => {}
        }
    }
}
