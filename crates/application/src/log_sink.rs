//! Structured log sink port and deterministic bounded memory implementation.

use std::sync::Mutex;

use crate::{EventLogRecord, LogError, LogSeverity};

const DEFAULT_SEGMENT_BYTES: usize = 1024 * 1024;
const DEFAULT_RETAINED_SEGMENTS: usize = 5;
const DEFAULT_MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_SEGMENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_RETAINED_SEGMENTS: usize = 64;
const MAX_RETENTION_AGE_MS: u64 = 365 * 24 * 60 * 60 * 1_000;

/// Hard-bounded segment rotation and age/count retention policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RotationPolicy {
    segment_bytes: usize,
    retained_segments: usize,
    age_ms: u64,
}

impl RotationPolicy {
    /// Creates a rotation policy within fixed disk-use safety limits.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::InvalidRotationPolicy`] when any value is zero or
    /// exceeds its hard maximum.
    pub const fn new(
        max_segment_bytes: usize,
        max_retained_segments: usize,
        max_age_ms: u64,
    ) -> Result<Self, LogError> {
        if max_segment_bytes == 0
            || max_segment_bytes > MAX_SEGMENT_BYTES
            || max_retained_segments == 0
            || max_retained_segments > MAX_RETAINED_SEGMENTS
            || max_age_ms == 0
            || max_age_ms > MAX_RETENTION_AGE_MS
        {
            return Err(LogError::InvalidRotationPolicy);
        }
        Ok(Self {
            segment_bytes: max_segment_bytes,
            retained_segments: max_retained_segments,
            age_ms: max_age_ms,
        })
    }

    /// Returns the encoded byte limit for one segment.
    #[must_use]
    pub const fn max_segment_bytes(self) -> usize {
        self.segment_bytes
    }

    /// Returns the maximum number of retained segments.
    #[must_use]
    pub const fn max_retained_segments(self) -> usize {
        self.retained_segments
    }

    /// Returns the inclusive maximum segment age in milliseconds.
    #[must_use]
    pub const fn max_age_ms(self) -> u64 {
        self.age_ms
    }
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            segment_bytes: DEFAULT_SEGMENT_BYTES,
            retained_segments: DEFAULT_RETAINED_SEGMENTS,
            age_ms: DEFAULT_MAX_AGE_MS,
        }
    }
}

/// Result of applying severity filtering to a valid record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmitOutcome {
    /// The record was stored by the sink.
    Recorded,
    /// The record was below the configured verbosity threshold.
    Filtered,
}

/// Application port for structured redacted event logs.
pub trait LogSink: Send + Sync {
    /// Emits one already-redacted typed record.
    ///
    /// # Errors
    ///
    /// Returns a safe logging error when serialization or the sink fails.
    fn emit(&self, record: &EventLogRecord) -> Result<EmitOutcome, LogError>;
}

/// Thread-safe deterministic memory sink used by tests and adapter fakes.
#[derive(Debug)]
pub struct InMemoryLogSink {
    threshold: LogSeverity,
    policy: RotationPolicy,
    state: Mutex<MemoryState>,
}

impl InMemoryLogSink {
    /// Creates an empty sink with explicit filtering and retention policy.
    #[must_use]
    pub const fn new(threshold: LogSeverity, policy: RotationPolicy) -> Self {
        Self {
            threshold,
            policy,
            state: Mutex::new(MemoryState {
                watermark_ms: i64::MIN,
                segments: Vec::new(),
            }),
        }
    }

    /// Returns retained JSON records in emission order.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::SinkUnavailable`] if another thread poisoned the
    /// memory sink lock.
    pub fn records(&self) -> Result<Vec<String>, LogError> {
        let state = self.state.lock().map_err(|_| LogError::SinkUnavailable)?;
        Ok(state
            .segments
            .iter()
            .flat_map(|segment| segment.records.iter().cloned())
            .collect())
    }

    /// Returns the current retained segment count.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::SinkUnavailable`] if the sink lock is poisoned.
    pub fn segment_count(&self) -> Result<usize, LogError> {
        let state = self.state.lock().map_err(|_| LogError::SinkUnavailable)?;
        Ok(state.segments.len())
    }
}

impl LogSink for InMemoryLogSink {
    fn emit(&self, record: &EventLogRecord) -> Result<EmitOutcome, LogError> {
        if !record.severity().enabled_by(self.threshold) {
            return Ok(EmitOutcome::Filtered);
        }
        let encoded = record.to_json_line()?;
        let encoded_bytes = encoded
            .len()
            .checked_add(1)
            .ok_or(LogError::RecordTooLarge)?;
        if encoded_bytes > self.policy.segment_bytes {
            return Err(LogError::RecordTooLarge);
        }

        let mut state = self.state.lock().map_err(|_| LogError::SinkUnavailable)?;
        state.watermark_ms = state.watermark_ms.max(record.timestamp_ms());
        state.prune_expired(self.policy.age_ms);

        let rotate = state.segments.last().is_none_or(|segment| {
            segment.bytes.saturating_add(encoded_bytes) > self.policy.segment_bytes
        });
        if rotate {
            state.segments.push(MemorySegment {
                last_timestamp_ms: record.timestamp_ms(),
                bytes: 0,
                records: Vec::new(),
            });
        }
        let segment = state.segments.last_mut().ok_or(LogError::SinkUnavailable)?;
        segment.last_timestamp_ms = segment.last_timestamp_ms.max(record.timestamp_ms());
        segment.bytes += encoded_bytes;
        segment.records.push(encoded);

        let excess = state
            .segments
            .len()
            .saturating_sub(self.policy.retained_segments);
        if excess > 0 {
            state.segments.drain(..excess);
        }
        Ok(EmitOutcome::Recorded)
    }
}

#[derive(Debug)]
struct MemoryState {
    watermark_ms: i64,
    segments: Vec<MemorySegment>,
}

impl MemoryState {
    fn prune_expired(&mut self, max_age_ms: u64) {
        let maximum_age = i64::try_from(max_age_ms).unwrap_or(i64::MAX);
        self.segments.retain(|segment| {
            self.watermark_ms.saturating_sub(segment.last_timestamp_ms) <= maximum_age
        });
    }
}

#[derive(Debug)]
struct MemorySegment {
    last_timestamp_ms: i64,
    bytes: usize,
    records: Vec<String>,
}
