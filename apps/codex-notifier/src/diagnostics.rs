//! Read-only health diagnostics and delivery-aware self-test reporting.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{fmt::Write as _, io::ErrorKind};

use codex_notifier_codex_source::{CapabilityAvailability, CodexCapabilityReport, CodexInterface};
use codex_notifier_config::{Config, Role};
use codex_notifier_core::EventKind;
use codex_notifier_ipc::AckStatus;
use codex_notifier_native_notification::{NotificationDiagnostic, NotificationStatus};
use codex_notifier_persistence::{PersistenceError, SqliteStore, StoredEventState};
use codex_notifier_ssh_transport::{
    DiagnosticStatus as SshStatus, OpenSshConfig, OpenSshDelivery, SshDeliveryError,
    diagnose_host_key, diagnose_openssh_client,
};
use serde::Serialize;

use crate::database_path;
use crate::desktop::{
    DesktopError, load_current_config_read_only, notification_diagnostic, probe_local_ipc,
    read_agent_status, submit_local_test,
};
use crate::installer::{InstallerError, StatusReport, StatusStorage};

const SCHEMA_VERSION: u16 = 1;

/// Stable process exit codes for Stage 17 health and self-test faults.
pub mod exit {
    /// All requested checks succeeded.
    pub const OK: i32 = 0;
    /// Configuration or installation state is invalid.
    pub const CONFIGURATION: i32 = 10;
    /// Codex is absent or not fixture-verified.
    pub const CODEX: i32 = 11;
    /// The agent or startup registration is unhealthy.
    pub const AGENT: i32 = 12;
    /// Same-user local IPC is unhealthy.
    pub const IPC: i32 = 13;
    /// Durable state cannot be inspected or contains failed deliveries.
    pub const STORAGE: i32 = 14;
    /// Native notification permissions or session state block delivery.
    pub const NOTIFICATION: i32 = 15;
    /// The system OpenSSH client is unavailable.
    pub const SSH_CLIENT: i32 = 16;
    /// Strict host-key verification is not ready.
    pub const SSH_HOST_KEY: i32 = 17;
    /// SSH authentication failed.
    pub const SSH_AUTHENTICATION: i32 = 18;
    /// The configured SSH destination is unreachable.
    pub const SSH_NETWORK: i32 = 19;
    /// The configured SSH destination timed out.
    pub const SSH_TIMEOUT: i32 = 20;
    /// The restricted receiver rejected the diagnostic exchange.
    pub const SSH_RECEIVER: i32 = 21;
    /// The SSH receiver response or process contract is invalid.
    pub const SSH_PROTOCOL: i32 = 22;
    /// A self-test could not be submitted.
    pub const TEST_SUBMISSION: i32 = 23;
    /// A self-test remained queued beyond its wait bound.
    pub const TEST_TIMEOUT: i32 = 24;
    /// A self-test reached a metadata-only dead letter.
    pub const TEST_DEAD_LETTER: i32 = 25;
}

/// User-selected stable output representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    /// Fixed line-oriented human-readable output.
    Human,
    /// Compact machine-readable JSON.
    Json,
}

/// Typed diagnostic outcome shared by both renderers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The check completed successfully.
    Ready,
    /// The check found an actionable fault.
    Failed,
    /// The check does not apply to the configured role or could not run safely.
    Skipped,
}

impl CheckStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

/// One payload-free health check with a fixed repair action.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticCheck {
    name: &'static str,
    status: CheckStatus,
    code: String,
    message: &'static str,
    remediation: &'static str,
    exit_code: i32,
}

impl DiagnosticCheck {
    fn ready(name: &'static str, code: &'static str, message: &'static str) -> Self {
        Self {
            name,
            status: CheckStatus::Ready,
            code: code.to_owned(),
            message,
            remediation: "none",
            exit_code: exit::OK,
        }
    }

    fn failed(
        name: &'static str,
        code: impl Into<String>,
        message: &'static str,
        remediation: &'static str,
        exit_code: i32,
    ) -> Self {
        Self {
            name,
            status: CheckStatus::Failed,
            code: code.into(),
            message,
            remediation,
            exit_code,
        }
    }

    fn skipped(name: &'static str) -> Self {
        Self {
            name,
            status: CheckStatus::Skipped,
            code: "not_applicable".to_owned(),
            message: "This check does not apply to the configured role.",
            remediation: "none",
            exit_code: exit::OK,
        }
    }

    fn blocked(name: &'static str) -> Self {
        Self {
            name,
            status: CheckStatus::Skipped,
            code: "configuration_prerequisite_failed".to_owned(),
            message: "This check could not run because configuration is invalid.",
            remediation: "Resolve the configuration check, then rerun doctor.",
            exit_code: exit::OK,
        }
    }
}

/// Complete read-only health report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    schema_version: u16,
    command: &'static str,
    status: CheckStatus,
    checks: Vec<DiagnosticCheck>,
}

