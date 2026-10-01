use axum::extract::{MatchedPath, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::Mutex;
use std::sync::atomic::AtomicI64;
use std::time::Instant;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct HttpLabels {
    method: String,
    route: String,
    status: String,
}

pub struct Metrics {
    registry: Mutex<Registry>,
    requests: Family<HttpLabels, Counter>,
    durations: Family<HttpLabels, Histogram>,
    claims_issued: Counter,
    accounts_claimed: Counter,
}

impl Metrics {
    pub fn new() -> Self {
        let requests = Family::<HttpLabels, Counter>::default();
        let durations = Family::<HttpLabels, Histogram>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.005, 2.0, 12))
        });
        let claims_issued = Counter::default();
        let accounts_claimed = Counter::default();
        let build = Gauge::<i64, AtomicI64>::default();
        build.set(1);

        let mut registry = Registry::default();
        registry.register(
            "tardy_http_requests",
            "Completed HTTP requests by method, matched route, and status.",
            requests.clone(),
        );
        registry.register(
            "tardy_http_request_duration_seconds",
            "HTTP request duration by method, matched route, and status.",
            durations.clone(),
        );
        registry.register(
            "tardy_agent_claim_codes_issued",
            "Agent onboarding claim codes issued.",
            claims_issued.clone(),
        );
        registry.register(
            "tardy_agent_accounts_claimed",
            "Agent accounts successfully claimed.",
            accounts_claimed.clone(),
        );
        registry.register("tardy_build_info", "Static service build marker.", build);

        Self {
            registry: Mutex::new(registry),
            requests,
            durations,
            claims_issued,
            accounts_claimed,
        }
    }

    pub fn note_claim_issued(&self) {
        self.claims_issued.inc();
    }

    pub fn note_account_claimed(&self) {
        self.accounts_claimed.inc();
    }

    pub fn encode(&self) -> Result<String, std::fmt::Error> {
        let registry = self
            .registry
            .lock()
            .expect("metrics registry lock poisoned");
        let mut output = String::new();
        encode(&mut output, &registry)?;
        Ok(output)
    }

    fn observe(&self, method: String, route: String, status: StatusCode, elapsed: f64) {
        let labels = HttpLabels {
            method,
            route,
            status: status.as_u16().to_string(),
        };
        self.requests.get_or_create(&labels).inc();
        self.durations.get_or_create(&labels).observe(elapsed);
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn track(metrics: std::sync::Arc<Metrics>, request: Request, next: Next) -> Response {
    let method = request.method().to_string();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or("unmatched")
        .to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    metrics.observe(
        method,
        route,
        response.status(),
        started.elapsed().as_secs_f64(),
    );
    response
}

pub fn response(metrics: &Metrics) -> Response {
    match metrics.encode() {
        Ok(body) => (
            StatusCode::OK,
            [(
                "content-type",
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )],
            body,
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to encode metrics: {error}"),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_build_and_onboarding_metrics() {
        let metrics = Metrics::new();
        metrics.note_claim_issued();
        metrics.note_account_claimed();
        let output = metrics.encode().unwrap();
        assert!(output.contains("tardy_build_info 1"));
        assert!(output.contains("tardy_agent_claim_codes_issued_total 1"));
        assert!(output.contains("tardy_agent_accounts_claimed_total 1"));
    }
}
