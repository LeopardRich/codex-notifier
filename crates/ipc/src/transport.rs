//! Deadline-bounded local socket client and server.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use codex_notifier_core::{CanonicalEvent, limits::MAX_EVENT_BYTES};
use interprocess::ConnectWaitMode;
use interprocess::local_socket::{
    ConnectOptions, ListenerOptions,
    tokio::{Listener, Stream, prelude::*},
};
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::protocol::MAX_ACK_BYTES;
use crate::{Acknowledgement, IpcEndpoint, IpcError};

const MAX_CONNECTIONS: usize = 256;
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_TIMEOUT: Duration = Duration::from_millis(10);

/// Bounded connect, I/O, and concurrency policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcPolicy {
    connect_timeout: Duration,
    io_timeout: Duration,
    connections: usize,
}

impl IpcPolicy {
    /// Creates a policy with fixed hard limits.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::InvalidEndpoint`] for timeouts outside 10 ms to 30
    /// seconds or a connection bound outside 1 to 256.
    pub fn new(
        connect_timeout: Duration,
        io_timeout: Duration,
        max_connections: usize,
    ) -> Result<Self, IpcError> {
        if connect_timeout < MIN_TIMEOUT
            || connect_timeout > MAX_TIMEOUT
            || io_timeout < MIN_TIMEOUT
            || io_timeout > MAX_TIMEOUT
            || max_connections == 0
            || max_connections > MAX_CONNECTIONS
        {
            return Err(IpcError::InvalidEndpoint);
        }
        Ok(Self {
            connect_timeout,
            io_timeout,
            connections: max_connections,
        })
    }

    /// Returns the connection establishment deadline.
    #[must_use]
    pub const fn connect_timeout(self) -> Duration {
        self.connect_timeout
    }

    /// Returns the per-frame read/write deadline.
    #[must_use]
    pub const fn io_timeout(self) -> Duration {
        self.io_timeout
    }

    /// Returns the maximum number of active connection tasks.
    #[must_use]
    pub const fn max_connections(self) -> usize {
        self.connections
    }
}

impl Default for IpcPolicy {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(2),
            io_timeout: Duration::from_secs(2),
            connections: 32,
        }
    }
}

/// Per-user local IPC client.
#[derive(Clone, Debug)]
pub struct IpcClient {
    endpoint: IpcEndpoint,
    policy: IpcPolicy,
}

impl IpcClient {
    /// Creates a client without consulting proxy environment variables.
    #[must_use]
    pub const fn new(endpoint: IpcEndpoint, policy: IpcPolicy) -> Self {
        Self { endpoint, policy }
    }

    /// Submits one canonical event and validates its matching acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns a classified connection, peer identity, deadline, frame, or
    /// acknowledgement failure.
    pub async fn submit(&self, event: &CanonicalEvent) -> Result<Acknowledgement, IpcError> {
        let name = self.endpoint.name()?;
        let options = ConnectOptions::new()
            .name(name)
            .wait_mode(ConnectWaitMode::Timeout(self.policy.connect_timeout));
        let mut stream = timeout(self.policy.connect_timeout, options.connect_tokio())
            .await
            .map_err(|_| IpcError::Timeout)?
            .map_err(|_| IpcError::ConnectionFailed)?;
        validate_peer(&stream)?;

        let request = event.to_json().map_err(|_| IpcError::MalformedEvent)?;
        timed_write_frame(&mut stream, &request, self.policy.io_timeout).await?;
        let response = timed_read_frame(&mut stream, MAX_ACK_BYTES, self.policy.io_timeout).await?;
        let acknowledgement = Acknowledgement::from_bytes(&response)?;
        if acknowledgement.event_id() != event.event_id() {
            return Err(IpcError::MalformedAcknowledgement);
        }
        Ok(acknowledgement)
    }
}

/// Per-user local IPC listener.
pub struct IpcServer {
    endpoint: IpcEndpoint,
    policy: IpcPolicy,
    listener: Listener,
}

impl IpcServer {
    /// Securely creates the endpoint or recovers an owned stale Unix socket.
    ///
    /// # Errors
    ///
    /// Returns an ownership/permission error or
    /// [`IpcError::AlreadyRunning`] when a live listener already owns the name.
    pub fn bind(endpoint: IpcEndpoint, policy: IpcPolicy) -> Result<Self, IpcError> {
        #[cfg(unix)]
        {
            endpoint.prepare_runtime_dir()?;
            endpoint.validate_existing_socket()?;
        }
        let name = endpoint.name()?;
        let options = listener_options(ListenerOptions::new().name(name))?;
        let listener = options.create_tokio().map_err(|error| {
            if error.kind() == io::ErrorKind::AddrInUse {
                IpcError::AlreadyRunning
            } else if error.kind() == io::ErrorKind::PermissionDenied {
                IpcError::InsecureEndpoint
            } else {
                IpcError::TransportFailure
            }
        })?;
        Ok(Self {
            endpoint,
            policy,
            listener,
        })
    }

