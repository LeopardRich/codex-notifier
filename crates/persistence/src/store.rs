//! Transactional `SQLite` store, migrations, and row validation.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use codex_notifier_core::{CanonicalEvent, EventId, EventKind};
use rusqlite::ffi::ErrorCode as SqliteErrorCode;
use rusqlite::{
    Connection, Error as SqliteError, OpenFlags, OptionalExtension, Transaction,
    TransactionBehavior, params,
};
use time::OffsetDateTime;

use crate::{
    DeadLetter, EnqueueOutcome, LeasedEvent, PersistenceError, ReceiptOutcome, RetryOutcome,
    StorePolicy, StoreSnapshot, StoredEventState,
};

/// Current on-disk `SQLite` schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;
const MAX_SAFE_CODE_BYTES: usize = 64;
const MAX_LEASE_TOKEN_BYTES: usize = 64;

const CREATE_SCHEMA_V1: &str = r"
CREATE TABLE IF NOT EXISTS outbox (
    event_id TEXT PRIMARY KEY NOT NULL,
    event_json BLOB NOT NULL CHECK(length(event_json) <= 16384),
    kind TEXT NOT NULL CHECK(kind IN ('approval_requested', 'task_completed')),
    occurred_at_ms INTEGER NOT NULL,
    enqueued_at_ms INTEGER NOT NULL,
    available_at_ms INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN ('queued', 'leased')),
    lease_token TEXT,
    lease_until_ms INTEGER,
    last_error_code TEXT,
    CHECK(
        (state = 'queued' AND lease_token IS NULL AND lease_until_ms IS NULL) OR
        (state = 'leased' AND lease_token IS NOT NULL AND lease_until_ms IS NOT NULL)
    )
);
CREATE INDEX IF NOT EXISTS outbox_available_idx
    ON outbox(state, available_at_ms, lease_until_ms, enqueued_at_ms);
