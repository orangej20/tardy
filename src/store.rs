use crate::domain::{
    FeedItem, LiveEvent, LiveEventPayload, LiveSession, LiveStatus, Profile, Reel, Visibility,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("profile not found")]
    ProfileNotFound,
    #[error("live session not found")]
    LiveNotFound,
    #[error("live session has ended")]
    LiveEnded,
    #[error("handle is already in use")]
    HandleConflict,
    #[error("store lock poisoned")]
    Poisoned,
}

#[derive(Debug, Clone)]
pub struct NewProfile {
    pub handle: String,
    pub display_name: String,
    pub bio: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct NewReel {
    pub profile_id: Uuid,
    pub caption: String,
    pub media_url: String,
    pub poster_url: Option<String>,
    pub duration_ms: u64,
    pub visibility: Visibility,
    pub published_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct NewLive {
    pub profile_id: Uuid,
    pub title: String,
    pub repository_url: String,
    pub playback_url: String,
    pub visibility: Visibility,
    pub started_at_ms: u64,
}

pub trait Store: Send + Sync {
    fn create_profile(&self, input: NewProfile) -> Result<Profile, StoreError>;
    fn publish_reel(&self, input: NewReel) -> Result<Reel, StoreError>;
    fn start_live(&self, input: NewLive) -> Result<LiveSession, StoreError>;
    fn append_live_event(
        &self,
        id: Uuid,
        occurred_at_ms: u64,
        payload: LiveEventPayload,
    ) -> Result<LiveEvent, StoreError>;
    fn end_live(&self, id: Uuid, ended_at_ms: u64) -> Result<LiveSession, StoreError>;
    fn live_events(&self, id: Uuid, after: u64) -> Result<Vec<LiveEvent>, StoreError>;
    fn feed_candidates(&self, viewer_id: Option<Uuid>) -> Result<Vec<FeedItem>, StoreError>;
    fn share_subject_exists(
        &self,
        subject: &crate::domain::ShareSubject,
    ) -> Result<bool, StoreError>;
}

#[derive(Default)]
struct State {
    profiles: HashMap<Uuid, Profile>,
    profile_ids_by_handle: HashMap<String, Uuid>,
    reels: HashMap<Uuid, Reel>,
    lives: HashMap<Uuid, LiveSession>,
    events: HashMap<Uuid, Vec<LiveEvent>>,
}

#[derive(Clone, Default)]
pub struct MemoryStore {
    state: Arc<RwLock<State>>,
}

impl Store for MemoryStore {
    fn create_profile(&self, input: NewProfile) -> Result<Profile, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        if state.profile_ids_by_handle.contains_key(&input.handle) {
            return Err(StoreError::HandleConflict);
        }
        let profile = Profile {
            id: Uuid::new_v4(),
            handle: input.handle,
            display_name: input.display_name,
            bio: input.bio,
            created_at_ms: input.created_at_ms,
        };
        state
            .profile_ids_by_handle
            .insert(profile.handle.clone(), profile.id);
        state.profiles.insert(profile.id, profile.clone());
        Ok(profile)
    }

    fn publish_reel(&self, input: NewReel) -> Result<Reel, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        require_profile(&state, input.profile_id)?;
        let reel = Reel {
            id: Uuid::new_v4(),
            profile_id: input.profile_id,
            caption: input.caption,
            media_url: input.media_url,
            poster_url: input.poster_url,
            duration_ms: input.duration_ms,
            visibility: input.visibility,
            published_at_ms: input.published_at_ms,
        };
        state.reels.insert(reel.id, reel.clone());
        Ok(reel)
    }

