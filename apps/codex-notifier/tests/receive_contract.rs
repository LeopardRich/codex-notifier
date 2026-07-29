//! Restricted receive command and real local IPC composition tests.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use codex_notifier_config::{PathEnvironment, Platform};
use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use codex_notifier_ipc::{AckStatus, Acknowledgement, IpcEndpoint, IpcPolicy, IpcServer};
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use tokio::sync::oneshot;

const SSH_CONNECTION: &str = "127.0.0.1 43123 127.0.0.1 22";

struct IsolatedEnvironment {
    _directory: TempDir,
    home: PathBuf,
    config_base: PathBuf,
    state_base: PathBuf,
    state_dir: PathBuf,
}

impl IsolatedEnvironment {
    fn new() -> Self {
        #[cfg(unix)]
        let directory = tempfile::Builder::new()
            .prefix("cnr")
            .tempdir_in("/tmp")
            .expect("temporary directory");
        #[cfg(windows)]
        let directory = tempfile::tempdir().expect("temporary directory");
        let home = directory.path().join("home");
        let config_base = directory.path().join("config");
        let state_base = directory.path().join("state");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&config_base).expect("config base");
        fs::create_dir_all(&state_base).expect("state base");

        #[cfg(windows)]
        let paths = PathEnvironment::new()
            .with_home(&home)
            .with_windows_app_data(&config_base)
            .with_windows_local_app_data(&state_base)
            .resolve(Platform::Windows)
            .expect("Windows paths");
        #[cfg(target_os = "macos")]
        let paths = PathEnvironment::new()
            .with_home(&home)
            .resolve(Platform::MacOs)
            .expect("macOS paths");
        #[cfg(all(unix, not(target_os = "macos")))]
        let paths = PathEnvironment::new()
            .with_home(&home)
            .with_xdg_config_home(&config_base)
            .with_xdg_state_home(&state_base)
            .resolve(Platform::Xdg)
            .expect("XDG paths");

        fs::create_dir_all(paths.config_file().parent().expect("config parent"))
            .expect("config directory");
        fs::create_dir_all(paths.state_dir()).expect("state directory");
        fs::write(paths.config_file(), b"config_version = 1\n").expect("configuration");

        Self {
            _directory: directory,
            home,
            config_base,
            state_base,
            state_dir: paths.state_dir().to_owned(),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codex-notifier"));
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("APPDATA", &self.config_base)
            .env("LOCALAPPDATA", &self.state_base)
            .env("XDG_CONFIG_HOME", &self.config_base)
            .env("XDG_STATE_HOME", &self.state_base)
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "socks5://127.0.0.1:1")
            .env_remove("SSH_TTY");
        command
    }
}

fn event() -> CanonicalEvent {
    let now = OffsetDateTime::now_utc();
    CanonicalEvent::new(
        EventId::new_v7(),
        EventKind::ApprovalRequested,
        now - Duration::seconds(1),
        EventSource::new(
            "relay;&|$()`'\"",
            Some("project > /tmp/marker".to_owned()),
            None,
        )
        .expect("source"),
        Presentation::new(
            "Approval ;&|$()`'\"",
            "Data only: ; & | $() ` > < * ? C:\\private /tmp/private",
            Urgency::High,
            Privacy::Public,
        )
        .expect("presentation"),
        None,
        Extensions::new(BTreeMap::new()).expect("extensions"),
        now,
    )
    .expect("event")
}

fn receive_child(
    environment: &IsolatedEnvironment,
    original_command: Option<&str>,
    tty: bool,
    input: &[u8],
) -> Output {
    let mut command = environment.command();
    command
        .arg("receive")
        .env("SSH_CONNECTION", SSH_CONNECTION)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(original_command) = original_command {
        command.env("SSH_ORIGINAL_COMMAND", original_command);
    } else {
        command.env_remove("SSH_ORIGINAL_COMMAND");
    }
    if tty {
        command.env("SSH_TTY", "/dev/pts/1");
    }
    let mut child = command.spawn().expect("receive child");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input)
        .expect("write input");
    child.wait_with_output().expect("receive output")
}