CREATE TABLE IF NOT EXISTS delivery_receipts (
    event_id TEXT PRIMARY KEY NOT NULL,
    delivered_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS delivery_receipts_time_idx
    ON delivery_receipts(delivered_at_ms, event_id);
CREATE TABLE IF NOT EXISTS dead_letters (
    event_id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('approval_requested', 'task_completed')),
    error_code TEXT NOT NULL,
    failed_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS dead_letters_time_idx
    ON dead_letters(failed_at_ms, event_id);
";

/// Single-connection transactional `SQLite` outbox and deduplication store.
pub struct SqliteStore {
    connection: Connection,
    policy: StorePolicy,
}

impl SqliteStore {
    /// Opens or migrates a file-backed database.
    ///
    /// # Errors
    ///
    /// Returns classified lock, writability, schema, migration, integrity, or
    /// database failures without exposing `path`.
    pub fn open(path: &Path, policy: StorePolicy) -> Result<Self, PersistenceError> {
        reject_symlink(path)?;
        let connection = Connection::open(path).map_err(map_sqlite_error)?;
        Self::initialize(connection, policy)
    }

    /// Opens a fresh in-memory database for deterministic tests.
    ///
    /// # Errors
    ///
    /// Returns a classified initialization failure.
    pub fn open_in_memory(policy: StorePolicy) -> Result<Self, PersistenceError> {
        let connection = Connection::open_in_memory().map_err(map_sqlite_error)?;
        Self::initialize(connection, policy)
    }

    /// Inspects bounded queue metadata through a read-only `SQLite` connection.
    ///
    /// This operation never creates or migrates a database and never runs
    /// retention maintenance.
    ///
    /// # Errors
    ///
    /// Returns a classified missing, schema, corruption, lock, or availability
    /// failure without exposing the database path or row contents.
    pub fn inspect_read_only(path: &Path) -> Result<StoreSnapshot, PersistenceError> {
        let connection = open_read_only(path)?;
        verify_current_schema(&connection)?;
        let (queue_entries, oldest_enqueued_at_ms): (i64, Option<i64>) = connection
            .query_row(
                "SELECT COUNT(*), MIN(enqueued_at_ms) FROM outbox",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(map_sqlite_error)?;
        let (receipt_entries, latest_delivered_at_ms): (i64, Option<i64>) = connection
            .query_row(
                "SELECT COUNT(*), MAX(delivered_at_ms) FROM delivery_receipts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(map_sqlite_error)?;
        let dead_letter_entries: i64 = connection
            .query_row("SELECT COUNT(*) FROM dead_letters", [], |row| row.get(0))
            .map_err(map_sqlite_error)?;
        let queue_entries = bounded_count(queue_entries)?;
        let receipt_entries = bounded_count(receipt_entries)?;
        validate_snapshot_timestamp(queue_entries, oldest_enqueued_at_ms)?;
        validate_snapshot_timestamp(receipt_entries, latest_delivered_at_ms)?;
        Ok(StoreSnapshot {
            queue_entries,
            oldest_enqueued_at_ms,
            receipt_entries,
            latest_delivered_at_ms,
            dead_letter_entries: bounded_count(dead_letter_entries)?,
        })
    }

    /// Inspects durable state for one event through a read-only connection.
    ///
    /// # Errors
    ///
    /// Returns a classified missing database, schema, corruption, lock, or
    /// availability failure. Dead-letter output is limited to its validated
    /// safe error code.
    pub fn inspect_event_read_only(
        path: &Path,
        event_id: EventId,
    ) -> Result<Option<StoredEventState>, PersistenceError> {
        let connection = open_read_only(path)?;
        verify_current_schema(&connection)?;
        let event_id = event_id.to_string();
        let (count, state, error_code): (i64, Option<String>, Option<String>) = connection
            .query_row(
                "SELECT COUNT(*), MAX(state), MAX(error_code) FROM (
                    SELECT 'pending' AS state, NULL AS error_code
                      FROM outbox WHERE event_id = ?1
                    UNION ALL
                    SELECT 'delivered', NULL
                      FROM delivery_receipts WHERE event_id = ?1
                    UNION ALL
                    SELECT 'dead_lettered', error_code
                      FROM dead_letters WHERE event_id = ?1
                 )",
                [&event_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(map_sqlite_error)?;
        match (count, state.as_deref(), error_code) {
            (0, None, None) => Ok(None),
            (1, Some("pending"), None) => Ok(Some(StoredEventState::Pending)),
            (1, Some("delivered"), None) => Ok(Some(StoredEventState::Delivered)),
            (1, Some("dead_lettered"), Some(error_code)) => {
                validate_safe_code(&error_code)?;
                Ok(Some(StoredEventState::DeadLettered { error_code }))
            }
            _ => Err(PersistenceError::CorruptData),
        }
    }

    fn initialize(
        mut connection: Connection,
        policy: StorePolicy,
    ) -> Result<Self, PersistenceError> {
        connection
            .busy_timeout(Duration::ZERO)
            .map_err(map_sqlite_error)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(map_sqlite_error)?;
        migrate(&mut connection)?;
        verify_integrity(&connection)?;
        Ok(Self { connection, policy })
    }

    /// Returns the current database schema version.
    ///
    /// # Errors
    ///
    /// Returns a classified database failure if the pragma cannot be read.
    pub fn schema_version(&self) -> Result<u32, PersistenceError> {
        self.connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(map_sqlite_error)
    }

    /// Enqueues one canonical event transactionally with cross-table deduplication.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::EventExpired`],
    /// [`PersistenceError::QueueFull`], or a classified database failure.
    pub fn enqueue(
        &mut self,
        event: &CanonicalEvent,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, PersistenceError> {
        let occurred_at_ms = event_timestamp_ms(event)?;
        if age_exceeds(now_ms, occurred_at_ms, self.policy.event_age_ms()) {
            return Err(PersistenceError::EventExpired);
        }
        let event_json = event.to_json().map_err(|_| PersistenceError::CorruptData)?;
        let event_id = event.event_id().to_string();
        let kind = event_kind_str(event.kind());
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        maintain(&transaction, now_ms, self.policy)?;
        if event_exists(&transaction, &event_id)? {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(EnqueueOutcome::Duplicate);
        }
        let queue_count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM outbox", [], |row| row.get(0))
            .map_err(map_sqlite_error)?;
        if usize::try_from(queue_count).unwrap_or(usize::MAX) >= self.policy.queue_entries() {
            return Err(PersistenceError::QueueFull);
        }
        transaction
            .execute(
                "INSERT INTO outbox (
                    event_id, event_json, kind, occurred_at_ms, enqueued_at_ms,
                    available_at_ms, attempts, state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, 0, 'queued')",
                params![event_id, event_json, kind, occurred_at_ms, now_ms],
            )
            .map_err(map_sqlite_error)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(EnqueueOutcome::Enqueued)
    }

    /// Leases the next available event, recovering expired leases atomically.
    ///
    /// # Errors
    ///
    /// Returns a classified validation, lock, corruption, or database error.
    pub fn lease_next(
        &mut self,
        now_ms: i64,
        lease_token: &str,
    ) -> Result<Option<LeasedEvent>, PersistenceError> {
        validate_identifier(lease_token, MAX_LEASE_TOKEN_BYTES)?;
        let lease_duration = i64::try_from(self.policy.lease_duration_ms())
            .map_err(|_| PersistenceError::InvalidValue)?;
        let lease_until_ms = now_ms
            .checked_add(lease_duration)
            .ok_or(PersistenceError::InvalidValue)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        maintain(&transaction, now_ms, self.policy)?;
        let row = transaction
            .query_row(
                "SELECT event_id, event_json, kind, attempts
                 FROM outbox
                 WHERE available_at_ms <= ?1
                   AND (state = 'queued' OR lease_until_ms <= ?1)
                 ORDER BY available_at_ms, enqueued_at_ms, event_id
                 LIMIT 1",
                [now_ms],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sqlite_error)?;
        let Some((event_id, event_json, kind, attempts)) = row else {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(None);
        };
        let attempt = attempts
            .checked_add(1)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(PersistenceError::CorruptData)?;
        transaction
            .execute(
                "UPDATE outbox
                 SET state = 'leased', lease_token = ?2, lease_until_ms = ?3,
                     attempts = ?4
                 WHERE event_id = ?1",
                params![event_id, lease_token, lease_until_ms, attempt],
            )
            .map_err(map_sqlite_error)?;
        let event = decode_event(&event_id, &kind, &event_json, now_ms)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(Some(LeasedEvent {
            event,
            lease_token: lease_token.to_owned(),
            attempt,
            lease_until_ms,
        }))
    }

    /// Acknowledges a leased delivery, writes its deduplication receipt, then
    /// removes the outbox payload in the same transaction.
    ///
    /// # Errors
    ///
    /// Returns a not-found or lease-conflict error for stale transitions.
    pub fn acknowledge(
        &mut self,
        event_id: EventId,
        lease_token: &str,
        delivered_at_ms: i64,
    ) -> Result<(), PersistenceError> {
        validate_identifier(lease_token, MAX_LEASE_TOKEN_BYTES)?;
        let event_id = event_id.to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        require_lease(&transaction, &event_id, lease_token)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO delivery_receipts(event_id, delivered_at_ms)
                 VALUES (?1, ?2)",
                params![event_id, delivered_at_ms],
            )
            .map_err(map_sqlite_error)?;
        transaction
            .execute("DELETE FROM outbox WHERE event_id = ?1", [&event_id])
            .map_err(map_sqlite_error)?;
        maintain(&transaction, delivered_at_ms, self.policy)?;
        transaction.commit().map_err(map_sqlite_error)
    }

    /// Returns a leased event to the queue or dead-letters it at the attempt limit.
    ///
    /// # Errors
    ///
    /// Returns a validation, not-found, lease-conflict, or database error.
    pub fn retry(
        &mut self,
        event_id: EventId,
        lease_token: &str,
        now_ms: i64,
        available_at_ms: i64,
        error_code: &str,
    ) -> Result<RetryOutcome, PersistenceError> {
        validate_identifier(lease_token, MAX_LEASE_TOKEN_BYTES)?;
        validate_safe_code(error_code)?;
        let retry_delay = available_at_ms.saturating_sub(now_ms);
        if available_at_ms < now_ms
            || retry_delay
                > i64::try_from(self.policy.event_age_ms())
                    .map_err(|_| PersistenceError::InvalidValue)?
        {
            return Err(PersistenceError::InvalidValue);
        }
        let event_id = event_id.to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let (kind, attempts) = require_lease(&transaction, &event_id, lease_token)?;
        let attempts = u32::try_from(attempts).map_err(|_| PersistenceError::CorruptData)?;
        let outcome = if attempts >= self.policy.attempts() {
            insert_dead_letter(&transaction, &event_id, &kind, "retry_exhausted", now_ms)?;
            transaction
                .execute("DELETE FROM outbox WHERE event_id = ?1", [&event_id])
                .map_err(map_sqlite_error)?;
            RetryOutcome::DeadLettered
        } else {
            transaction
                .execute(
                    "UPDATE outbox
                     SET state = 'queued', lease_token = NULL, lease_until_ms = NULL,
                         available_at_ms = ?2, last_error_code = ?3
                     WHERE event_id = ?1",
                    params![event_id, available_at_ms, error_code],
                )
                .map_err(map_sqlite_error)?;
            RetryOutcome::Scheduled
        };
        maintain(&transaction, now_ms, self.policy)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(outcome)
    }

    /// Returns a cancelled lease to immediate availability without consuming
    /// a delivery attempt.
    ///
    /// # Errors
    ///
    /// Returns a validation, not-found, lease-conflict, or database error.
    pub fn release_lease(
        &mut self,
        event_id: EventId,
        lease_token: &str,
        now_ms: i64,
        error_code: &str,
    ) -> Result<(), PersistenceError> {
        validate_identifier(lease_token, MAX_LEASE_TOKEN_BYTES)?;
        validate_safe_code(error_code)?;
        let event_id = event_id.to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let (_, attempts) = require_lease(&transaction, &event_id, lease_token)?;
        if attempts <= 0 {
            return Err(PersistenceError::CorruptData);
        }
        transaction
            .execute(
                "UPDATE outbox
                 SET state = 'queued', lease_token = NULL, lease_until_ms = NULL,
                     available_at_ms = ?2, attempts = attempts - 1,
                     last_error_code = ?3
                 WHERE event_id = ?1",
                params![event_id, now_ms, error_code],
            )
            .map_err(map_sqlite_error)?;
        maintain(&transaction, now_ms, self.policy)?;
        transaction.commit().map_err(map_sqlite_error)
    }

    /// Moves a leased event to metadata-only dead letters transactionally.
    ///
    /// # Errors
    ///
    /// Returns a validation, not-found, lease-conflict, or database error.
    pub fn dead_letter(
        &mut self,
        event_id: EventId,
        lease_token: &str,
        error_code: &str,
        failed_at_ms: i64,
    ) -> Result<(), PersistenceError> {
        validate_identifier(lease_token, MAX_LEASE_TOKEN_BYTES)?;
        validate_safe_code(error_code)?;
        let event_id = event_id.to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let (kind, _) = require_lease(&transaction, &event_id, lease_token)?;
        insert_dead_letter(&transaction, &event_id, &kind, error_code, failed_at_ms)?;
        transaction
            .execute("DELETE FROM outbox WHERE event_id = ?1", [&event_id])
            .map_err(map_sqlite_error)?;
        maintain(&transaction, failed_at_ms, self.policy)?;
        transaction.commit().map_err(map_sqlite_error)
    }

    /// Records a desktop delivery receipt with idempotent deduplication.
    ///
    /// # Errors
    ///
    /// Returns a classified lock or database error.
    pub fn record_delivery(
        &mut self,
        event_id: EventId,
        delivered_at_ms: i64,
    ) -> Result<ReceiptOutcome, PersistenceError> {
        let event_id = event_id.to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        maintain(&transaction, delivered_at_ms, self.policy)?;
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO delivery_receipts(event_id, delivered_at_ms)
                 VALUES (?1, ?2)",
                params![event_id, delivered_at_ms],
            )
            .map_err(map_sqlite_error)?;
        maintain(&transaction, delivered_at_ms, self.policy)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(if inserted == 0 {
            ReceiptOutcome::Duplicate
        } else {
            ReceiptOutcome::Recorded
        })
    }

    /// Returns the number of pending or leased outbox events.
    ///
    /// # Errors
    ///
    /// Returns a classified database failure.
    pub fn queue_len(&self) -> Result<usize, PersistenceError> {
        count_table(&self.connection, "outbox")
    }

    /// Returns when queued state next needs lease, recovery, or age-limit
    /// maintenance, expressed as Unix milliseconds.
    ///
    /// # Errors
    ///
    /// Returns a corruption or classified database failure for invalid state.
    pub fn next_wake_at_ms(&self, now_ms: i64) -> Result<Option<i64>, PersistenceError> {
        let age = i64::try_from(self.policy.event_age_ms())
            .map_err(|_| PersistenceError::InvalidValue)?;
        let (available, expiration): (Option<i64>, Option<i64>) = self
            .connection
            .query_row(
                "SELECT
                    MIN(CASE
                        WHEN state = 'leased' AND lease_until_ms > ?1
                            THEN max(available_at_ms, lease_until_ms)
                        ELSE available_at_ms
                    END),
                    MIN(CASE
                        WHEN state = 'leased' AND lease_until_ms > ?1
                            THEN max(occurred_at_ms + ?2 + 1, lease_until_ms)
                        ELSE occurred_at_ms + ?2 + 1
                    END)
                 FROM outbox",
                params![now_ms, age],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(map_sqlite_error)?;
        Ok(match (available, expiration) {
            (Some(available), Some(expiration)) => Some(available.min(expiration)),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        })
    }

    /// Returns the number of retained delivery receipts.
    ///
    /// # Errors
    ///
    /// Returns a classified database failure.
    pub fn receipt_count(&self) -> Result<usize, PersistenceError> {
        count_table(&self.connection, "delivery_receipts")
    }

    /// Returns the number of retained metadata-only dead letters.
    ///
    /// # Errors
    ///
    /// Returns a classified database failure.
    pub fn dead_letter_count(&self) -> Result<usize, PersistenceError> {
        count_table(&self.connection, "dead_letters")
    }

    /// Loads safe dead-letter metadata by event ID.
    ///
    /// # Errors
    ///
    /// Returns a corruption or classified database failure for invalid rows.
    pub fn dead_letter_entry(
        &self,
        event_id: EventId,
    ) -> Result<Option<DeadLetter>, PersistenceError> {
        let expected_id = event_id.to_string();
        let row = self
            .connection
            .query_row(
                "SELECT event_id, kind, error_code, failed_at_ms
                 FROM dead_letters WHERE event_id = ?1",
                [&expected_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sqlite_error)?;
        row.map(|(id, kind, code, failed_at_ms)| {
            let event_id = EventId::parse(&id).map_err(|_| PersistenceError::CorruptData)?;
            let event_kind = parse_event_kind(&kind)?;
            validate_safe_code(&code)?;
            Ok(DeadLetter {
                event_id,
                event_kind,
                error_code: code,
                failed_at_ms,
            })
        })
        .transpose()
    }
}

fn open_read_only(path: &Path) -> Result<Connection, PersistenceError> {
    let metadata = path.symlink_metadata().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PersistenceError::NotFound
        } else {
            PersistenceError::StorageUnwritable
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PersistenceError::StorageUnwritable);
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(map_sqlite_error)
}

fn verify_current_schema(connection: &Connection) -> Result<(), PersistenceError> {
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(map_sqlite_error)?;
    match version.cmp(&CURRENT_SCHEMA_VERSION) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Greater => Err(PersistenceError::UnsupportedSchema),
        std::cmp::Ordering::Less => Err(PersistenceError::MigrationFailed),
    }
}

fn bounded_count(value: i64) -> Result<usize, PersistenceError> {
    usize::try_from(value).map_err(|_| PersistenceError::CorruptData)
}

fn validate_snapshot_timestamp(
    count: usize,
    timestamp_ms: Option<i64>,
) -> Result<(), PersistenceError> {
    if (count == 0) != timestamp_ms.is_none() {
        return Err(PersistenceError::CorruptData);
    }
    timestamp_ms
        .map(timestamp_from_ms)
        .transpose()
        .map(|_| ())
        .map_err(|_| PersistenceError::CorruptData)
}

fn migrate(connection: &mut Connection) -> Result<(), PersistenceError> {
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(map_migration_error)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedSchema);
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_migration_error)?;
    if version == 0 {
        let has_outbox: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'outbox')",
                [],
                |row| row.get(0),
            )
            .map_err(map_migration_error)?;
        if has_outbox {
            migrate_v0(&transaction)?;
        }
        transaction
            .execute_batch(CREATE_SCHEMA_V1)
            .map_err(map_migration_error)?;
        transaction
            .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)
            .map_err(map_migration_error)?;
    }
    validate_schema_v1(&transaction)?;
    transaction.commit().map_err(map_migration_error)
}

