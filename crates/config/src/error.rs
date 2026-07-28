//! Stable configuration errors.

use thiserror::Error;

/// Stable machine-readable configuration error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Configuration text is malformed or contains unknown fields.
    Malformed,
    /// A current configuration file omits its required version.
    MissingVersion,
    /// The required configuration version is newer than this binary.
    UnsupportedVersion,
    /// A legacy configuration cannot be migrated safely.
    MigrationFailed,
    /// Configuration attempts to store a prohibited secret or raw event.
    SensitiveField,
    /// Agent role is invalid.
    InvalidRole,
    /// IPC endpoint is invalid.
    InvalidEndpoint,
    /// Relay role lacks a required SSH host alias.
    MissingRelayHost,
    /// SSH host alias is invalid.
    InvalidRelayHost,
    /// A platform path base is unavailable.
    MissingPathBase,
    /// The selected state directory is not writable.
    UnwritableStateDirectory,
    /// Another bounded configuration value is invalid.
    InvalidValue,
}

impl ErrorCode {
    /// Returns the stable wire and diagnostic code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Malformed => "config_malformed",
            Self::MissingVersion => "config_version_missing",
            Self::UnsupportedVersion => "config_version_unsupported",
            Self::MigrationFailed => "config_migration_failed",
            Self::SensitiveField => "config_sensitive_field",
            Self::InvalidRole => "config_invalid_role",
            Self::InvalidEndpoint => "config_invalid_endpoint",
            Self::MissingRelayHost => "config_missing_relay_host",
            Self::InvalidRelayHost => "config_invalid_relay_host",
            Self::MissingPathBase => "config_path_base_missing",
            Self::UnwritableStateDirectory => "config_state_unwritable",
            Self::InvalidValue => "config_invalid_value",
        }
    }
}

/// A safe configuration failure that never includes source values.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ConfigError {
    /// TOML is malformed or contains an unknown field/type.
    #[error("configuration syntax or shape is invalid")]
    Malformed,
    /// A non-legacy file omits `config_version`.
    #[error("configuration version is required")]
    MissingVersion,
    /// The required version is unsupported.
    #[error("configuration version is unsupported")]
    UnsupportedVersion,
    /// A legacy document cannot be migrated.
    #[error("configuration migration failed")]
    MigrationFailed,
    /// A prohibited sensitive key is present.
    #[error("configuration contains a prohibited sensitive field")]
    SensitiveField,
    /// Role is not `desktop` or `relay`.
    #[error("configuration role is invalid")]
    InvalidRole,
    /// Endpoint does not match the bounded endpoint grammar.
    #[error("configuration IPC endpoint is invalid")]
    InvalidEndpoint,
    /// Relay role has no SSH alias.
    #[error("relay role requires an SSH host alias")]
    MissingRelayHost,
    /// SSH alias is invalid.
    #[error("relay SSH host alias is invalid")]
    InvalidRelayHost,
    /// Required platform path environment is absent.
    #[error("platform path base is unavailable")]
    MissingPathBase,
    /// State directory cannot be written.
    #[error("state directory is not writable")]
    UnwritableStateDirectory,
    /// A bounded configuration value is invalid.
    #[error("configuration value is invalid")]
    InvalidValue,
}

impl ConfigError {
    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Malformed => ErrorCode::Malformed,
            Self::MissingVersion => ErrorCode::MissingVersion,
            Self::UnsupportedVersion => ErrorCode::UnsupportedVersion,
            Self::MigrationFailed => ErrorCode::MigrationFailed,
            Self::SensitiveField => ErrorCode::SensitiveField,
            Self::InvalidRole => ErrorCode::InvalidRole,
            Self::InvalidEndpoint => ErrorCode::InvalidEndpoint,
            Self::MissingRelayHost => ErrorCode::MissingRelayHost,
            Self::InvalidRelayHost => ErrorCode::InvalidRelayHost,
            Self::MissingPathBase => ErrorCode::MissingPathBase,
            Self::UnwritableStateDirectory => ErrorCode::UnwritableStateDirectory,
            Self::InvalidValue => ErrorCode::InvalidValue,
        }
    }
}
