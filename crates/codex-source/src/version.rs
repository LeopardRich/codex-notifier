//! Fixture-gated Codex version and interface selection.

use std::str::FromStr;

use crate::SourceError;

/// Codex CLI releases with committed real-event fixtures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CodexCliVersion {
    /// Initial verified release from Stage 01.
    V0_144_5,
}

impl CodexCliVersion {
    /// Returns the exact release string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V0_144_5 => "0.144.5",
        }
    }
}

impl FromStr for CodexCliVersion {
    type Err = SourceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "0.144.5" => Ok(Self::V0_144_5),
            _ => Err(SourceError::UnsupportedVersion),
        }
    }
}

/// External Codex surfaces that can produce source events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CodexInterface {
    /// Lifecycle command hook used by `codex exec` and the interactive CLI.
    CliHook,
    /// JSON-RPC app-server interface.
    AppServer,
}

impl CodexInterface {
    /// Returns the stable machine-readable interface name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CliHook => "cli_hook",
            Self::AppServer => "app_server",
        }
    }
}
