//! Desktop configuration, native delivery composition, and local commands.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use codex_notifier_application::{AgentError, EventDelivery, RoleDeliveryFactory};
use codex_notifier_config::{
    CliOverrides, Config, ConfigError, ConfigLoader, ConfigPaths, FileSystemStateProbe,
    NotificationPrivacy, PathEnvironment, Platform,
};
use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use codex_notifier_ipc::{AckStatus, IpcClient, IpcEndpoint, IpcError, IpcPolicy};
use codex_notifier_native_notification::{
    NativeNotificationAdapter, NotificationContentPolicy, NotificationDiagnostic,
    NotificationPolicy,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

use crate::{AgentHost, HostError};

const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const STATUS_FILE: &str = "agent-status.json";
const SHUTDOWN_FILE: &str = "agent-shutdown-request";

/// Safe desktop command failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DesktopError {
    /// Required user path environment is missing or invalid.
    #[error("desktop path environment is invalid")]
    Config(#[from] ConfigError),
    /// The configuration file is missing, oversized, or unreadable.
    #[error("desktop configuration file is unavailable")]
    ConfigFile,
    /// The desktop notification adapter is unavailable on this platform.
    #[error("desktop notifications are unsupported on this platform")]
    UnsupportedPlatform,
    /// Agent composition or execution failed.
    #[error("desktop agent failed")]
    Host(#[from] HostError),
    /// A local test could not reach or submit to the agent.
    #[error("desktop test submission failed")]
    Ipc(#[from] IpcError),
    /// A synthetic test event could not be constructed.
    #[error("desktop test event is invalid")]
    TestEvent,
    /// Agent status could not be written safely.
    #[error("desktop agent status is unavailable")]
    Status,
}

impl DesktopError {
    /// Returns the stable machine-readable classification.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Config(error) => error.code().as_str(),
            Self::ConfigFile => "desktop_config_unavailable",
            Self::UnsupportedPlatform => "notification_platform_unsupported",
            Self::Host(error) => host_error_code(error),
            Self::Ipc(error) => error.code().as_str(),
            Self::TestEvent => "desktop_test_event_invalid",
            Self::Status => "agent_status_unavailable",
        }
    }
}

/// Read-only local agent state from its bounded status record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentStatus {
    /// Whether a status record exists and its process is still present.
    pub running: bool,
    /// Whether a status record exists for a process that no longer exists.
    pub stale: bool,
    /// Safe configured profile, if a valid status record exists.
    pub profile: Option<String>,
    /// Installed binary version reported by the agent.
    pub version: Option<String>,
}

/// Resolves current-user configuration paths without consulting proxy state.
///
/// # Errors
///
/// Returns a stable configuration path error when required environment bases
/// are missing or not absolute.
pub fn current_config_paths() -> Result<ConfigPaths, DesktopError> {
    let environment = PathEnvironment::new();
    #[cfg(windows)]
    let (environment, platform) = (
        environment
            .with_home(required_env("USERPROFILE")?)
            .with_windows_app_data(required_env("APPDATA")?)
            .with_windows_local_app_data(required_env("LOCALAPPDATA")?),
        Platform::Windows,
    );
    #[cfg(target_os = "macos")]
    let (environment, platform) = (
        environment.with_home(required_env("HOME")?),
        Platform::MacOs,
    );
    #[cfg(all(unix, not(target_os = "macos")))]
    let (environment, platform) = {
        let mut value = environment.with_home(required_env("HOME")?);
        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
            value = value.with_xdg_config_home(path);
        }
        if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
            value = value.with_xdg_state_home(path);
        }
        (value, Platform::Xdg)
    };
    environment.resolve(platform).map_err(DesktopError::from)
}

/// Loads the validated current-user configuration.
///
/// # Errors
///
/// Returns a stable file or configuration error without logging file content.
pub fn load_current_config() -> Result<(ConfigPaths, Config), DesktopError> {
    let paths = current_config_paths()?;
    let config = load_config(&paths, true)?;
    Ok((paths, config))
}

