use serde::{Deserialize, Serialize};

/// User permissions as a bitmask. `ADMIN` implies everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Permissions(pub u32);

impl Permissions {
    pub const NONE: Self = Self(0);
    pub const ADMIN: Self = Self(1 << 0);
    pub const MANAGE_REQUESTS: Self = Self(1 << 1);
    pub const REQUEST: Self = Self(1 << 2);
    pub const AUTO_APPROVE: Self = Self(1 << 3);
    pub const MANAGE_LIBRARY: Self = Self(1 << 4);
    pub const PLAY: Self = Self(1 << 5);

    /// Default for a newly invited user.
    pub const DEFAULT_USER: Self = Self(Self::REQUEST.0 | Self::PLAY.0);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn has(self, required: Self) -> bool {
        self.0 & Self::ADMIN.0 != 0 || self.0 & required.0 == required.0
    }
}

impl std::ops::BitOr for Permissions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_has_everything() {
        assert!(Permissions::ADMIN.has(Permissions::MANAGE_LIBRARY | Permissions::PLAY));
    }

    #[test]
    fn requires_all_bits() {
        let p = Permissions::DEFAULT_USER;
        assert!(p.has(Permissions::REQUEST));
        assert!(p.has(Permissions::REQUEST | Permissions::PLAY));
        assert!(!p.has(Permissions::REQUEST | Permissions::AUTO_APPROVE));
        assert!(!p.has(Permissions::MANAGE_REQUESTS));
    }
}
