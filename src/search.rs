use crate::domain::{FeedItem, Visibility};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

const MAX_CANDIDATES: usize = 100;
const DEFAULT_MODEL: &str = "rerank-2.5-lite";
const DEFAULT_ENDPOINT: &str = "https://api.voyageai.com/v1/rerank";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SearchResult {
    pub item: FeedItem,
    pub relevance_score: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("search reranking is not configured")]
    Unavailable,
    #[error("search query must not be empty")]
    EmptyQuery,
    #[error("search provider request failed: {0}")]
    Provider(String),
    #[error("search provider returned an invalid result")]
    InvalidResult,
}

#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub id: Uuid,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RankedDocument {
    pub index: usize,
    pub score: f64,
}

#[async_trait]
pub trait Reranker: Send + Sync {
    fn provider(&self) -> &'static str;
    async fn rerank(
        &self,
        query: &str,
        documents: &[SearchDocument],
        limit: usize,
    ) -> Result<Vec<RankedDocument>, SearchError>;
}

pub struct SearchService {
    reranker: Option<Arc<dyn Reranker>>,
}

impl SearchService {
    pub fn disabled() -> Self {
        Self { reranker: None }
    }

    pub fn from_env() -> Result<Self, SearchError> {
        let Ok(api_key) = std::env::var("VOYAGE_API_KEY") else {
            return Ok(Self::disabled());
        };
        let endpoint =
            std::env::var("VOYAGE_RERANK_URL").unwrap_or_else(|_| DEFAULT_ENDPOINT.into());
        let model = std::env::var("VOYAGE_RERANK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        Ok(Self {
            reranker: Some(Arc::new(VoyageReranker::new(api_key, endpoint, model)?)),
        })
    }

    pub fn with_reranker(reranker: Arc<dyn Reranker>) -> Self {
        Self {
            reranker: Some(reranker),
        }
    }

    pub fn provider(&self) -> Option<&'static str> {
        self.reranker.as_ref().map(|value| value.provider())
    }

    pub async fn search(
        &self,
        query: &str,
        mut candidates: Vec<FeedItem>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        if query.trim().is_empty() {
            return Err(SearchError::EmptyQuery);
        }
        let reranker = self.reranker.as_ref().ok_or(SearchError::Unavailable)?;
        // Until the external-AI consent ledger covers private content, only public
        // candidate text may leave Tardy. Authorization is still checked upstream.
        candidates.retain(|item| item.visibility() == Visibility::Public);
        candidates.sort_by_key(|item| std::cmp::Reverse(item.published_at_ms()));
        candidates.truncate(MAX_CANDIDATES);
        let documents = candidates
            .iter()
            .map(|item| SearchDocument {
                id: item.id(),
                text: document_text(item),
            })
            .collect::<Vec<_>>();
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let ranked = reranker
            .rerank(query.trim(), &documents, limit.min(documents.len()))
            .await?;
        ranked
            .into_iter()
            .map(|ranked| {
                let item = candidates
                    .get(ranked.index)
                    .cloned()
                    .ok_or(SearchError::InvalidResult)?;
                if !ranked.score.is_finite() {
                    return Err(SearchError::InvalidResult);
                }
                Ok(SearchResult {
                    item,
                    relevance_score: ranked.score,
                })
            })
            .collect()
    }
}

fn document_text(item: &FeedItem) -> String {
    match item {
        FeedItem::Reel(reel) => format!("Reel\n{}", reel.caption),
        FeedItem::Live(live) => format!(
            "Live coding session\n{}\n{}",
            live.title, live.repository_url
        ),
    }
}

pub struct VoyageReranker {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
    model: String,
}

impl VoyageReranker {
    pub fn new(api_key: String, endpoint: String, model: String) -> Result<Self, SearchError> {
        if api_key.trim().is_empty() || endpoint.trim().is_empty() || model.trim().is_empty() {
            return Err(SearchError::Unavailable);
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|error| SearchError::Provider(error.to_string()))?;
        Ok(Self {
            client,
            api_key,
            endpoint,
            model,
        })
    }
}

#[derive(Serialize)]
struct VoyageRequest<'a> {
    query: &'a str,
    documents: Vec<&'a str>,
    model: &'a str,
    top_k: usize,
    return_documents: bool,
    truncation: bool,
}

#[derive(Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageResult>,
}

#[derive(Deserialize)]
struct VoyageResult {
    index: usize,
    relevance_score: f64,
}

#[async_trait]
impl Reranker for VoyageReranker {
    fn provider(&self) -> &'static str {
        "voyage"
    }

    async fn rerank(
        &self,
        query: &str,
        documents: &[SearchDocument],
        limit: usize,
    ) -> Result<Vec<RankedDocument>, SearchError> {
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&VoyageRequest {
                query,
                documents: documents.iter().map(|value| value.text.as_str()).collect(),
                model: &self.model,
                top_k: limit,
                return_documents: false,
                truncation: true,
            })
            .send()
            .await
            .map_err(|error| SearchError::Provider(error.to_string()))?;
        if !response.status().is_success() {
            return Err(SearchError::Provider(format!("HTTP {}", response.status())));
        }
        let response = response
            .json::<VoyageResponse>()
            .await
            .map_err(|error| SearchError::Provider(error.to_string()))?;
        Ok(response
            .data
            .into_iter()
            .map(|value| RankedDocument {
                index: value.index,
                score: value.relevance_score,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Reel;

    fn reel(caption: &str, visibility: Visibility, published_at_ms: u64) -> FeedItem {
        FeedItem::Reel(Reel {
            id: Uuid::new_v4(),
            profile_id: Uuid::new_v4(),
            caption: caption.into(),
            media_url: "https://media.test/reel.mp4".into(),
            poster_url: None,
            duration_ms: 1_000,
            visibility,
            published_at_ms,
        })
    }

    struct ReverseReranker;

    #[async_trait]
    impl Reranker for ReverseReranker {
        fn provider(&self) -> &'static str {
            "test"
        }

        async fn rerank(
            &self,
            _query: &str,
            documents: &[SearchDocument],
            limit: usize,
        ) -> Result<Vec<RankedDocument>, SearchError> {
            Ok((0..documents.len())
                .rev()
                .take(limit)
                .map(|index| RankedDocument {
                    index,
                    score: index as f64,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn excludes_private_text_before_calling_the_reranker() {
        let public = reel("public", Visibility::Public, 2);
        let private = reel("private", Visibility::Private, 3);
        let results = SearchService::with_reranker(Arc::new(ReverseReranker))
            .search("rust", vec![public.clone(), private], 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item, public);
    }

    #[tokio::test]
    async fn voyage_adapter_sends_candidates_and_maps_ranked_indexes() {
        let app = axum::Router::new().route(
            "/rerank",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({
                    "data": [{"index": 1, "relevance_score": 0.9}]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let reranker = VoyageReranker::new(
            "secret".into(),
            format!("http://{address}/rerank"),
            "test-model".into(),
        )
        .unwrap();
        let ranked = reranker
            .rerank(
                "query",
                &[
                    SearchDocument {
                        id: Uuid::new_v4(),
                        text: "one".into(),
                    },
                    SearchDocument {
                        id: Uuid::new_v4(),
                        text: "two".into(),
                    },
                ],
                1,
            )
            .await
            .unwrap();
        assert_eq!(
            ranked,
            vec![RankedDocument {
                index: 1,
                score: 0.9
            }]
        );
    }
}