/// Loads existing configuration or validated platform defaults.
///
/// # Errors
///
/// Returns a stable file or configuration error.
pub fn load_config_or_defaults(paths: &ConfigPaths) -> Result<Config, DesktopError> {
    load_config(paths, false)
}

/// Runs the configured desktop agent until an operating-system stop signal.
///
/// # Errors
///
/// Returns a stable configuration, native adapter, IPC, persistence, or
/// lifecycle failure.
pub async fn run_agent() -> Result<(), DesktopError> {
    let (_, config) = load_current_config()?;
    let shutdown_path = config.storage().state_dir().join(SHUTDOWN_FILE);
    let _ = fs::remove_file(&shutdown_path);
    let factory = NativeRoleFactory::new(&config);
    let host = AgentHost::from_config(&config, &factory)?;
    let _status = AgentStatusGuard::create(&config)?;
    host.run_until(shutdown_signal(shutdown_path)).await?;
    Ok(())
}

/// Submits one synthetic canonical event through the installed local agent.
///
/// # Errors
///
/// Returns a stable configuration, event, or IPC failure.
pub async fn submit_local_test(kind: EventKind) -> Result<(EventId, AckStatus), DesktopError> {
    let (_, config) = load_current_config()?;
    let endpoint = endpoint_for(&config)?;
    let event = synthetic_event(kind)?;
    let event_id = event.event_id();
    let acknowledgement = IpcClient::new(endpoint, IpcPolicy::default())
        .submit(&event)
        .await?;
    Ok((event_id, acknowledgement.status()))
}

/// Returns the current native notification diagnostic.
///
/// # Errors
///
/// Returns unsupported platform when no desktop backend exists.
pub fn notification_diagnostic(config: &Config) -> Result<NotificationDiagnostic, DesktopError> {
    #[cfg(windows)]
    {
        const DIAGNOSTIC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

        let policy = notification_policy(config);
        match run_bounded(DIAGNOSTIC_TIMEOUT, move || {
            native_adapter_for_policy(policy).map(|adapter| adapter.diagnose())
        }) {
            Some(result) => result,
            None => Ok(NotificationDiagnostic::new(
                codex_notifier_native_notification::NotificationStatus::Unavailable,
                codex_notifier_native_notification::FocusStatus::Unknown,
            )),
        }
    }
    #[cfg(not(windows))]
    {
        Ok(native_adapter(config)?.diagnose())
    }
}

/// Reads the bounded agent status record and validates its process ID.
#[must_use]
pub fn read_agent_status(state_dir: &Path) -> AgentStatus {
    let path = state_dir.join(STATUS_FILE);
    let Ok(bytes) = fs::read(path) else {
        return AgentStatus {
            running: false,
            stale: false,
            profile: None,
            version: None,
        };
    };
    if bytes.len() > 4 * 1024 {
        return AgentStatus {
            running: false,
            stale: true,
            profile: None,
            version: None,
        };
    }
    let Ok(record) = serde_json::from_slice::<AgentStatusRecord>(&bytes) else {
        return AgentStatus {
            running: false,
            stale: true,
            profile: None,
            version: None,
        };
    };
    let running = process_exists(record.pid);
    AgentStatus {
        running,
        stale: !running,
        profile: Some(record.profile),
        version: Some(record.version),
    }
}