impl DoctorReport {
    fn new(checks: Vec<DiagnosticCheck>) -> Self {
        let status = if checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed)
        {
            CheckStatus::Failed
        } else {
            CheckStatus::Ready
        };
        Self {
            schema_version: SCHEMA_VERSION,
            command: "doctor",
            status,
            checks,
        }
    }

    /// Returns the first actionable exit code in documented check order.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.checks
            .iter()
            .find(|check| check.status == CheckStatus::Failed)
            .map_or(exit::OK, |check| check.exit_code)
    }

    /// Renders the report without interpolating inspected values.
    #[must_use]
    pub fn render(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Json => serde_json::to_string(self)
                .unwrap_or_else(|_| "{\"schema_version\":1,\"command\":\"doctor\",\"status\":\"failed\",\"checks\":[]}".to_owned()),
            OutputFormat::Human => {
                let mut output = format!("status={}\n", self.status.as_str());
                for check in &self.checks {
                    let _ = write!(
                        output,
                        "{}.status={}\n{}.code={}\n{}.exit_code={}\n{}.message={}\n{}.remediation={}\n",
                        check.name,
                        check.status.as_str(),
                        check.name,
                        check.code,
                        check.name,
                        check.exit_code,
                        check.name,
                        check.message,
                        check.name,
                        check.remediation,
                    );
                }
                output
            }
        }
    }
}

/// Runs all role-relevant health checks without modifying managed state.
pub async fn doctor() -> DoctorReport {
    let (_, config) = match load_current_config_read_only() {
        Ok(value) => value,
        Err(error) => {
            let mut checks = vec![configuration_failure(&error)];
            for name in [
                "codex",
                "agent",
                "ipc",
                "storage",
                "notification",
                "openssh",
                "ssh_host_key",
                "ssh_target",
            ] {
                checks.push(DiagnosticCheck::blocked(name));
            }
            return DoctorReport::new(checks);
        }
    };

    let mut checks = vec![DiagnosticCheck::ready(
        "configuration",
        "configuration_ready",
        "Configuration is valid.",
    )];
    checks.push(codex_check());

    let agent = read_agent_status(config.storage().state_dir());
    checks.push(if agent.running {
        DiagnosticCheck::ready("agent", "agent_ready", "The configured agent is running.")
    } else if agent.stale {
        DiagnosticCheck::failed(
            "agent",
            "agent_status_stale",
            "The agent status record is stale or malformed.",
            "Restart the per-user agent, then rerun doctor.",
            exit::AGENT,
        )
    } else {
        DiagnosticCheck::failed(
            "agent",
            "agent_not_running",
            "The configured agent is not running.",
            "Start the per-user agent, then rerun doctor.",
            exit::AGENT,
        )
    });

    checks.push(match probe_local_ipc(&config).await {
        Ok(()) => DiagnosticCheck::ready(
            "ipc",
            "ipc_ready",
            "Same-user local IPC accepted a connection.",
        ),
        Err(error) => DiagnosticCheck::failed(
            "ipc",
            error.code(),
            "Same-user local IPC is unavailable.",
            "Confirm the agent is running under the current user and restart it.",
            exit::IPC,
        ),
    });
    checks.push(storage_check(config.storage().state_dir()));

    checks.extend(role_checks(&config).await);
    DoctorReport::new(checks)
}

async fn role_checks(config: &Config) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::with_capacity(4);
    match config.agent().role() {
        Role::Desktop => {
            checks.push(notification_check(notification_diagnostic(config)));
            checks.push(DiagnosticCheck::skipped("openssh"));
            checks.push(DiagnosticCheck::skipped("ssh_host_key"));
            checks.push(DiagnosticCheck::skipped("ssh_target"));
        }
        Role::Relay => {
            checks.push(DiagnosticCheck::skipped("notification"));
            let client = diagnose_openssh_client();
            checks.push(ssh_prerequisite_check(
                "openssh",
                client,
                "openssh_ready",
                "The system OpenSSH client is available.",
                "ssh_executable_unavailable",
                "Install or enable the system OpenSSH client.",
                exit::SSH_CLIENT,
            ));
            let host_key = ssh_known_hosts_path().map_or(SshStatus::Unavailable, |path| {
                diagnose_host_key(config.relay().ssh_host_alias(), &path, None)
            });
            checks.push(ssh_prerequisite_check(
                "ssh_host_key",
                host_key,
                "ssh_host_key_ready",
                "Strict host-key verification is configured.",
                "ssh_host_key_not_ready",
                "Pin the destination host key and require strict verification.",
                exit::SSH_HOST_KEY,
            ));
            if client == SshStatus::Ready && host_key == SshStatus::Ready {
                checks.push(ssh_target_check(config).await);
            } else {
                checks.push(DiagnosticCheck {
                    name: "ssh_target",
                    status: CheckStatus::Skipped,
                    code: "ssh_prerequisite_failed".to_owned(),
                    message: "Target reachability was not attempted because an SSH prerequisite failed.",
                    remediation: "Resolve the preceding SSH check, then rerun doctor.",
                    exit_code: exit::OK,
                });
            }
        }
    }
    checks
}

fn configuration_failure(error: &DesktopError) -> DiagnosticCheck {
    DiagnosticCheck::failed(
        "configuration",
        error.code(),
        "Configuration could not be loaded safely.",
        "Repair or recreate the current-user configuration, then rerun doctor.",
        exit::CONFIGURATION,
    )
}

