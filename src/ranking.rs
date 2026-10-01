use crate::domain::{FeedItem, LiveStatus};
use mlua::{Function, Lua, StdLib, Table, Value};
use std::cmp::Ordering;

const DEFAULT_POLICY: &str = r#"
function score(item, now_ms)
  local age_minutes = math.max(0, now_ms - item.published_at_ms) / 60000
  local recency = math.max(0, 10000 - age_minutes)
  local live_bonus = 0
  if item.kind == "live" and item.is_live then
    live_bonus = 100000
  end
  return live_bonus + recency
end
"#;

#[derive(Debug, thiserror::Error)]
pub enum RankingError {
    #[error("invalid Lua ranking policy: {0}")]
    Lua(#[from] mlua::Error),
    #[error("ranking policy must return a finite number")]
    InvalidScore,
}

/// A deliberately narrow Lua extension point: input facts in, numeric score out.
/// It cannot perform I/O and does not decide orchestration or mutate feed state.
pub struct LuaRanker {
    policy: String,
}

impl LuaRanker {
    pub fn default_policy() -> Result<Self, RankingError> {
        Self::new(DEFAULT_POLICY)
    }

    pub fn new(policy: &str) -> Result<Self, RankingError> {
        let ranker = Self {
            policy: policy.into(),
        };
        let lua = ranker.load_policy()?;
        let _: Function = lua.globals().get("score")?;
        Ok(ranker)
    }

    pub fn rank(&self, items: Vec<FeedItem>, now_ms: u64) -> Result<Vec<FeedItem>, RankingError> {
        // Lua is request-local: no interpreter state crosses worker threads and
        // policies cannot smuggle mutable state from one feed request to another.
        let lua = self.load_policy()?;
        let score_fn: Function = lua.globals().get("score")?;
        let mut scored = items
            .into_iter()
            .map(|item| {
                let facts = Self::facts(&lua, &item)?;
                let score: f64 = score_fn.call((facts, now_ms))?;
                if !score.is_finite() {
                    return Err(RankingError::InvalidScore);
                }
                Ok((score, item))
            })
            .collect::<Result<Vec<_>, RankingError>>()?;

        scored.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .partial_cmp(left_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| right.published_at_ms().cmp(&left.published_at_ms()))
                .then_with(|| left.id().cmp(&right.id()))
        });
        Ok(scored.into_iter().map(|(_, item)| item).collect())
    }

    fn load_policy(&self) -> Result<Lua, mlua::Error> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::MATH | StdLib::STRING,
            Default::default(),
        )?;
        lua.load(&self.policy).set_name("ranking_policy").exec()?;
        Ok(lua)
    }

    fn facts<'lua>(lua: &'lua Lua, item: &FeedItem) -> Result<Table, mlua::Error> {
        let table = lua.create_table()?;
        table.set("id", item.id().to_string())?;
        table.set("profile_id", item.profile_id().to_string())?;
        table.set("published_at_ms", item.published_at_ms())?;
        match item {
            FeedItem::Reel(reel) => {
                table.set("kind", "reel")?;
                table.set("duration_ms", reel.duration_ms)?;
                table.set("is_live", false)?;
            }
            FeedItem::Live(live) => {
                table.set("kind", "live")?;
                table.set("duration_ms", Value::Nil)?;
                table.set("is_live", live.status == LiveStatus::Live)?;
            }
        }
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{LiveSession, Reel, Visibility};
    use uuid::Uuid;

    fn reel(published_at_ms: u64) -> FeedItem {
        FeedItem::Reel(Reel {
            id: Uuid::new_v4(),
            profile_id: Uuid::new_v4(),
            caption: String::new(),
            media_url: "https://media.test/reel.mp4".into(),
            poster_url: None,
            duration_ms: 1_000,
            visibility: Visibility::Public,
            published_at_ms,
        })
    }

    #[test]
    fn default_policy_prioritizes_live_then_recency() {
        let old_reel = reel(1_000);
        let fresh_reel = reel(9_000);
        let live = FeedItem::Live(LiveSession {
            id: Uuid::new_v4(),
            profile_id: Uuid::new_v4(),
            title: "coding".into(),
            repository_url: "https://github.com/example/repo".into(),
            playback_url: "https://live.test/session.m3u8".into(),
            visibility: Visibility::Public,
            status: LiveStatus::Live,
            started_at_ms: 500,
            ended_at_ms: None,
            latest_sequence: 0,
        });
        let ranked = LuaRanker::default_policy()
            .unwrap()
            .rank(vec![old_reel, live.clone(), fresh_reel.clone()], 10_000)
            .unwrap();
        assert_eq!(ranked[0], live);
        assert_eq!(ranked[1], fresh_reel);
    }

    #[test]
    fn rejects_non_numeric_policy_results() {
        let ranker = LuaRanker::new("function score() return 'high' end").unwrap();
        assert!(ranker.rank(vec![reel(1)], 1).is_err());
    }
}