    /// Returns the bound logical endpoint.
    #[must_use]
    pub const fn endpoint(&self) -> &IpcEndpoint {
        &self.endpoint
    }

    /// Accepts connections until `shutdown`, with a hard task bound.
    ///
    /// The handler must return an acknowledgement for the same event ID.
    /// Per-connection protocol failures close only that connection.
    ///
    /// # Errors
    ///
    /// Returns a classified listener or task failure.
    pub async fn serve_until<F, S>(
        &self,
        handler: Arc<F>,
        shutdown: S,
    ) -> Result<ServeReport, IpcError>
    where
        F: Fn(CanonicalEvent) -> Acknowledgement + Send + Sync + 'static,
        S: Future<Output = ()> + Send,
    {
        tokio::pin!(shutdown);
        let semaphore = Arc::new(Semaphore::new(self.policy.connections));
        let mut tasks = JoinSet::new();
        let mut report = ServeReport::default();

        loop {
            while let Some(result) = tasks.try_join_next() {
                record_task_result(&mut report, &result)?;
            }
            let permit = tokio::select! {
                () = &mut shutdown => break,
                permit = Arc::clone(&semaphore).acquire_owned() => {
                    permit.map_err(|_| IpcError::TransportFailure)?
                }
            };
            let stream = tokio::select! {
                () = &mut shutdown => {
                    drop(permit);
                    break;
                }
                result = self.listener.accept() => {
                    result.map_err(|_| IpcError::TransportFailure)?
                }
            };
            let handler = Arc::clone(&handler);
            let io_timeout = self.policy.io_timeout;
            tasks.spawn(async move {
                let _permit = permit;
                handle_connection(stream, handler, io_timeout).await
            });
        }

        while let Some(result) = tasks.join_next().await {
            record_task_result(&mut report, &result)?;
        }
        Ok(report)
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        #[cfg(unix)]
        self.endpoint.remove_owned_socket();
    }
}

/// Completed connection counts from one server run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServeReport {
    /// Connections that completed a request/acknowledgement exchange.
    pub completed: usize,
    /// Connections rejected for identity, framing, timeout, or malformed data.
    pub rejected: usize,
}

async fn handle_connection<F>(
    mut stream: Stream,
    handler: Arc<F>,
    io_timeout: Duration,
) -> Result<(), IpcError>
where
    F: Fn(CanonicalEvent) -> Acknowledgement + Send + Sync + 'static,
{
    validate_peer(&stream)?;
    let request = timed_read_frame(&mut stream, MAX_EVENT_BYTES, io_timeout).await?;
    let event = CanonicalEvent::from_json(&request, OffsetDateTime::now_utc())
        .map_err(|_| IpcError::MalformedEvent)?;
    let acknowledgement = handler(event.clone());
    if acknowledgement.event_id() != event.event_id() {
        return Err(IpcError::MalformedAcknowledgement);
    }
    let response = acknowledgement.to_bytes()?;
    timed_write_frame(&mut stream, &response, io_timeout).await
}

fn record_task_result(
    report: &mut ServeReport,
    result: &Result<Result<(), IpcError>, tokio::task::JoinError>,
) -> Result<(), IpcError> {
    match result {
        Ok(Ok(())) => report.completed += 1,
        Ok(Err(_)) => report.rejected += 1,
        Err(_) => return Err(IpcError::TransportFailure),
    }
    Ok(())
}

async fn timed_read_frame<R>(
    reader: &mut R,
    maximum: usize,
    deadline: Duration,
) -> Result<Vec<u8>, IpcError>
where
    R: AsyncRead + Unpin,
{
    timeout(deadline, read_frame(reader, maximum))
        .await
        .map_err(|_| IpcError::Timeout)?
}

async fn timed_write_frame<W>(
    writer: &mut W,
    bytes: &[u8],
    deadline: Duration,
) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    timeout(deadline, write_frame(writer, bytes))
        .await
        .map_err(|_| IpcError::Timeout)?
}

async fn read_frame<R>(reader: &mut R, maximum: usize) -> Result<Vec<u8>, IpcError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 4];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|error| map_read_error(&error))?;
    let length =
        usize::try_from(u32::from_be_bytes(header)).map_err(|_| IpcError::FrameTooLarge)?;
    if length == 0 || length > maximum {
        return Err(IpcError::FrameTooLarge);
    }
    let mut bytes = vec![0_u8; length];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|error| map_read_error(&error))?;
    Ok(bytes)
}

