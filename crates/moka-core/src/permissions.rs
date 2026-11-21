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
    /// An app that acts for other users (the Discord bot): its API key may
    /// name a user to act as, and then has exactly that user's permissions.
    pub const ACT_AS_USER: Self = Self(1 << 6);
    /// Request or auto-approve films only, or shows only. `REQUEST` and
    /// `AUTO_APPROVE` stay the rights for everything (music and books too).
    pub const REQUEST_MOVIES: Self = Self(1 << 7);
    pub const REQUEST_SHOWS: Self = Self(1 << 8);
    pub const AUTO_APPROVE_MOVIES: Self = Self(1 << 9);
    pub const AUTO_APPROVE_SHOWS: Self = Self(1 << 10);
    /// Every bit in use.
    pub const ALL: Self = Self((1 << 11) - 1);

    /// Default for a newly invited user.
    pub const DEFAULT_USER: Self = Self(Self::REQUEST.0 | Self::PLAY.0);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn has(self, required: Self) -> bool {
        self.0 & Self::ADMIN.0 != 0 || self.0 & required.0 == required.0
    }

    /// May request this kind of title (`movie`, `show`, anything else): the
    /// right for everything, or the one for films or shows.
    pub fn may_request(self, kind: &str) -> bool {
        self.has(Self::REQUEST) || self.kind_bit(kind, Self::REQUEST_MOVIES, Self::REQUEST_SHOWS)
    }

    /// Requests of this kind are approved at once.
    pub fn auto_approves(self, kind: &str) -> bool {
        self.has(Self::AUTO_APPROVE)
            || self.kind_bit(kind, Self::AUTO_APPROVE_MOVIES, Self::AUTO_APPROVE_SHOWS)
    }

    fn kind_bit(self, kind: &str, movies: Self, shows: Self) -> bool {
        match kind {
            "movie" => self.has(movies),
            "show" => self.has(shows),
            _ => false,
        }
    }

    /// The rights for films and shows set as given. Keeps the right for
    /// everything when both are allowed; otherwise uses the film and show bits.
    pub fn with_kinds(self, request: (bool, bool), auto: (bool, bool)) -> Self {
        let pick = |bits: u32, all: Self, (movies, shows): (bool, bool), m: Self, s: Self| {
            let had_all = bits & all.0 != 0;
            let mut b = bits & !(all.0 | m.0 | s.0);
            if movies && shows && had_all {
                b |= all.0;
            } else {
                if movies {
                    b |= m.0;
                }
                if shows {
                    b |= s.0;
                }
            }
            b
        };
        let bits = pick(self.0, Self::REQUEST, request, Self::REQUEST_MOVIES, Self::REQUEST_SHOWS);
        Self(pick(bits, Self::AUTO_APPROVE, auto, Self::AUTO_APPROVE_MOVIES, Self::AUTO_APPROVE_SHOWS))
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
    fn films_and_shows_apart() {
        let films = Permissions(Permissions::REQUEST_MOVIES.0 | Permissions::AUTO_APPROVE_SHOWS.0);
        assert!(films.may_request("movie") && !films.may_request("show") && !films.may_request("album"));
        assert!(films.auto_approves("show") && !films.auto_approves("movie"));
        assert!(Permissions::REQUEST.may_request("show") && Permissions::REQUEST.may_request("album"));

        let all = Permissions::DEFAULT_USER;
        assert_eq!(
            all.with_kinds((true, true), (false, false)),
            all,
            "both allowed keeps the right for everything"
        );
        let shows_only = all.with_kinds((false, true), (false, true));
        assert!(
            !shows_only.may_request("movie")
                && shows_only.may_request("show")
                && shows_only.auto_approves("show")
        );
        assert!(shows_only.has(Permissions::PLAY), "other rights are left alone");
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
