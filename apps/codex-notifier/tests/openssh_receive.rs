//! Real system OpenSSH forced-command acceptance and rejection matrix.

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use codex_notifier::AgentHost;
use codex_notifier_application::{
    AgentError, AgentState, CancellationToken, DeliveryFuture, DeliveryOutcome, EventDelivery,
    RoleDeliveryFactory,
};
use codex_notifier_config::{
    CliOverrides, ConfigLoader, FileSystemStateProbe, PathEnvironment, Platform,
};
use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use codex_notifier_ipc::{AckStatus, Acknowledgement, IpcClient, IpcEndpoint, IpcPolicy};
use codex_notifier_ssh_transport::{OpenSshConfig, OpenSshDelivery};
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use tokio::sync::oneshot;

struct RecordingDelivery {
    events: Mutex<Vec<CanonicalEvent>>,
}

impl RecordingDelivery {
    const fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn count(&self) -> usize {
        self.events.lock().expect("delivery lock").len()
    }

    fn events(&self) -> Vec<CanonicalEvent> {
        self.events.lock().expect("delivery lock").clone()
    }
}

impl EventDelivery for RecordingDelivery {
    fn deliver<'a>(
        &'a self,
        event: &'a CanonicalEvent,
        _cancellation: CancellationToken,
    ) -> DeliveryFuture<'a> {
        Box::pin(async move {
            self.events
                .lock()
                .expect("delivery lock")
                .push(event.clone());
            DeliveryOutcome::Delivered
        })
    }
}

struct RecordingFactory {
    delivery: Arc<RecordingDelivery>,
}

impl RoleDeliveryFactory for RecordingFactory {
    fn desktop(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        Ok(self.delivery.clone())
    }

    fn relay(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        Err(AgentError::DeliveryInitialization)
    }
}

struct OpenSshFactory {
    delivery: OpenSshDelivery,
}

impl RoleDeliveryFactory for OpenSshFactory {
    fn desktop(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        Err(AgentError::DeliveryInitialization)
    }

    fn relay(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        Ok(Arc::new(self.delivery.clone()))
    }
}

struct SshdGuard {
    child: Child,
    log: PathBuf,
}

impl Drop for SshdGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            if let Ok(log) = fs::read_to_string(&self.log) {
                eprintln!("temporary sshd log:\n{log}");
            }
        }
    }
}

