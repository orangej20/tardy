use crate::domain::{DirectMessagePolicy, Profile, ProfileVisibility, ResharePolicy, Visibility};
use uuid::Uuid;

/// Central privacy policy. Blocking is passed in by the relationship store and
/// always wins over profile/content preferences.
pub struct PrivacyPolicy;

impl PrivacyPolicy {
    pub fn can_view_profile(viewer: Option<Uuid>, profile: &Profile, blocked: bool) -> bool {
        if viewer == Some(profile.id) {
            return true;
        }
        if blocked {
            return false;
        }
        matches!(
            profile.privacy.profile_visibility,
            ProfileVisibility::Public | ProfileVisibility::Unlisted
        )
    }

    pub fn can_view_content(
        viewer: Option<Uuid>,
        owner_id: Uuid,
        visibility: Visibility,
        blocked: bool,
    ) -> bool {
        if viewer == Some(owner_id) {
            return true;
        }
        !blocked && visibility == Visibility::Public
    }

    pub fn can_message(sender: Uuid, recipient: &Profile, blocked: bool) -> bool {
        sender != recipient.id
            && !blocked
            && recipient.privacy.direct_messages == DirectMessagePolicy::Everyone
    }

    pub fn can_share(actor: Uuid, owner_id: Uuid, policy: ResharePolicy, can_view: bool) -> bool {
        actor == owner_id || (policy == ResharePolicy::AnyoneWhoCanView && can_view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ProfilePrivacy;

    fn profile() -> Profile {
        Profile {
            id: Uuid::new_v4(),
            handle: "private_agent".into(),
            display_name: "Private Agent".into(),
            bio: String::new(),
            privacy: ProfilePrivacy::default(),
            created_at_ms: 1,
        }
    }

    #[test]
    fn defaults_deny_strangers_and_allow_the_owner() {
        let profile = profile();
        assert!(PrivacyPolicy::can_view_profile(
            Some(profile.id),
            &profile,
            false
        ));
        assert!(!PrivacyPolicy::can_view_profile(None, &profile, false));
        assert!(!PrivacyPolicy::can_message(Uuid::new_v4(), &profile, false));
    }

    #[test]
    fn a_block_overrides_public_preferences() {
        let mut profile = profile();
        profile.privacy.profile_visibility = ProfileVisibility::Public;
        profile.privacy.direct_messages = DirectMessagePolicy::Everyone;
        let stranger = Uuid::new_v4();
        assert!(!PrivacyPolicy::can_view_profile(
            Some(stranger),
            &profile,
            true
        ));
        assert!(!PrivacyPolicy::can_message(stranger, &profile, true));
    }
}
