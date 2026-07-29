//! Restricted system OpenSSH receive boundary and security diagnostics.

use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use codex_notifier_application::{
    CancellationToken, DeliveryFailure, DeliveryFuture, DeliveryOutcome, EventDelivery,
    SafeErrorCode,
};
use codex_notifier_core::limits::{MAX_ACK_BYTES, MAX_EVENT_BYTES};
use codex_notifier_core::{CanonicalEvent, EventError, EventId};
use codex_notifier_ipc::{AckError, AckStatus, Acknowledgement, IpcError};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command as TokioCommand};
use tokio::task::JoinHandle;

/// The only client-requested command accepted by the forced entry point.
pub const REQUESTED_COMMAND: &str = "codex-notifier receive";

const MAX_SSH_CONNECTION_BYTES: usize = 512;
const MAX_EVENT_READ_BYTES: u64 = 16_385;
const MAX_SSH_CONFIG_BYTES: usize = 64 * 1024;
const MAX_SSH_STDERR_BYTES: usize = 8 * 1024;
const MIN_CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(120);
const OPERATION_TIMEOUT_ALLOWANCE: Duration = Duration::from_secs(5);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(5);
#[cfg(windows)]
const WINDOWS_PERMISSION_SCRIPT: &str = r"$ErrorActionPreference='Stop';$p=$env:CODEX_NOTIFIER_AUTHORIZED_KEYS;$acl=[System.IO.File]::GetAccessControl($p);$sidType=[Security.Principal.SecurityIdentifier];$me=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$allowed=@($me,'S-1-5-18','S-1-5-32-544');$owner=$acl.GetOwner($sidType).Value;$rules=$acl.GetAccessRules($true,$true,$sidType);$unsafe=$false;foreach($rule in $rules){$sid=$rule.IdentityReference.Value;$write=($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::WriteData) -or ($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::Modify) -or ($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::FullControl);if($rule.AccessControlType -eq 'Allow' -and $write -and $allowed -notcontains $sid){$unsafe=$true}};if((-not $acl.AreAccessRulesProtected)-or($allowed -notcontains $owner)-or$unsafe){exit 3};exit 0";

/// Validated fixed inputs for one system OpenSSH relay relationship.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenSshConfig {
    host_alias: String,
    connect_timeout: Duration,
    config_file: Option<PathBuf>,
}

impl OpenSshConfig {
    /// Creates a bounded system OpenSSH configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SshDeliveryError::InvalidConfiguration`] for an unsafe host
    /// alias or a connection timeout outside 100 milliseconds to 120 seconds.
    pub fn new(
        host_alias: impl Into<String>,
        connect_timeout: Duration,
    ) -> Result<Self, SshDeliveryError> {
        let host_alias = host_alias.into();
        if !valid_host_alias(&host_alias)
            || !(MIN_CONNECT_TIMEOUT..=MAX_CONNECT_TIMEOUT).contains(&connect_timeout)
        {
            return Err(SshDeliveryError::InvalidConfiguration);
        }
        Ok(Self {
            host_alias,
            connect_timeout,
            config_file: None,
        })
    }

    /// Selects an absolute OpenSSH client configuration file.
    ///
    /// This is primarily useful for isolated deployments and verification.
    /// Event data can never select or alter the path.
    ///
    /// # Errors
    ///
    /// Returns [`SshDeliveryError::InvalidConfiguration`] for a relative path.
    pub fn with_config_file(mut self, path: impl Into<PathBuf>) -> Result<Self, SshDeliveryError> {
        let path = path.into();
        if !path.is_absolute() {
            return Err(SshDeliveryError::InvalidConfiguration);
        }
        self.config_file = Some(path);
        Ok(self)
    }

    fn arguments(&self) -> Vec<OsString> {
        let mut arguments = Vec::with_capacity(24);
        if let Some(path) = &self.config_file {
            arguments.push(OsString::from("-F"));
            arguments.push(path.as_os_str().to_owned());
        }
        let timeout_seconds = self
            .connect_timeout
            .as_secs()
            .saturating_add(u64::from(self.connect_timeout.subsec_nanos() > 0));
        arguments.extend([
            OsString::from("-T"),
            OsString::from("-o"),
            OsString::from("BatchMode=yes"),
            OsString::from("-o"),
            OsString::from("ClearAllForwardings=yes"),
            OsString::from("-o"),
            OsString::from("ConnectionAttempts=1"),
            OsString::from("-o"),
            OsString::from(format!("ConnectTimeout={timeout_seconds}")),
            OsString::from("-o"),
            OsString::from("ForwardAgent=no"),
            OsString::from("-o"),
            OsString::from("NumberOfPasswordPrompts=0"),
            OsString::from("-o"),
            OsString::from("RequestTTY=no"),
            OsString::from("-o"),
            OsString::from("StrictHostKeyChecking=yes"),
            OsString::from("--"),
            OsString::from(&self.host_alias),
            OsString::from(REQUESTED_COMMAND),
        ]);
        arguments
    }

    fn operation_timeout(&self) -> Duration {
        self.connect_timeout
            .checked_add(OPERATION_TIMEOUT_ALLOWANCE)
            .unwrap_or(MAX_CONNECT_TIMEOUT + OPERATION_TIMEOUT_ALLOWANCE)
    }
}