fn codex_check() -> DiagnosticCheck {
    let Some(version) = detect_codex_version() else {
        return DiagnosticCheck::failed(
            "codex",
            "codex_version_unavailable",
            "The Codex CLI version could not be detected.",
            "Install the supported Codex CLI and ensure it is available on PATH.",
            exit::CODEX,
        );
    };
    let report = CodexCapabilityReport::inspect(&version, CodexInterface::CliHook);
    if report.task_completed() == CapabilityAvailability::Supported {
        DiagnosticCheck::ready(
            "codex",
            "codex_capability_ready",
            "The Codex task-completion hook is fixture-verified.",
        )
    } else {
        DiagnosticCheck::failed(
            "codex",
            "codex_version_unsupported",
            "The detected Codex CLI is not fixture-verified.",
            "Install a supported Codex CLI version before enabling the hook.",
            exit::CODEX,
        )
    }
}

fn detect_codex_version() -> Option<String> {
    #[cfg(windows)]
    let output = Command::new("cmd.exe")
        .args(["/d", "/c", "codex", "--version"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    #[cfg(not(windows))]
    let output = Command::new("codex")
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 128 {
        return None;
    }
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .strip_prefix("codex-cli ")
        .filter(|value| !value.is_empty() && value.is_ascii())
        .map(str::to_owned)
}

fn storage_check(state_dir: &Path) -> DiagnosticCheck {
    let metadata = match state_dir.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return DiagnosticCheck::failed(
                "storage",
                "storage_unwritable",
                "The configured state path is not a safe directory.",
                "Repair the state-directory ownership and type, then restart the agent.",
                exit::STORAGE,
            );
        }
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return DiagnosticCheck::failed(
                "storage",
                "storage_not_found",
                "The configured state directory does not exist.",
                "Start the agent once to initialize durable state.",
                exit::STORAGE,
            );
        }
        Err(_) => {
            return DiagnosticCheck::failed(
                "storage",
                "storage_unwritable",
                "The configured state directory cannot be inspected.",
                "Repair current-user access to the state directory.",
                exit::STORAGE,
            );
        }
    };
    if metadata.permissions().readonly() {
        return DiagnosticCheck::failed(
            "storage",
            "storage_unwritable",
            "The configured state directory is read-only.",
            "Restore current-user write access to the state directory.",
            exit::STORAGE,
        );
    }
    let database = database_path(state_dir);
    if !database.exists() {
        return DiagnosticCheck::ready(
            "storage",
            "storage_empty",
            "The state directory is ready and no database exists yet.",
        );
    }
    match SqliteStore::inspect_read_only(&database) {
        Ok(_) => DiagnosticCheck::ready(
            "storage",
            "storage_ready",
            "The durable state database is readable and current.",
        ),
        Err(error) => DiagnosticCheck::failed(
            "storage",
            error.code().as_str(),
            "The durable state database cannot be inspected safely.",
            "Stop the agent and repair or restore the state database.",
            exit::STORAGE,
        ),
    }
}

fn notification_check(result: Result<NotificationDiagnostic, DesktopError>) -> DiagnosticCheck {
    match result {
        Ok(diagnostic) if diagnostic.status() == NotificationStatus::Ready => {
            DiagnosticCheck::ready(
                "notification",
                "notification_ready",
                "Native notification delivery is enabled.",
            )
        }
        Ok(diagnostic) => DiagnosticCheck::failed(
            "notification",
            notification_code(diagnostic.status()),
            "Native notification delivery is not ready.",
            notification_remediation(diagnostic.status()),
            exit::NOTIFICATION,
        ),
        Err(error) => DiagnosticCheck::failed(
            "notification",
            error.code(),
            "Native notification state could not be inspected.",
            "Run doctor in an interactive supported desktop session.",
            exit::NOTIFICATION,
        ),
    }
}

const fn notification_code(status: NotificationStatus) -> &'static str {
    match status {
        NotificationStatus::Ready => "notification_ready",
        NotificationStatus::UnsupportedPlatform => "notification_platform_unsupported",
        NotificationStatus::ApplicationIdentityMissing => "notification_identity_missing",
        NotificationStatus::AuthorizationNotDetermined => "notification_authorization_required",
        NotificationStatus::DisabledForApplication => "notification_application_disabled",
        NotificationStatus::DisabledForUser => "notification_user_disabled",
        NotificationStatus::DisabledByPolicy => "notification_policy_disabled",
        NotificationStatus::NoInteractiveSession => "notification_session_unavailable",
        _ => "notification_status_unavailable",
    }
}

const fn notification_remediation(status: NotificationStatus) -> &'static str {
    match status {
        NotificationStatus::Ready => "none",
        NotificationStatus::ApplicationIdentityMissing => {
            "Reinstall the per-user desktop application identity."
        }
        NotificationStatus::AuthorizationNotDetermined => {
            "Launch the installed application interactively and grant notification access."
        }
        NotificationStatus::DisabledForApplication => {
            "Enable Codex Notifier in system notification settings."
        }
        NotificationStatus::DisabledForUser => {
            "Enable notifications for the current operating-system user."
        }
        NotificationStatus::DisabledByPolicy => {
            "Ask the system administrator to permit native notifications."
        }
        NotificationStatus::NoInteractiveSession => {
            "Run the desktop agent in the signed-in graphical user session."
        }
        _ => "Run the desktop role on a supported interactive Windows or macOS host.",
    }
}

