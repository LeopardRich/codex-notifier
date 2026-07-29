//! Version-specific Codex event source adapters.

mod error;
mod task_completed;
mod version;

pub use error::{SourceError, SourceErrorCode};
pub use task_completed::{
    MAX_TASK_COMPLETED_INPUT_BYTES, TaskCompletedAdapter, TaskCompletedContext,
};
pub use version::{CodexCliVersion, CodexInterface};
