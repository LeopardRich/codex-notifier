//! Stable safe errors for structured logging and sinks.

use thiserror::Error;

/// Stable machine-readable logging error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LogErrorCode {
    /// A correlation identifier violates its grammar or size bound.
    InvalidCorrelationId,
    /// A safe error code violates its grammar or size bound.
    InvalidErrorCode,
    /// Event status and optional error code are inconsistent.
    InvalidOutcome,
    /// An operation duration exceeds the fixed upper bound.
    InvalidDuration,
    /// Rotation or retention values are zero or exceed hard limits.
    InvalidRotationPolicy,
    /// One encoded record cannot fit in a bounded segment.
    RecordTooLarge,
    /// Structured JSON encoding failed unexpectedly.
    Serialization,
    /// A sink lock or backing implementation is unavailable.
    SinkUnavailable,
}

impl LogErrorCode {
    /// Returns the stable diagnostic code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidCorrelationId => "log_invalid_correlation_id",
            Self::InvalidErrorCode => "log_invalid_error_code",
            Self::InvalidOutcome => "log_invalid_outcome",
            Self::InvalidDuration => "log_invalid_duration",
            Self::InvalidRotationPolicy => "log_invalid_rotation_policy",
            Self::RecordTooLarge => "log_record_too_large",
            Self::Serialization => "log_serialization_failed",
            Self::SinkUnavailable => "log_sink_unavailable",
        }
    }
}

/// A logging failure whose display text never contains source values.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum LogError {
    /// A correlation identifier is invalid.
    #[error("log correlation identifier is invalid")]
    InvalidCorrelationId,
    /// A safe error code is invalid.
    #[error("log error code is invalid")]
    InvalidErrorCode,
    /// Status and error code do not form a valid outcome.
    #[error("log event outcome is invalid")]
    InvalidOutcome,
    /// Duration is outside the supported bound.
    #[error("log duration is invalid")]
    InvalidDuration,
    /// Rotation or retention policy is invalid.
    #[error("log rotation policy is invalid")]
    InvalidRotationPolicy,
    /// An encoded record is larger than one segment.
    #[error("log record exceeds the segment size limit")]
    RecordTooLarge,
    /// JSON serialization failed.
    #[error("log record serialization failed")]
    Serialization,
    /// The sink cannot accept or return records.
    #[error("log sink is unavailable")]
    SinkUnavailable,
}

impl LogError {
    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(&self) -> LogErrorCode {
        match self {
            Self::InvalidCorrelationId => LogErrorCode::InvalidCorrelationId,
            Self::InvalidErrorCode => LogErrorCode::InvalidErrorCode,
            Self::InvalidOutcome => LogErrorCode::InvalidOutcome,
            Self::InvalidDuration => LogErrorCode::InvalidDuration,
            Self::InvalidRotationPolicy => LogErrorCode::InvalidRotationPolicy,
            Self::RecordTooLarge => LogErrorCode::RecordTooLarge,
            Self::Serialization => LogErrorCode::Serialization,
            Self::SinkUnavailable => LogErrorCode::SinkUnavailable,
        }
    }
}
