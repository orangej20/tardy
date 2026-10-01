use crate::domain::{
    DirectMessage, DirectThread, FeedItem, LiveEvent, LiveEventPayload, LiveSession, LiveStatus,
    Profile, ProfilePrivacy, PublicProfile, Reel, ShareGrant, ShareSubject, Visibility,
};
use crate::privacy::PrivacyPolicy;
use std::collections::{HashMap, HashSet};
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
    #[error("direct-message thread not found")]
    ThreadNotFound,
    #[error("share grant not found")]
    ShareNotFound,
    #[error("access denied")]
    Forbidden,
    #[error("direct messages are not accepted by this profile")]
    DirectMessagesClosed,
    #[error("message body must not be empty")]
    EmptyMessage,
    #[error("store lock poisoned")]
    Poisoned,
}

#[derive(Debug, Clone)]
pub struct NewProfile {
    pub handle: String,
    pub display_name: String,
    pub bio: String,
    pub privacy: ProfilePrivacy,
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
    fn publish_reel(&self, actor: Uuid, input: NewReel) -> Result<Reel, StoreError>;
    fn start_live(&self, actor: Uuid, input: NewLive) -> Result<LiveSession, StoreError>;
    fn append_live_event(
        &self,
        actor: Uuid,
        id: Uuid,
        occurred_at_ms: u64,
        payload: LiveEventPayload,
    ) -> Result<LiveEvent, StoreError>;
    fn end_live(&self, actor: Uuid, id: Uuid, ended_at_ms: u64) -> Result<LiveSession, StoreError>;
    fn live_events(
        &self,
        viewer: Option<Uuid>,
        id: Uuid,
        after: u64,
    ) -> Result<Vec<LiveEvent>, StoreError>;
    fn feed_candidates(&self, viewer_id: Option<Uuid>) -> Result<Vec<FeedItem>, StoreError>;
    fn public_profile(
        &self,
        handle: &str,
        viewer: Option<Uuid>,
    ) -> Result<PublicProfile, StoreError>;
    fn update_privacy(&self, actor: Uuid, privacy: ProfilePrivacy) -> Result<Profile, StoreError>;
    fn block_profile(&self, actor: Uuid, blocked: Uuid) -> Result<(), StoreError>;
    fn create_thread(
        &self,
        sender: Uuid,
        recipient: Uuid,
        at_ms: u64,
    ) -> Result<DirectThread, StoreError>;
    fn send_message(
        &self,
        actor: Uuid,
        thread: Uuid,
        body: String,
        at_ms: u64,
    ) -> Result<DirectMessage, StoreError>;
    fn messages(
        &self,
        actor: Uuid,
        thread: Uuid,
        after: u64,
    ) -> Result<Vec<DirectMessage>, StoreError>;
    fn create_share(
        &self,
        actor: Uuid,
        subject: ShareSubject,
        at_ms: u64,
        expires_at_ms: Option<u64>,
    ) -> Result<ShareGrant, StoreError>;
    fn resolve_share(&self, token: Uuid, at_ms: u64) -> Result<ShareGrant, StoreError>;
    fn revoke_share(&self, actor: Uuid, id: Uuid, at_ms: u64) -> Result<ShareGrant, StoreError>;
    fn can_share_subject(&self, actor: Uuid, subject: &ShareSubject) -> Result<bool, StoreError>;
}

