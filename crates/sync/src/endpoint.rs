//! A side of a sync: a built-in [`Platform`] or a configured adapter.
//!
//! `Platform` stays a closed enum (it is `Copy`, a clap `ValueEnum` and
//! serialized by name), so configured adapters get their own type instead of
//! a `Platform::Custom` variant that would break every exhaustive match on it.

use crate::Platform;
use std::convert::Infallible;

/// Where a sync reads from or writes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SyncEndpoint {
    /// One of the built-in platforms.
    Builtin(Platform),
    /// An adapter from `~/.skrills/adapters/<name>.toml`, by lowercase name.
    /// Whether it exists is checked when an adapter is created, since
    /// parsing a command line must not read the disk.
    Custom(String),
}

impl SyncEndpoint {
    /// The name used on the command line.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Builtin(platform) => platform.as_str(),
            Self::Custom(name) => name,
        }
    }

    /// The target of a sync with no `--to`: [`Platform::default_target`] for
    /// a built-in source, Claude for a custom one.
    pub fn default_target(&self) -> Self {
        match self {
            Self::Builtin(platform) => Self::Builtin(platform.default_target()),
            Self::Custom(_) => Self::Builtin(Platform::Claude),
        }
    }

    /// Every built-in other than `self`. A configured adapter is only ever
    /// written when named, so it is never in this list.
    pub fn other_targets(&self) -> Vec<Self> {
        Platform::ALL
            .into_iter()
            .map(Self::Builtin)
            .filter(|endpoint| endpoint != self)
            .collect()
    }
}

impl From<Platform> for SyncEndpoint {
    fn from(platform: Platform) -> Self {
        Self::Builtin(platform)
    }
}

impl std::fmt::Display for SyncEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SyncEndpoint {
    type Err = Infallible;

    /// Case-insensitive, like [`Platform`]'s parser: a built-in name always
    /// wins, anything else is a custom name.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Ok(name
            .parse::<Platform>()
            .map(Self::Builtin)
            .unwrap_or_else(|_| Self::Custom(name.to_ascii_lowercase())))
    }
}
