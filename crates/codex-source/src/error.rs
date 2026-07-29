//! Stable safe source-adapter failures.

use thiserror::Error;

/// Stable machine-readable Codex source error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourceErrorCode {
    /// The installed Codex CLI version has no verified adapter.
    UnsupportedVersion,
    /// The selected Codex interface has no verified adapter for this event.
    UnsupportedInterface,
    /// Hook input exceeds its byte limit.
    PayloadTooLarge,
    /// JSON is malformed or incompatible with the verified version shape.
    IncompatiblePayload,
    /// A fixed context label or route is invalid.
    InvalidContext,
    /// A validated canonical event could not be constructed.
    EventBuildFailed,
}

impl SourceErrorCode {
    /// Returns the stable diagnostic code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedVersion => "codex_version_unsupported",
            Self::UnsupportedInterface => "codex_interface_unsupported",
            Self::PayloadTooLarge => "codex_payload_too_large",
            Self::IncompatiblePayload => "codex_payload_incompatible",
            Self::InvalidContext => "codex_context_invalid",
            Self::EventBuildFailed => "codex_event_build_failed",
        }
    }
}

/// A source failure whose display text never contains the raw hook payload.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum SourceError {
    /// No fixture-verified adapter exists for this version.
    #[error("Codex CLI version is not supported")]
    UnsupportedVersion,
    /// This event is not supported through the selected interface.
    #[error("Codex interface is not supported for this event")]
    UnsupportedInterface,
    /// Hook stdin exceeds the fixed byte limit.
    #[error("Codex hook payload exceeds its size limit")]
    PayloadTooLarge,
    /// Hook JSON does not match the verified versioned schema.
    #[error("Codex hook payload is incompatible")]
    IncompatiblePayload,
    /// Adapter-owned source labels or routing are invalid.
    #[error("Codex source context is invalid")]
    InvalidContext,
    /// Canonical validation rejected the mapped event.
    #[error("Codex event could not be normalized")]
    EventBuildFailed,
}

impl SourceError {
    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(self) -> SourceErrorCode {
        match self {
            Self::UnsupportedVersion => SourceErrorCode::UnsupportedVersion,
            Self::UnsupportedInterface => SourceErrorCode::UnsupportedInterface,
            Self::PayloadTooLarge => SourceErrorCode::PayloadTooLarge,
            Self::IncompatiblePayload => SourceErrorCode::IncompatiblePayload,
            Self::InvalidContext => SourceErrorCode::InvalidContext,
            Self::EventBuildFailed => SourceErrorCode::EventBuildFailed,
        }
    }
}
