//! Bounded storage policy and persistence operation results.

use codex_notifier_core::{CanonicalEvent, EventId, EventKind};

use crate::PersistenceError;

const MAX_QUEUE_ENTRIES: usize = 100_000;
const MAX_EVENT_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_LEASE_DURATION_MS: u64 = 60 * 60 * 1_000;
const MAX_ATTEMPTS: u32 = 1_000;
const MAX_RETENTION_ENTRIES: usize = 1_000_000;
const MAX_RETENTION_MS: u64 = 365 * 24 * 60 * 60 * 1_000;

/// Hard-bounded queue, lease, and metadata-retention policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorePolicy {
    queue_entries: usize,
    event_age_ms: u64,
    lease_duration_ms: u64,
    attempts: u32,
    receipt_entries: usize,
    receipt_age_ms: u64,
    dead_letter_entries: usize,
    dead_letter_age_ms: u64,
}

impl StorePolicy {
    /// Sets the outbox entry limit.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::InvalidValue`] for zero or over 100,000.
    pub const fn with_queue_limit(mut self, value: usize) -> Result<Self, PersistenceError> {
        if value == 0 || value > MAX_QUEUE_ENTRIES {
            return Err(PersistenceError::InvalidValue);
        }
        self.queue_entries = value;
        Ok(self)
    }

    /// Sets the maximum age since event occurrence.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::InvalidValue`] for zero or over seven days.
    pub const fn with_max_event_age_ms(mut self, value: u64) -> Result<Self, PersistenceError> {
        if value == 0 || value > MAX_EVENT_AGE_MS {
            return Err(PersistenceError::InvalidValue);
        }
        self.event_age_ms = value;
        Ok(self)
    }

    /// Sets the lease duration.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::InvalidValue`] for zero or over one hour.
    pub const fn with_lease_duration_ms(mut self, value: u64) -> Result<Self, PersistenceError> {
        if value == 0 || value > MAX_LEASE_DURATION_MS {
            return Err(PersistenceError::InvalidValue);
        }
        self.lease_duration_ms = value;
        Ok(self)
    }

    /// Sets the maximum delivery attempts before dead-lettering.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::InvalidValue`] for zero or over 1,000.
    pub const fn with_max_attempts(mut self, value: u32) -> Result<Self, PersistenceError> {
        if value == 0 || value > MAX_ATTEMPTS {
            return Err(PersistenceError::InvalidValue);
        }
        self.attempts = value;
        Ok(self)
    }

    /// Sets delivery receipt count and age retention.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::InvalidValue`] for zero or excessive values.
    pub const fn with_receipt_retention(
        mut self,
        entries: usize,
        age_ms: u64,
    ) -> Result<Self, PersistenceError> {
        validate_retention(entries, age_ms)?;
        self.receipt_entries = entries;
        self.receipt_age_ms = age_ms;
        Ok(self)
    }

    /// Sets metadata-only dead-letter count and age retention.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::InvalidValue`] for zero or excessive values.
    pub const fn with_dead_letter_retention(
        mut self,
        entries: usize,
        age_ms: u64,
    ) -> Result<Self, PersistenceError> {
        validate_retention(entries, age_ms)?;
        self.dead_letter_entries = entries;
        self.dead_letter_age_ms = age_ms;
        Ok(self)
    }

    pub(crate) const fn queue_entries(self) -> usize {
        self.queue_entries
    }

    pub(crate) const fn event_age_ms(self) -> u64 {
        self.event_age_ms
    }

    pub(crate) const fn lease_duration_ms(self) -> u64 {
        self.lease_duration_ms
    }

    pub(crate) const fn attempts(self) -> u32 {
        self.attempts
    }

    pub(crate) const fn receipt_entries(self) -> usize {
        self.receipt_entries
    }

    pub(crate) const fn receipt_age_ms(self) -> u64 {
        self.receipt_age_ms
    }

    pub(crate) const fn dead_letter_entries(self) -> usize {
        self.dead_letter_entries
    }

    pub(crate) const fn dead_letter_age_ms(self) -> u64 {
        self.dead_letter_age_ms
    }
}

impl Default for StorePolicy {
    fn default() -> Self {
        Self {
            queue_entries: 1_000,
            event_age_ms: MAX_EVENT_AGE_MS,
            lease_duration_ms: 30_000,
            attempts: 20,
            receipt_entries: 100_000,
            receipt_age_ms: 30 * 24 * 60 * 60 * 1_000,
            dead_letter_entries: 1_000,
            dead_letter_age_ms: 7 * 24 * 60 * 60 * 1_000,
        }
    }
}

const fn validate_retention(entries: usize, age_ms: u64) -> Result<(), PersistenceError> {
    if entries == 0 || entries > MAX_RETENTION_ENTRIES || age_ms == 0 || age_ms > MAX_RETENTION_MS {
        Err(PersistenceError::InvalidValue)
    } else {
        Ok(())
    }
}

/// Result of a deduplicating transactional enqueue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnqueueOutcome {
    /// A new outbox row was committed.
    Enqueued,
    /// The event ID already exists in the outbox, receipts, or dead letters.
    Duplicate,
}

/// Result of recording a desktop delivery receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptOutcome {
    /// A new receipt was committed.
    Recorded,
    /// A receipt for the event ID already exists.
    Duplicate,
}

/// Result of a transient retry transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryOutcome {
    /// The event returned to the queue for its next available time.
    Scheduled,
    /// The attempt limit moved safe metadata to dead letters.
    DeadLettered,
}

/// One leased canonical event and its compare-and-set token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeasedEvent {
    pub(crate) event: CanonicalEvent,
    pub(crate) lease_token: String,
    pub(crate) attempt: u32,
    pub(crate) lease_until_ms: i64,
}

impl LeasedEvent {
    /// Returns the validated canonical event.
    #[must_use]
    pub const fn event(&self) -> &CanonicalEvent {
        &self.event
    }

    /// Returns the caller-provided validated lease token.
    #[must_use]
    pub fn lease_token(&self) -> &str {
        &self.lease_token
    }

    /// Returns the one-based delivery attempt number.
    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Returns the inclusive lease expiry in Unix milliseconds.
    #[must_use]
    pub const fn lease_until_ms(&self) -> i64 {
        self.lease_until_ms
    }
}

/// Metadata-only permanent failure record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadLetter {
    pub(crate) event_id: EventId,
    pub(crate) event_kind: EventKind,
    pub(crate) error_code: String,
    pub(crate) failed_at_ms: i64,
}

impl DeadLetter {
    /// Returns the failed event identifier.
    #[must_use]
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Returns the failed event kind.
    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }

    /// Returns the validated safe error code.
    #[must_use]
    pub fn error_code(&self) -> &str {
        &self.error_code
    }

    /// Returns the failure time in Unix milliseconds.
    #[must_use]
    pub const fn failed_at_ms(&self) -> i64 {
        self.failed_at_ms
    }
}
