use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub const X402_VERSION: u8 = 2;
pub const PAYMENT_REQUIRED_HEADER: &str = "PAYMENT-REQUIRED";
pub const PAYMENT_SIGNATURE_HEADER: &str = "PAYMENT-SIGNATURE";
pub const PAYMENT_RESPONSE_HEADER: &str = "PAYMENT-RESPONSE";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdCampaign {
    pub id: Uuid,
    pub advertiser_profile_id: Uuid,
    pub creative_reel_id: Uuid,
    pub destination_url: String,
    pub labels: Vec<String>,
    pub impressions: u64,
    pub status: AdStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdStatus {
    AwaitingPayment,
    Active,
    Complete,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired {
    pub x402_version: u8,
    pub error: String,
    pub resource: ResourceInfo,
    pub accepts: Vec<PaymentRequirements>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    pub url: String,
    pub description: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements {
    pub scheme: String,
    /// CAIP-2 network identifier.
    pub network: String,
    /// Atomic token units, represented as a string by x402.
    pub amount: String,
    pub asset: String,
    pub pay_to: String,
    pub max_timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdRate {
    pub price_per_thousand_impressions_atomic: u64,
    pub requirements: PaymentRequirements,
}

impl AdRate {
    pub fn quote(&self, campaign_url: String, impressions: u64) -> PaymentRequired {
        let units = impressions.div_ceil(1_000);
        let mut accepted = self.requirements.clone();
        accepted.amount = self
            .price_per_thousand_impressions_atomic
            .saturating_mul(units)
            .to_string();
        PaymentRequired {
            x402_version: X402_VERSION,
            error: format!("{PAYMENT_SIGNATURE_HEADER} header is required"),
            resource: ResourceInfo {
                url: campaign_url,
                description: format!("Activate a Tardy campaign for {impressions} impressions"),
                mime_type: "application/json".into(),
            },
            accepts: vec![accepted],
            extensions: Map::new(),
        }
    }
}

/// Settlement stays behind this narrow boundary. Implementations must call an
/// x402 facilitator's `/verify` and `/settle`; a signature alone never activates an ad.
#[async_trait::async_trait]
pub trait AdPaymentProcessor: Send + Sync {
    async fn verify_and_settle(
        &self,
        idempotency_key: Uuid,
        payment_signature: &str,
        requirement: &PaymentRequirements,
    ) -> Result<Settlement, PaymentError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub transaction: String,
    pub network: String,
    pub payer: String,
    pub amount: String,
    pub asset: String,
    pub pay_to: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PaymentError {
    #[error("payment verification failed: {0}")]
    Verification(String),
    #[error("payment settlement failed: {0}")]
    Settlement(String),
}

/// Money is always stored in integer micros of the campaign's ISO-4217 currency.
/// This avoids floating-point drift and keeps provider settlement behind a boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCampaign {
    pub advertiser_profile_id: Uuid,
    pub name: String,
    pub currency: String,
    pub budget_micros: i64,
    pub attribution_window_seconds: i64,
    pub attribution_model: AttributionModel,
    pub boosted_reel_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionModel {
    LastTouch,
    FirstTouch,
}

impl AttributionModel {
    fn as_str(self) -> &'static str {
        match self {
            Self::LastTouch => "last_touch",
            Self::FirstTouch => "first_touch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCreative {
    pub campaign_id: Uuid,
    pub reel_id: Uuid,
    pub destination_url: String,
    pub disclosure: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCreatorPartnership {
    pub campaign_id: Uuid,
    pub creator_profile_id: Uuid,
    /// Creator share in basis points (10_000 = 100%).
    pub revenue_share_bps: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdEventKind {
    Impression,
    QualifiedView,
    Click,
    Conversion,
    Refund,
    Spend,
}

impl AdEventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Impression => "impression",
            Self::QualifiedView => "qualified_view",
            Self::Click => "click",
            Self::Conversion => "conversion",
            Self::Refund => "refund",
            Self::Spend => "spend",
        }
    }
}

/// `idempotency_key` is supplied by the event producer and is unique per campaign.
/// Conversion/refund values are signed through `revenue_micros`; spend is positive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewAdEvent {
    pub campaign_id: Uuid,
    pub creative_id: Option<Uuid>,
    pub creator_profile_id: Option<Uuid>,
    pub kind: AdEventKind,
    pub idempotency_key: String,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub revenue_micros: i64,
    pub spend_micros: i64,
    pub attributed_touch_event_id: Option<Uuid>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct CampaignReport {
    pub campaign_id: Uuid,
    pub impressions: i64,
    pub qualified_views: i64,
    pub clicks: i64,
    pub conversions: i64,
    pub spend_micros: i64,
    pub attributed_revenue_micros: i64,
    pub creator_earnings_micros: i64,
}

impl CampaignReport {
    pub fn roas(&self) -> Option<f64> {
        (self.spend_micros > 0)
            .then(|| self.attributed_revenue_micros as f64 / self.spend_micros as f64)
    }
}

#[derive(Clone)]
pub struct PgAdsStore {
    pool: PgPool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FundingIntent {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub requirement: PaymentRequirements,
    pub budget_credit_micros: i64,
}

impl PgAdsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_campaign(&self, value: NewCampaign) -> Result<Uuid, AdsError> {
        validate_campaign(&value)?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ad_campaigns (id, advertiser_profile_id, name, currency, budget_micros, attribution_window_seconds, attribution_model, boosted_reel_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(id).bind(value.advertiser_profile_id).bind(value.name).bind(value.currency)
        .bind(value.budget_micros).bind(value.attribution_window_seconds)
        .bind(value.attribution_model.as_str()).bind(value.boosted_reel_id)
        .execute(&self.pool).await?;
        Ok(id)
    }

    pub async fn create_funding_intent(
        &self,
        campaign_id: Uuid,
        requirement: PaymentRequirements,
        budget_credit_micros: i64,
    ) -> Result<FundingIntent, AdsError> {
        if budget_credit_micros <= 0 || requirement.amount.parse::<u128>().is_err() {
            return Err(AdsError::Validation("invalid funding amount".into()));
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO ad_funding_intents (id,campaign_id,scheme,network,amount,asset,pay_to,budget_credit_micros) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(id).bind(campaign_id).bind(&requirement.scheme).bind(&requirement.network)
            .bind(&requirement.amount).bind(&requirement.asset).bind(&requirement.pay_to)
            .bind(budget_credit_micros).execute(&self.pool).await?;
        Ok(FundingIntent {
            id,
            campaign_id,
            requirement,
            budget_credit_micros,
        })
    }

    /// A payment signature is only input to the facilitator. Campaign state changes
    /// after a bound verify+settle receipt has been durably recorded.
    pub async fn settle_funding(
        &self,
        intent_id: Uuid,
        payment_signature: &str,
        processor: &dyn AdPaymentProcessor,
    ) -> Result<Settlement, AdsError> {
        if payment_signature.trim().is_empty() {
            return Err(AdsError::Payment(PaymentError::Verification(
                "empty signature".into(),
            )));
        }
        let row: (Uuid, String, String, String, String, String, i64, String) = sqlx::query_as(
            "SELECT campaign_id,scheme,network,amount,asset,pay_to,budget_credit_micros,status FROM ad_funding_intents WHERE id=$1",
        ).bind(intent_id).fetch_one(&self.pool).await?;
        if row.7 == "settled" {
            return Ok(sqlx::query_as::<_, (String,String,String,String,String,String)>("SELECT transaction_id,network,payer,amount,asset,pay_to FROM ad_payment_receipts WHERE funding_intent_id=$1")
                .bind(intent_id).fetch_one(&self.pool).await.map(|r| Settlement { transaction:r.0, network:r.1, payer:r.2, amount:r.3, asset:r.4, pay_to:r.5 })?);
        }
        let requirement = PaymentRequirements {
            scheme: row.1,
            network: row.2,
            amount: row.3,
            asset: row.4,
            pay_to: row.5,
            max_timeout_seconds: 60,
            extra: Map::new(),
        };
        let receipt = processor
            .verify_and_settle(intent_id, payment_signature, &requirement)
            .await?;
        if receipt.network != requirement.network
            || receipt.amount != requirement.amount
            || receipt.asset != requirement.asset
            || receipt.pay_to != requirement.pay_to
        {
            return Err(AdsError::Payment(PaymentError::Verification(
                "facilitator receipt does not match quoted network/amount/asset/payTo".into(),
            )));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO ad_payment_receipts (id,funding_intent_id,transaction_id,network,payer,amount,asset,pay_to) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (funding_intent_id) DO NOTHING")
            .bind(Uuid::new_v4()).bind(intent_id).bind(&receipt.transaction).bind(&receipt.network)
            .bind(&receipt.payer).bind(&receipt.amount).bind(&receipt.asset).bind(&receipt.pay_to)
            .execute(&mut *tx).await?;
        let activated = sqlx::query("UPDATE ad_funding_intents SET status='settled',settled_at=now() WHERE id=$1 AND status='pending'")
            .bind(intent_id).execute(&mut *tx).await?;
        if activated.rows_affected() == 1 {
            sqlx::query("UPDATE ad_campaigns SET status='active',funded_micros=funded_micros+$2 WHERE id=$1")
                .bind(row.0).bind(row.6).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(receipt)
    }

    pub async fn add_creative(&self, value: NewCreative) -> Result<Uuid, AdsError> {
        if value.destination_url.len() > 2048 || value.disclosure.trim().is_empty() {
            return Err(AdsError::Validation(
                "creative requires a disclosure and bounded URL".into(),
            ));
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO ad_creatives (id,campaign_id,reel_id,destination_url,disclosure) VALUES ($1,$2,$3,$4,$5)")
            .bind(id).bind(value.campaign_id).bind(value.reel_id).bind(value.destination_url).bind(value.disclosure)
            .execute(&self.pool).await?;
        Ok(id)
    }

    pub async fn add_creator_partnership(
        &self,
        value: NewCreatorPartnership,
    ) -> Result<Uuid, AdsError> {
        if !(0..=10_000).contains(&value.revenue_share_bps) {
            return Err(AdsError::Validation(
                "revenue share must be between 0 and 10000 bps".into(),
            ));
        }
        let id = sqlx::query_scalar("INSERT INTO creator_partnerships (id,campaign_id,creator_profile_id,revenue_share_bps) VALUES ($1,$2,$3,$4) ON CONFLICT (campaign_id,creator_profile_id) DO UPDATE SET revenue_share_bps=EXCLUDED.revenue_share_bps RETURNING id")
            .bind(Uuid::new_v4()).bind(value.campaign_id).bind(value.creator_profile_id).bind(value.revenue_share_bps)
            .fetch_one(&self.pool).await?;
        Ok(id)
    }

    /// Records an immutable event and its downstream notification atomically.
    /// Replaying the same idempotency key returns the original event without double counting.
    pub async fn record_event(&self, event: NewAdEvent) -> Result<Uuid, AdsError> {
        validate_event(&event)?;
        let mut tx = self.pool.begin().await?;
        if let Some(id) = existing_event(&mut tx, event.campaign_id, &event.idempotency_key).await?
        {
            tx.commit().await?;
            return Ok(id);
        }
        if let Some(touch_id) = event.attributed_touch_event_id {
            let valid: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM ad_events touch JOIN ad_campaigns c ON c.id=touch.campaign_id WHERE touch.id=$1 AND touch.campaign_id=$2 AND touch.kind IN ('impression','qualified_view','click') AND touch.occurred_at <= $3 AND touch.occurred_at >= $3-make_interval(secs => c.attribution_window_seconds::double precision))",
            )
            .bind(touch_id).bind(event.campaign_id).bind(event.occurred_at)
            .fetch_one(&mut *tx).await?;
            if !valid {
                return Err(AdsError::Validation(
                    "attributed touch is outside the campaign window or campaign".into(),
                ));
            }
        }
        let id = Uuid::new_v4();
        let inserted = sqlx::query("INSERT INTO ad_events (id,campaign_id,creative_id,creator_profile_id,kind,idempotency_key,occurred_at,revenue_micros,spend_micros,attributed_touch_event_id,metadata) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT (campaign_id,idempotency_key) DO NOTHING")
            .bind(id).bind(event.campaign_id).bind(event.creative_id).bind(event.creator_profile_id)
            .bind(event.kind.as_str()).bind(&event.idempotency_key).bind(event.occurred_at)
            .bind(event.revenue_micros).bind(event.spend_micros).bind(event.attributed_touch_event_id).bind(event.metadata)
            .execute(&mut *tx).await?;
        let event_id = if inserted.rows_affected() == 1 {
            id
        } else {
            existing_event(&mut tx, event.campaign_id, &event.idempotency_key)
                .await?
                .ok_or_else(|| AdsError::Conflict("idempotent event disappeared".into()))?
        };
        if inserted.rows_affected() == 1 {
            sqlx::query("INSERT INTO outbox (id,topic,aggregate_type,aggregate_id,payload,available_at,created_at) VALUES ($1,'ads.event_recorded.v1','ad_event',$2,$3,now(),now()) ON CONFLICT (topic,aggregate_type,aggregate_id) DO NOTHING")
                .bind(Uuid::new_v4()).bind(event_id.to_string()).bind(serde_json::json!({"event_id": event_id, "campaign_id": event.campaign_id, "kind": event.kind.as_str()}))
                .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(event_id)
    }

    pub async fn campaign_report(&self, campaign_id: Uuid) -> Result<CampaignReport, AdsError> {
        Ok(sqlx::query_as::<_, CampaignReport>(
            "SELECT $1::uuid campaign_id, COUNT(*) FILTER (WHERE e.kind='impression')::bigint impressions, COUNT(*) FILTER (WHERE e.kind='qualified_view')::bigint qualified_views, COUNT(*) FILTER (WHERE e.kind='click')::bigint clicks, COUNT(*) FILTER (WHERE e.kind='conversion')::bigint conversions, COALESCE(SUM(e.spend_micros),0)::bigint spend_micros, COALESCE(SUM(e.revenue_micros),0)::bigint attributed_revenue_micros, COALESCE(SUM(CASE WHEN e.creator_profile_id IS NOT NULL THEN e.revenue_micros * p.revenue_share_bps / 10000 ELSE 0 END),0)::bigint creator_earnings_micros FROM ad_events e LEFT JOIN creator_partnerships p ON p.campaign_id=e.campaign_id AND p.creator_profile_id=e.creator_profile_id WHERE e.campaign_id=$1",
        ).bind(campaign_id).fetch_one(&self.pool).await?)
    }
}

async fn existing_event(
    tx: &mut Transaction<'_, Postgres>,
    campaign_id: Uuid,
    key: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM ad_events WHERE campaign_id=$1 AND idempotency_key=$2")
        .bind(campaign_id)
        .bind(key)
        .fetch_optional(&mut **tx)
        .await
}

fn validate_campaign(value: &NewCampaign) -> Result<(), AdsError> {
    if value.name.trim().is_empty() || value.name.len() > 160 || value.budget_micros <= 0 {
        return Err(AdsError::Validation(
            "campaign requires a name and positive budget".into(),
        ));
    }
    if value.currency.len() != 3 || !value.currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(AdsError::Validation(
            "currency must be an uppercase ISO-4217 code".into(),
        ));
    }
    if !(60..=2_592_000).contains(&value.attribution_window_seconds) {
        return Err(AdsError::Validation(
            "attribution window must be 60 seconds to 30 days".into(),
        ));
    }
    Ok(())
}

fn validate_event(value: &NewAdEvent) -> Result<(), AdsError> {
    if value.idempotency_key.is_empty()
        || value.idempotency_key.len() > 200
        || value.spend_micros < 0
        || !value.metadata.is_object()
    {
        return Err(AdsError::Validation(
            "invalid event idempotency key or spend".into(),
        ));
    }
    match value.kind {
        AdEventKind::Conversion if value.revenue_micros < 0 => Err(AdsError::Validation(
            "conversion revenue cannot be negative".into(),
        )),
        AdEventKind::Refund if value.revenue_micros > 0 => Err(AdsError::Validation(
            "refund revenue must be negative".into(),
        )),
        AdEventKind::Spend if value.spend_micros == 0 => {
            Err(AdsError::Validation("spend event must carry spend".into()))
        }
        AdEventKind::Conversion | AdEventKind::Refund if value.spend_micros != 0 => Err(
            AdsError::Validation("revenue events cannot also carry spend".into()),
        ),
        AdEventKind::Impression | AdEventKind::QualifiedView | AdEventKind::Click
            if value.revenue_micros != 0 || value.spend_micros != 0 =>
        {
            Err(AdsError::Validation(
                "engagement events cannot carry money".into(),
            ))
        }
        _ => Ok(()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdsError {
    #[error("invalid ads request: {0}")]
    Validation(String),
    #[error("ads event conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Payment(#[from] PaymentError),
    #[error("ads database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_rounds_impressions_up_and_uses_x402_v2_names() {
        let rate = AdRate {
            price_per_thousand_impressions_atomic: 10_000,
            requirements: PaymentRequirements {
                scheme: "exact".into(),
                network: "eip155:8453".into(),
                amount: String::new(),
                asset: "0xasset".into(),
                pay_to: "0xmerchant".into(),
                max_timeout_seconds: 60,
                extra: Map::new(),
            },
        };
        let quote = rate.quote("https://tardy.test/v1/ads/abc/activate".into(), 1_001);
        assert_eq!(quote.accepts[0].amount, "20000");
        let json = serde_json::to_value(quote).unwrap();
        assert_eq!(json["x402Version"], 2);
        assert_eq!(json["accepts"][0]["payTo"], "0xmerchant");
    }

    #[test]
    fn campaign_validation_keeps_money_and_attribution_bounded() {
        let campaign = NewCampaign {
            advertiser_profile_id: Uuid::new_v4(),
            name: "Launch".into(),
            currency: "USD".into(),
            budget_micros: 5_000_000,
            attribution_window_seconds: 86_400,
            attribution_model: AttributionModel::LastTouch,
            boosted_reel_id: None,
        };
        assert!(validate_campaign(&campaign).is_ok());
        assert!(
            validate_campaign(&NewCampaign {
                currency: "usd".into(),
                ..campaign
            })
            .is_err()
        );
    }

    #[test]
    fn report_exposes_reproducible_roas() {
        let report = CampaignReport {
            campaign_id: Uuid::new_v4(),
            impressions: 10,
            qualified_views: 8,
            clicks: 3,
            conversions: 1,
            spend_micros: 2_000_000,
            attributed_revenue_micros: 5_000_000,
            creator_earnings_micros: 500_000,
        };
        assert_eq!(report.roas(), Some(2.5));
    }

    #[test]
    fn refunds_are_compensating_negative_revenue() {
        let event = NewAdEvent {
            campaign_id: Uuid::new_v4(),
            creative_id: None,
            creator_profile_id: None,
            kind: AdEventKind::Refund,
            idempotency_key: "refund/order-1".into(),
            occurred_at: chrono::Utc::now(),
            revenue_micros: -1_000_000,
            spend_micros: 0,
            attributed_touch_event_id: None,
            metadata: serde_json::json!({}),
        };
        assert!(validate_event(&event).is_ok());
        assert!(
            validate_event(&NewAdEvent {
                revenue_micros: 1,
                ..event
            })
            .is_err()
        );
    }
}
