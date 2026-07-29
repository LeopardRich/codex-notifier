//! Stage 17 diagnostic and delivery-aware command contracts.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use codex_notifier::AgentHost;
use codex_notifier_application::{
    AgentError, AgentState, CancellationToken, DeliveryFailure, DeliveryFuture, DeliveryOutcome,
    EventDelivery, RoleDeliveryFactory, SafeErrorCode,
};
use codex_notifier_config::{
    CliOverrides, ConfigLoader, FileSystemStateProbe, PathEnvironment, Platform, Role,
};
use codex_notifier_core::CanonicalEvent;
use tempfile::TempDir;
use tokio::sync::oneshot;

static NEXT_PROFILE: AtomicUsize = AtomicUsize::new(0);

struct IsolatedEnvironment {
    _directory: TempDir,
    home: PathBuf,
    config_base: PathBuf,
    state_base: PathBuf,
    config_file: PathBuf,
    state_dir: PathBuf,
}

impl IsolatedEnvironment {
    fn new(role: Role) -> Self {
        #[cfg(unix)]
        let directory = tempfile::Builder::new()
            .prefix("cnd")
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
        let profile = format!(
            "d{}_{}",
            std::process::id(),
            NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
        );
        let document = match role {
            Role::Desktop => format!("config_version = 1\n[agent]\nprofile = \"{profile}\"\n"),
            Role::Relay => format!(
                concat!(
                    "config_version = 1\n",
                    "[agent]\nrole = \"relay\"\nprofile = \"{}\"\n",
                    "[relay]\nssh_host_alias = \"desktop-test\"\n",
                    "retry_initial_delay_ms = 100\n",
                    "retry_max_delay_ms = 100\n",
                    "retry_max_attempts = 2\n",
                ),
                profile
            ),
        };
        fs::write(paths.config_file(), &document).expect("configuration");
        let config = ConfigLoader::load(
            &paths,
            Some(&document),
            None,
            CliOverrides::new(),
            &FileSystemStateProbe,
        )
        .expect("validated configuration");
        assert_eq!(config.agent().role(), role);

        Self {
            _directory: directory,
            home,
            config_base,
            state_base,
            config_file: paths.config_file().to_owned(),
            state_dir: paths.state_dir().to_owned(),
        }
    }

    fn config(&self) -> codex_notifier_config::Config {
        let document = fs::read_to_string(&self.config_file).expect("configuration");
        let paths = self.paths();
        ConfigLoader::load(
            &paths,
            Some(&document),
            None,
            CliOverrides::new(),
            &FileSystemStateProbe,
        )
        .expect("validated configuration")
    }

    fn paths(&self) -> codex_notifier_config::ConfigPaths {
        #[cfg(windows)]
        return PathEnvironment::new()
            .with_home(&self.home)
            .with_windows_app_data(&self.config_base)
            .with_windows_local_app_data(&self.state_base)
            .resolve(Platform::Windows)
            .expect("Windows paths");
        #[cfg(target_os = "macos")]
        return PathEnvironment::new()
            .with_home(&self.home)
            .resolve(Platform::MacOs)
            .expect("macOS paths");
        #[cfg(all(unix, not(target_os = "macos")))]
        return PathEnvironment::new()
            .with_home(&self.home)
            .with_xdg_config_home(&self.config_base)
            .with_xdg_state_home(&self.state_base)
            .resolve(Platform::Xdg)
            .expect("XDG paths");
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
            .env("ALL_PROXY", "socks5://127.0.0.1:1");
        command
    }
}

#[derive(Default)]
struct ImmediateDelivery;

impl EventDelivery for ImmediateDelivery {
    fn deliver<'a>(
        &'a self,
        _event: &'a CanonicalEvent,
        _cancellation: CancellationToken,
    ) -> DeliveryFuture<'a> {
        Box::pin(async { DeliveryOutcome::Delivered })
    }
}

#[derive(Default)]
struct RecordingFactory {
    desktop: AtomicUsize,
    relay: AtomicUsize,
}

impl RoleDeliveryFactory for RecordingFactory {
    fn desktop(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        self.desktop.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(ImmediateDelivery))
    }

    fn relay(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        self.relay.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(ImmediateDelivery))
    }
}

struct FailingDelivery {
    retryable: bool,
}

impl EventDelivery for FailingDelivery {
    fn deliver<'a>(
        &'a self,
        _event: &'a CanonicalEvent,
        _cancellation: CancellationToken,
    ) -> DeliveryFuture<'a> {
        let retryable = self.retryable;
        Box::pin(async move {
            DeliveryOutcome::Failed(DeliveryFailure::new(
                SafeErrorCode::parse("diagnostic_test_failure").expect("safe error code"),
                retryable,
            ))
        })
    }
}

struct FailingFactory {
    retryable: bool,
}

impl RoleDeliveryFactory for FailingFactory {
    fn desktop(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        Ok(Arc::new(FailingDelivery {
            retryable: self.retryable,
        }))
    }