fn migrate_v0(transaction: &Transaction<'_>) -> Result<(), PersistenceError> {
    let expected = BTreeSet::from([
        "attempts",
        "available_at_ms",
        "enqueued_at_ms",
        "event_id",
        "event_json",
        "kind",
        "occurred_at_ms",
    ]);
    if table_columns(transaction, "outbox")? != expected {
        return Err(PersistenceError::MigrationFailed);
    }
    transaction
        .execute_batch(
            "ALTER TABLE outbox ADD COLUMN state TEXT NOT NULL DEFAULT 'queued';
             ALTER TABLE outbox ADD COLUMN lease_token TEXT;
             ALTER TABLE outbox ADD COLUMN lease_until_ms INTEGER;
             ALTER TABLE outbox ADD COLUMN last_error_code TEXT;",
        )
        .map_err(map_migration_error)
}

fn validate_schema_v1(transaction: &Transaction<'_>) -> Result<(), PersistenceError> {
    let outbox = table_columns(transaction, "outbox")?;
    for column in [
        "event_id",
        "event_json",
        "kind",
        "occurred_at_ms",
        "enqueued_at_ms",
        "available_at_ms",
        "attempts",
        "state",
        "lease_token",
        "lease_until_ms",
        "last_error_code",
    ] {
        if !outbox.contains(column) {
            return Err(PersistenceError::MigrationFailed);
        }
    }
    for table in ["delivery_receipts", "dead_letters"] {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )
            .map_err(map_migration_error)?;
        if !exists {
            return Err(PersistenceError::MigrationFailed);
        }
    }
    Ok(())
}

