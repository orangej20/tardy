use tardy::domain::{Reel, Visibility};
use tardy::subscriptions::{DeliveryMode, NewSubscription, PgSubscriptionStore, SubscriptionKind};
use uuid::Uuid;

#[tokio::test]
async fn hashtag_polling_and_webhooks_share_one_durable_event() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    sqlx::query("TRUNCATE webhook_deliveries,feed_subscriptions,feed_events CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    let store = PgSubscriptionStore::new(
        pool,
        "https://tardy.test".into(),
        Some(b"test-signing-key".to_vec()),
    );
    let account = Uuid::new_v4();
    let poll = store
        .create(
            account,
            NewSubscription {
                kind: SubscriptionKind::Hashtag,
                hashtag: Some("#Rust".into()),
                delivery: DeliveryMode::Poll,
                webhook_url: None,
            },
        )
        .await
        .unwrap();
    let webhook = store
        .create(
            account,
            NewSubscription {
                kind: SubscriptionKind::Hashtag,
                hashtag: Some("rust".into()),
                delivery: DeliveryMode::Webhook,
                webhook_url: Some("https://agent.test/tardy".into()),
            },
        )
        .await
        .unwrap();
    assert!(webhook.webhook_secret.is_some());
    let reel = Reel {
        id: Uuid::new_v4(),
        profile_id: Uuid::new_v4(),
        caption: "Shipping #Rust with #Agents".into(),
        media_url: "https://cdn.test/reel.mp4".into(),
        poster_url: None,
        duration_ms: 1000,
        visibility: Visibility::Public,
        published_at_ms: 1,
    };
    store.publish_reel(&reel).await.unwrap();
    store.publish_reel(&reel).await.unwrap();
    let events = store.poll(account, poll.id, 0, 50).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].hashtags, vec!["agents", "rust"]);
    let deliveries = store.claim_webhooks("worker", 10).await.unwrap();
    assert_eq!(deliveries.len(), 1);
    let body = serde_json::to_vec(&deliveries[0].event).unwrap();
    assert!(
        store
            .sign(webhook.id, &body)
            .unwrap()
            .starts_with("sha256=")
    );
    store
        .finish_webhook("worker", &deliveries[0], Some(204), None)
        .await
        .unwrap();
    assert!(
        store
            .claim_webhooks("worker-2", 10)
            .await
            .unwrap()
            .is_empty()
    );
}