/// Requests cooperative agent shutdown and waits for its bounded status record.
///
/// # Errors
///
/// Returns a status error when the request cannot be written or the agent does
/// not stop within the configured deadline.
pub async fn request_agent_shutdown(
    state_dir: &Path,
    timeout: std::time::Duration,
) -> Result<(), DesktopError> {
    if !read_agent_status(state_dir).running {
        let _ = fs::remove_file(state_dir.join(SHUTDOWN_FILE));
        return Ok(());
    }
    fs::write(state_dir.join(SHUTDOWN_FILE), b"stop\n").map_err(|_| DesktopError::Status)?;
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if !read_agent_status(state_dir).running {
            let _ = fs::remove_file(state_dir.join(SHUTDOWN_FILE));
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err(DesktopError::Status)
}

fn endpoint_for(config: &Config) -> Result<IpcEndpoint, DesktopError> {
    let endpoint_label = config
        .agent()
        .ipc_endpoint()
        .name()
        .unwrap_or_else(|| config.agent().profile());
    IpcEndpoint::new(config.storage().state_dir().join("run"), endpoint_label)
        .map_err(DesktopError::from)
}

fn synthetic_event(kind: EventKind) -> Result<CanonicalEvent, DesktopError> {
    let now = OffsetDateTime::now_utc();
    let (title, body, urgency) = match kind {
        EventKind::TaskCompleted => (
            "Codex task finished",
            "Open Codex to review the result.",
            Urgency::Normal,
        ),
        EventKind::ApprovalRequested => (
            "Codex needs approval",
            "Open Codex to review the request.",
            Urgency::High,
        ),
    };
    CanonicalEvent::new(
        EventId::new_v7(),
        kind,
        now,
        EventSource::new("local-test", None, None).map_err(|_| DesktopError::TestEvent)?,
        Presentation::new(title, body, urgency, Privacy::Private)
            .map_err(|_| DesktopError::TestEvent)?,
        None,
        Extensions::new(std::collections::BTreeMap::new()).map_err(|_| DesktopError::TestEvent)?,
        now,
    )
    .map_err(|_| DesktopError::TestEvent)
}

struct NativeRoleFactory {
    policy: NotificationPolicy,
}

impl NativeRoleFactory {
    fn new(config: &Config) -> Self {
        Self {
            policy: notification_policy(config),
        }
    }
}

impl RoleDeliveryFactory for NativeRoleFactory {
    fn desktop(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        native_adapter_for_policy(self.policy)
            .map(|adapter| Arc::new(adapter) as Arc<dyn EventDelivery>)
            .map_err(|_| AgentError::DeliveryInitialization)
    }

    fn relay(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        Err(AgentError::DeliveryInitialization)
    }
}

#[cfg(not(windows))]
fn native_adapter(config: &Config) -> Result<NativeNotificationAdapter, DesktopError> {
    native_adapter_for_policy(notification_policy(config))
}

fn native_adapter_for_policy(
    policy: NotificationPolicy,
) -> Result<NativeNotificationAdapter, DesktopError> {
    #[cfg(any(windows, target_os = "macos"))]
    #[allow(clippy::unnecessary_wraps)]
    fn supported(
        adapter: NativeNotificationAdapter,
    ) -> Result<NativeNotificationAdapter, DesktopError> {
        Ok(adapter)
    }
    #[cfg(windows)]
    {
        use codex_notifier_native_notification::WindowsNotificationBackend;
        supported(NativeNotificationAdapter::new(
            Arc::new(WindowsNotificationBackend::codex_notifier()),
            policy,
        ))
    }
    #[cfg(target_os = "macos")]
    {
        use codex_notifier_native_notification::MacOsNotificationBackend;
        supported(NativeNotificationAdapter::new(
            Arc::new(MacOsNotificationBackend::codex_notifier()),
            policy,
        ))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = policy;
        Err(DesktopError::UnsupportedPlatform)
    }
}

fn notification_policy(config: &Config) -> NotificationPolicy {
    let content = match config.desktop().privacy() {
        NotificationPrivacy::Private => NotificationContentPolicy::Private,
        NotificationPrivacy::Public => NotificationContentPolicy::Public,
    };
    NotificationPolicy::new(content, config.desktop().quiet_hours())
}

#[derive(Serialize, Deserialize)]
struct AgentStatusRecord {
    schema_version: u16,
    pid: u32,
    profile: String,
    version: String,
    started_at_ms: i64,
}

struct AgentStatusGuard {
    path: PathBuf,
}

impl AgentStatusGuard {
    fn create(config: &Config) -> Result<Self, DesktopError> {
        let path = config.storage().state_dir().join(STATUS_FILE);
        let timestamp = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
        let record = AgentStatusRecord {
            schema_version: 1,
            pid: std::process::id(),
            profile: config.agent().profile().to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            started_at_ms: i64::try_from(timestamp).map_err(|_| DesktopError::Status)?,
        };
        let bytes = serde_json::to_vec(&record).map_err(|_| DesktopError::Status)?;
        fs::create_dir_all(config.storage().state_dir()).map_err(|_| DesktopError::Status)?;
        fs::write(&path, bytes).map_err(|_| DesktopError::Status)?;
        Ok(Self { path })
    }
}

impl Drop for AgentStatusGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn process_exists(pid: u32) -> bool {
    let system = sysinfo::System::new_all();
    system.process(sysinfo::Pid::from_u32(pid)).is_some()
}

#[cfg(any(windows, test))]
fn run_bounded<T, F>(timeout: std::time::Duration, operation: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(operation());
    });
    receiver.recv_timeout(timeout).ok()
}

