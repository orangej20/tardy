use mlua::{Function, Lua, LuaSerdeExt, StdLib, Table};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

const USER_AGENT: &str = "tardy-ingest/0.1 (+https://tardy.example)";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightsMode {
    Facts,
    Index,
    Syndicate,
    Mirror,
    RequiresLicense,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RightsPolicy {
    pub mode: RightsMode,
    pub attribution: String,
    #[serde(default)]
    pub commercial_use: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transport {
    Rss { url: String },
    GithubReleases { owner: String, repo: String },
    HackerNews { list: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDefinition {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub limit: usize,
    pub transport: Transport,
    pub rights: RightsPolicy,
    pub transform: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedItem {
    pub source_id: String,
    pub external_id: String,
    pub title: String,
    pub canonical_url: String,
    pub author: Option<String>,
    pub published_at_ms: Option<u64>,
    pub summary: Option<String>,
    #[serde(default)]
    pub facts: BTreeMap<String, String>,
}

impl NormalizedItem {
    pub fn dedupe_key(&self) -> String {
        format!("{}:{}", self.source_id, self.external_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmPlan {
    pub system: String,
    pub prompt: String,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarouselPlan {
    pub format: String,
    pub slides: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformPlan {
    pub headline: String,
    pub attribution: String,
    pub source_url: String,
    pub carousel: CarouselPlan,
    pub llm: Option<LlmPlan>,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("invalid ingestion Lua: {0}")]
    Lua(#[from] mlua::Error),
    #[error("source not found or disabled")]
    SourceUnavailable,
    #[error("source requires a content license")]
    RequiresLicense,
    #[error("transport request failed: {0}")]
    Transport(String),
    #[error("invalid source response: {0}")]
    InvalidResponse(String),
    #[error("unknown carousel format: {0}")]
    UnknownCarousel(String),
}

pub struct Ingestor {
    script: String,
    client: reqwest::Client,
}

impl Ingestor {
    pub fn new(script: impl Into<String>) -> Result<Self, IngestError> {
        let value = Self {
            script: script.into(),
            client: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|error| IngestError::Transport(error.to_string()))?,
        };
        value.sources()?;
        Ok(value)
    }

    pub fn bundled() -> Result<Self, IngestError> {
        Self::new(include_str!("../ingest/sources.lua"))
    }

    pub fn sources(&self) -> Result<Vec<SourceDefinition>, IngestError> {
        let lua = sandbox()?;
        let root: Table = lua.load(&self.script).set_name("sources.lua").eval()?;
        Ok(lua.from_value(root.get("sources")?)?)
    }

    pub async fn preview(&self, source_id: &str) -> Result<Vec<TransformPlan>, IngestError> {
        let source = self
            .sources()?
            .into_iter()
            .find(|source| source.id == source_id && source.enabled)
            .ok_or(IngestError::SourceUnavailable)?;
        if source.rights.mode == RightsMode::RequiresLicense {
            return Err(IngestError::RequiresLicense);
        }
        let items = self.fetch(&source).await?;
        items
            .iter()
            .map(|item| self.transform(&source, item))
            .collect()
    }

    pub fn transform(
        &self,
        source: &SourceDefinition,
        item: &NormalizedItem,
    ) -> Result<TransformPlan, IngestError> {
        let lua = sandbox()?;
        let root: Table = lua.load(&self.script).set_name("sources.lua").eval()?;
        let transforms: Table = root.get("transforms")?;
        let transform: Function = transforms.get(source.transform.as_str())?;
        let plan: TransformPlan =
            lua.from_value(transform.call((lua.to_value(item)?, lua.to_value(&source.rights)?))?)?;
        if !matches!(
            plan.carousel.format.as_str(),
            "headline_source_v1" | "release_notes_v1"
        ) {
            return Err(IngestError::UnknownCarousel(plan.carousel.format));
        }
        Ok(plan)
    }

    async fn fetch(&self, source: &SourceDefinition) -> Result<Vec<NormalizedItem>, IngestError> {
        let mut items = match &source.transport {
            Transport::Rss { url } => self.fetch_rss(source, url).await?,
            Transport::GithubReleases { owner, repo } => {
                self.fetch_github(source, owner, repo).await?
            }
            Transport::HackerNews { list } => self.fetch_hacker_news(source, list).await?,
        };
        items.truncate(source.limit.min(100));
        Ok(items)
    }

    async fn fetch_rss(
        &self,
        source: &SourceDefinition,
        url: &str,
    ) -> Result<Vec<NormalizedItem>, IngestError> {
        let bytes = self.get(url).await?;
        let feed = feed_rs::parser::parse(bytes.as_slice())
            .map_err(|error| IngestError::InvalidResponse(error.to_string()))?;
        Ok(feed
            .entries
            .into_iter()
            .filter_map(|entry| {
                let link = entry.links.first()?.href.clone();
                Some(NormalizedItem {
                    source_id: source.id.clone(),
                    external_id: if entry.id.is_empty() {
                        link.clone()
                    } else {
                        entry.id
                    },
                    title: entry.title.map(|value| value.content).unwrap_or_default(),
                    canonical_url: link,
                    author: entry.authors.first().and_then(|value| value.name.clone()),
                    published_at_ms: entry
                        .published
                        .or(entry.updated)
                        .and_then(|value| value.timestamp_millis().try_into().ok()),
                    summary: entry.summary.map(|value| value.content),
                    facts: BTreeMap::new(),
                })
            })
            .collect())
    }

    async fn fetch_github(
        &self,
        source: &SourceDefinition,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<NormalizedItem>, IngestError> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/releases?per_page=100");
        let values: Vec<GithubRelease> = self.get_json(&url).await?;
        Ok(values
            .into_iter()
            .filter(|value| !value.draft)
            .map(|value| NormalizedItem {
                source_id: source.id.clone(),
                external_id: value.id.to_string(),
                title: value.name.unwrap_or_else(|| value.tag_name.clone()),
                canonical_url: value.html_url,
                author: value.author.map(|value| value.login),
                published_at_ms: None,
                summary: value.body,
                facts: BTreeMap::from([
                    ("tag".into(), value.tag_name),
                    ("prerelease".into(), value.prerelease.to_string()),
                ]),
            })
            .collect())
    }

    async fn fetch_hacker_news(
        &self,
        source: &SourceDefinition,
        list: &str,
    ) -> Result<Vec<NormalizedItem>, IngestError> {
        let endpoint = match list {
            "top" => "topstories",
            "best" => "beststories",
            "show" => "showstories",
            "ask" => "askstories",
            _ => return Err(IngestError::InvalidResponse("unknown HN list".into())),
        };
        let ids: Vec<u64> = self
            .get_json(&format!(
                "https://hacker-news.firebaseio.com/v0/{endpoint}.json"
            ))
            .await?;
        let mut items = Vec::new();
        for id in ids.into_iter().take(source.limit.min(100)) {
            let value: HackerNewsItem = self
                .get_json(&format!(
                    "https://hacker-news.firebaseio.com/v0/item/{id}.json"
                ))
                .await?;
            if value.deleted || value.dead {
                continue;
            }
            let discussion = format!("https://news.ycombinator.com/item?id={id}");
            items.push(NormalizedItem {
                source_id: source.id.clone(),
                external_id: id.to_string(),
                title: value.title.unwrap_or_default(),
                canonical_url: value.url.unwrap_or_else(|| discussion.clone()),
                author: value.by,
                published_at_ms: value.time.and_then(|value| value.checked_mul(1_000)),
                summary: None,
                facts: BTreeMap::from([
                    ("discussion_url".into(), discussion),
                    ("score".into(), value.score.unwrap_or_default().to_string()),
                    (
                        "comments".into(),
                        value.descendants.unwrap_or_default().to_string(),
                    ),
                ]),
            });
        }
        Ok(items)
    }

    async fn get(&self, url: &str) -> Result<Vec<u8>, IngestError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| IngestError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(IngestError::Transport(format!(
                "HTTP {}",
                response.status()
            )));
        }
        Ok(response
            .bytes()
            .await
            .map_err(|error| IngestError::Transport(error.to_string()))?
            .to_vec())
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, IngestError> {
        let bytes = self.get(url).await?;
        serde_json::from_slice(&bytes)
            .map_err(|error| IngestError::InvalidResponse(error.to_string()))
    }
}

fn sandbox() -> Result<Lua, mlua::Error> {
    Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        Default::default(),
    )
}

#[derive(Deserialize)]
struct GithubRelease {
    id: u64,
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    draft: bool,
    prerelease: bool,
    author: Option<GithubAuthor>,
}

#[derive(Deserialize)]
struct GithubAuthor {
    login: String,
}

#[derive(Deserialize)]
struct HackerNewsItem {
    title: Option<String>,
    url: Option<String>,
    by: Option<String>,
    time: Option<u64>,
    score: Option<u64>,
    descendants: Option<u64>,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    dead: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_sources_are_typed_and_licensed_sources_are_disabled() {
        let ingestor = Ingestor::bundled().unwrap();
        let sources = ingestor.sources().unwrap();
        assert!(sources.iter().any(|source| source.id == "hacker-news-top"));
        let bbc = sources
            .iter()
            .find(|source| source.id == "bbc-pidgin")
            .unwrap();
        assert!(!bbc.enabled);
        assert_eq!(bbc.rights.mode, RightsMode::RequiresLicense);
    }

    #[test]
    fn lua_builds_a_validated_prompt_and_carousel_plan() {
        let ingestor = Ingestor::bundled().unwrap();
        let source = ingestor
            .sources()
            .unwrap()
            .into_iter()
            .find(|source| source.id == "hacker-news-top")
            .unwrap();
        let item = NormalizedItem {
            source_id: source.id.clone(),
            external_id: "42".into(),
            title: "A useful release".into(),
            canonical_url: "https://example.test/release".into(),
            author: Some("builder".into()),
            published_at_ms: Some(1),
            summary: Some("It shipped.".into()),
            facts: BTreeMap::new(),
        };
        assert_eq!(item.dedupe_key(), "hacker-news-top:42");
        let plan = ingestor.transform(&source, &item).unwrap();
        assert_eq!(plan.carousel.format, "headline_source_v1");
        assert!(
            plan.llm
                .unwrap()
                .prompt
                .contains("https://example.test/release")
        );
    }

    #[test]
    fn lua_has_no_operating_system_library() {
        let error = Ingestor::new("return { sources = os.getenv('HOME'), transforms = {} }")
            .err()
            .unwrap();
        assert!(matches!(error, IngestError::Lua(_)));
    }

    #[test]
    fn unknown_carousel_formats_fail_loudly() {
        let script = r#"
          return {
            sources = {{ id="x", display_name="X", enabled=true, limit=1,
              transport={kind="rss", url="https://example.test/feed"},
              rights={mode="facts", attribution="X", commercial_use=true}, transform="x" }},
            transforms = { x = function(item, rights) return {
              headline=item.title, attribution=rights.attribution, source_url=item.canonical_url,
              carousel={format="made_up", slides={}}, llm=nil
            } end }
          }
        "#;
        let ingestor = Ingestor::new(script).unwrap();
        let source = ingestor.sources().unwrap().remove(0);
        let item = NormalizedItem {
            source_id: "x".into(),
            external_id: "1".into(),
            title: "x".into(),
            canonical_url: "https://example.test/x".into(),
            author: None,
            published_at_ms: None,
            summary: None,
            facts: BTreeMap::new(),
        };
        assert!(matches!(
            ingestor.transform(&source, &item),
            Err(IngestError::UnknownCarousel(_))
        ));
    }
}