fn table_columns(
    transaction: &Transaction<'_>,
    table: &str,
) -> Result<BTreeSet<&'static str>, PersistenceError> {
    let query = match table {
        "outbox" => "PRAGMA table_info(outbox)",
        _ => return Err(PersistenceError::MigrationFailed),
    };
    let mut statement = transaction.prepare(query).map_err(map_migration_error)?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(map_migration_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_migration_error)?;
    names
        .into_iter()
        .map(|name| match name.as_str() {
            "event_id" => Ok("event_id"),
            "event_json" => Ok("event_json"),
            "kind" => Ok("kind"),
            "occurred_at_ms" => Ok("occurred_at_ms"),
            "enqueued_at_ms" => Ok("enqueued_at_ms"),
            "available_at_ms" => Ok("available_at_ms"),
            "attempts" => Ok("attempts"),
            "state" => Ok("state"),
            "lease_token" => Ok("lease_token"),
            "lease_until_ms" => Ok("lease_until_ms"),
            "last_error_code" => Ok("last_error_code"),
            _ => Err(PersistenceError::MigrationFailed),
        })
        .collect()
}

fn verify_integrity(connection: &Connection) -> Result<(), PersistenceError> {
    let result: String = connection
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if result == "ok" {
        Ok(())
    } else {
        Err(PersistenceError::CorruptData)
    }
}