fn read_bounded(path: &Path) -> Result<String, DesktopError> {
    let metadata = fs::metadata(path).map_err(|_| DesktopError::ConfigFile)?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        return Err(DesktopError::ConfigFile);
    }
    fs::read_to_string(path).map_err(|_| DesktopError::ConfigFile)
}

fn load_config(paths: &ConfigPaths, required: bool) -> Result<Config, DesktopError> {
    let input = match read_bounded(paths.config_file()) {
        Ok(value) => Some(value),
        Err(DesktopError::ConfigFile) if !required && !paths.config_file().exists() => None,
        Err(error) => return Err(error),
    };
    ConfigLoader::load(
        paths,
        input.as_deref(),
        None,
        CliOverrides::new(),
        &FileSystemStateProbe,
    )
    .map_err(DesktopError::from)
}

fn required_env(name: &str) -> Result<PathBuf, DesktopError> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or(ConfigError::MissingPathBase)
        .map_err(DesktopError::from)
}

async fn shutdown_signal(shutdown_path: PathBuf) {
    let requested = async {
        loop {
            if shutdown_path.exists() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let interrupt = tokio::signal::ctrl_c();
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = interrupt => {}
                _ = terminate.recv() => {}
                () = requested => {}
            }
        } else {
            tokio::select! {
                _ = interrupt => {}
                () = requested => {}
            }
        }
    }
    #[cfg(windows)]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = requested => {}
        }
    }
}

fn host_error_code(error: &HostError) -> &str {
    match error {
        HostError::Ipc(error) => error.code().as_str(),
        HostError::Persistence(error) => error.code().as_str(),
        HostError::Agent(AgentError::DeliveryInitialization) => {
            "agent_delivery_initialization_failed"
        }
        HostError::Agent(AgentError::InvalidPolicy) => "agent_policy_invalid",
        HostError::Agent(AgentError::NotReady) => "agent_not_ready",
        HostError::Agent(AgentError::Draining) => "agent_draining",
        HostError::Agent(AgentError::Queue(error)) => error.code(),
        HostError::Agent(_) => "agent_runtime_failed",
        HostError::StateDirectory => "agent_state_directory_unavailable",
        HostError::Acknowledgement => "agent_acknowledgement_invalid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn status_record_distinguishes_live_stale_and_malformed_processes() {
        let directory = TempDir::new().expect("temporary directory");
        let path = directory.path().join(STATUS_FILE);
        let live = AgentStatusRecord {
            schema_version: 1,
            pid: std::process::id(),
            profile: "default".to_owned(),
            version: "0.1.0".to_owned(),
            started_at_ms: 1,
        };
        fs::write(&path, serde_json::to_vec(&live).expect("status JSON")).expect("live status");
        let status = read_agent_status(directory.path());
        assert!(status.running);
        assert!(!status.stale);

        fs::write(&path, b"not-json").expect("malformed status");
        let status = read_agent_status(directory.path());
        assert!(!status.running);
        assert!(status.stale);
    }

    #[test]
    fn bounded_operation_returns_results_and_times_out() {
        let ready = run_bounded(std::time::Duration::from_secs(1), || 7);
        assert_eq!(ready, Some(7));

        let timed_out = run_bounded(std::time::Duration::from_millis(1), || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            9
        });
        assert_eq!(timed_out, None);
    }
}