async fn write_frame<W>(writer: &mut W, bytes: &[u8]) -> Result<(), IpcError>
where
    W: AsyncWrite + Unpin,
{
    let length = u32::try_from(bytes.len()).map_err(|_| IpcError::FrameTooLarge)?;
    if length == 0 {
        return Err(IpcError::FrameTooLarge);
    }
    writer
        .write_all(&length.to_be_bytes())
        .await
        .map_err(|_| IpcError::TransportFailure)?;
    writer
        .write_all(bytes)
        .await
        .map_err(|_| IpcError::TransportFailure)?;
    writer.flush().await.map_err(|_| IpcError::TransportFailure)
}

fn map_read_error(error: &io::Error) -> IpcError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        IpcError::TruncatedFrame
    } else {
        IpcError::TransportFailure
    }
}

#[cfg(unix)]
fn listener_options(
    options: ListenerOptions<'static>,
) -> Result<ListenerOptions<'static>, IpcError> {
    use interprocess::os::unix::local_socket::ListenerOptionsExt;
    Ok(options.mode(0o600).reclaim_name(true).try_overwrite(false))
}

#[cfg(windows)]
fn listener_options(
    options: ListenerOptions<'static>,
) -> Result<ListenerOptions<'static>, IpcError> {
    use interprocess::os::windows::local_socket::ListenerOptionsExt;
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use widestring::U16CString;

    let sddl = U16CString::from_str("D:P(A;;GA;;;OW)").map_err(|_| IpcError::InsecureEndpoint)?;
    let descriptor =
        SecurityDescriptor::deserialize(&sddl).map_err(|_| IpcError::InsecureEndpoint)?;
    Ok(options
        .security_descriptor(descriptor)
        .reclaim_name(true)
        .try_overwrite(false))
}

#[cfg(unix)]
fn validate_peer(stream: &Stream) -> Result<(), IpcError> {
    let credentials = stream
        .peer_creds()
        .map_err(|_| IpcError::UnauthorizedPeer)?;
    validate_user_ids(
        rustix::process::geteuid().as_raw(),
        credentials.euid().ok_or(IpcError::UnauthorizedPeer)?,
    )
}

#[cfg(unix)]
fn validate_user_ids(expected: u32, actual: u32) -> Result<(), IpcError> {
    validate_user_match(expected == actual)
}

fn validate_user_match(matches: bool) -> Result<(), IpcError> {
    if matches {
        Ok(())
    } else {
        Err(IpcError::UnauthorizedPeer)
    }
}

#[cfg(windows)]
fn validate_peer(stream: &Stream) -> Result<(), IpcError> {
    use sysinfo::{Pid, ProcessesToUpdate, System, get_current_pid};

    let peer_pid = stream
        .peer_creds()
        .map_err(|_| IpcError::UnauthorizedPeer)?
        .pid()
        .ok_or(IpcError::UnauthorizedPeer)?;
    let current_pid = get_current_pid().map_err(|_| IpcError::UnauthorizedPeer)?;
    let peer_pid = Pid::from_u32(peer_pid);
    let process_ids = [current_pid, peer_pid];
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&process_ids), true);
    let current_user = system
        .process(current_pid)
        .and_then(sysinfo::Process::user_id)
        .ok_or(IpcError::UnauthorizedPeer)?;
    let peer_user = system
        .process(peer_pid)
        .and_then(sysinfo::Process::user_id)
        .ok_or(IpcError::UnauthorizedPeer)?;
    validate_user_match(current_user == peer_user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, duplex};

    #[tokio::test]
    async fn oversized_truncated_and_slow_frames_are_classified() {
        let (mut client, mut server) = duplex(64);
        client
            .write_all(&(u32::try_from(MAX_EVENT_BYTES + 1).expect("bounded")).to_be_bytes())
            .await
            .expect("write header");
        assert_eq!(
            read_frame(&mut server, MAX_EVENT_BYTES).await,
            Err(IpcError::FrameTooLarge)
        );

        let (mut client, mut server) = duplex(64);
        client
            .write_all(&4_u32.to_be_bytes())
            .await
            .expect("header");
        client.write_all(b"ab").await.expect("partial frame");
        drop(client);
        assert_eq!(
            read_frame(&mut server, MAX_EVENT_BYTES).await,
            Err(IpcError::TruncatedFrame)
        );

        let (_client, mut server) = duplex(64);
        assert_eq!(
            timed_read_frame(&mut server, MAX_EVENT_BYTES, Duration::from_millis(10)).await,
            Err(IpcError::Timeout)
        );
    }

    #[test]
    fn non_current_user_identity_is_rejected_on_every_platform() {
        assert_eq!(validate_user_match(false), Err(IpcError::UnauthorizedPeer));
        assert_eq!(validate_user_match(true), Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn non_current_user_identity_is_rejected() {
        assert_eq!(
            validate_user_ids(1000, 1001),
            Err(IpcError::UnauthorizedPeer)
        );
        assert_eq!(validate_user_ids(1000, 1000), Ok(()));
    }
}