/// Stable, payload-free system OpenSSH delivery failures.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum SshDeliveryError {
    /// The fixed host alias, timeout, or optional configuration path is invalid.
    #[error("OpenSSH relay configuration is invalid")]
    InvalidConfiguration,
    /// The system OpenSSH client could not be started.
    #[error("system OpenSSH client is unavailable")]
    ExecutableUnavailable,
    /// The bounded SSH operation exceeded its deadline.
    #[error("OpenSSH connection timed out")]
    ConnectionTimeout,
    /// The configured destination could not be reached.
    #[error("OpenSSH destination is unavailable")]
    NetworkUnavailable,
    /// Public-key or account authentication failed.
    #[error("OpenSSH authentication failed")]
    AuthenticationFailed,
    /// Strict host-key verification failed.
    #[error("OpenSSH host-key verification failed")]
    HostKeyVerificationFailed,
    /// The OpenSSH process or its bounded pipes failed.
    #[error("OpenSSH delivery process failed")]
    ProcessFailed,
    /// Remote stdout or diagnostic stderr exceeded its hard limit.
    #[error("OpenSSH output exceeded its size limit")]
    OutputTooLarge,
    /// The remote response was malformed or did not match the request.
    #[error("OpenSSH acknowledgement is invalid")]
    AcknowledgementInvalid,
    /// The restricted receiver returned one validated safe rejection.
    #[error("OpenSSH receiver rejected the event")]
    RemoteRejected {
        /// Validated machine-readable code supplied by the receiver.
        code: SafeErrorCode,
        /// Whether the receiver classified the same operation as retryable.
        retryable: bool,
    },
    /// Cooperative shutdown cancelled the attempt before acknowledgement.
    #[error("OpenSSH delivery was cancelled")]
    Cancelled,
}

impl SshDeliveryError {
    /// Returns the stable safe error code used by retry and dead-letter state.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::InvalidConfiguration => "ssh_configuration_invalid",
            Self::ExecutableUnavailable => "ssh_executable_unavailable",
            Self::ConnectionTimeout => "ssh_connection_timeout",
            Self::NetworkUnavailable => "ssh_network_unavailable",
            Self::AuthenticationFailed => "ssh_authentication_failed",
            Self::HostKeyVerificationFailed => "ssh_host_key_failed",
            Self::ProcessFailed => "ssh_process_failed",
            Self::OutputTooLarge => "ssh_output_too_large",
            Self::AcknowledgementInvalid => "ssh_acknowledgement_invalid",
            Self::RemoteRejected { code, .. } => code.as_str(),
            Self::Cancelled => "ssh_cancelled",
        }
    }

    /// Returns whether a later attempt with unchanged event data may succeed.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        match self {
            Self::ExecutableUnavailable
            | Self::ConnectionTimeout
            | Self::NetworkUnavailable
            | Self::ProcessFailed => true,
            Self::RemoteRejected { retryable, .. } => *retryable,
            Self::InvalidConfiguration
            | Self::AuthenticationFailed
            | Self::HostKeyVerificationFailed
            | Self::OutputTooLarge
            | Self::AcknowledgementInvalid
            | Self::Cancelled => false,
        }
    }
}

/// Role-delivery adapter that invokes the system OpenSSH client once per event.
#[derive(Clone, Debug)]
pub struct OpenSshDelivery {
    config: OpenSshConfig,
}

impl OpenSshDelivery {
    /// Creates a system OpenSSH delivery adapter from validated fixed inputs.
    #[must_use]
    pub const fn new(config: OpenSshConfig) -> Self {
        Self { config }
    }

    async fn deliver_once(
        &self,
        event: &CanonicalEvent,
        cancellation: CancellationToken,
    ) -> Result<(), SshDeliveryError> {
        if cancellation.is_cancelled() {
            return Err(SshDeliveryError::Cancelled);
        }
        let input = event
            .to_json()
            .map_err(|_| SshDeliveryError::AcknowledgementInvalid)?;
        let capture = run_ssh_process(&self.config, input, cancellation).await?;
        if !capture.status.success() {
            return Err(classify_process_failure(&capture.stderr));
        }
        if capture.stdin_failed || capture.stdout_failed || capture.stderr_failed {
            return Err(SshDeliveryError::ProcessFailed);
        }
        validate_delivery_acknowledgement(event.event_id(), &capture.stdout)
    }
}

fn validate_delivery_acknowledgement(
    event_id: EventId,
    bytes: &[u8],
) -> Result<(), SshDeliveryError> {
    let acknowledgement =
        Acknowledgement::from_json(bytes).map_err(|_| SshDeliveryError::AcknowledgementInvalid)?;
    if acknowledgement.event_id() != event_id {
        return Err(SshDeliveryError::AcknowledgementInvalid);
    }
    match acknowledgement.status() {
        AckStatus::Accepted | AckStatus::Duplicate | AckStatus::Delivered => Ok(()),
        AckStatus::Rejected => {
            let error = acknowledgement
                .error()
                .ok_or(SshDeliveryError::AcknowledgementInvalid)?;
            let code = SafeErrorCode::parse(error.code())
                .map_err(|_| SshDeliveryError::AcknowledgementInvalid)?;
            Err(SshDeliveryError::RemoteRejected {
                code,
                retryable: error.retryable(),
            })
        }
    }
}