fn maintain(
    transaction: &Transaction<'_>,
    now_ms: i64,
    policy: StorePolicy,
) -> Result<(), PersistenceError> {
    let event_age =
        i64::try_from(policy.event_age_ms()).map_err(|_| PersistenceError::InvalidValue)?;
    let event_cutoff = now_ms.saturating_sub(event_age);
    transaction
        .execute(
            "INSERT OR IGNORE INTO dead_letters(event_id, kind, error_code, failed_at_ms)
             SELECT event_id, kind, 'event_expired', ?1 FROM outbox
             WHERE occurred_at_ms < ?2
               AND (state = 'queued' OR lease_until_ms <= ?1)",
            params![now_ms, event_cutoff],
        )
        .map_err(map_sqlite_error)?;
    transaction
        .execute(
            "DELETE FROM outbox
             WHERE occurred_at_ms < ?1
               AND (state = 'queued' OR lease_until_ms <= ?2)",
            params![event_cutoff, now_ms],
        )
        .map_err(map_sqlite_error)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO dead_letters(event_id, kind, error_code, failed_at_ms)
             SELECT event_id, kind, 'retry_exhausted', ?1 FROM outbox
             WHERE attempts >= ?2 AND (state = 'queued' OR lease_until_ms <= ?1)",
            params![now_ms, policy.attempts()],
        )
        .map_err(map_sqlite_error)?;
    transaction
        .execute(
            "DELETE FROM outbox
             WHERE attempts >= ?1 AND (state = 'queued' OR lease_until_ms <= ?2)",
            params![policy.attempts(), now_ms],
        )
        .map_err(map_sqlite_error)?;

    prune_table(
        transaction,
        "delivery_receipts",
        "delivered_at_ms",
        now_ms,
        policy.receipt_age_ms(),
        policy.receipt_entries(),
    )?;
    prune_table(
        transaction,
        "dead_letters",
        "failed_at_ms",
        now_ms,
        policy.dead_letter_age_ms(),
        policy.dead_letter_entries(),
    )
}

