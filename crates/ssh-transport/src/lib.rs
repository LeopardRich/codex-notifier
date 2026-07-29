//! Restricted system OpenSSH receive boundary and security diagnostics.

use std::ffi::OsStr;
#[cfg(unix)]
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use codex_notifier_core::limits::{MAX_ACK_BYTES, MAX_EVENT_BYTES};
use codex_notifier_core::{CanonicalEvent, EventError, EventId};
use codex_notifier_ipc::{AckError, Acknowledgement, IpcError};
use thiserror::Error;
use time::OffsetDateTime;

/// The only client-requested command accepted by the forced entry point.
pub const REQUESTED_COMMAND: &str = "codex-notifier receive";

const MAX_SSH_CONNECTION_BYTES: usize = 512;
const MAX_EVENT_READ_BYTES: u64 = 16_385;
const MAX_SSH_CONFIG_BYTES: usize = 64 * 1024;

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
    const SCRIPT: &str = r"$ErrorActionPreference='Stop';$p=$env:CODEX_NOTIFIER_AUTHORIZED_KEYS;$acl=Get-Acl -LiteralPath $p;$sidType=[Security.Principal.SecurityIdentifier];$me=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$allowed=@($me,'S-1-5-18','S-1-5-32-544');$owner=$acl.GetOwner($sidType).Value;$rules=$acl.GetAccessRules($true,$true,$sidType);$unsafe=$false;foreach($rule in $rules){$sid=$rule.IdentityReference.Value;$write=($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::WriteData) -or ($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::Modify) -or ($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::FullControl);if($rule.AccessControlType -eq 'Allow' -and $write -and $allowed -notcontains $sid){$unsafe=$true}};if((-not $acl.AreAccessRulesProtected)-or($allowed -notcontains $owner)-or$unsafe){exit 3};exit 0";
    // `-Command` is a fixed program string; the path is carried separately and
    // never interpolated. All subprocess output is discarded.
    match Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            SCRIPT,
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
            assert_eq!(diagnose_authorized_keys(&path), expected);
        }
    }
}