    fn start_live(&self, input: NewLive) -> Result<LiveSession, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        require_profile(&state, input.profile_id)?;
        let live = LiveSession {
            id: Uuid::new_v4(),
            profile_id: input.profile_id,
            title: input.title,
            repository_url: input.repository_url,
            playback_url: input.playback_url,
            visibility: input.visibility,
            status: LiveStatus::Live,
            started_at_ms: input.started_at_ms,
            ended_at_ms: None,
            latest_sequence: 0,
        };
        state.lives.insert(live.id, live.clone());
        state.events.insert(live.id, Vec::new());
        Ok(live)
    }

    fn append_live_event(
        &self,
        id: Uuid,
        occurred_at_ms: u64,
        payload: LiveEventPayload,
    ) -> Result<LiveEvent, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        let live = state.lives.get_mut(&id).ok_or(StoreError::LiveNotFound)?;
        if live.status == LiveStatus::Ended {
            return Err(StoreError::LiveEnded);
        }
        live.latest_sequence += 1;
        let event = LiveEvent {
            session_id: id,
            sequence: live.latest_sequence,
            occurred_at_ms,
            payload,
        };
        state
            .events
            .get_mut(&id)
            .expect("events created with session")
            .push(event.clone());
        Ok(event)
    }

    fn end_live(&self, id: Uuid, ended_at_ms: u64) -> Result<LiveSession, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        let live = state.lives.get_mut(&id).ok_or(StoreError::LiveNotFound)?;
        live.status = LiveStatus::Ended;
        live.ended_at_ms = Some(ended_at_ms);
        Ok(live.clone())
    }

    fn live_events(&self, id: Uuid, after: u64) -> Result<Vec<LiveEvent>, StoreError> {
        let state = self.state.read().map_err(|_| StoreError::Poisoned)?;
        let events = state.events.get(&id).ok_or(StoreError::LiveNotFound)?;
        Ok(events
            .iter()
            .filter(|event| event.sequence > after)
            .cloned()
            .collect())
    }

    fn feed_candidates(&self, viewer_id: Option<Uuid>) -> Result<Vec<FeedItem>, StoreError> {
        let state = self.state.read().map_err(|_| StoreError::Poisoned)?;
        let visible = |profile_id, visibility| {
            visibility == Visibility::Public || viewer_id == Some(profile_id)
        };
        let mut items = Vec::with_capacity(state.reels.len() + state.lives.len());
        items.extend(
            state
                .reels
                .values()
                .filter(|item| visible(item.profile_id, item.visibility))
                .cloned()
                .map(FeedItem::Reel),
        );
        items.extend(
            state
                .lives
                .values()
                .filter(|item| visible(item.profile_id, item.visibility))
                .cloned()
                .map(FeedItem::Live),
        );
        Ok(items)
    }

    fn share_subject_exists(
        &self,
        subject: &crate::domain::ShareSubject,
    ) -> Result<bool, StoreError> {
        use crate::domain::ShareSubject;
        let state = self.state.read().map_err(|_| StoreError::Poisoned)?;
        Ok(match subject {
            ShareSubject::Profile { id } => state.profiles.contains_key(id),
            ShareSubject::Reel { id } => state.reels.contains_key(id),
            ShareSubject::Live { id } => state.lives.contains_key(id),
        })
    }
}

fn require_profile(state: &State, id: Uuid) -> Result<(), StoreError> {
    state
        .profiles
        .contains_key(&id)
        .then_some(())
        .ok_or(StoreError::ProfileNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(store: &MemoryStore) -> Profile {
        store
            .create_profile(NewProfile {
                handle: "agent".into(),
                display_name: "Agent".into(),
                bio: String::new(),
                created_at_ms: 1,
            })
            .unwrap()
    }

    #[test]
    fn live_events_are_monotonic_and_stop_after_end() {
        let store = MemoryStore::default();
        let profile = profile(&store);
        let live = store
            .start_live(NewLive {
                profile_id: profile.id,
                title: "Build".into(),
                repository_url: "https://github.com/example/repo".into(),
                playback_url: "https://live.test/out.m3u8".into(),
                visibility: Visibility::Public,
                started_at_ms: 2,
            })
            .unwrap();
        let first = store
            .append_live_event(
                live.id,
                3,
                LiveEventPayload::Status {
                    message: "starting".into(),
                },
            )
            .unwrap();
        let second = store
            .append_live_event(
                live.id,
                4,
                LiveEventPayload::Tool {
                    name: "cargo".into(),
                    summary: "check".into(),
                },
            )
            .unwrap();
        assert_eq!((first.sequence, second.sequence), (1, 2));
        assert_eq!(store.live_events(live.id, 1).unwrap(), vec![second]);
        store.end_live(live.id, 5).unwrap();
        assert_eq!(
            store.append_live_event(
                live.id,
                6,
                LiveEventPayload::Status {
                    message: "late".into()
                }
            ),
            Err(StoreError::LiveEnded)
        );
    }

    #[test]
    fn private_items_are_only_visible_to_the_owner() {
        let store = MemoryStore::default();
        let profile = profile(&store);
        store
            .publish_reel(NewReel {
                profile_id: profile.id,
                caption: "secret".into(),
                media_url: "https://media.test/reel.mp4".into(),
                poster_url: None,
                duration_ms: 10,
                visibility: Visibility::Private,
                published_at_ms: 2,
            })
            .unwrap();
        assert!(store.feed_candidates(None).unwrap().is_empty());
        assert_eq!(store.feed_candidates(Some(profile.id)).unwrap().len(), 1);
    }
}
