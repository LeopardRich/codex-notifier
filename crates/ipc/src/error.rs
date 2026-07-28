//! Stable safe IPC failures.

use thiserror::Error;

/// Stable machine-readable local IPC error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum IpcErrorCode {
    /// The profile or runtime endpoint is invalid.
    InvalidEndpoint,
    /// Endpoint ownership, type, or permissions are unsafe.
    InsecureEndpoint,
    /// Another live agent already owns the endpoint.
    AlreadyRunning,
    /// A connection could not be established.
    ConnectionFailed,
    /// A bounded operation exceeded its deadline.
    Timeout,
    /// A length prefix exceeds the frame limit.
    FrameTooLarge,
    /// A frame ended before its declared length.
    TruncatedFrame,
    /// The connected process does not belong to the current user.
    UnauthorizedPeer,
    /// Event bytes are not a valid canonical event.
    MalformedEvent,
    /// Acknowledgement bytes or semantics are invalid.
    MalformedAcknowledgement,
    /// The local handler rejected the event.
    HandlerRejected,
    /// Another local transport operation failed.
    TransportFailure,
}

impl IpcErrorCode {
    /// Returns the stable diagnostic code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidEndpoint => "ipc_invalid_endpoint",
            Self::InsecureEndpoint => "ipc_insecure_endpoint",
            Self::AlreadyRunning => "ipc_already_running",
            Self::ConnectionFailed => "ipc_connection_failed",
            Self::Timeout => "ipc_timeout",
            Self::FrameTooLarge => "ipc_frame_too_large",
            Self::TruncatedFrame => "ipc_truncated_frame",
            Self::UnauthorizedPeer => "ipc_unauthorized_peer",
            Self::MalformedEvent => "ipc_malformed_event",
            Self::MalformedAcknowledgement => "ipc_malformed_acknowledgement",
            Self::HandlerRejected => "ipc_handler_rejected",
            Self::TransportFailure => "ipc_transport_failed",
        }
    }
}

/// A local IPC failure whose display text contains no endpoint or payload data.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum IpcError {
    /// Endpoint configuration is invalid.
    #[error("local IPC endpoint is invalid")]
    InvalidEndpoint,
    /// Endpoint ownership, type, or permissions are unsafe.
    #[error("local IPC endpoint is not securely owned")]
    InsecureEndpoint,
    /// A live process already owns the endpoint.
    #[error("local IPC agent is already running")]
    AlreadyRunning,
    /// The client cannot reach the endpoint.
    #[error("local IPC connection failed")]
    ConnectionFailed,
    /// An operation exceeded its fixed deadline.
    #[error("local IPC operation timed out")]
    Timeout,
    /// A frame exceeds its byte limit.
    #[error("local IPC frame exceeds its size limit")]
    FrameTooLarge,
    /// A frame is incomplete.
    #[error("local IPC frame is truncated")]
    TruncatedFrame,
    /// Peer credentials do not match the current user.
    #[error("local IPC peer is not authorized")]
    UnauthorizedPeer,
    /// Request bytes are not a canonical event.
    #[error("local IPC event is invalid")]
    MalformedEvent,
    /// Response bytes or semantics are invalid.
    #[error("local IPC acknowledgement is invalid")]
    MalformedAcknowledgement,
    /// The handler returned a rejected acknowledgement.
    #[error("local IPC event was rejected")]
    HandlerRejected,
    /// Another transport operation failed.
    #[error("local IPC transport failed")]
    TransportFailure,
}

impl IpcError {
    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(&self) -> IpcErrorCode {
        match self {
            Self::InvalidEndpoint => IpcErrorCode::InvalidEndpoint,
            Self::InsecureEndpoint => IpcErrorCode::InsecureEndpoint,
            Self::AlreadyRunning => IpcErrorCode::AlreadyRunning,
            Self::ConnectionFailed => IpcErrorCode::ConnectionFailed,
            Self::Timeout => IpcErrorCode::Timeout,
            Self::FrameTooLarge => IpcErrorCode::FrameTooLarge,
            Self::TruncatedFrame => IpcErrorCode::TruncatedFrame,
            Self::UnauthorizedPeer => IpcErrorCode::UnauthorizedPeer,
            Self::MalformedEvent => IpcErrorCode::MalformedEvent,
            Self::MalformedAcknowledgement => IpcErrorCode::MalformedAcknowledgement,
            Self::HandlerRejected => IpcErrorCode::HandlerRejected,
            Self::TransportFailure => IpcErrorCode::TransportFailure,
        }
    }
}
