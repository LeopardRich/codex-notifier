//! Stable protocol validation errors.

use thiserror::Error;

/// Stable machine-readable error codes from protocol version 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Input is not one complete JSON object.
    MalformedJson,
    /// Input exceeds the encoded event size limit.
    PayloadTooLarge,
    /// The schema version is not supported.
    UnsupportedSchema,
    /// An object contains an unknown field.
    UnknownField,
    /// The event identifier is not a canonical `UUIDv7`.
    InvalidEventId,
    /// The event kind is not supported by protocol version 1.
    UnknownKind,
    /// The timestamp is outside the initial-ingestion window.
    TimestampOutOfRange,
    /// A bounded string exceeds its limit.
    FieldTooLong,
    /// Routing is not null or a valid profile.
    InvalidRouting,
    /// Extension namespace, type, depth, count, or size is invalid.
    InvalidExtension,
}

impl ErrorCode {
    /// Returns the stable wire code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MalformedJson => "malformed_json",
            Self::PayloadTooLarge => "payload_too_large",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::UnknownField => "unknown_field",
            Self::InvalidEventId => "invalid_event_id",
            Self::UnknownKind => "unknown_kind",
            Self::TimestampOutOfRange => "timestamp_out_of_range",
            Self::FieldTooLong => "field_too_long",
            Self::InvalidRouting => "invalid_routing",
            Self::InvalidExtension => "invalid_extension",
        }
    }
}

/// A safe field identifier suitable for diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    /// Source host label.
    HostLabel,
    /// Optional source project label.
    ProjectLabel,
    /// Optional source session identifier.
    SessionId,
    /// Notification title.
    Title,
    /// Notification body.
    Body,
    /// Routing profile.
    RoutingProfile,
}

/// Validation failure for a canonical event.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum EventError {
    /// Input is not valid duplicate-free JSON.
    #[error("event is not valid protocol JSON")]
    MalformedJson,
    /// Encoded event is larger than the protocol limit.
    #[error("event exceeds the encoded size limit")]
    PayloadTooLarge,
    /// Schema version is unsupported.
    #[error("event schema version is unsupported")]
    UnsupportedSchema,
    /// An object contains an unknown field.
    #[error("event contains an unknown field")]
    UnknownField,
    /// A required field is absent or has the wrong JSON type.
    #[error("event has an invalid object shape")]
    InvalidShape,
    /// Event identifier is not canonical `UUIDv7`.
    #[error("event identifier is not canonical UUIDv7")]
    InvalidEventId,
    /// Event kind is not part of protocol version 1.
    #[error("event kind is unknown")]
    UnknownKind,
    /// Timestamp is not canonical RFC 3339 UTC.
    #[error("event timestamp is invalid")]
    InvalidTimestamp,
    /// Timestamp is outside the accepted ingestion window.
    #[error("event timestamp is outside the accepted range")]
    TimestampOutOfRange,
    /// A bounded field is too long.
    #[error("event field exceeds its length limit")]
    FieldTooLong(Field),
    /// A field contains forbidden characters or values.
    #[error("event field is invalid")]
    InvalidField(Field),
    /// Routing profile is invalid.
    #[error("event routing profile is invalid")]
    InvalidRouting,
    /// Extension data violates protocol limits.
    #[error("event extensions are invalid")]
    InvalidExtension,
    /// Canonical serialization failed unexpectedly.
    #[error("event could not be serialized")]
    Serialization,
}

impl EventError {
    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::MalformedJson => ErrorCode::MalformedJson,
            Self::PayloadTooLarge => ErrorCode::PayloadTooLarge,
            Self::UnsupportedSchema => ErrorCode::UnsupportedSchema,
            Self::UnknownField => ErrorCode::UnknownField,
            Self::InvalidShape | Self::InvalidField(_) | Self::Serialization => {
                ErrorCode::MalformedJson
            }
            Self::InvalidEventId => ErrorCode::InvalidEventId,
            Self::UnknownKind => ErrorCode::UnknownKind,
            Self::InvalidTimestamp | Self::TimestampOutOfRange => ErrorCode::TimestampOutOfRange,
            Self::FieldTooLong(_) => ErrorCode::FieldTooLong,
            Self::InvalidRouting => ErrorCode::InvalidRouting,
            Self::InvalidExtension => ErrorCode::InvalidExtension,
        }
    }
}
