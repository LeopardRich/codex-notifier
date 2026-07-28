//! Application use cases and adapter ports.

mod log_error;
mod log_model;
mod log_sink;

pub use log_error::{LogError, LogErrorCode};
pub use log_model::{
    CorrelationId, EventLogRecord, EventOutcome, EventStatus, LogSeverity, LogTiming,
    SafeDiagnostic, SafeErrorCode,
};
pub use log_sink::{EmitOutcome, InMemoryLogSink, LogSink, RotationPolicy};