#[derive(Default)]
struct State {
    profiles: HashMap<Uuid, Profile>,
    profile_ids_by_handle: HashMap<String, Uuid>,
    reels: HashMap<Uuid, Reel>,
    lives: HashMap<Uuid, LiveSession>,
    events: HashMap<Uuid, Vec<LiveEvent>>,
    blocks: HashSet<(Uuid, Uuid)>,
    threads: HashMap<Uuid, DirectThread>,
    messages: HashMap<Uuid, Vec<DirectMessage>>,
    shares: HashMap<Uuid, ShareGrant>,
    share_ids_by_token: HashMap<Uuid, Uuid>,
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
            privacy: input.privacy,
            created_at_ms: input.created_at_ms,
        };
        state
            .profile_ids_by_handle
            .insert(profile.handle.clone(), profile.id);
        state.profiles.insert(profile.id, profile.clone());
        Ok(profile)
    }

    fn publish_reel(&self, actor: Uuid, input: NewReel) -> Result<Reel, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        if actor != input.profile_id {
            return Err(StoreError::Forbidden);
        }
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

    fn start_live(&self, actor: Uuid, input: NewLive) -> Result<LiveSession, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        if actor != input.profile_id {
            return Err(StoreError::Forbidden);
        }
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
        actor: Uuid,
        id: Uuid,
        occurred_at_ms: u64,
        payload: LiveEventPayload,
    ) -> Result<LiveEvent, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        let live = state.lives.get_mut(&id).ok_or(StoreError::LiveNotFound)?;
        if live.profile_id != actor {
            return Err(StoreError::Forbidden);
        }
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

    fn end_live(&self, actor: Uuid, id: Uuid, ended_at_ms: u64) -> Result<LiveSession, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        let live = state.lives.get_mut(&id).ok_or(StoreError::LiveNotFound)?;
        if live.profile_id != actor {
            return Err(StoreError::Forbidden);
        }
        live.status = LiveStatus::Ended;
        live.ended_at_ms = Some(ended_at_ms);
        Ok(live.clone())
    }

    fn live_events(
        &self,
        viewer: Option<Uuid>,
        id: Uuid,
        after: u64,
    ) -> Result<Vec<LiveEvent>, StoreError> {
        let state = self.state.read().map_err(|_| StoreError::Poisoned)?;
        let live = state.lives.get(&id).ok_or(StoreError::LiveNotFound)?;
        if !PrivacyPolicy::can_view_content(
            viewer,
            live.profile_id,
            live.visibility,
            blocked_between(&state, viewer, live.profile_id),
        ) {
            return Err(StoreError::LiveNotFound);
        }
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
            PrivacyPolicy::can_view_content(
                viewer_id,
                profile_id,
                visibility,
                blocked_between(&state, viewer_id, profile_id),
            )
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

    fn public_profile(
        &self,
        handle: &str,
        viewer: Option<Uuid>,
    ) -> Result<PublicProfile, StoreError> {
        let state = self.state.read().map_err(|_| StoreError::Poisoned)?;
        let id = state
            .profile_ids_by_handle
            .get(handle)
            .ok_or(StoreError::ProfileNotFound)?;
        let profile = state.profiles.get(id).ok_or(StoreError::ProfileNotFound)?;
        if !PrivacyPolicy::can_view_profile(viewer, profile, blocked_between(&state, viewer, *id)) {
            return Err(StoreError::ProfileNotFound);
        }
        Ok(profile.into())
    }

    fn update_privacy(&self, actor: Uuid, privacy: ProfilePrivacy) -> Result<Profile, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        let profile = state
            .profiles
            .get_mut(&actor)
            .ok_or(StoreError::ProfileNotFound)?;
        profile.privacy = privacy;
        Ok(profile.clone())
    }

    fn block_profile(&self, actor: Uuid, blocked: Uuid) -> Result<(), StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        require_profile(&state, actor)?;
        require_profile(&state, blocked)?;
        if actor == blocked {
            return Err(StoreError::Forbidden);
        }
        state.blocks.insert((actor, blocked));
        Ok(())
    }

    fn create_thread(
        &self,
        sender: Uuid,
        recipient: Uuid,
        at_ms: u64,
    ) -> Result<DirectThread, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        require_profile(&state, sender)?;
        let recipient_profile = state
            .profiles
            .get(&recipient)
            .ok_or(StoreError::ProfileNotFound)?;
        if !PrivacyPolicy::can_message(
            sender,
            recipient_profile,
            blocked_pair(&state, sender, recipient),
        ) {
            return Err(StoreError::DirectMessagesClosed);
        }
        if let Some(existing) = state.threads.values().find(|thread| {
            thread.participants.contains(&sender) && thread.participants.contains(&recipient)
        }) {
            return Ok(existing.clone());
        }
        let thread = DirectThread {
            id: Uuid::new_v4(),
            participants: [sender, recipient],
            created_at_ms: at_ms,
            latest_sequence: 0,
        };
        state.messages.insert(thread.id, Vec::new());
        state.threads.insert(thread.id, thread.clone());
        Ok(thread)
    }

    fn send_message(
        &self,
        actor: Uuid,
        thread_id: Uuid,
        body: String,
        at_ms: u64,
    ) -> Result<DirectMessage, StoreError> {
        if body.trim().is_empty() {
            return Err(StoreError::EmptyMessage);
        }
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        let thread = state
            .threads
            .get(&thread_id)
            .ok_or(StoreError::ThreadNotFound)?;
        if !thread.participants.contains(&actor) {
            return Err(StoreError::Forbidden);
        }
        let recipient = *thread
            .participants
            .iter()
            .find(|id| **id != actor)
            .ok_or(StoreError::Forbidden)?;
        if blocked_pair(&state, actor, recipient) {
            return Err(StoreError::Forbidden);
        }
        let thread = state
            .threads
            .get_mut(&thread_id)
            .expect("thread checked above");
        thread.latest_sequence += 1;
        let message = DirectMessage {
            thread_id,
            sequence: thread.latest_sequence,
            sender_id: actor,
            body,
            sent_at_ms: at_ms,
        };
        state
            .messages
            .get_mut(&thread_id)
            .expect("messages created with thread")
            .push(message.clone());
        Ok(message)
    }

    fn messages(
        &self,
        actor: Uuid,
        thread_id: Uuid,
        after: u64,
    ) -> Result<Vec<DirectMessage>, StoreError> {
        let state = self.state.read().map_err(|_| StoreError::Poisoned)?;
        let thread = state
            .threads
            .get(&thread_id)
            .ok_or(StoreError::ThreadNotFound)?;
        if !thread.participants.contains(&actor) {
            return Err(StoreError::Forbidden);
        }
        let other = *thread
            .participants
            .iter()
            .find(|id| **id != actor)
            .ok_or(StoreError::Forbidden)?;
        if blocked_pair(&state, actor, other) {
            return Err(StoreError::Forbidden);
        }
        Ok(state
            .messages
            .get(&thread_id)
            .expect("messages created with thread")
            .iter()
            .filter(|message| message.sequence > after)
            .cloned()
            .collect())
    }

    fn create_share(
        &self,
        actor: Uuid,
        subject: ShareSubject,
        at_ms: u64,
        expires_at_ms: Option<u64>,
    ) -> Result<ShareGrant, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        if expires_at_ms.is_some_and(|expires| expires <= at_ms) {
            return Err(StoreError::Forbidden);
        }
        if !can_share(&state, actor, &subject)? {
            return Err(StoreError::Forbidden);
        }
        let share = ShareGrant {
            id: Uuid::new_v4(),
            token: Uuid::new_v4(),
            created_by: actor,
            subject,
            created_at_ms: at_ms,
            expires_at_ms,
            revoked_at_ms: None,
        };
        state.share_ids_by_token.insert(share.token, share.id);
        state.shares.insert(share.id, share.clone());
        Ok(share)
    }

    fn resolve_share(&self, token: Uuid, at_ms: u64) -> Result<ShareGrant, StoreError> {
        let state = self.state.read().map_err(|_| StoreError::Poisoned)?;
        let id = state
            .share_ids_by_token
            .get(&token)
            .ok_or(StoreError::ShareNotFound)?;
        let share = state.shares.get(id).ok_or(StoreError::ShareNotFound)?;
        if share.revoked_at_ms.is_some()
            || share.expires_at_ms.is_some_and(|expires| expires <= at_ms)
        {
            return Err(StoreError::ShareNotFound);
        }
        Ok(share.clone())
    }

    fn revoke_share(&self, actor: Uuid, id: Uuid, at_ms: u64) -> Result<ShareGrant, StoreError> {
        let mut state = self.state.write().map_err(|_| StoreError::Poisoned)?;
        let share = state.shares.get_mut(&id).ok_or(StoreError::ShareNotFound)?;
        if share.created_by != actor {
            return Err(StoreError::Forbidden);
        }
        share.revoked_at_ms = Some(at_ms);
        Ok(share.clone())
    }

    fn can_share_subject(&self, actor: Uuid, subject: &ShareSubject) -> Result<bool, StoreError> {
        let state = self.state.read().map_err(|_| StoreError::Poisoned)?;
        can_share(&state, actor, subject)
    }
}