impl EventDelivery for OpenSshDelivery {
    fn deliver<'a>(
        &'a self,
        event: &'a CanonicalEvent,
        cancellation: CancellationToken,
    ) -> DeliveryFuture<'a> {
        Box::pin(async move {
            match self.deliver_once(event, cancellation).await {
                Ok(()) => DeliveryOutcome::Delivered,
                Err(SshDeliveryError::Cancelled) => DeliveryOutcome::Cancelled,
                Err(error) => DeliveryOutcome::Failed(DeliveryFailure::new(
                    SafeErrorCode::parse(error.code()).expect("SSH delivery codes are valid"),
                    error.retryable(),
                )),
            }
        })
    }
}

struct ProcessCapture {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdin_failed: bool,
    stdout_failed: bool,
    stderr_failed: bool,
}

async fn run_ssh_process(
    config: &OpenSshConfig,
    input: Vec<u8>,
    cancellation: CancellationToken,
) -> Result<ProcessCapture, SshDeliveryError> {
    let mut command = TokioCommand::new("ssh");
    command
        .args(config.arguments())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|_| SshDeliveryError::ExecutableUnavailable)?;
    let stdin = child.stdin.take().ok_or(SshDeliveryError::ProcessFailed)?;
    let stdout = child.stdout.take().ok_or(SshDeliveryError::ProcessFailed)?;
    let stderr = child.stderr.take().ok_or(SshDeliveryError::ProcessFailed)?;
    let mut stdin_task = Some(tokio::spawn(async move {
        let mut stdin = stdin;
        let result = stdin.write_all(&input).await;
        let shutdown = stdin.shutdown().await;
        result.and(shutdown)
    }));
    let mut stdout_task = Some(tokio::spawn(read_bounded_async(stdout, MAX_ACK_BYTES)));
    let mut stderr_task = Some(tokio::spawn(read_bounded_async(
        stderr,
        MAX_SSH_STDERR_BYTES,
    )));
    let mut stdin_failed = None;
    let mut stdout_result = None;
    let mut stderr_result = None;
    let mut status = None;
    let deadline = tokio::time::Instant::now() + config.operation_timeout();

    loop {
        collect_pipe_results(
            &mut stdin_task,
            &mut stdout_task,
            &mut stderr_task,
            &mut stdin_failed,
            &mut stdout_result,
            &mut stderr_result,
        )
        .await?;
        if matches!(stdout_result, Some(Ok(ref bytes)) if bytes.len() > MAX_ACK_BYTES)
            || matches!(stderr_result, Some(Ok(ref bytes)) if bytes.len() > MAX_SSH_STDERR_BYTES)
        {
            terminate_process(
                &mut child,
                &mut stdin_task,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(SshDeliveryError::OutputTooLarge);
        }
        if status.is_none() {
            status = child
                .try_wait()
                .map_err(|_| SshDeliveryError::ProcessFailed)?;
        }
        if status.is_some()
            && stdin_failed.is_some()
            && stdout_result.is_some()
            && stderr_result.is_some()
        {
            let stdout = stdout_result.take().expect("completed stdout capture");
            let stderr = stderr_result.take().expect("completed stderr capture");
            return Ok(ProcessCapture {
                status: status.expect("completed SSH process"),
                stdout_failed: stdout.is_err(),
                stderr_failed: stderr.is_err(),
                stdout: stdout.unwrap_or_default(),
                stderr: stderr.unwrap_or_default(),
                stdin_failed: stdin_failed.expect("completed stdin writer"),
            });
        }
        tokio::select! {
            () = cancellation.cancelled() => {
                terminate_process(
                    &mut child,
                    &mut stdin_task,
                    &mut stdout_task,
                    &mut stderr_task,
                ).await;
                return Err(SshDeliveryError::Cancelled);
            }
            () = tokio::time::sleep_until(deadline) => {
                terminate_process(
                    &mut child,
                    &mut stdin_task,
                    &mut stdout_task,
                    &mut stderr_task,
                ).await;
                return Err(SshDeliveryError::ConnectionTimeout);
            }
            () = tokio::time::sleep(PROCESS_POLL_INTERVAL) => {}
        }
    }
}