fn ssh_prerequisite_check(
    name: &'static str,
    status: SshStatus,
    ready_code: &'static str,
    ready_message: &'static str,
    failure_code: &'static str,
    remediation: &'static str,
    exit_code: i32,
) -> DiagnosticCheck {
    if status == SshStatus::Ready {
        DiagnosticCheck::ready(name, ready_code, ready_message)
    } else {
        DiagnosticCheck::failed(
            name,
            match status {
                SshStatus::Missing => format!("{failure_code}_missing"),
                SshStatus::Insecure => format!("{failure_code}_insecure"),
                SshStatus::NotConfigured => format!("{failure_code}_not_configured"),
                SshStatus::Unavailable | SshStatus::Ready => failure_code.to_owned(),
            },
            "An OpenSSH prerequisite is not ready.",
            remediation,
            exit_code,
        )
    }
}

async fn ssh_target_check(config: &Config) -> DiagnosticCheck {
    let delivery = config
        .relay()
        .ssh_host_alias()
        .and_then(|alias| {
            OpenSshConfig::new(
                alias,
                Duration::from_millis(config.relay().connect_timeout_ms()),
            )
            .ok()
        })
        .map(OpenSshDelivery::new);
    let Some(delivery) = delivery else {
        return ssh_delivery_failure(&SshDeliveryError::InvalidConfiguration);
    };
    match delivery.probe_receiver().await {
        Ok(()) => DiagnosticCheck::ready(
            "ssh_target",
            "ssh_target_ready",
            "The restricted SSH receiver is reachable and authenticated.",
        ),
        Err(error) => ssh_delivery_failure(&error),
    }
}

fn ssh_delivery_failure(error: &SshDeliveryError) -> DiagnosticCheck {
    let (message, remediation, exit_code) = match error {
        SshDeliveryError::ExecutableUnavailable => (
            "The system OpenSSH client could not start.",
            "Install or enable the system OpenSSH client.",
            exit::SSH_CLIENT,
        ),
        SshDeliveryError::HostKeyVerificationFailed => (
            "Strict SSH host-key verification failed.",
            "Verify and repin the destination host key before retrying.",
            exit::SSH_HOST_KEY,
        ),
        SshDeliveryError::AuthenticationFailed => (
            "SSH public-key authentication failed.",
            "Verify the dedicated key and restricted authorized-key entry.",
            exit::SSH_AUTHENTICATION,
        ),
        SshDeliveryError::NetworkUnavailable => (
            "The configured SSH destination is unreachable.",
            "Verify the destination network and SSH service.",
            exit::SSH_NETWORK,
        ),
        SshDeliveryError::ConnectionTimeout => (
            "The configured SSH destination timed out.",
            "Verify routing, firewall policy, and the SSH service.",
            exit::SSH_TIMEOUT,
        ),
        SshDeliveryError::RemoteRejected { .. } => (
            "The restricted receiver rejected the diagnostic exchange.",
            "Run doctor on the desktop and verify the forced receiver command.",
            exit::SSH_RECEIVER,
        ),
        _ => (
            "The restricted SSH receiver contract could not be verified.",
            "Verify the fixed SSH configuration and installed receiver version.",
            exit::SSH_PROTOCOL,
        ),
    };
    DiagnosticCheck::failed("ssh_target", error.code(), message, remediation, exit_code)
}

fn ssh_known_hosts_path() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join(".ssh").join("known_hosts"))
}

/// Delivery-aware result for one explicit synthetic event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TestReport {
    schema_version: u16,
    command: &'static str,
    status: CheckStatus,
    code: String,
    exit_code: i32,
    message: &'static str,
    remediation: &'static str,
    route: &'static str,
    event_kind: &'static str,
    event_id: Option<String>,
    acknowledgement: Option<&'static str>,
    delivery: &'static str,
    detail_code: Option<String>,
}

impl TestReport {
    /// Returns the stable process exit code.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Renders the same typed result as human lines or compact JSON.
    #[must_use]
    pub fn render(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Json => serde_json::to_string(self).unwrap_or_else(|_| {
                "{\"schema_version\":1,\"command\":\"test\",\"status\":\"failed\"}".to_owned()
            }),
            OutputFormat::Human => format!(
                "status={}\ncode={}\nexit_code={}\nmessage={}\nremediation={}\nroute={}\nevent_kind={}\nevent_id={}\nacknowledgement={}\ndelivery={}\ndetail_code={}\n",
                self.status.as_str(),
                self.code,
                self.exit_code,
                self.message,
                self.remediation,
                self.route,
                self.event_kind,
                self.event_id.as_deref().unwrap_or("none"),
                self.acknowledgement.unwrap_or("none"),
                self.delivery,
                self.detail_code.as_deref().unwrap_or("none"),
            ),
        }
    }
}

/// Submits one explicit event and waits for its local outbox receipt/dead letter.
pub async fn run_test(kind: EventKind, wait: Option<Duration>) -> TestReport {
    let config = match load_current_config_read_only() {
        Ok((_, config)) => config,
        Err(error) => return test_submission_failure(kind, "unknown", &error),
    };
    let route = role_route(config.agent().role());
    let wait = wait.unwrap_or_else(|| match config.agent().role() {
        Role::Desktop => Duration::from_secs(15),
        Role::Relay => Duration::from_millis(config.relay().connect_timeout_ms())
            .saturating_add(Duration::from_secs(10)),
    });
    let (event_id, acknowledgement) = match submit_local_test(kind).await {
        Ok(value) => value,
        Err(error) => return test_submission_failure(kind, route, &error),
    };
    if acknowledgement == AckStatus::Rejected {
        return test_rejected(kind, route, event_id);
    }
    wait_for_test_delivery(
        kind,
        route,
        event_id,
        acknowledgement,
        config.storage().state_dir(),
        wait,
    )
    .await
}