fn prune_table(
    transaction: &Transaction<'_>,
    table: &str,
    time_column: &str,
    now_ms: i64,
    age_ms: u64,
    entries: usize,
) -> Result<(), PersistenceError> {
    let age = i64::try_from(age_ms).map_err(|_| PersistenceError::InvalidValue)?;
    let cutoff = now_ms.saturating_sub(age);
    let delete_age = match (table, time_column) {
        ("delivery_receipts", "delivered_at_ms") => {
            "DELETE FROM delivery_receipts WHERE delivered_at_ms < ?1"
        }
        ("dead_letters", "failed_at_ms") => "DELETE FROM dead_letters WHERE failed_at_ms < ?1",
        _ => return Err(PersistenceError::DatabaseFailure),
    };
    transaction
        .execute(delete_age, [cutoff])
        .map_err(map_sqlite_error)?;
    let delete_count = match table {
        "delivery_receipts" => {
            "DELETE FROM delivery_receipts WHERE event_id IN (
                SELECT event_id FROM delivery_receipts
                ORDER BY delivered_at_ms DESC, event_id DESC LIMIT -1 OFFSET ?1
             )"
        }
        "dead_letters" => {
            "DELETE FROM dead_letters WHERE event_id IN (
                SELECT event_id FROM dead_letters
                ORDER BY failed_at_ms DESC, event_id DESC LIMIT -1 OFFSET ?1
             )"
        }
        _ => return Err(PersistenceError::DatabaseFailure),
    };
    transaction
        .execute(delete_count, [entries])
        .map_err(map_sqlite_error)?;
    Ok(())
}