async fn read_bounded_async(
    reader: impl AsyncRead + Unpin,
    maximum: usize,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(maximum.min(1024));
    reader
        .take(u64::try_from(maximum + 1).expect("bounded SSH output"))
        .read_to_end(&mut bytes)
        .await?;
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
async fn collect_pipe_results(
    stdin_task: &mut Option<JoinHandle<std::io::Result<()>>>,
    stdout_task: &mut Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    stderr_task: &mut Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    stdin_failed: &mut Option<bool>,
    stdout_result: &mut Option<std::io::Result<Vec<u8>>>,
    stderr_result: &mut Option<std::io::Result<Vec<u8>>>,
) -> Result<(), SshDeliveryError> {
    if stdin_task.as_ref().is_some_and(JoinHandle::is_finished) {
        let result = stdin_task
            .take()
            .expect("finished stdin task")
            .await
            .map_err(|_| SshDeliveryError::ProcessFailed)?;
        *stdin_failed = Some(result.is_err());
    }
    if stdout_task.as_ref().is_some_and(JoinHandle::is_finished) {
        *stdout_result = Some(
            stdout_task
                .take()
                .expect("finished stdout task")
                .await
                .map_err(|_| SshDeliveryError::ProcessFailed)?,
        );
    }
    if stderr_task.as_ref().is_some_and(JoinHandle::is_finished) {
        *stderr_result = Some(
            stderr_task
                .take()
                .expect("finished stderr task")
                .await
                .map_err(|_| SshDeliveryError::ProcessFailed)?,
        );
    }
    Ok(())
}

async fn terminate_process(
    child: &mut Child,
    stdin_task: &mut Option<JoinHandle<std::io::Result<()>>>,
    stdout_task: &mut Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    stderr_task: &mut Option<JoinHandle<std::io::Result<Vec<u8>>>>,
) {
    let _ = child.kill().await;
    if let Some(task) = stdin_task.take() {
        task.abort();
    }
    if let Some(task) = stdout_task.take() {
        task.abort();
    }
    if let Some(task) = stderr_task.take() {
        task.abort();
    }
}

fn classify_process_failure(stderr: &[u8]) -> SshDeliveryError {
    let diagnostic = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if diagnostic.contains("remote host identification has changed")
        || diagnostic.contains("host key verification failed")
        || diagnostic.contains("possible dns spoofing detected")
        || diagnostic.contains("no matching host key type found")
        || diagnostic.contains("host key for") && diagnostic.contains("has changed")
    {
        SshDeliveryError::HostKeyVerificationFailed
    } else if diagnostic.contains("permission denied")
        || diagnostic.contains("authentication failed")
        || diagnostic.contains("too many authentication failures")
        || diagnostic.contains("no supported authentication methods")
    {
        SshDeliveryError::AuthenticationFailed
    } else if diagnostic.contains("connection timed out")
        || diagnostic.contains("operation timed out")
        || diagnostic.contains("connect to host") && diagnostic.contains("timed out")
    {
        SshDeliveryError::ConnectionTimeout
    } else if diagnostic.contains("connection refused")
        || diagnostic.contains("network is unreachable")
        || diagnostic.contains("no route to host")
        || diagnostic.contains("could not resolve hostname")
        || diagnostic.contains("name or service not known")
        || diagnostic.contains("connection reset")
        || diagnostic.contains("connection closed")
    {
        SshDeliveryError::NetworkUnavailable
    } else {
        SshDeliveryError::ProcessFailed
    }
}

/// Fixed, payload-free receive failure classifications.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ReceiveError {
    /// The forced SSH session did not request exactly the receive command.
    #[error("SSH receive session is not authorized")]
    SessionRejected,
    /// Reading the bounded stdin stream failed.
    #[error("SSH receive input is unavailable")]
    Stdin,
    /// The event failed canonical protocol validation.
    #[error("SSH receive event is invalid")]
    Event(#[from] EventError),
    /// The acknowledgement could not be serialized within its bound.
    #[error("SSH receive acknowledgement is unavailable")]
    Acknowledgement,
}

impl ReceiveError {
    /// Returns the stable safe acknowledgement error code.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::SessionRejected => "ssh_session_rejected",
            Self::Stdin => "receive_stdin_failed",
            Self::Event(error) => error.code().as_str(),
            Self::Acknowledgement => "receive_acknowledgement_failed",
        }
    }

    /// Returns whether retrying the same operation may succeed.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Stdin | Self::Acknowledgement)
    }

    /// Returns a fixed single-line message that contains no input data.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            Self::SessionRejected => "SSH session request is not allowed",
            Self::Stdin => "Event input could not be read",
            Self::Event(EventError::PayloadTooLarge) => "Event exceeds the size limit",
            Self::Event(_) => "Event does not satisfy protocol version 1",
            Self::Acknowledgement => "Acknowledgement could not be encoded",
        }
    }
}

/// Validates the environment supplied to the forced SSH command.
///
/// A legitimate client requests exactly [`REQUESTED_COMMAND`]. OpenSSH places
/// that untrusted request in `SSH_ORIGINAL_COMMAND` while executing the fixed
/// authorized-key command. Shell requests have no original command, and a PTY
/// sets `SSH_TTY`; both are rejected before stdin is read.
///
/// # Errors
///
/// Returns [`ReceiveError::SessionRejected`] for a local invocation, shell,
/// PTY, extra argument, non-Unicode value, or malformed SSH connection marker.
pub fn validate_receive_session(
    original_command: Option<&OsStr>,
    ssh_connection: Option<&OsStr>,
    ssh_tty: Option<&OsStr>,
) -> Result<(), ReceiveError> {
    let original_command = original_command
        .and_then(OsStr::to_str)
        .ok_or(ReceiveError::SessionRejected)?;
    let ssh_connection = ssh_connection
        .and_then(OsStr::to_str)
        .ok_or(ReceiveError::SessionRejected)?;
    if original_command != REQUESTED_COMMAND
        || ssh_tty.is_some()
        || ssh_connection.is_empty()
        || ssh_connection.len() > MAX_SSH_CONNECTION_BYTES
        || ssh_connection.chars().any(char::is_control)
    {
        return Err(ReceiveError::SessionRejected);
    }
    Ok(())
}