async fn wait_for_test_delivery(
    kind: EventKind,
    route: &'static str,
    event_id: codex_notifier_core::EventId,
    acknowledgement: AckStatus,
    state_dir: &Path,
    wait: Duration,
) -> TestReport {
    match wait_for_event_state(state_dir, event_id, wait).await {
        Ok(StoredEventState::Delivered) => test_delivered(kind, route, event_id, acknowledgement),
        Ok(StoredEventState::DeadLettered { error_code }) => {
            test_dead_lettered(kind, route, event_id, acknowledgement, error_code)
        }
        Ok(StoredEventState::Pending) => test_timed_out(kind, route, event_id, acknowledgement),
        Err(error) => test_storage_failure(kind, route, event_id, acknowledgement, &error),
    }
}

/// Waits for one event to reach a receipt or dead letter using read-only queries.
///
/// The pending result also represents a missing database/row at the deadline.
///
/// # Errors
///
/// Returns a classified schema, corruption, lock, or availability failure.
pub async fn wait_for_event_state(
    state_dir: &Path,
    event_id: codex_notifier_core::EventId,
    wait: Duration,
) -> Result<StoredEventState, PersistenceError> {
    let database = database_path(state_dir);
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match SqliteStore::inspect_event_read_only(&database, event_id) {
            Ok(Some(
                state @ (StoredEventState::Delivered | StoredEventState::DeadLettered { .. }),
            )) => {
                return Ok(state);
            }
            Ok(Some(StoredEventState::Pending) | None) | Err(PersistenceError::NotFound) => {}
            Err(error) => return Err(error),
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(StoredEventState::Pending);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn test_rejected(
    kind: EventKind,
    route: &'static str,
    event_id: codex_notifier_core::EventId,
) -> TestReport {
    TestReport {
        schema_version: SCHEMA_VERSION,
        command: "test",
        status: CheckStatus::Failed,
        code: "test_submission_rejected".to_owned(),
        exit_code: exit::TEST_SUBMISSION,
        message: "The agent rejected the synthetic event.",
        remediation: "Run doctor and resolve the reported agent or storage fault.",
        route,
        event_kind: event_kind_name(kind),
        event_id: Some(event_id.to_string()),
        acknowledgement: Some("rejected"),
        delivery: "not_submitted",
        detail_code: None,
    }
}

fn test_delivered(
    kind: EventKind,
    route: &'static str,
    event_id: codex_notifier_core::EventId,
    acknowledgement: AckStatus,
) -> TestReport {
    TestReport {
        schema_version: SCHEMA_VERSION,
        command: "test",
        status: CheckStatus::Ready,
        code: "test_delivery_succeeded".to_owned(),
        exit_code: exit::OK,
        message: "The synthetic event reached a successful delivery receipt.",
        remediation: "none",
        route,
        event_kind: event_kind_name(kind),
        event_id: Some(event_id.to_string()),
        acknowledgement: Some(acknowledgement_name(acknowledgement)),
        delivery: "delivered",
        detail_code: None,
    }
}

fn test_dead_lettered(
    kind: EventKind,
    route: &'static str,
    event_id: codex_notifier_core::EventId,
    acknowledgement: AckStatus,
    error_code: String,
) -> TestReport {
    TestReport {
        schema_version: SCHEMA_VERSION,
        command: "test",
        status: CheckStatus::Failed,
        code: "test_delivery_dead_lettered".to_owned(),
        exit_code: exit::TEST_DEAD_LETTER,
        message: "The synthetic event reached a permanent delivery failure.",
        remediation: "Run doctor and resolve the reported delivery prerequisite.",
        route,
        event_kind: event_kind_name(kind),
        event_id: Some(event_id.to_string()),
        acknowledgement: Some(acknowledgement_name(acknowledgement)),
        delivery: "dead_lettered",
        detail_code: Some(error_code),
    }
}

fn test_storage_failure(
    kind: EventKind,
    route: &'static str,
    event_id: codex_notifier_core::EventId,
    acknowledgement: AckStatus,
    error: &PersistenceError,
) -> TestReport {
    TestReport {
        schema_version: SCHEMA_VERSION,
        command: "test",
        status: CheckStatus::Failed,
        code: "test_storage_unavailable".to_owned(),
        exit_code: exit::STORAGE,
        message: "Self-test delivery state could not be inspected.",
        remediation: "Stop the agent and repair or restore the state database.",
        route,
        event_kind: event_kind_name(kind),
        event_id: Some(event_id.to_string()),
        acknowledgement: Some(acknowledgement_name(acknowledgement)),
        delivery: "unknown",
        detail_code: Some(error.code().as_str().to_owned()),
    }
}

fn test_timed_out(
    kind: EventKind,
    route: &'static str,
    event_id: codex_notifier_core::EventId,
    acknowledgement: AckStatus,
) -> TestReport {
    TestReport {
        schema_version: SCHEMA_VERSION,
        command: "test",
        status: CheckStatus::Failed,
        code: "test_delivery_timeout".to_owned(),
        exit_code: exit::TEST_TIMEOUT,
        message: "The synthetic event remains pending after the wait deadline.",
        remediation: "Run doctor; the durable event will continue under normal retry policy.",
        route,
        event_kind: event_kind_name(kind),
        event_id: Some(event_id.to_string()),
        acknowledgement: Some(acknowledgement_name(acknowledgement)),
        delivery: "pending",
        detail_code: None,
    }
}

fn test_submission_failure(
    kind: EventKind,
    route: &'static str,
    error: &DesktopError,
) -> TestReport {
    TestReport {
        schema_version: SCHEMA_VERSION,
        command: "test",
        status: CheckStatus::Failed,
        code: error.code().to_owned(),
        exit_code: match error {
            DesktopError::Config(_) | DesktopError::ConfigFile => exit::CONFIGURATION,
            DesktopError::Ipc(_) => exit::IPC,
            DesktopError::UnsupportedPlatform => exit::NOTIFICATION,
            DesktopError::Ssh(_) => exit::SSH_PROTOCOL,
            DesktopError::Host(_) | DesktopError::TestEvent | DesktopError::Status => {
                exit::TEST_SUBMISSION
            }
        },
        message: "The synthetic event could not be submitted.",
        remediation: "Run doctor and resolve the first reported fault.",
        route,
        event_kind: event_kind_name(kind),
        event_id: None,
        acknowledgement: None,
        delivery: "not_submitted",
        detail_code: None,
    }
}

const fn role_route(role: Role) -> &'static str {
    match role {
        Role::Desktop => "local",
        Role::Relay => "remote",
    }
}

const fn event_kind_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::TaskCompleted => "task_completed",
        EventKind::ApprovalRequested => "approval_requested",
    }
}

