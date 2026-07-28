//! Typed redacted event records and safe diagnostics.

use codex_notifier_core::{CanonicalEvent, ErrorCode as EventErrorCode, EventId, EventKind};
use serde::Serialize;

use crate::LogError;

const MAX_IDENTIFIER_BYTES: usize = 64;
const MAX_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

/// Structured log severity used for deterministic filtering.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogSeverity {
    /// An operation failed permanently.
    Error,
    /// An operation failed transiently or needs attention.
    Warn,
    /// Normal event lifecycle information.
    Info,
    /// Detailed lifecycle diagnostics with the same mandatory redaction.
    Debug,
    /// Maximum lifecycle detail with the same mandatory redaction.
    Trace,
}

impl LogSeverity {
    /// Returns whether a record at this severity passes `threshold`.
    #[must_use]
    pub const fn enabled_by(self, threshold: Self) -> bool {
        self.rank() <= threshold.rank()
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
            Self::Info => 2,
            Self::Debug => 3,
            Self::Trace => 4,
        }
    }
}

/// Event lifecycle status suitable for logs and diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    /// The event passed validation and was accepted.
    Accepted,
    /// The event was already processed or queued.
    Duplicate,
    /// The event is durably queued for later work.
    Queued,
    /// Delivery or forwarding is currently in progress.
    Delivering,
    /// Delivery completed successfully.
    Delivered,
    /// The event was permanently rejected.
    Rejected,
    /// A transient failure scheduled another attempt.
    RetryScheduled,
    /// A permanent failure moved the event to a bounded dead letter.
    DeadLettered,
}

impl EventStatus {
    const fn requires_error(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::RetryScheduled | Self::DeadLettered
        )
    }

    const fn diagnostic_message(self) -> &'static str {
        match self {
            Self::Accepted => "Event accepted.",
            Self::Duplicate => "Event was already processed.",
            Self::Queued => "Event queued.",
            Self::Delivering => "Event delivery started.",
            Self::Delivered => "Event delivered.",
            Self::Rejected => "Event rejected.",
            Self::RetryScheduled => "Event retry scheduled.",
            Self::DeadLettered => "Event moved to dead letter.",
        }
    }
}

/// A validated bounded operation correlation identifier.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CorrelationId(String);

impl CorrelationId {
    /// Parses an ASCII identifier that cannot inject log fields or lines.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::InvalidCorrelationId`] for empty, overlong, or
    /// non-identifier input.
    pub fn parse(value: impl Into<String>) -> Result<Self, LogError> {
        let value = value.into();
        if valid_identifier(&value, true) {
            Ok(Self(value))
        } else {
            Err(LogError::InvalidCorrelationId)
        }
    }

    /// Returns the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated stable error code, never an arbitrary error message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SafeErrorCode(String);

impl SafeErrorCode {
    /// Parses a lowercase machine-readable error code.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::InvalidErrorCode`] for empty, overlong, uppercase,
    /// control-containing, or punctuation-injecting input.
    pub fn parse(value: impl Into<String>) -> Result<Self, LogError> {
        let value = value.into();
        if valid_identifier(&value, false) {
            Ok(Self(value))
        } else {
            Err(LogError::InvalidErrorCode)
        }
    }

    /// Returns the validated machine-readable code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<EventErrorCode> for SafeErrorCode {
    fn from(value: EventErrorCode) -> Self {
        Self(value.as_str().to_owned())
    }
}

/// Timestamp and optional bounded duration for one event operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogTiming {
    timestamp_ms: i64,
    duration_ms: Option<u64>,
}

impl LogTiming {
    /// Creates deterministic millisecond timing metadata.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::InvalidDuration`] when the duration exceeds seven
    /// days. Zero and the exact upper bound are valid.
    pub const fn new(timestamp_ms: i64, duration_ms: Option<u64>) -> Result<Self, LogError> {
        if matches!(duration_ms, Some(value) if value > MAX_DURATION_MS) {
            return Err(LogError::InvalidDuration);
        }
        Ok(Self {
            timestamp_ms,
            duration_ms,
        })
    }

    /// Returns the Unix timestamp in milliseconds.
    #[must_use]
    pub const fn timestamp_ms(self) -> i64 {
        self.timestamp_ms
    }

    /// Returns the optional operation duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(self) -> Option<u64> {
        self.duration_ms
    }
}

/// A validated status and optional safe failure code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventOutcome {
    status: EventStatus,
    error_code: Option<SafeErrorCode>,
}