/// Reads and validates exactly one canonical event from a stream.
///
/// # Errors
///
/// Returns a stable stdin or canonical protocol error. At most 16,385 bytes
/// are read, so an oversized sender cannot cause an unbounded allocation.
pub fn read_event(
    reader: &mut impl Read,
    received_at: OffsetDateTime,
) -> Result<CanonicalEvent, ReceiveError> {
    let mut input = Vec::with_capacity(MAX_EVENT_BYTES.min(4 * 1024));
    reader
        .take(MAX_EVENT_READ_BYTES)
        .read_to_end(&mut input)
        .map_err(|_| ReceiveError::Stdin)?;
    if input.len() > MAX_EVENT_BYTES {
        return Err(ReceiveError::Event(EventError::PayloadTooLarge));
    }
    CanonicalEvent::from_json(&input, received_at).map_err(ReceiveError::from)
}

/// Creates a fixed, bounded rejection acknowledgement.
///
/// When parsing failed before a trustworthy request identifier existed, the
/// generated `UUIDv7` is a response correlation identifier rather than an echo
/// of untrusted input.
///
/// # Errors
///
/// Returns [`ReceiveError::Acknowledgement`] only if an internal fixed
/// template violates the frozen acknowledgement contract.
pub fn rejection_acknowledgement(
    event_id: Option<EventId>,
    error: &ReceiveError,
) -> Result<Acknowledgement, ReceiveError> {
    let detail = AckError::new(error.code(), error.retryable(), error.message())
        .map_err(|_| ReceiveError::Acknowledgement)?;
    Ok(Acknowledgement::rejected(
        event_id.unwrap_or_else(EventId::new_v7),
        detail,
    ))
}

/// Creates a bounded rejection for a safe error owned by the composition root.
///
/// # Errors
///
/// Returns [`ReceiveError::Acknowledgement`] when the caller supplies an
/// invalid code/message or the resulting object violates the wire limit.
pub fn safe_rejection(
    event_id: EventId,
    code: &str,
    retryable: bool,
    message: &str,
) -> Result<Acknowledgement, ReceiveError> {
    let detail =
        AckError::new(code, retryable, message).map_err(|_| ReceiveError::Acknowledgement)?;
    let acknowledgement = Acknowledgement::rejected(event_id, detail);
    encode_acknowledgement(&acknowledgement)?;
    Ok(acknowledgement)
}

/// Serializes one compact acknowledgement without a trailing delimiter.
///
/// # Errors
///
/// Returns [`ReceiveError::Acknowledgement`] for serialization failure or a
/// response larger than 2,048 bytes.
pub fn encode_acknowledgement(acknowledgement: &Acknowledgement) -> Result<Vec<u8>, ReceiveError> {
    let output = serde_json::to_vec(acknowledgement).map_err(|_| ReceiveError::Acknowledgement)?;
    if output.len() > MAX_ACK_BYTES {
        return Err(ReceiveError::Acknowledgement);
    }
    Ok(output)
}

/// Writes exactly one bounded acknowledgement and flushes it.
///
/// # Errors
///
/// Returns [`ReceiveError::Acknowledgement`] without exposing writer details.
pub fn write_acknowledgement(
    writer: &mut impl Write,
    acknowledgement: &Acknowledgement,
) -> Result<(), ReceiveError> {
    let output = encode_acknowledgement(acknowledgement)?;
    writer
        .write_all(&output)
        .and_then(|()| writer.flush())
        .map_err(|_| ReceiveError::Acknowledgement)
}

/// Redacted state of one SSH setup prerequisite.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticStatus {
    /// The prerequisite is configured and passed its safety check.
    Ready,
    /// The expected file, key entry, or configuration is absent.
    Missing,
    /// The authorized file or its containing directory has unsafe ownership or permissions.
    Insecure,
    /// The prerequisite could not be inspected safely.
    Unavailable,
    /// No relay host alias is configured on this machine.
    NotConfigured,
}

impl DiagnosticStatus {
    /// Returns the stable machine-readable status name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Insecure => "insecure",
            Self::Unavailable => "unavailable",
            Self::NotConfigured => "not_configured",
        }
    }
}

/// Checks host-key enrollment with the system `ssh-keygen` parser.
///
/// The alias and path are passed as distinct process arguments. Command output
/// is discarded because it may contain hostnames or public-key material.
#[must_use]
pub fn diagnose_host_key(
    alias: Option<&str>,
    known_hosts: &Path,
    ssh_config: Option<&Path>,
) -> DiagnosticStatus {
    let Some(alias) = alias else {
        return DiagnosticStatus::NotConfigured;
    };
    if !valid_host_alias(alias) {
        return DiagnosticStatus::Unavailable;
    }
    if !known_hosts.is_file() {
        return DiagnosticStatus::Missing;
    }
    let Some(configuration) = resolved_ssh_configuration(alias, ssh_config) else {
        return DiagnosticStatus::Unavailable;
    };
    if !configuration.strict {
        return DiagnosticStatus::Insecure;
    }
    match Command::new("ssh-keygen")
        .args([
            OsStr::new("-F"),
            OsStr::new(&configuration.lookup),
            OsStr::new("-f"),
        ])
        .arg(known_hosts)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => DiagnosticStatus::Ready,
        Ok(status) if status.code() == Some(1) => DiagnosticStatus::Missing,
        Ok(_) | Err(_) => DiagnosticStatus::Unavailable,
    }
}