const fn acknowledgement_name(status: AckStatus) -> &'static str {
    match status {
        AckStatus::Accepted => "accepted",
        AckStatus::Duplicate => "duplicate",
        AckStatus::Delivered => "delivered",
        AckStatus::Rejected => "rejected",
    }
}

#[derive(Serialize)]
#[allow(clippy::struct_excessive_bools)]
struct StatusOutput<'a> {
    schema_version: u16,
    command: &'static str,
    status: CheckStatus,
    code: &'a str,
    exit_code: i32,
    remediation: &'static str,
    role: &'static str,
    installed: bool,
    version: &'a str,
    startup_registered: bool,
    agent_running: bool,
    agent_stale: bool,
    profile_configured: bool,
    storage: &'static str,
    storage_error: Option<&'static str>,
    queue_pending: Option<usize>,
    oldest_queued_age_ms: Option<u64>,
    delivery_receipts: Option<usize>,
    latest_delivery_at_ms: Option<i64>,
    dead_letters: Option<usize>,
    notification: &'static str,
    notification_error: Option<&'a str>,
    focus: &'static str,
}

#[derive(Serialize)]
struct StatusFailureOutput<'a> {
    schema_version: u16,
    command: &'static str,
    status: CheckStatus,
    code: &'a str,
    exit_code: i32,
    remediation: &'static str,
}

/// Renders an early status failure using the same health envelope.
#[must_use]
pub fn render_status_error(error: &InstallerError, format: OutputFormat) -> (String, i32) {
    let (exit_code, remediation) = match error {
        InstallerError::QueueStatus => (
            exit::STORAGE,
            "Stop the agent and repair or restore the state database.",
        ),
        InstallerError::AgentStart => (exit::AGENT, "Restart the per-user agent."),
        InstallerError::Platform(_) => (
            exit::CONFIGURATION,
            "Repair the current-user desktop installation paths and ownership.",
        ),
        InstallerError::Desktop(_) => (
            exit::CONFIGURATION,
            "Repair or recreate the current-user configuration.",
        ),
        InstallerError::RelayRoleRequired => (
            exit::CONFIGURATION,
            "Set the current-user agent role to relay before managing its Codex hook.",
        ),
        InstallerError::Lifecycle(_)
        | InstallerError::NotInstalled
        | InstallerError::UnsupportedCodex
        | InstallerError::UnsafeHookCommand => (
            exit::CONFIGURATION,
            "Repair or reinstall the current-user notifier resources.",
        ),
    };
    let output = StatusFailureOutput {
        schema_version: SCHEMA_VERSION,
        command: "status",
        status: CheckStatus::Failed,
        code: error.code(),
        exit_code,
        remediation,
    };
    let rendered = match format {
        OutputFormat::Json => serde_json::to_string(&output).unwrap_or_else(|_| {
            "{\"schema_version\":1,\"command\":\"status\",\"status\":\"failed\"}".to_owned()
        }),
        OutputFormat::Human => format!(
            "status=failed\ncode={}\nexit_code={}\nremediation={}\n",
            output.code, output.exit_code, output.remediation
        ),
    };
    (rendered, exit_code)
}

