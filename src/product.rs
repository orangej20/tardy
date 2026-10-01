pub const MOTTO: &str = "Don't be Tardy.";

pub const REAL_TARDY_MONTHLY_USD_CENTS: u32 = 2_000;
pub const SUPER_TARDY_LIFETIME_USD_CENTS: u32 = 25_000;
pub const SUPER_TARDY_GLOBAL_SLOT_LIMIT: u32 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    Free,
    /// Recurring verified membership.
    RealTardy,
    /// Lifetime membership from the globally limited allocation.
    SuperTardy {
        slot: u32,
    },
}

impl Membership {
    pub fn is_verified(self) -> bool {
        matches!(self, Self::RealTardy | Self::SuperTardy { .. })
    }

    pub fn validate(self) -> Result<(), MembershipError> {
        match self {
            Self::SuperTardy { slot } if !(1..=SUPER_TARDY_GLOBAL_SLOT_LIMIT).contains(&slot) => {
                Err(MembershipError::InvalidSuperSlot(slot))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MembershipError {
    #[error("SUPER Tardy slot {0} is outside the global 1..=1000 allocation")]
    InvalidSuperSlot(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn super_tardy_enforces_the_global_slot_range() {
        assert!(Membership::SuperTardy { slot: 1 }.validate().is_ok());
        assert!(Membership::SuperTardy { slot: 1_000 }.validate().is_ok());
        assert_eq!(
            Membership::SuperTardy { slot: 1_001 }.validate(),
            Err(MembershipError::InvalidSuperSlot(1_001))
        );
    }
}