struct ResolvedSshConfiguration {
    lookup: String,
    strict: bool,
}

fn resolved_ssh_configuration(
    alias: &str,
    ssh_config: Option<&Path>,
) -> Option<ResolvedSshConfiguration> {
    let mut command = Command::new("ssh");
    if let Some(ssh_config) = ssh_config {
        command.args(["-F"]).arg(ssh_config);
    }
    let mut child = command
        .args(["-G", "--", alias])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut output = Vec::new();
    let read_result = child
        .stdout
        .take()?
        .take(u64::try_from(MAX_SSH_CONFIG_BYTES + 1).ok()?)
        .read_to_end(&mut output);
    if read_result.is_err() || output.len() > MAX_SSH_CONFIG_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    if !child.wait().ok()?.success() {
        return None;
    }
    parse_ssh_configuration(&output)
}

fn parse_ssh_configuration(output: &[u8]) -> Option<ResolvedSshConfiguration> {
    let output = std::str::from_utf8(output).ok()?;
    let mut hostname = None;
    let mut host_key_alias = None;
    let mut port = None;
    let mut strict = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key {
            "hostname" => hostname = Some(value),
            "hostkeyalias" if value != "none" => host_key_alias = Some(value),
            "port" => port = value.parse::<u16>().ok(),
            "stricthostkeychecking" => strict = Some(matches!(value, "yes" | "true")),
            _ => {}
        }
    }
    let host = host_key_alias.or(hostname)?;
    if host.is_empty()
        || host.len() > 512
        || !host.is_ascii()
        || host.chars().any(char::is_whitespace)
        || host.chars().any(char::is_control)
    {
        return None;
    }
    let port = port?;
    let lookup = if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    };
    Some(ResolvedSshConfiguration {
        lookup,
        strict: strict?,
    })
}

/// Checks that an authorized-keys file and its `.ssh` directory are private.
#[must_use]
pub fn diagnose_authorized_keys(path: &Path) -> DiagnosticStatus {
    if !path.is_file() {
        return DiagnosticStatus::Missing;
    }

    #[cfg(unix)]
    {
        let Some(parent) = path.parent() else {
            return DiagnosticStatus::Unavailable;
        };
        diagnose_unix_permissions(path, parent)
    }
    #[cfg(windows)]
    {
        diagnose_windows_permissions(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = parent;
        DiagnosticStatus::Unavailable
    }
}

fn valid_host_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        })
}

#[cfg(unix)]
fn diagnose_unix_permissions(path: &Path, parent: &Path) -> DiagnosticStatus {
    use std::os::unix::fs::MetadataExt;

    let Ok(file) = fs::symlink_metadata(path) else {
        return DiagnosticStatus::Unavailable;
    };
    let Ok(directory) = fs::symlink_metadata(parent) else {
        return DiagnosticStatus::Unavailable;
    };
    let current_uid = rustix::process::geteuid().as_raw();
    if file.file_type().is_symlink()
        || !file.is_file()
        || directory.file_type().is_symlink()
        || !directory.is_dir()
        || file.uid() != current_uid
        || directory.uid() != current_uid
        || file.mode() & 0o777 != 0o600
        || directory.mode() & 0o777 != 0o700
    {
        return DiagnosticStatus::Insecure;
    }
    DiagnosticStatus::Ready
}

#[cfg(windows)]
fn diagnose_windows_permissions(path: &Path) -> DiagnosticStatus {
    // `-Command` is a fixed program string; the path is carried separately and
    // never interpolated. All subprocess output is discarded.
    match Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            WINDOWS_PERMISSION_SCRIPT,
        ])
        .env("CODEX_NOTIFIER_AUTHORIZED_KEYS", path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => DiagnosticStatus::Ready,
        Ok(status) if status.code() == Some(3) => DiagnosticStatus::Insecure,
        Ok(_) | Err(_) => DiagnosticStatus::Unavailable,
    }
}

