//! Canonical event types, validation, routing, and policy.

mod error;
mod event;
mod json;
pub mod limits;

pub use error::{ErrorCode, EventError, Field};
pub use event::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Routing,
    Urgency,
};