fn run(command: &mut Command, operation: &str) -> Output {
    let output = command.output().unwrap_or_else(|_| panic!("{operation}"));
    assert!(
        output.status.success(),
        "{operation} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("temporary port")
        .local_addr()
        .expect("local address")
        .port()
}

fn event(label: &str) -> CanonicalEvent {
    event_from("real-ssh;&|$()`'\"", label)
}

fn event_from(host_label: &str, label: &str) -> CanonicalEvent {
    event_from_kind(host_label, label, EventKind::TaskCompleted)
}

fn event_from_kind(host_label: &str, label: &str, kind: EventKind) -> CanonicalEvent {
    let now = OffsetDateTime::now_utc();
    CanonicalEvent::new(
        EventId::new_v7(),
        kind,
        now - Duration::seconds(1),
        EventSource::new(host_label, Some(label.to_owned()), None).expect("source"),
        Presentation::new(
            "Finished ;&|$()`'\"",
            "Data only: ; & | $() ` > < * ? /tmp/not-a-command",
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

fn ssh_command(identity: &Path, known_hosts: &Path) -> Command {
    let mut command = Command::new("ssh");
    command
        .args(["-F", "/dev/null", "-i"])
        .arg(identity)
        .args([
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "ServerAliveInterval=2",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
        ])
        .arg(format!("UserKnownHostsFile={}", known_hosts.display()));
    command
}

fn ssh_request(
    mut command: Command,
    port: u16,
    user: &str,
    remote_command: Option<&str>,
    input: &[u8],
) -> Output {
    command
        .args(["-p", &port.to_string()])
        .arg(format!("{user}@127.0.0.1"));
    if let Some(remote_command) = remote_command {
        command.arg(remote_command);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("SSH client");
    child
        .stdin
        .take()
        .expect("SSH stdin")
        .write_all(input)
        .expect("SSH input");
    child.wait_with_output().expect("SSH output")
}

fn acknowledgement(output: &Output) -> Acknowledgement {
    assert!(
        output.status.success(),
        "SSH request failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("SSH acknowledgement")
}

fn assert_rejection(output: &Output, code: &str, forbidden: &[&str]) {
    let acknowledgement = acknowledgement(output);
    assert_eq!(acknowledgement.status(), AckStatus::Rejected);
    assert_eq!(
        acknowledgement
            .error()
            .map(codex_notifier_ipc::AckError::code),
        Some(code)
    );
    let response = String::from_utf8_lossy(&output.stdout);
    for value in forbidden {
        assert!(!response.contains(value), "response echoed forbidden input");
    }
}

fn wait_for_sshd(port: u16) {
    let deadline = Instant::now() + StdDuration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(StdDuration::from_millis(25));
    }
    panic!("temporary sshd did not start");
}

fn assert_local_forward_rejected(mut command: Command, port: u16, user: &str) {
    let target = TcpListener::bind(("127.0.0.1", 0)).expect("forward target");
    target.set_nonblocking(true).expect("nonblocking target");
    let target_port = target.local_addr().expect("target address").port();
    let forwarding_port = free_port();
    command
        .args(["-N", "-o", "ExitOnForwardFailure=yes", "-L"])
        .arg(format!(
            "127.0.0.1:{forwarding_port}:127.0.0.1:{target_port}"
        ))
        .args(["-p", &port.to_string()])
        .arg(format!("{user}@127.0.0.1"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("local forwarding client");
    let deadline = Instant::now() + StdDuration::from_secs(5);
    let mut forwarded_client = loop {
        match TcpStream::connect(("127.0.0.1", forwarding_port)) {
            Ok(stream) => break stream,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(StdDuration::from_millis(25));
            }
            Err(error) => panic!("local forwarding listener did not start: {error}"),
        }
    };
    forwarded_client
        .set_read_timeout(Some(StdDuration::from_secs(2)))
        .expect("forward read timeout");
    let _ = forwarded_client.write_all(b"forwarding probe");
    let mut response = [0_u8; 1];
    assert!(
        matches!(forwarded_client.read(&mut response), Ok(0) | Err(_)),
        "local forwarding returned target data"
    );
    assert!(
        target.accept().is_err(),
        "local forwarding reached its target"
    );
    let _ = child.kill();
    let output = child.wait_with_output().expect("local forwarding output");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("administratively prohibited"),
        "local forwarding denial was not reported"
    );
}

async fn wait_for_deliveries(delivery: &RecordingDelivery, expected: usize) {
    tokio::time::timeout(StdDuration::from_secs(5), async {
        while delivery.count() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("delivery timeout");
}

async fn wait_for_agent(runtime: &codex_notifier_application::AgentRuntime) {
    tokio::time::timeout(StdDuration::from_secs(5), async {
        while runtime.state() != AgentState::Ready {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("agent readiness timeout");
}

fn create_temp_root() -> TempDir {
    let home = std::env::var_os("HOME").expect("HOME");
    tempfile::Builder::new()
        .prefix("codex-notifier-openssh-")
        .tempdir_in(home)
        .expect("temporary OpenSSH root")
}

#[allow(clippy::too_many_arguments)]
fn write_sshd_config(
    path: &Path,
    port: u16,
    user: &str,
    host_key: &Path,
    authorized_keys: &Path,
    pid_file: &Path,
    config_base: &Path,
    state_base: &Path,
) {
    let document = format!(
        "Port {port}\nListenAddress 127.0.0.1\nAddressFamily inet\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\nStrictModes yes\nPubkeyAuthentication yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nPermitEmptyPasswords no\nAllowUsers {user}\nAllowAgentForwarding yes\nAllowTcpForwarding yes\nX11Forwarding yes\nSetEnv XDG_CONFIG_HOME={} XDG_STATE_HOME={}\nLogLevel VERBOSE\n",
        host_key.display(),
        pid_file.display(),
        authorized_keys.display(),
        config_base.display(),
        state_base.display(),
    );
    fs::write(path, document).expect("sshd configuration");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires sudo and the system OpenSSH server"]
#[allow(clippy::too_many_lines)]
async fn real_forced_openssh_session_enforces_the_receive_boundary() {
    assert_eq!(
        std::env::var("CODEX_NOTIFIER_OPENSSH_TEST").as_deref(),
        Ok("1"),
        "explicit OpenSSH test opt-in is required"
    );
    let root = create_temp_root();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).expect("root mode");
    let user = std::env::var("USER").expect("USER");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_codex-notifier"))
        .canonicalize()
        .expect("receiver executable");
    assert!(
        executable.to_string_lossy().is_ascii()
            && !executable.to_string_lossy().contains(['"', '\'', ' ', ','])
    );

    let config_base = root.path().join("config");
    let state_base = root.path().join("state");
    fs::create_dir_all(&config_base).expect("config base");
    fs::create_dir_all(&state_base).expect("state base");
    let paths = PathEnvironment::new()
        .with_home(root.path())
        .with_xdg_config_home(&config_base)
        .with_xdg_state_home(&state_base)
        .resolve(Platform::Xdg)
        .expect("XDG paths");
    fs::create_dir_all(paths.config_file().parent().expect("config parent"))
        .expect("config directory");
    fs::create_dir_all(paths.state_dir()).expect("state directory");
    fs::write(paths.config_file(), b"config_version = 1\n").expect("configuration");
    let config = ConfigLoader::load(
        &paths,
        Some("config_version = 1\n"),
        None,
        CliOverrides::new(),
        &FileSystemStateProbe,
    )
    .expect("agent configuration");

    let delivery = Arc::new(RecordingDelivery::new());
    let factory = RecordingFactory {
        delivery: delivery.clone(),
    };
    let host = AgentHost::from_config(&config, &factory).expect("desktop agent");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let agent_task = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_for_agent(&runtime).await;

    let client_key = root.path().join("client_ed25519");
    let host_key = root.path().join("host_ed25519");
    run(
        Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&client_key),
        "client key generation",
    );
    run(
        Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&host_key),
        "host key generation",
    );

    let public_key = fs::read_to_string(client_key.with_extension("pub")).expect("public key");
    let authorized_keys = root.path().join("authorized_keys");
    fs::write(
        &authorized_keys,
        format!(
            "restrict,command=\"{} receive\" {}\n",
            executable.display(),
            public_key.trim()
        ),
    )
    .expect("authorized keys");
    fs::set_permissions(&authorized_keys, fs::Permissions::from_mode(0o600))
        .expect("authorized keys mode");

    let port = free_port();
    let sshd_config = root.path().join("sshd_config");
    let pid_file = root.path().join("sshd.pid");
    write_sshd_config(
        &sshd_config,
        port,
        &user,
        &host_key,
        &authorized_keys,
        &pid_file,
        &config_base,
        &state_base,
    );
    run(
        Command::new("sudo")
            .args(["/usr/sbin/sshd", "-t", "-f"])
            .arg(&sshd_config),
        "sshd configuration check",
    );

    let host_public = fs::read_to_string(host_key.with_extension("pub")).expect("host public key");
    let mut host_parts = host_public.split_ascii_whitespace();
    let algorithm = host_parts.next().expect("host algorithm");
    let key = host_parts.next().expect("host key");
    let known_hosts = root.path().join("known_hosts");
    fs::write(
        &known_hosts,
        format!("[127.0.0.1]:{port} {algorithm} {key}\n"),
    )
    .expect("known hosts");
    fs::set_permissions(&known_hosts, fs::Permissions::from_mode(0o600)).expect("known hosts mode");

    let client_config = root.path().join("ssh_config");
    assert!(
        [&client_key, &known_hosts]
            .iter()
            .all(|path| !path.to_string_lossy().contains(['"', '\n', '\r']))
    );
    fs::write(
        &client_config,
        format!(
            "Host stage16-desktop\n  HostName 127.0.0.1\n  Port {port}\n  User {user}\n  IdentityFile \"{}\"\n  IdentitiesOnly yes\n  UserKnownHostsFile \"{}\"\n  StrictHostKeyChecking yes\n  BatchMode yes\n",
            client_key.display(),
            known_hosts.display(),
        ),
    )
    .expect("relay SSH configuration");
    fs::set_permissions(&client_config, fs::Permissions::from_mode(0o600))
        .expect("relay SSH config mode");

    let relay_config_base = root.path().join("relay-config");
    let relay_state_base = root.path().join("relay-state");
    fs::create_dir_all(&relay_config_base).expect("relay config base");
    fs::create_dir_all(&relay_state_base).expect("relay state base");
    let relay_paths = PathEnvironment::new()
        .with_home(root.path())
        .with_xdg_config_home(&relay_config_base)
        .with_xdg_state_home(&relay_state_base)
        .resolve(Platform::Xdg)
        .expect("relay XDG paths");
    let relay_profile = "real-relay";
    let relay_config = ConfigLoader::load(
        &relay_paths,
        Some(
            "config_version = 1\n[relay]\nretry_initial_delay_ms = 1000\nretry_max_delay_ms = 1000\nretry_max_attempts = 5\n",
        ),
        None,
        CliOverrides::new()
            .with_role("relay")
            .with_profile(relay_profile)
            .with_relay_host("stage16-desktop"),
        &FileSystemStateProbe,
    )
    .expect("relay agent configuration");
    let relay_endpoint = IpcEndpoint::new(
        relay_config.storage().state_dir().join("run"),
        relay_profile,
    )
    .expect("relay endpoint");
    let relay_factory = OpenSshFactory {
        delivery: OpenSshDelivery::new(
            OpenSshConfig::new("stage16-desktop", StdDuration::from_secs(5))
                .expect("OpenSSH delivery configuration")
                .with_config_file(&client_config)
                .expect("OpenSSH client config path"),
        ),
    };
    let relay_host =
        AgentHost::from_config(&relay_config, &relay_factory).expect("relay agent host");
    let relay_runtime = relay_host.runtime();
    let (relay_shutdown_tx, relay_shutdown_rx) = oneshot::channel();
    let relay_task = tokio::spawn(async move {
        relay_host
            .run_until(async {
                let _ = relay_shutdown_rx.await;
            })
            .await
    });
    wait_for_agent(&relay_runtime).await;
    let relayed = event("offline-retry");
    let relay_acknowledgement = IpcClient::new(relay_endpoint.clone(), IpcPolicy::default())
        .submit(&relayed)
        .await
        .expect("relay IPC submission");
    assert_eq!(relay_acknowledgement.status(), AckStatus::Accepted);
    tokio::time::sleep(StdDuration::from_millis(500)).await;
    assert_eq!(delivery.count(), 0, "desktop SSH server is still offline");

    let log_path = root.path().join("sshd.log");
    let log = File::create(&log_path).expect("sshd log");
    let sshd = Command::new("sudo")
        .args(["/usr/sbin/sshd", "-D", "-e", "-f"])
        .arg(&sshd_config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .expect("temporary sshd");
    let _sshd = SshdGuard {
        child: sshd,
        log: log_path,
    };
    wait_for_sshd(port);
    wait_for_deliveries(&delivery, 1).await;

    OpenSshDelivery::new(
        OpenSshConfig::new("stage16-desktop", StdDuration::from_secs(5))
            .expect("OpenSSH probe configuration")
            .with_config_file(&client_config)
            .expect("OpenSSH probe config path"),
    )
    .probe_receiver()
    .await
    .expect("empty receiver reachability probe");
    assert_eq!(
        delivery.count(),
        1,
        "reachability probe must not submit an event"
    );

    let remote_test = event_from("local-test", "stage17-remote-test");
    let remote_test_acknowledgement = IpcClient::new(relay_endpoint.clone(), IpcPolicy::default())
        .submit(&remote_test)
        .await
        .expect("remote self-test IPC submission");
    assert_eq!(remote_test_acknowledgement.status(), AckStatus::Accepted);
    wait_for_deliveries(&delivery, 2).await;

    let remote_approval = event_from_kind(
        "real-ssh-approval",
        "stage18-approval-request",
        EventKind::ApprovalRequested,
    );
    let remote_approval_acknowledgement =
        IpcClient::new(relay_endpoint.clone(), IpcPolicy::default())
            .submit(&remote_approval)
            .await
            .expect("remote approval IPC submission");
    assert_eq!(
        remote_approval_acknowledgement.status(),
        AckStatus::Accepted
    );
    wait_for_deliveries(&delivery, 3).await;

    let output = ssh_request(
        ssh_command(&client_key, &known_hosts),
        port,
        &user,
        Some("codex-notifier receive"),
        &relayed.to_json().expect("duplicate relayed JSON"),
    );
    let duplicate = acknowledgement(&output);
    assert_eq!(duplicate.status(), AckStatus::Duplicate);
    assert_eq!(duplicate.event_id(), relayed.event_id());
    assert_eq!(delivery.count(), 3);

    let valid = event("metacharacters");
    let output = ssh_request(
        ssh_command(&client_key, &known_hosts),
        port,
        &user,
        Some("codex-notifier receive"),
        &valid.to_json().expect("valid JSON"),
    );
    let accepted = acknowledgement(&output);
    assert_eq!(accepted.status(), AckStatus::Accepted);
    assert_eq!(accepted.event_id(), valid.event_id());
    wait_for_deliveries(&delivery, 4).await;
    assert!(
        delivery
            .events()
            .iter()
            .any(|delivered| delivered.to_json().ok() == valid.to_json().ok()),
        "direct event was not delivered"
    );

    let marker = root.path().join("must-not-exist");
    let hostile_command = format!("codex-notifier receive; touch {}", marker.display());
    let secret = "PRIVATE_PAYLOAD /home/user/.ssh/id_ed25519 STACK_TRACE";
    let output = ssh_request(
        ssh_command(&client_key, &known_hosts),
        port,
        &user,
        Some(&hostile_command),
        secret.as_bytes(),
    );
    assert_rejection(
        &output,
        "ssh_session_rejected",
        &[secret, "touch", "id_ed25519"],
    );
    assert!(!marker.exists());

    let output = ssh_request(
        ssh_command(&client_key, &known_hosts),
        port,
        &user,
        None,
        secret.as_bytes(),
    );
    assert_rejection(&output, "ssh_session_rejected", &[secret, "id_ed25519"]);

    let mut concatenated = valid.to_json().expect("valid JSON");
    concatenated.extend_from_slice(&valid.to_json().expect("valid JSON"));
    let output = ssh_request(
        ssh_command(&client_key, &known_hosts),
        port,
        &user,
        Some("codex-notifier receive"),
        &concatenated,
    );
    assert_rejection(&output, "malformed_json", &["metacharacters", "Finished"]);

    let mut pty = ssh_command(&client_key, &known_hosts);
    pty.arg("-tt");
    let output = ssh_request(pty, port, &user, Some("codex-notifier receive"), b"");
    assert!(String::from_utf8_lossy(&output.stderr).contains("PTY allocation request failed"));

    assert_local_forward_rejected(ssh_command(&client_key, &known_hosts), port, &user);

    let forwarding_port = free_port();
    let mut remote_forwarding = ssh_command(&client_key, &known_hosts);
    remote_forwarding
        .args(["-o", "ExitOnForwardFailure=yes", "-R"])
        .arg(format!("127.0.0.1:{forwarding_port}:127.0.0.1:22"));
    let output = ssh_request(
        remote_forwarding,
        port,
        &user,
        Some("codex-notifier receive"),
        &event("forwarding").to_json().expect("forwarding event"),
    );
    assert!(
        !output.status.success(),
        "remote forwarding was not rejected"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("administratively prohibited")
            || String::from_utf8_lossy(&output.stderr).contains("forwarding failed")
    );

    assert_eq!(delivery.count(), 4);
    relay_shutdown_tx.send(()).expect("relay agent shutdown");
    let relay_report = relay_task
        .await
        .expect("relay agent task")
        .expect("relay agent run");
    assert!(relay_report.agent.retried >= 1);
    assert_eq!(relay_report.agent.delivered, 3);
    assert_eq!(relay_report.agent.dead_lettered, 0);
    shutdown_tx.send(()).expect("agent shutdown");
    let report = agent_task.await.expect("agent task").expect("agent run");
    assert_eq!(report.agent.delivered, 4);
}