fn event_exists(transaction: &Transaction<'_>, event_id: &str) -> Result<bool, PersistenceError> {
    transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM outbox WHERE event_id = ?1
                UNION ALL SELECT 1 FROM delivery_receipts WHERE event_id = ?1
                UNION ALL SELECT 1 FROM dead_letters WHERE event_id = ?1
             )",
            [event_id],
            |row| row.get(0),
        )
        .map_err(map_sqlite_error)
}

fn require_lease(
    transaction: &Transaction<'_>,
    event_id: &str,
    lease_token: &str,
) -> Result<(String, i64), PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT kind, attempts, state, lease_token FROM outbox WHERE event_id = ?1",
            [event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite_error)?;
    match row {
        None => Err(PersistenceError::NotFound),
        Some((kind, attempts, state, token))
            if state == "leased" && token.as_deref() == Some(lease_token) =>
        {
            Ok((kind, attempts))
        }
        Some(_) => Err(PersistenceError::LeaseConflict),
    }
}

fn insert_dead_letter(
    transaction: &Transaction<'_>,
    event_id: &str,
    kind: &str,
    error_code: &str,
    failed_at_ms: i64,
) -> Result<(), PersistenceError> {
    validate_safe_code(error_code)?;
    parse_event_kind(kind)?;
    transaction
        .execute(
            "INSERT OR REPLACE INTO dead_letters(event_id, kind, error_code, failed_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![event_id, kind, error_code, failed_at_ms],
        )
        .map_err(map_sqlite_error)?;
    Ok(())
}

fn decode_event(
    indexed_id: &str,
    indexed_kind: &str,
    bytes: &[u8],
    now_ms: i64,
) -> Result<CanonicalEvent, PersistenceError> {
    let received_at = timestamp_from_ms(now_ms)?;
    let event =
        CanonicalEvent::from_json(bytes, received_at).map_err(|_| PersistenceError::CorruptData)?;
    if event.event_id().to_string() != indexed_id || event_kind_str(event.kind()) != indexed_kind {
        return Err(PersistenceError::CorruptData);
    }
    Ok(event)
}