/// Renders a read-only status report and returns its health exit code.
#[must_use]
pub fn render_status(report: &StatusReport, format: OutputFormat) -> (String, i32) {
    let (status, code, exit_code, remediation) = status_health(report);
    let version = report
        .version
        .as_deref()
        .filter(|value| safe_version(value))
        .unwrap_or("none");
    let output = StatusOutput {
        schema_version: SCHEMA_VERSION,
        command: "status",
        status,
        code,
        exit_code,
        remediation,
        role: role_name(report.role),
        installed: report.installed,
        version,
        startup_registered: report.startup_registered,
        agent_running: report.agent.running,
        agent_stale: report.agent.stale,
        profile_configured: report.agent.profile.is_some(),
        storage: report.storage.as_str(),
        storage_error: report.storage_error,
        queue_pending: report.queue_pending,
        oldest_queued_age_ms: report.oldest_queued_age_ms,
        delivery_receipts: report.delivery_receipts,
        latest_delivery_at_ms: report.latest_delivery_at_ms,
        dead_letters: report.dead_letters,
        notification: report.notification.map_or_else(
            || {
                if report.role == Role::Desktop {
                    "unavailable"
                } else {
                    "not_applicable"
                }
            },
            |value| value.status().as_str(),
        ),
        notification_error: report.notification_error.as_deref(),
        focus: report
            .notification
            .map_or("not_applicable", |value| value.focus().as_str()),
    };
    let rendered = match format {
        OutputFormat::Json => serde_json::to_string(&output).unwrap_or_else(|_| {
            "{\"schema_version\":1,\"command\":\"status\",\"status\":\"failed\"}".to_owned()
        }),
        OutputFormat::Human => format!(
            "status={}\ncode={}\nexit_code={}\nremediation={}\nrole={}\ninstalled={}\nversion={}\nstartup_registered={}\nagent_running={}\nagent_stale={}\nprofile_configured={}\nstorage={}\nstorage_error={}\nqueue_pending={}\noldest_queued_age_ms={}\ndelivery_receipts={}\nlatest_delivery_at_ms={}\ndead_letters={}\nnotification={}\nnotification_error={}\nfocus={}\n",
            output.status.as_str(),
            output.code,
            output.exit_code,
            output.remediation,
            output.role,
            output.installed,
            output.version,
            output.startup_registered,
            output.agent_running,
            output.agent_stale,
            output.profile_configured,
            output.storage,
            output.storage_error.unwrap_or("none"),
            optional_count(output.queue_pending),
            optional_u64(output.oldest_queued_age_ms),
            optional_count(output.delivery_receipts),
            optional_i64(output.latest_delivery_at_ms),
            optional_count(output.dead_letters),
            output.notification,
            output.notification_error.unwrap_or("none"),
            output.focus,
        ),
    };
    (rendered, exit_code)
}

fn status_health(report: &StatusReport) -> (CheckStatus, &'static str, i32, &'static str) {
    if report.role == Role::Desktop && !report.installed {
        return (
            CheckStatus::Failed,
            "status_install_missing",
            exit::CONFIGURATION,
            "Install the per-user desktop application.",
        );
    }
    if report.role == Role::Desktop && !report.startup_registered {
        return (
            CheckStatus::Failed,
            "status_startup_missing",
            exit::AGENT,
            "Repair the per-user startup registration.",
        );
    }
    if report.agent.stale {
        return (
            CheckStatus::Failed,
            "agent_status_stale",
            exit::AGENT,
            "Restart the per-user agent.",
        );
    }
    if !report.agent.running {
        return (
            CheckStatus::Failed,
            "agent_not_running",
            exit::AGENT,
            "Start the per-user agent.",
        );
    }
    if report.storage == StatusStorage::Missing {
        return (
            CheckStatus::Failed,
            "storage_not_found",
            exit::STORAGE,
            "Start the agent once to initialize durable state.",
        );
    }
    if report.storage == StatusStorage::Unavailable {
        return (
            CheckStatus::Failed,
            report.storage_error.unwrap_or("storage_database_failed"),
            exit::STORAGE,
            "Stop the agent and repair or restore the state database.",
        );
    }
    if report.dead_letters.is_some_and(|count| count > 0) {
        return (
            CheckStatus::Failed,
            "status_dead_letters_present",
            exit::STORAGE,
            "Run doctor and resolve the delivery prerequisite before retrying.",
        );
    }
    if let Some(notification) = report.notification {
        if notification.status() != NotificationStatus::Ready {
            return (
                CheckStatus::Failed,
                notification_code(notification.status()),
                exit::NOTIFICATION,
                notification_remediation(notification.status()),
            );
        }
    }
    if let Some(error) = report.notification_error.as_deref() {
        return (
            CheckStatus::Failed,
            if error == "notification_platform_unsupported" {
                "notification_platform_unsupported"
            } else {
                "notification_status_unavailable"
            },
            exit::NOTIFICATION,
            "Run doctor in an interactive supported desktop session.",
        );
    }
    (CheckStatus::Ready, "status_ready", exit::OK, "none")
}

const fn role_name(role: Role) -> &'static str {
    match role {
        Role::Desktop => "desktop",
        Role::Relay => "relay",
    }
}

fn safe_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
}

fn optional_count(value: Option<usize>) -> String {
    value.map_or_else(|| "unavailable".to_owned(), |value| value.to_string())
}

fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| value.to_string())
}