impl EventOutcome {
    /// Creates a status/error pair with strict success/failure consistency.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::InvalidOutcome`] when a failure status has no code
    /// or a non-failure status carries a code.
    pub fn new(status: EventStatus, error_code: Option<SafeErrorCode>) -> Result<Self, LogError> {
        if status.requires_error() != error_code.is_some() {
            return Err(LogError::InvalidOutcome);
        }
        Ok(Self { status, error_code })
    }

    /// Returns the event lifecycle status.
    #[must_use]
    pub const fn status(&self) -> EventStatus {
        self.status
    }

    /// Returns the optional safe error code.
    #[must_use]
    pub fn error_code(&self) -> Option<&SafeErrorCode> {
        self.error_code.as_ref()
    }
}

/// One structured event lifecycle record with no display or raw-payload field.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EventLogRecord {
    timestamp_ms: i64,
    severity: LogSeverity,
    event_id: EventId,
    event_kind: EventKind,
    status: EventStatus,
    duration_ms: Option<u64>,
    correlation_id: CorrelationId,
    error_code: Option<SafeErrorCode>,
}

impl EventLogRecord {
    /// Extracts only safe identifiers and type information from an event.
    #[must_use]
    pub fn for_event(
        event: &CanonicalEvent,
        severity: LogSeverity,
        timing: LogTiming,
        correlation_id: CorrelationId,
        outcome: EventOutcome,
    ) -> Self {
        Self {
            timestamp_ms: timing.timestamp_ms,
            severity,
            event_id: event.event_id(),
            event_kind: event.kind(),
            status: outcome.status,
            duration_ms: timing.duration_ms,
            correlation_id,
            error_code: outcome.error_code,
        }
    }

    /// Returns the record timestamp in Unix milliseconds.
    #[must_use]
    pub const fn timestamp_ms(&self) -> i64 {
        self.timestamp_ms
    }

    /// Returns the record severity.
    #[must_use]
    pub const fn severity(&self) -> LogSeverity {
        self.severity
    }

    /// Returns the canonical event identifier.
    #[must_use]
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Returns the canonical event kind.
    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }

    /// Returns the event lifecycle status.
    #[must_use]
    pub const fn status(&self) -> EventStatus {
        self.status
    }

    /// Returns the optional operation duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Returns the validated correlation identifier.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    /// Returns the optional validated safe error code.
    #[must_use]
    pub fn error_code(&self) -> Option<&SafeErrorCode> {
        self.error_code.as_ref()
    }

    /// Serializes one compact JSON record without a physical newline.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::Serialization`] if structured encoding fails.
    pub fn to_json_line(&self) -> Result<String, LogError> {
        serde_json::to_string(self).map_err(|_| LogError::Serialization)
    }

    /// Creates a fixed-message redacted diagnostic for this outcome.
    #[must_use]
    pub fn safe_diagnostic(&self) -> SafeDiagnostic {
        SafeDiagnostic {
            status: self.status,
            error_code: self.error_code.clone(),
            message: self.status.diagnostic_message(),
        }
    }
}

/// Human- and machine-readable diagnostic with fixed non-interpolated text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SafeDiagnostic {
    status: EventStatus,
    error_code: Option<SafeErrorCode>,
    message: &'static str,
}

impl SafeDiagnostic {
    /// Returns the status represented by the diagnostic.
    #[must_use]
    pub const fn status(&self) -> EventStatus {
        self.status
    }

    /// Returns the optional safe error code.
    #[must_use]
    pub fn error_code(&self) -> Option<&SafeErrorCode> {
        self.error_code.as_ref()
    }

    /// Returns fixed text that never interpolates event or adapter input.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }

    /// Serializes the diagnostic as compact structured JSON.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::Serialization`] if encoding fails.
    pub fn to_json(&self) -> Result<String, LogError> {
        serde_json::to_string(self).map_err(|_| LogError::Serialization)
    }

    /// Renders a fixed single-line human diagnostic.
    #[must_use]
    pub fn to_human_line(&self) -> String {
        match &self.error_code {
            Some(code) => format!("{} error_code={}", self.message, code.as_str()),
            None => self.message.to_owned(),
        }
    }
}

fn valid_identifier(value: &str, allow_uppercase: bool) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| {
            let alphanumeric = if allow_uppercase {
                byte.is_ascii_alphanumeric()
            } else {
                byte.is_ascii_lowercase() || byte.is_ascii_digit()
            };
            alphanumeric || (index > 0 && matches!(byte, b'_' | b'-' | b'.'))
        })
}