/// Maps a local IPC failure to a fixed receive rejection message.
///
/// # Errors
///
/// Returns [`ReceiveError::Acknowledgement`] only if the fixed rejection
/// cannot satisfy the acknowledgement contract.
pub fn ipc_rejection(event_id: EventId, error: &IpcError) -> Result<Acknowledgement, ReceiveError> {
    let retryable = matches!(
        error,
        IpcError::ConnectionFailed | IpcError::Timeout | IpcError::TransportFailure
    );
    safe_rejection(
        event_id,
        error.code().as_str(),
        retryable,
        "Desktop agent submission failed",
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::io::Cursor;

    use codex_notifier_core::{EventKind, EventSource, Extensions, Presentation, Privacy, Urgency};

    use super::*;

    fn event(now: OffsetDateTime) -> CanonicalEvent {
        CanonicalEvent::new(
            EventId::new_v7(),
            EventKind::TaskCompleted,
            now,
            EventSource::new("relay", None, None).expect("source"),
            Presentation::new(
                "Finished ;&|$()`'\"",
                "Metacharacters stay data: ; & | $() ` > < * ?",
                Urgency::Normal,
                Privacy::Public,
            )
            .expect("presentation"),
            None,
            Extensions::new(BTreeMap::new()).expect("extensions"),
            now,
        )
        .expect("event")
    }

    #[test]
    fn session_requires_exact_command_without_pty() {
        let connection = OsStr::new("127.0.0.1 12345 127.0.0.1 22");
        assert_eq!(
            validate_receive_session(Some(OsStr::new(REQUESTED_COMMAND)), Some(connection), None),
            Ok(())
        );
        for command in [
            None,
            Some(""),
            Some("codex-notifier receive extra"),
            Some("sh"),
        ] {
            assert_eq!(
                validate_receive_session(command.map(OsStr::new), Some(connection), None),
                Err(ReceiveError::SessionRejected)
            );
        }
        assert_eq!(
            validate_receive_session(
                Some(OsStr::new(REQUESTED_COMMAND)),
                Some(connection),
                Some(OsStr::new("tty"))
            ),
            Err(ReceiveError::SessionRejected)
        );
    }

    #[test]
    fn bounded_reader_rejects_oversize_and_concatenation() {
        let now = OffsetDateTime::now_utc();
        let mut oversized = Cursor::new(vec![b'x'; MAX_EVENT_BYTES + 1]);
        assert_eq!(
            read_event(&mut oversized, now),
            Err(ReceiveError::Event(EventError::PayloadTooLarge))
        );

        let bytes = event(now).to_json().expect("json");
        let mut concatenated = bytes.clone();
        concatenated.extend_from_slice(&bytes);
        assert!(matches!(
            read_event(&mut Cursor::new(concatenated), now),
            Err(ReceiveError::Event(EventError::MalformedJson))
        ));
    }

    #[test]
    fn metacharacters_round_trip_only_as_event_data() {
        let now = OffsetDateTime::now_utc();
        let expected = event(now);
        let parsed = read_event(&mut Cursor::new(expected.to_json().expect("json")), now)
            .expect("valid event");
        assert_eq!(
            parsed.presentation().title(),
            expected.presentation().title()
        );
        assert_eq!(parsed.presentation().body(), expected.presentation().body());
    }

    #[test]
    fn rejection_is_bounded_and_does_not_echo_input() {
        let acknowledgement =
            rejection_acknowledgement(None, &ReceiveError::Event(EventError::MalformedJson))
                .expect("rejection");
        let bytes = encode_acknowledgement(&acknowledgement).expect("encoded");
        assert!(bytes.len() <= MAX_ACK_BYTES);
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.contains("malformed_json"));
        assert!(!text.contains("payload"));
        assert!(!text.contains("stack"));
    }

    #[test]
    fn resolved_ssh_config_requires_pinning_and_handles_nondefault_ports() {
        let ready = parse_ssh_configuration(
            b"hostname desktop.example\nport 2222\nhostkeyalias desktop-key\nstricthostkeychecking yes\n",
        )
        .expect("configuration");
        assert_eq!(ready.lookup, "[desktop-key]:2222");
        assert!(ready.strict);

        let unsafe_configuration = parse_ssh_configuration(
            b"hostname desktop.example\nport 22\nstricthostkeychecking accept-new\n",
        )
        .expect("configuration");
        assert_eq!(unsafe_configuration.lookup, "desktop.example");
        assert!(!unsafe_configuration.strict);
        assert!(parse_ssh_configuration(b"hostname bad host\nport 22\n").is_none());
    }

    #[test]
    fn relay_arguments_are_fixed_and_event_data_stays_out_of_argv() {
        let config = OpenSshConfig::new("desktop-test", Duration::from_millis(1_001))
            .expect("OpenSSH configuration")
            .with_config_file(
                std::env::current_dir()
                    .expect("current directory")
                    .join("ssh.conf"),
            )
            .expect("absolute SSH configuration");
        let arguments = config.arguments();
        let rendered = arguments
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>();
        assert_eq!(rendered[0], "-F");
        assert_eq!(rendered[2], "-T");
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("BatchMode=yes")));
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("ClearAllForwardings=yes")));
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("StrictHostKeyChecking=yes")));
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("ConnectTimeout=2")));
        assert_eq!(rendered[rendered.len() - 3], "--");
        assert_eq!(rendered[rendered.len() - 2], "desktop-test");
        assert_eq!(rendered[rendered.len() - 1], REQUESTED_COMMAND);
        assert!(!rendered.join(" ").contains("Finished ;&|$()`"));
    }

    #[test]
    fn relay_configuration_bounds_alias_timeout_and_optional_path() {
        assert_eq!(
            OpenSshConfig::new("-unsafe", Duration::from_secs(1)),
            Err(SshDeliveryError::InvalidConfiguration)
        );
        assert_eq!(
            OpenSshConfig::new("desktop", Duration::from_millis(99)),
            Err(SshDeliveryError::InvalidConfiguration)
        );
        assert_eq!(
            OpenSshConfig::new("desktop", Duration::from_secs(121)),
            Err(SshDeliveryError::InvalidConfiguration)
        );
        assert_eq!(
            OpenSshConfig::new("desktop", Duration::from_secs(1))
                .expect("valid configuration")
                .with_config_file("relative/config"),
            Err(SshDeliveryError::InvalidConfiguration)
        );
    }

    #[test]
    fn process_failures_have_distinct_retry_classifications() {
        let cases: &[(&[u8], SshDeliveryError)] = &[
            (
                b"ssh: connect to host desktop port 22: Connection timed out",
                SshDeliveryError::ConnectionTimeout,
            ),
            (
                b"ssh: connect to host desktop port 22: Connection refused",
                SshDeliveryError::NetworkUnavailable,
            ),
            (
                b"user@desktop: Permission denied (publickey).",
                SshDeliveryError::AuthenticationFailed,
            ),
            (
                b"WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!",
                SshDeliveryError::HostKeyVerificationFailed,
            ),
            (
                b"unclassified fixed diagnostic",
                SshDeliveryError::ProcessFailed,
            ),
        ];
        for (diagnostic, expected) in cases {
            let actual = classify_process_failure(diagnostic);
            assert_eq!(&actual, expected);
        }
        assert!(SshDeliveryError::ConnectionTimeout.retryable());
        assert!(SshDeliveryError::NetworkUnavailable.retryable());
        assert!(!SshDeliveryError::AuthenticationFailed.retryable());
        assert!(!SshDeliveryError::HostKeyVerificationFailed.retryable());
    }

    #[test]
    fn relay_acknowledgements_match_id_and_preserve_safe_rejection_policy() {
        let event_id = EventId::new_v7();
        for acknowledgement in [
            Acknowledgement::accepted(event_id),
            Acknowledgement::duplicate(event_id),
            Acknowledgement::delivered(event_id),
        ] {
            assert_eq!(
                validate_delivery_acknowledgement(
                    event_id,
                    &acknowledgement.to_json().expect("acknowledgement JSON")
                ),
                Ok(())
            );
        }

        let rejection = Acknowledgement::rejected(
            event_id,
            AckError::new("desktop_busy", true, "Desktop agent is busy").expect("safe rejection"),
        );
        let error = validate_delivery_acknowledgement(
            event_id,
            &rejection.to_json().expect("rejection JSON"),
        )
        .expect_err("remote rejection");
        assert_eq!(error.code(), "desktop_busy");
        assert!(error.retryable());

        let wrong_id = Acknowledgement::accepted(EventId::new_v7());
        assert_eq!(
            validate_delivery_acknowledgement(
                event_id,
                &wrong_id.to_json().expect("wrong-ID acknowledgement")
            ),
            Err(SshDeliveryError::AcknowledgementInvalid)
        );
        assert_eq!(
            validate_delivery_acknowledgement(event_id, b"not-json"),
            Err(SshDeliveryError::AcknowledgementInvalid)
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_authorized_file_permissions_are_exact() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("tempdir");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("directory mode");
        let path = directory.path().join("authorized_keys");
        std::fs::write(&path, "ssh-ed25519 placeholder").expect("authorized keys");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("file mode");
        assert_eq!(diagnose_authorized_keys(&path), DiagnosticStatus::Ready);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("file mode");
        assert_eq!(diagnose_authorized_keys(&path), DiagnosticStatus::Insecure);
    }

    #[cfg(windows)]
    #[test]
    fn windows_authorized_file_permissions_reject_untrusted_writers() {
        const SECURE_ACL: &str = r"$ErrorActionPreference='Stop';$p=$env:CODEX_NOTIFIER_TEST_FILE;$me=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;& icacls.exe $p /inheritance:r /grant:r ('*'+$me+':(F)') '*S-1-5-18:(F)' '*S-1-5-32-544:(F)' | Out-Null;if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}";
        const UNSAFE_ACL: &str = r"$ErrorActionPreference='Stop';$p=$env:CODEX_NOTIFIER_TEST_FILE;& icacls.exe $p /grant '*S-1-1-0:(M)' | Out-Null;if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}";

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("authorized_keys");
        std::fs::write(&path, "ssh-ed25519 placeholder").expect("authorized keys");
        for (script, expected) in [
            (SECURE_ACL, DiagnosticStatus::Ready),
            (UNSAFE_ACL, DiagnosticStatus::Insecure),
        ] {
            let output = Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    script,
                ])
                .env("CODEX_NOTIFIER_TEST_FILE", &path)
                .stdin(Stdio::null())
                .output()
                .expect("PowerShell ACL setup");
            assert!(
                output.status.success(),
                "PowerShell ACL setup failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let actual = diagnose_authorized_keys(&path);
            if actual == DiagnosticStatus::Unavailable {
                let diagnostic = Command::new("powershell.exe")
                    .args([
                        "-NoLogo",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        WINDOWS_PERMISSION_SCRIPT,
                    ])
                    .env("CODEX_NOTIFIER_AUTHORIZED_KEYS", &path)
                    .output()
                    .expect("PowerShell ACL diagnostic");
                panic!(
                    "ACL diagnostic failed with {:?}: {}",
                    diagnostic.status.code(),
                    String::from_utf8_lossy(&diagnostic.stderr)
                );
            }
            assert_eq!(actual, expected);
        }
    }
}
