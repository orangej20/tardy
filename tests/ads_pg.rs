use tardy::ads::{
    AdEventKind, AdPaymentProcessor, AttributionModel, NewAdEvent, NewCampaign,
    NewCreatorPartnership, PaymentError, PaymentRequirements, PgAdsStore, Settlement,
};
use uuid::Uuid;

static DATABASE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn immutable_events_are_idempotent_and_reports_show_partner_roas() {
    let _guard = DATABASE_TEST_LOCK.lock().await;
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    sqlx::query("TRUNCATE ad_events, creator_partnerships, ad_creatives, ad_campaigns CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    let store = PgAdsStore::new(pool.clone());
    let creator = Uuid::new_v4();
    let campaign = store
        .create_campaign(NewCampaign {
            advertiser_profile_id: Uuid::new_v4(),
            name: "Transparent launch".into(),
            currency: "USD".into(),
            budget_micros: 10_000_000,
            attribution_window_seconds: 86_400,
            attribution_model: AttributionModel::LastTouch,
            boosted_reel_id: Some(Uuid::new_v4()),
        })
        .await
        .unwrap();
    store
        .add_creator_partnership(NewCreatorPartnership {
            campaign_id: campaign,
            creator_profile_id: creator,
            revenue_share_bps: 2_000,
        })
        .await
        .unwrap();

    let conversion = NewAdEvent {
        campaign_id: campaign,
        creative_id: None,
        creator_profile_id: Some(creator),
        kind: AdEventKind::Conversion,
        idempotency_key: "checkout/order-42".into(),
        occurred_at: chrono::Utc::now(),
        revenue_micros: 5_000_000,
        spend_micros: 0,
        attributed_touch_event_id: None,
        metadata: serde_json::json!({"order": "42"}),
    };
    let first = store.record_event(conversion.clone()).await.unwrap();
    assert_eq!(first, store.record_event(conversion).await.unwrap());
    store
        .record_event(NewAdEvent {
            campaign_id: campaign,
            creative_id: None,
            creator_profile_id: None,
            kind: AdEventKind::Spend,
            idempotency_key: "settlement/1".into(),
            occurred_at: chrono::Utc::now(),
            revenue_micros: 0,
            spend_micros: 2_000_000,
            attributed_touch_event_id: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let report = store.campaign_report(campaign).await.unwrap();
    assert_eq!(report.conversions, 1);
    assert_eq!(report.spend_micros, 2_000_000);
    assert_eq!(report.attributed_revenue_micros, 5_000_000);
    assert_eq!(report.creator_earnings_micros, 1_000_000);
    assert_eq!(report.roas(), Some(2.5));

    let mutation = sqlx::query("UPDATE ad_events SET revenue_micros=0 WHERE id=$1")
        .bind(first)
        .execute(&pool)
        .await;
    assert!(mutation.is_err());
}

struct MockFacilitator {
    alter_binding: bool,
}

#[async_trait::async_trait]
impl AdPaymentProcessor for MockFacilitator {
    async fn verify_and_settle(
        &self,
        idempotency_key: Uuid,
        signature: &str,
        requirement: &PaymentRequirements,
    ) -> Result<Settlement, PaymentError> {
        if signature != "facilitator-verifiable-signature" {
            return Err(PaymentError::Verification("invalid signature".into()));
        }
        Ok(Settlement {
            transaction: format!("tx-{idempotency_key}"),
            network: requirement.network.clone(),
            payer: "payer".into(),
            amount: requirement.amount.clone(),
            asset: requirement.asset.clone(),
            pay_to: if self.alter_binding {
                "attacker".into()
            } else {
                requirement.pay_to.clone()
            },
        })
    }
}

#[tokio::test]
async fn x402_settlement_is_bound_durable_and_idempotent() {
    let _guard = DATABASE_TEST_LOCK.lock().await;
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    sqlx::query("TRUNCATE ad_payment_receipts, ad_funding_intents, ad_events, creator_partnerships, ad_creatives, ad_campaigns CASCADE")
        .execute(&pool).await.unwrap();
    let store = PgAdsStore::new(pool.clone());
    let campaign = store
        .create_campaign(NewCampaign {
            advertiser_profile_id: Uuid::new_v4(),
            name: "Boost".into(),
            currency: "USD".into(),
            budget_micros: 9_000_000,
            attribution_window_seconds: 86_400,
            attribution_model: AttributionModel::LastTouch,
            boosted_reel_id: Some(Uuid::new_v4()),
        })
        .await
        .unwrap();
    let requirement = PaymentRequirements {
        scheme: "exact".into(),
        network: "eip155:8453".into(),
        amount: "10000".into(),
        asset: "USDC".into(),
        pay_to: "tardy-wallet".into(),
        max_timeout_seconds: 60,
        extra: serde_json::Map::new(),
    };
    let intent = store
        .create_funding_intent(campaign, requirement, 4_000_000)
        .await
        .unwrap();

    assert!(
        store
            .settle_funding(
                intent.id,
                "just-a-signature",
                &MockFacilitator {
                    alter_binding: false
                }
            )
            .await
            .is_err()
    );
    let before: (String, i64) =
        sqlx::query_as("SELECT status,funded_micros FROM ad_campaigns WHERE id=$1")
            .bind(campaign)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, ("draft".into(), 0));
    assert!(
        store
            .settle_funding(
                intent.id,
                "facilitator-verifiable-signature",
                &MockFacilitator {
                    alter_binding: true
                }
            )
            .await
            .is_err()
    );

    let receipt = store
        .settle_funding(
            intent.id,
            "facilitator-verifiable-signature",
            &MockFacilitator {
                alter_binding: false,
            },
        )
        .await
        .unwrap();
    let replay = store
        .settle_funding(
            intent.id,
            "anything",
            &MockFacilitator {
                alter_binding: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(receipt, replay);
    let after: (String, i64) =
        sqlx::query_as("SELECT status,funded_micros FROM ad_campaigns WHERE id=$1")
            .bind(campaign)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, ("active".into(), 4_000_000));
}
