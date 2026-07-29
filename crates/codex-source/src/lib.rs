//! Version-specific Codex event source adapters.

mod approval_requested;
mod capability;
mod context;
mod error;
mod privacy;
mod task_completed;
mod version;

pub use approval_requested::{ApprovalRequestedAdapter, MAX_APPROVAL_REQUESTED_INPUT_BYTES};
pub use capability::{ApprovalInstallation, CapabilityAvailability, CodexCapabilityReport};
pub use context::SourceContext;
pub use error::{SourceError, SourceErrorCode};
pub(crate) use privacy::hash_source_id;
pub use task_completed::{MAX_TASK_COMPLETED_INPUT_BYTES, TaskCompletedAdapter};
pub use version::{CodexCliVersion, CodexInterface};

/// Trusted source labels for the task-completion adapter.
pub type TaskCompletedContext = SourceContext;

/// Trusted source labels for the approval-request adapter.
pub type ApprovalRequestedContext = SourceContext;
