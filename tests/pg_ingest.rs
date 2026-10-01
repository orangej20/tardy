use chrono::Utc;
use sqlx::PgPool;
use std::collections::BTreeMap;
use tardy::ingest::{
    CarouselPlan, NormalizedItem, RightsMode, RightsPolicy, SourceDefinition, TransformPlan,
    Transport,
};
use tardy::pg_ingest::PgIngestStore;
use tardy::push::{
    ApnsEnvironment, NewNotification, NotificationPreference, PgPushStore, RegisterPushDevice,
};

#[tokio::test]
async fn polling_is_deduplicated_and_enqueues_exactly_once() {
    let Ok(database_url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    let pool = PgPool::connect(&database_url).await.unwrap();
    let store = PgIngestStore::connect(&database_url, 2).await.unwrap();
    store.migrate().await.unwrap();
    sqlx::query(
        "TRUNCATE push_deliveries, push_notifications, notification_preferences, push_devices,
         outbox, transformation_runs, source_items, source_channels RESTART IDENTITY CASCADE",
    )
    .execute(&pool)
    .await
    .unwrap();

    let source = source();
    store
        .sync_sources(std::slice::from_ref(&source))
        .await
        .unwrap();
    let claimed = store
        .claim_due_source("poller-a", 60)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.definition, source);

    let item = item();
    let plan = plan();
    let inserted = store
        .record_success(
            "poller-a",
            &source,
            &[(item.clone(), plan.clone())],
            Some("etag-1"),
            None,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(inserted, 1);

    sqlx::query("UPDATE source_channels SET next_poll_at=now() WHERE id=$1")
        .bind(&source.id)
        .execute(&pool)
        .await
        .unwrap();
    store
        .claim_due_source("poller-b", 60)
        .await
        .unwrap()
        .unwrap();
    let inserted = store
        .record_success("poller-b", &source, &[(item, plan)], None, None, Utc::now())
        .await
        .unwrap();
    assert_eq!(inserted, 0);

    let events = store.claim_outbox("publisher-a", 10).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].topic, "source.item_ingested.v1");
    store
        .mark_outbox_delivered("publisher-a", events[0].id)
        .await
        .unwrap();
    assert!(
        store
            .claim_outbox("publisher-b", 10)
            .await
            .unwrap()
            .is_empty()
    );

    let push = PgPushStore::connect(&database_url, 2).await.unwrap();
    let account_id = uuid::Uuid::new_v4();
    let device = push
        .register_device(
            account_id,
            RegisterPushDevice {
                token: "ab".repeat(32),
                environment: ApnsEnvironment::Sandbox,
                topic: "com.tardy.app".into(),
            },
        )
        .await
        .unwrap();
    push.set_preference(
        account_id,
        NotificationPreference {
            category: "hyper_tardy".into(),
            enabled: true,
        },
    )
    .await
    .unwrap();
    let source_event_id = uuid::Uuid::new_v4();
    let notification = NewNotification {
        source_event_id,
        account_id,
        category: "hyper_tardy".into(),
        title: "Hyper-Tardy".into(),
        body: "A post is breaking.".into(),
        deep_link: Some("tardy://posts/post-1".into()),
        data: serde_json::Map::new(),
    };
    assert_eq!(
        push.enqueue(notification.clone()).await.unwrap(),
        source_event_id
    );
    assert_eq!(push.enqueue(notification).await.unwrap(), source_event_id);
    let deliveries = push
        .claim_deliveries("push-a", ApnsEnvironment::Sandbox, "com.tardy.app", 10)
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].device_id, device.id);
    push.mark_delivered("push-a", deliveries[0].id, None)
        .await
        .unwrap();
}

fn source() -> SourceDefinition {
    SourceDefinition {
        id: "test-source".into(),
        display_name: "Test Source".into(),
        enabled: true,
        limit: 10,
        transport: Transport::Rss {
            url: "https://example.com/feed.xml".into(),
        },
        rights: RightsPolicy {
            mode: RightsMode::Facts,
            attribution: "Example".into(),
            commercial_use: true,
        },
        transform: "news".into(),
    }
}

fn item() -> NormalizedItem {
    NormalizedItem {
        source_id: "test-source".into(),
        external_id: "item-1".into(),
        title: "A durable event".into(),
        canonical_url: "https://example.com/item-1".into(),
        author: Some("Example".into()),
        published_at_ms: Some(1_700_000_000_000),
        summary: Some("Facts only".into()),
        facts: BTreeMap::new(),
    }
}

fn plan() -> TransformPlan {
    TransformPlan {
        headline: "A durable event".into(),
        attribution: "Example".into(),
        source_url: "https://example.com/item-1".into(),
        carousel: CarouselPlan {
            format: "headline_source_v1".into(),
            slides: vec!["A durable event".into()],
        },
        llm: None,
    }
}
