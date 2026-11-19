//! Domain types shared by every crate: title lifecycle, quality model, permissions.

mod permissions;
mod quality;
mod title;

pub use permissions::Permissions;
pub use quality::{Modifier, Quality, Resolution, Revision, Source};
pub use title::{TitleKind, TitleState};

/// Returned when a stored string does not map to a known enum value.
#[derive(Debug, thiserror::Error)]
#[error("unknown {kind} value: {value}")]
pub struct UnknownValue {
    pub kind: &'static str,
    pub value: String,
}

/// Generates `as_str`, `FromStr` and `Display` for a fieldless enum with
/// snake_case string forms, matching its serde representation.
macro_rules! string_enum {
    ($name:ident, $kind:literal { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::UnknownValue;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok($name::$variant),)+
                    _ => Err($crate::UnknownValue { kind: $kind, value: s.to_owned() }),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}
pub(crate) use string_enum;