fn acknowledgement(output: &Output) -> Acknowledgement {
    assert!(
        output.status.success(),
        "receive stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).expect("structured acknowledgement")
}

fn assert_rejected_without_echo(output: &Output, code: &str, forbidden: &[&str]) {
    let acknowledgement = acknowledgement(output);
    assert_eq!(acknowledgement.status(), AckStatus::Rejected);
    assert_eq!(
        acknowledgement
            .error()
            .map(codex_notifier_ipc::AckError::code),
        Some(code)
    );
    let output = String::from_utf8_lossy(&output.stdout);
    for value in forbidden {
        assert!(!output.contains(value), "acknowledgement echoed input");
    }
    assert!(!output.contains("stack"));
    assert!(!output.contains(env!("CARGO_MANIFEST_DIR")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_event_crosses_receive_process_and_real_local_ipc() {
    let environment = IsolatedEnvironment::new();
    let endpoint =
        IpcEndpoint::new(environment.state_dir.join("run"), "default").expect("IPC endpoint");
    let server = Arc::new(IpcServer::bind(endpoint, IpcPolicy::default()).expect("IPC server"));
    let received = Arc::new(Mutex::new(Vec::new()));
    let handler_received = Arc::clone(&received);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .serve_until(
                Arc::new(move |event: CanonicalEvent| {
                    handler_received
                        .lock()
                        .expect("received lock")
                        .push(event.clone());
                    Acknowledgement::accepted(event.event_id())
                }),
                async {
                    let _ = shutdown_rx.await;
                },
            )
            .await
    });

    let expected = event();
    let input = expected.to_json().expect("event JSON");
    let output = tokio::time::timeout(
        StdDuration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            receive_child(&environment, Some("codex-notifier receive"), false, &input)
        }),
    )
    .await
    .expect("receive timeout")
    .expect("receive task");
    let acknowledgement = acknowledgement(&output);
    assert_eq!(acknowledgement.event_id(), expected.event_id());
    assert_eq!(acknowledgement.status(), AckStatus::Accepted);

    {
        let received = received.lock().expect("received lock");
        assert_eq!(received.len(), 1);
        assert_eq!(
            received[0].to_json().expect("received JSON"),
            expected.to_json().expect("expected JSON")
        );
    }
    shutdown_tx.send(()).expect("shutdown");
    let report = server_task.await.expect("server task").expect("server run");
    assert_eq!(report.completed, 1);
    assert_eq!(report.rejected, 0);
}

#[test]
fn session_shape_and_event_framing_fail_closed_with_redacted_acknowledgements() {
    let environment = IsolatedEnvironment::new();
    let secret = r"C:\private\id_ed25519 /Users/name/.ssh/key STACK_SECRET";

    for (original, tty) in [
        (None, false),
        (Some("codex-notifier receive extra"), false),
        (Some("sh -c whoami"), false),
        (Some("codex-notifier receive"), true),
    ] {
        let output = receive_child(&environment, original, tty, secret.as_bytes());
        assert_rejected_without_echo(&output, "ssh_session_rejected", &[secret, "whoami"]);
    }

    let valid = event().to_json().expect("event JSON");
    let mut concatenated = valid.clone();
    concatenated.extend_from_slice(&valid);
    let output = receive_child(
        &environment,
        Some("codex-notifier receive"),
        false,
        &concatenated,
    );
    assert_rejected_without_echo(&output, "malformed_json", &["Approval", "private"]);

    let oversized = vec![b'x'; codex_notifier_core::limits::MAX_EVENT_BYTES + 1];
    let output = receive_child(
        &environment,
        Some("codex-notifier receive"),
        false,
        &oversized,
    );
    assert_rejected_without_echo(&output, "payload_too_large", &["xxxx"]);
}

#[test]
fn doctor_ssh_reports_only_bounded_statuses() {
    let environment = IsolatedEnvironment::new();
    let output = environment
        .command()
        .args(["doctor", "ssh"])
        .output()
        .expect("doctor SSH");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let output = String::from_utf8(output.stdout).expect("UTF-8 diagnostic");
    assert_eq!(output, "host_key=not_configured\nauthorized_keys=missing\n");
    assert!(!output.contains(environment.home.to_string_lossy().as_ref()));
}