    fn relay(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        Err(AgentError::DeliveryInitialization)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_command_waits_for_receipts_for_both_events_on_local_and_remote_routes() {
    for role in [Role::Desktop, Role::Relay] {
        for (argument, expected_kind) in [
            ("task-completed", "task_completed"),
            ("approval-requested", "approval_requested"),
        ] {
            let environment = IsolatedEnvironment::new(role);
            let factory = RecordingFactory::default();
            let host = AgentHost::from_config(&environment.config(), &factory).expect("agent host");
            assert_eq!(
                factory.desktop.load(Ordering::SeqCst),
                usize::from(role == Role::Desktop)
            );
            assert_eq!(
                factory.relay.load(Ordering::SeqCst),
                usize::from(role == Role::Relay)
            );
            let runtime = host.runtime();
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let host_task = tokio::spawn(host.run_until(async {
                let _ = shutdown_rx.await;
            }));
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            while runtime.state() != AgentState::Ready {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "agent readiness timeout"
                );
                tokio::task::yield_now().await;
            }

            let output = tokio::task::spawn_blocking(move || {
                environment
                    .command()
                    .args(["test", argument, "--format", "json", "--wait-ms", "5000"])
                    .output()
                    .expect("test command")
            })
            .await
            .expect("test child task");
            assert!(
                output.status.success(),
                "test failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stderr.is_empty());
            let report: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("test report JSON");
            assert_eq!(report["status"], "ready");
            assert_eq!(report["code"], "test_delivery_succeeded");
            assert_eq!(report["event_kind"], expected_kind);
            assert_eq!(
                report["route"],
                if role == Role::Desktop {
                    "local"
                } else {
                    "remote"
                }
            );
            assert_eq!(report["delivery"], "delivered");
            assert_eq!(report["exit_code"], 0);

            shutdown_tx.send(()).expect("shutdown");
            let host_report = host_task.await.expect("host task").expect("host report");
            assert_eq!(host_report.agent.delivered, 1);
            assert_eq!(host_report.agent.dead_lettered, 0);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_command_distinguishes_pending_timeout_and_permanent_dead_letter() {
    for (retryable, expected_code, expected_exit, expected_delivery) in [
        (true, "test_delivery_timeout", 24, "pending"),
        (false, "test_delivery_dead_lettered", 25, "dead_lettered"),
    ] {
        let environment = IsolatedEnvironment::new(Role::Desktop);
        let host = AgentHost::from_config(&environment.config(), &FailingFactory { retryable })
            .expect("agent host");
        let runtime = host.runtime();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let host_task = tokio::spawn(host.run_until(async {
            let _ = shutdown_rx.await;
        }));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.state() != AgentState::Ready {
            assert!(
                tokio::time::Instant::now() < deadline,
                "agent readiness timeout"
            );
            tokio::task::yield_now().await;
        }

        let output = tokio::task::spawn_blocking(move || {
            environment
                .command()
                .args([
                    "test",
                    "task-completed",
                    "--format",
                    "json",
                    "--wait-ms",
                    "100",
                ])
                .output()
                .expect("test command")
        })
        .await
        .expect("test child task");
        assert_eq!(output.status.code(), Some(expected_exit));
        assert!(output.stderr.is_empty());
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("test report JSON");
        assert_eq!(report["status"], "failed");
        assert_eq!(report["code"], expected_code);
        assert_eq!(report["exit_code"], expected_exit);
        assert_eq!(report["delivery"], expected_delivery);
        assert_ne!(report["remediation"], "none");

        shutdown_tx.send(()).expect("shutdown");
        host_task.await.expect("host task").expect("host report");
    }
}

#[test]
fn relay_status_is_read_only_and_does_not_require_a_desktop_platform() {
    let environment = IsolatedEnvironment::new(Role::Relay);
    let entries_before = fs::read_dir(&environment.state_dir)
        .expect("state directory")
        .count();
    let output = environment
        .command()
        .args(["status", "--format", "json"])
        .output()
        .expect("relay status");
    assert_eq!(output.status.code(), Some(12));
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read_dir(&environment.state_dir)
            .expect("state directory")
            .count(),
        entries_before
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status JSON");
    assert_eq!(report["role"], "relay");
    assert_eq!(report["code"], "agent_not_running");
    assert_eq!(report["storage"], "ready");
    assert_eq!(report["notification"], "not_applicable");
}

#[test]
fn comprehensive_doctor_is_read_only_and_redacts_machine_values() {
    let environment = IsolatedEnvironment::new(Role::Desktop);
    fs::write(&environment.config_file, b"config_version = 999\n").expect("invalid config");
    let state_before = fs::read_dir(&environment.state_dir)
        .expect("state directory")
        .count();
    let output = environment
        .command()
        .args(["doctor", "--format", "json"])
        .output()
        .expect("doctor command");
    assert_eq!(output.status.code(), Some(10));
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read_dir(&environment.state_dir)
            .expect("state directory")
            .count(),
        state_before
    );
    assert_eq!(
        fs::read(&environment.config_file).expect("configuration"),
        b"config_version = 999\n"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor report JSON");
    assert_eq!(report["status"], "failed");
    assert_eq!(report["checks"][0]["code"], "config_version_unsupported");
    let text = String::from_utf8(output.stdout).expect("UTF-8 report");
    for forbidden in [
        environment.home.to_string_lossy().as_ref(),
        environment.config_file.to_string_lossy().as_ref(),
        "private-user",
        "PRIVATE KEY",
        "event body",
    ] {
        assert!(!text.contains(forbidden));
    }

    let status = environment
        .command()
        .args(["status", "--format", "json"])
        .output()
        .expect("status command");
    assert_eq!(status.status.code(), Some(10));
    assert!(status.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("status failure JSON");
    assert_eq!(report["status"], "failed");
    assert_eq!(report["code"], "config_version_unsupported");
    assert_ne!(report["remediation"], "none");
}
