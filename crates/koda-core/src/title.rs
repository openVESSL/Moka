use serde::{Deserialize, Serialize};

use crate::string_enum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleKind {
    Movie,
    Show,
    Season,
    Episode,
}

string_enum!(TitleKind, "title kind" {
    Movie => "movie",
    Show => "show",
    Season => "season",
    Episode => "episode",
});

/// Where a title is in its lifecycle. See `docs/architecture.md`, section 2.2.
///
/// "Watched" is deliberately not here: it is per user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleState {
    Requested,
    Wanted,
    Searching,
    Grabbed,
    Downloading,
    Importing,
    Available,
}

string_enum!(TitleState, "title state" {
    Requested => "requested",
    Wanted => "wanted",
    Searching => "searching",
    Grabbed => "grabbed",
    Downloading => "downloading",
    Importing => "importing",
    Available => "available",
});

impl TitleState {
    /// Allowed transitions. Anything that fails falls back to `Wanted`.
    pub fn can_move_to(self, next: TitleState) -> bool {
        use TitleState::*;
        matches!(
            (self, next),
            (Requested, Wanted)
                | (Wanted, Searching)
                | (Searching, Grabbed | Wanted)
                | (Grabbed, Downloading | Importing | Wanted)
                | (Downloading, Importing | Wanted)
                | (Importing, Available | Wanted)
                // manual grab or re-search on a title that already has a file
                | (Available, Searching | Grabbed)
        )
    }

    /// States where the background loop is actively working on the title.
    pub fn is_in_progress(self) -> bool {
        matches!(
            self,
            TitleState::Searching
                | TitleState::Grabbed
                | TitleState::Downloading
                | TitleState::Importing
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_strings() {
        for s in TitleState::ALL {
            assert_eq!(s.as_str().parse::<TitleState>().unwrap(), *s);
            assert_eq!(serde_json::to_string(s).unwrap(), format!("\"{}\"", s.as_str()));
        }
        assert!("watched".parse::<TitleState>().is_err());
    }

    #[test]
    fn happy_path_is_allowed_and_skips_are_not() {
        use TitleState::*;
        let path = [Requested, Wanted, Searching, Grabbed, Downloading, Importing, Available];
        for w in path.windows(2) {
            assert!(w[0].can_move_to(w[1]), "{:?} -> {:?}", w[0], w[1]);
        }
        assert!(!Requested.can_move_to(Available));
        assert!(!Wanted.can_move_to(Downloading));
        assert!(Downloading.can_move_to(Wanted));
    }
}