fn optional_i64(value: Option<i64>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::AgentStatus;
    use codex_notifier_native_notification::FocusStatus;

    #[test]
    fn notification_faults_have_fixed_codes_and_exit_codes() {
        for (status, code) in [
            (
                NotificationStatus::ApplicationIdentityMissing,
                "notification_identity_missing",
            ),
            (
                NotificationStatus::AuthorizationNotDetermined,
                "notification_authorization_required",
            ),
            (
                NotificationStatus::DisabledForApplication,
                "notification_application_disabled",
            ),
            (
                NotificationStatus::DisabledForUser,
                "notification_user_disabled",
            ),
            (
                NotificationStatus::DisabledByPolicy,
                "notification_policy_disabled",
            ),
            (
                NotificationStatus::NoInteractiveSession,
                "notification_session_unavailable",
            ),
            (
                NotificationStatus::Unavailable,
                "notification_status_unavailable",
            ),
        ] {
            let check = notification_check(Ok(NotificationDiagnostic::new(
                status,
                FocusStatus::Unknown,
            )));
            assert_eq!(check.status, CheckStatus::Failed);
            assert_eq!(check.code, code);
            assert_eq!(check.exit_code, exit::NOTIFICATION);
            assert_ne!(check.remediation, "none");
        }
    }

    #[test]
    fn ssh_fault_classes_keep_distinct_process_exit_codes() {
        let cases = [
            (SshDeliveryError::ExecutableUnavailable, exit::SSH_CLIENT),
            (
                SshDeliveryError::HostKeyVerificationFailed,
                exit::SSH_HOST_KEY,
            ),
            (
                SshDeliveryError::AuthenticationFailed,
                exit::SSH_AUTHENTICATION,
            ),
            (SshDeliveryError::NetworkUnavailable, exit::SSH_NETWORK),
            (SshDeliveryError::ConnectionTimeout, exit::SSH_TIMEOUT),
            (SshDeliveryError::AcknowledgementInvalid, exit::SSH_PROTOCOL),
        ];
        for (error, expected) in cases {
            let check = ssh_delivery_failure(&error);
            assert_eq!(check.exit_code, expected);
            assert_eq!(check.status, CheckStatus::Failed);
        }
    }

    #[test]
    fn human_and_json_doctor_outputs_share_typed_results_and_redact_inputs() {
        let report = DoctorReport::new(vec![
            DiagnosticCheck::ready(
                "configuration",
                "configuration_ready",
                "Configuration is valid.",
            ),
            DiagnosticCheck::failed(
                "storage",
                "storage_corrupt_data",
                "The durable state database cannot be inspected safely.",
                "Stop the agent and repair or restore the state database.",
                exit::STORAGE,
            ),
        ]);
        let human = report.render(OutputFormat::Human);
        let json = report.render(OutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("diagnostic JSON");
        assert_eq!(parsed["status"], "failed");
        assert_eq!(parsed["checks"][1]["code"], "storage_corrupt_data");
        assert!(human.contains("storage.code=storage_corrupt_data"));
        for forbidden in [
            "C:\\Users\\private-user",
            "/Users/private-user",
            "PRIVATE KEY",
            "model response",
            "event body",
        ] {
            assert!(!human.contains(forbidden));
            assert!(!json.contains(forbidden));
        }
    }

    #[test]
    fn status_human_and_json_hide_profile_and_reject_unsafe_version_text() {
        let report = StatusReport {
            role: Role::Relay,
            installed: false,
            version: Some("private-user\nC:\\Users\\private-user".to_owned()),
            startup_registered: false,
            agent: AgentStatus {
                running: true,
                stale: false,
                profile: Some("private-user".to_owned()),
                version: Some("0.1.0".to_owned()),
            },
            queue_pending: Some(0),
            delivery_receipts: Some(1),
            dead_letters: Some(0),
            oldest_queued_age_ms: None,
            latest_delivery_at_ms: Some(1_700_000_000_000),
            storage: StatusStorage::Ready,
            storage_error: None,
            notification: None,
            notification_error: None,
        };
        let (human, human_exit) = render_status(&report, OutputFormat::Human);
        let (json, json_exit) = render_status(&report, OutputFormat::Json);
        assert_eq!(human_exit, exit::OK);
        assert_eq!(json_exit, exit::OK);
        assert!(human.contains("profile_configured=true"));
        assert!(human.contains("version=none"));
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("status JSON");
        assert_eq!(parsed["profile_configured"], true);
        assert_eq!(parsed["version"], "none");
        for forbidden in ["private-user", "C:\\Users", "event body", "PRIVATE KEY"] {
            assert!(!human.contains(forbidden));
            assert!(!json.contains(forbidden));
        }
    }

    #[test]
    fn status_storage_failure_has_stable_code_exit_and_remediation() {
        let report = StatusReport {
            role: Role::Relay,
            installed: false,
            version: None,
            startup_registered: false,
            agent: AgentStatus {
                running: true,
                stale: false,
                profile: None,
                version: None,
            },
            queue_pending: None,
            delivery_receipts: None,
            dead_letters: None,
            oldest_queued_age_ms: None,
            latest_delivery_at_ms: None,
            storage: StatusStorage::Unavailable,
            storage_error: Some("storage_corrupt_data"),
            notification: None,
            notification_error: None,
        };
        let (json, exit_code) = render_status(&report, OutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("status JSON");
        assert_eq!(exit_code, exit::STORAGE);
        assert_eq!(parsed["code"], "storage_corrupt_data");
        assert_ne!(parsed["remediation"], "none");
    }
}
