use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
pub trait AdPaymentProcessor: Send + Sync {
    fn verify_and_settle(
        &self,
        payment_signature: &str,
        requirement: &PaymentRequired,
    ) -> Result<Settlement, PaymentError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub transaction: String,
    pub network: String,
    pub payer: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PaymentError {
    #[error("payment verification failed: {0}")]
    Verification(String),
    #[error("payment settlement failed: {0}")]
    Settlement(String),
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
}
