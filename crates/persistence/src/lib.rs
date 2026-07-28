//! `SQLite` outbox, delivery receipt, and deduplication adapters.

mod error;
mod model;
mod store;

pub use error::{PersistenceError, PersistenceErrorCode};
pub use model::{
    DeadLetter, EnqueueOutcome, LeasedEvent, ReceiptOutcome, RetryOutcome, StorePolicy,
};
pub use store::{CURRENT_SCHEMA_VERSION, SqliteStore};