fn event_timestamp_ms(event: &CanonicalEvent) -> Result<i64, PersistenceError> {
    i64::try_from(event.occurred_at().unix_timestamp_nanos() / 1_000_000)
        .map_err(|_| PersistenceError::InvalidValue)
}

fn timestamp_from_ms(value: i64) -> Result<OffsetDateTime, PersistenceError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000_000)
        .map_err(|_| PersistenceError::InvalidValue)
}

fn age_exceeds(now_ms: i64, occurred_at_ms: i64, maximum_age_ms: u64) -> bool {
    let age = now_ms.saturating_sub(occurred_at_ms);
    age > i64::try_from(maximum_age_ms).unwrap_or(i64::MAX)
}

fn event_kind_str(kind: EventKind) -> &'static str {
    match kind {
        EventKind::ApprovalRequested => "approval_requested",
        EventKind::TaskCompleted => "task_completed",
    }
}

fn parse_event_kind(value: &str) -> Result<EventKind, PersistenceError> {
    match value {
        "approval_requested" => Ok(EventKind::ApprovalRequested),
        "task_completed" => Ok(EventKind::TaskCompleted),
        _ => Err(PersistenceError::CorruptData),
    }
}

fn validate_safe_code(value: &str) -> Result<(), PersistenceError> {
    validate_identifier(value, MAX_SAFE_CODE_BYTES)?;
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        Err(PersistenceError::InvalidValue)
    } else {
        Ok(())
    }
}

fn validate_identifier(value: &str, maximum: usize) -> Result<(), PersistenceError> {
    if value.is_empty()
        || value.len() > maximum
        || !value.is_ascii()
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'_' | b'-' | b'.'))
        })
    {
        Err(PersistenceError::InvalidValue)
    } else {
        Ok(())
    }
}

fn count_table(connection: &Connection, table: &str) -> Result<usize, PersistenceError> {
    let query = match table {
        "outbox" => "SELECT COUNT(*) FROM outbox",
        "delivery_receipts" => "SELECT COUNT(*) FROM delivery_receipts",
        "dead_letters" => "SELECT COUNT(*) FROM dead_letters",
        _ => return Err(PersistenceError::DatabaseFailure),
    };
    let count: i64 = connection
        .query_row(query, [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    usize::try_from(count).map_err(|_| PersistenceError::CorruptData)
}

fn reject_symlink(path: &Path) -> Result<(), PersistenceError> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(PersistenceError::StorageUnwritable)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(PersistenceError::StorageUnwritable),
    }
}

fn map_migration_error(error: SqliteError) -> PersistenceError {
    match map_sqlite_error(error) {
        PersistenceError::DatabaseLocked => PersistenceError::DatabaseLocked,
        PersistenceError::StorageUnwritable => PersistenceError::StorageUnwritable,
        _ => PersistenceError::MigrationFailed,
    }
}

fn map_sqlite_error(error: SqliteError) -> PersistenceError {
    match error {
        SqliteError::SqliteFailure(failure, source_message) => {
            drop(source_message);
            match failure.code {
                SqliteErrorCode::DatabaseBusy | SqliteErrorCode::DatabaseLocked => {
                    PersistenceError::DatabaseLocked
                }
                SqliteErrorCode::ReadOnly
                | SqliteErrorCode::CannotOpen
                | SqliteErrorCode::DiskFull
                | SqliteErrorCode::SystemIoFailure
                | SqliteErrorCode::PermissionDenied => PersistenceError::StorageUnwritable,
                SqliteErrorCode::DatabaseCorrupt | SqliteErrorCode::NotADatabase => {
                    PersistenceError::CorruptData
                }
                _ => PersistenceError::DatabaseFailure,
            }
        }
        corrupt @ (SqliteError::FromSqlConversionFailure(..)
        | SqliteError::IntegralValueOutOfRange(..)
        | SqliteError::Utf8Error(..)
        | SqliteError::InvalidColumnType(..)) => {
            drop(corrupt);
            PersistenceError::CorruptData
        }
        other => {
            drop(other);
            PersistenceError::DatabaseFailure
        }
    }
}
