//! Per-user local IPC transport adapters.

mod endpoint;
mod error;
mod protocol;
mod transport;

pub use endpoint::IpcEndpoint;
pub use error::{IpcError, IpcErrorCode};
pub use protocol::{AckError, AckStatus, Acknowledgement};
pub use transport::{IpcClient, IpcPolicy, IpcServer, ServeReport};
