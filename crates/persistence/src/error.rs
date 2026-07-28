//! Stable safe persistence errors.

use thiserror::Error;

/// Stable machine-readable persistence error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PersistenceErrorCode {
    /// The database is busy or locked by another writer.
    DatabaseLocked,
    /// The database path cannot be created or written.
    StorageUnwritable,
    /// The bounded outbox has reached capacity.
    QueueFull,
    /// The event is too old for the configured queue policy.
    EventExpired,
    /// A lease token or transition does not match current state.
    LeaseConflict,
    /// A requested event row does not exist.
    NotFound,
    /// Stored bytes do not decode to the indexed canonical event.
    CorruptData,
    /// The database schema is newer than this binary.
    UnsupportedSchema,
    /// A schema migration or schema validation failed.
    MigrationFailed,
    /// A bounded policy or transition input is invalid.
    InvalidValue,
    /// Another SQLite operation failed safely.
    DatabaseFailure,
}

impl PersistenceErrorCode {
    /// Returns the stable diagnostic code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DatabaseLocked => "storage_database_locked",
            Self::StorageUnwritable => "storage_unwritable",
            Self::QueueFull => "storage_queue_full",
            Self::EventExpired => "storage_event_expired",
            Self::LeaseConflict => "storage_lease_conflict",
            Self::NotFound => "storage_not_found",
            Self::CorruptData => "storage_corrupt_data",
            Self::UnsupportedSchema => "storage_schema_unsupported",
            Self::MigrationFailed => "storage_migration_failed",
            Self::InvalidValue => "storage_invalid_value",
            Self::DatabaseFailure => "storage_database_failed",
        }
    }
}

/// A persistence failure that never contains paths, SQL, or row values.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum PersistenceError {
    /// SQLite reports a busy or locked database.
    #[error("state database is locked")]
    DatabaseLocked,
    /// SQLite cannot create or write the selected database.
    #[error("state database is not writable")]
    StorageUnwritable,
    /// The queue is at its configured hard limit.
    #[error("outbox queue is full")]
    QueueFull,
    /// The event exceeds the allowed queue age.
    #[error("event is too old for the outbox")]
    EventExpired,
    /// The requested leased transition is no longer valid.
    #[error("outbox lease does not match current state")]
    LeaseConflict,
    /// The requested event is absent.
    #[error("outbox event was not found")]
    NotFound,
    /// A stored row fails canonical validation or index consistency.
    #[error("state database contains invalid event data")]
    CorruptData,
    /// The schema version is newer than supported.
    #[error("state database schema is unsupported")]
    UnsupportedSchema,
    /// Migration or schema validation failed.
    #[error("state database migration failed")]
    MigrationFailed,
    /// A policy or transition input violates a fixed bound.
    #[error("persistence value is invalid")]
    InvalidValue,
    /// Another SQLite operation failed.
    #[error("state database operation failed")]
    DatabaseFailure,
}

impl PersistenceError {
    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(&self) -> PersistenceErrorCode {
        match self {
            Self::DatabaseLocked => PersistenceErrorCode::DatabaseLocked,
            Self::StorageUnwritable => PersistenceErrorCode::StorageUnwritable,
            Self::QueueFull => PersistenceErrorCode::QueueFull,
            Self::EventExpired => PersistenceErrorCode::EventExpired,
            Self::LeaseConflict => PersistenceErrorCode::LeaseConflict,
            Self::NotFound => PersistenceErrorCode::NotFound,
            Self::CorruptData => PersistenceErrorCode::CorruptData,
            Self::UnsupportedSchema => PersistenceErrorCode::UnsupportedSchema,
            Self::MigrationFailed => PersistenceErrorCode::MigrationFailed,
            Self::InvalidValue => PersistenceErrorCode::InvalidValue,
            Self::DatabaseFailure => PersistenceErrorCode::DatabaseFailure,
        }
    }
}