fn blocked_pair(state: &State, left: Uuid, right: Uuid) -> bool {
    state.blocks.contains(&(left, right)) || state.blocks.contains(&(right, left))
}

fn blocked_between(state: &State, viewer: Option<Uuid>, owner: Uuid) -> bool {
    viewer.is_some_and(|viewer| blocked_pair(state, viewer, owner))
}

fn subject_owner_and_visibility(
    state: &State,
    subject: &ShareSubject,
) -> Result<(Uuid, bool), StoreError> {
    match subject {
        ShareSubject::Profile { id } => state
            .profiles
            .get(id)
            .map(|profile| {
                (
                    *id,
                    profile.privacy.profile_visibility != crate::domain::ProfileVisibility::Private,
                )
            })
            .ok_or(StoreError::ProfileNotFound),
        ShareSubject::Reel { id } => state
            .reels
            .get(id)
            .map(|reel| (reel.profile_id, reel.visibility == Visibility::Public))
            .ok_or(StoreError::ShareNotFound),
        ShareSubject::Live { id } => state
            .lives
            .get(id)
            .map(|live| (live.profile_id, live.visibility == Visibility::Public))
            .ok_or(StoreError::ShareNotFound),
    }
}

fn can_share(state: &State, actor: Uuid, subject: &ShareSubject) -> Result<bool, StoreError> {
    require_profile(state, actor)?;
    let (owner, publicly_viewable) = subject_owner_and_visibility(state, subject)?;
    let profile = state
        .profiles
        .get(&owner)
        .ok_or(StoreError::ProfileNotFound)?;
    Ok(!blocked_pair(state, actor, owner)
        && PrivacyPolicy::can_share(actor, owner, profile.privacy.resharing, publicly_viewable))
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
                privacy: ProfilePrivacy::default(),
                created_at_ms: 1,
            })
            .unwrap()
    }

    #[test]
    fn live_events_are_monotonic_and_stop_after_end() {
        let store = MemoryStore::default();
        let profile = profile(&store);
        let live = store
            .start_live(
                profile.id,
                NewLive {
                    profile_id: profile.id,
                    title: "Build".into(),
                    repository_url: "https://github.com/example/repo".into(),
                    playback_url: "https://live.test/out.m3u8".into(),
                    visibility: Visibility::Public,
                    started_at_ms: 2,
                },
            )
            .unwrap();
        let first = store
            .append_live_event(
                profile.id,
                live.id,
                3,
                LiveEventPayload::Status {
                    message: "starting".into(),
                },
            )
            .unwrap();
        let second = store
            .append_live_event(
                profile.id,
                live.id,
                4,
                LiveEventPayload::Tool {
                    name: "cargo".into(),
                    summary: "check".into(),
                },
            )
            .unwrap();
        assert_eq!((first.sequence, second.sequence), (1, 2));
        assert_eq!(
            store.live_events(Some(profile.id), live.id, 1).unwrap(),
            vec![second]
        );
        store.end_live(profile.id, live.id, 5).unwrap();
        assert_eq!(
            store.append_live_event(
                profile.id,
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
            .publish_reel(
                profile.id,
                NewReel {
                    profile_id: profile.id,
                    caption: "secret".into(),
                    media_url: "https://media.test/reel.mp4".into(),
                    poster_url: None,
                    duration_ms: 10,
                    visibility: Visibility::Private,
                    published_at_ms: 2,
                },
            )
            .unwrap();
        assert!(store.feed_candidates(None).unwrap().is_empty());
        assert_eq!(store.feed_candidates(Some(profile.id)).unwrap().len(), 1);
    }

    #[test]
    fn dms_require_opt_in_and_a_block_stops_existing_threads() {
        let store = MemoryStore::default();
        let sender = profile(&store);
        let recipient = store
            .create_profile(NewProfile {
                handle: "recipient".into(),
                display_name: "Recipient".into(),
                bio: String::new(),
                privacy: ProfilePrivacy::default(),
                created_at_ms: 1,
            })
            .unwrap();
        assert_eq!(
            store.create_thread(sender.id, recipient.id, 2),
            Err(StoreError::DirectMessagesClosed)
        );
        let mut privacy = recipient.privacy;
        privacy.direct_messages = crate::domain::DirectMessagePolicy::Everyone;
        store.update_privacy(recipient.id, privacy).unwrap();
        let thread = store.create_thread(sender.id, recipient.id, 3).unwrap();
        store
            .send_message(sender.id, thread.id, "hello".into(), 4)
            .unwrap();
        store.block_profile(recipient.id, sender.id).unwrap();
        assert_eq!(
            store.send_message(sender.id, thread.id, "blocked".into(), 5),
            Err(StoreError::Forbidden)
        );
    }

    #[test]
    fn private_share_grants_are_owner_only_expiring_and_revocable() {
        let store = MemoryStore::default();
        let owner = profile(&store);
        let subject = ShareSubject::Profile { id: owner.id };
        let share = store.create_share(owner.id, subject, 10, Some(20)).unwrap();
        assert_eq!(store.resolve_share(share.token, 19).unwrap(), share);
        store.revoke_share(owner.id, share.id, 19).unwrap();
        assert_eq!(
            store.resolve_share(share.token, 19),
            Err(StoreError::ShareNotFound)
        );
    }
}
