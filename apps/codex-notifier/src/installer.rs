//! Stage 14 desktop install, upgrade, status, and uninstall orchestration.

use std::path::Path;
use std::time::Duration;

use codex_notifier_codex_source::{CapabilityAvailability, CodexCapabilityReport, CodexInterface};
use codex_notifier_native_notification::NotificationDiagnostic;
use codex_notifier_persistence::{SqliteStore, StorePolicy};
use thiserror::Error;

use crate::database_path;
use crate::desktop::{
    AgentStatus, DesktopError, current_config_paths, load_config_or_defaults,
    notification_diagnostic, read_agent_status, request_agent_shutdown,
};
use crate::lifecycle::{
    InstallManifest, LifecycleError, ManagedRemovalReport, install_managed_documents,
    read_manifest, stop_hook_group, uninstall_managed_documents,
};
use crate::platform::{
    PlatformError, begin_platform_install, current_platform_paths, platform_ownership,
    startup_resource_exists, uninstall_platform,
};

const AGENT_START_TIMEOUT: Duration = Duration::from_secs(10);
const AGENT_STOP_MARGIN: Duration = Duration::from_secs(2);

/// Stable install/uninstall command failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InstallerError {
    /// The requested Codex release lacks the verified task hook.
    #[error("Codex task hook is unsupported")]
    UnsupportedCodex,
    /// The command string cannot safely represent the installed paths.
    #[error("Codex hook command path is unsafe")]
    UnsafeHookCommand,
    /// No owned installation exists.
    #[error("codex-notifier is not installed")]
    NotInstalled,
    /// Configuration or local agent state failed.
    #[error("desktop lifecycle operation failed")]
    Desktop(#[from] DesktopError),
    /// Ownership metadata or structural hook editing failed.
    #[error("installation ownership operation failed")]
    Lifecycle(#[from] LifecycleError),
    /// Native platform resources failed.
    #[error("platform installation operation failed")]
    Platform(#[from] PlatformError),
    /// The installed agent did not reach its ready process state.
    #[error("installed desktop agent did not start")]
    AgentStart,
    /// Queue status could not be read.
    #[error("installed queue status is unavailable")]
    QueueStatus,
}

impl InstallerError {
    /// Returns the stable machine-readable classification.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::UnsupportedCodex => "install_codex_unsupported",
            Self::UnsafeHookCommand => "install_hook_command_unsafe",
            Self::NotInstalled => "install_not_found",
            Self::Desktop(error) => error.code(),
            Self::Lifecycle(error) => error.code(),
            Self::Platform(error) => error.code(),
            Self::AgentStart => "install_agent_start_failed",
            Self::QueueStatus => "status_queue_unavailable",
        }
    }
}

/// Successful install/upgrade result with no machine paths.
#[derive(Clone, Debug)]
pub struct InstallReport {
    /// Whether an owned prior install was upgraded or repaired.
    pub upgraded: bool,
    /// Whether the installed agent reached its running state.
    pub agent_running: bool,
    /// Current native permission and Focus classification.
    pub notification: NotificationDiagnostic,
    /// Fixed Codex hook trust action required from the user.
    pub hook_trust: &'static str,
    /// Fixed approval capability notice for the verified CLI interface.
    pub approval_notice: &'static str,
}

/// Successful uninstall result.
#[derive(Clone, Debug)]
pub struct UninstallReport {
    /// Exact document preservation/removal decisions.
    pub managed: ManagedRemovalReport,
    /// Persistent event state is intentionally retained.
    pub state_preserved: bool,
}

/// Read-only installed state without payload or machine paths.
#[derive(Clone, Debug)]
pub struct StatusReport {
    /// Whether a valid ownership manifest exists.
    pub installed: bool,
    /// Recorded installed version, when present.
    pub version: Option<String>,
    /// Whether the exact startup resource exists.
    pub startup_registered: bool,
    /// Current bounded agent process record.
    pub agent: AgentStatus,
    /// Pending event count, if an existing database could be opened.
    pub queue_pending: Option<usize>,
    /// Delivery receipt count, if an existing database could be opened.
    pub delivery_receipts: Option<usize>,
    /// Metadata-only dead-letter count, if an existing database could be opened.
    pub dead_letters: Option<usize>,
    /// Current native diagnostic when configuration is valid.
    pub notification: Option<NotificationDiagnostic>,
}

struct QueueStatus {
    pending: Option<usize>,
    receipts: Option<usize>,
    dead_letters: Option<usize>,
}

/// Installs or upgrades the current executable for a local desktop user.
///
/// # Errors
///
/// Returns a stable capability, configuration, ownership, platform, or agent
/// startup failure. Platform state rolls back if document installation fails.
pub async fn install(codex_version: &str) -> Result<InstallReport, InstallerError> {
    let capability = CodexCapabilityReport::inspect(codex_version, CodexInterface::CliHook);
    if capability.task_completed() != CapabilityAvailability::Supported {
        return Err(InstallerError::UnsupportedCodex);
    }
    let config_paths = current_config_paths()?;
    let config = load_config_or_defaults(&config_paths)?;
    let platform_paths = current_platform_paths(&config_paths, config.storage().state_dir())?;
    let previous = read_manifest(platform_paths.lifecycle())?;
    if previous.is_some() {
        let stop_timeout = Duration::from_millis(config.agent().shutdown_timeout_ms())
            .saturating_add(AGENT_STOP_MARGIN);
        request_agent_shutdown(config.storage().state_dir(), stop_timeout).await?;
    }

    let (command, windows_command) = hook_commands(
        platform_paths.lifecycle().installed_executable(),
        config.storage().state_dir(),
        config.agent().profile(),
        codex_version,
    )?;
    let hook_group = stop_hook_group(&command, windows_command.as_deref());
    let new_manifest = install_managed_documents(
        platform_paths.lifecycle(),
        env!("CARGO_PKG_VERSION"),
        hook_group,
        platform_ownership(&platform_paths),
    )?;

    let transaction = match begin_platform_install(&platform_paths, previous.as_ref()) {
        Ok(transaction) => transaction,
        Err(error) => {
            restore_documents(platform_paths.lifecycle(), &new_manifest, previous.as_ref())?;
            return Err(error.into());
        }
    };
    if !wait_for_agent(config.storage().state_dir(), AGENT_START_TIMEOUT).await {
        transaction.rollback()?;
        restore_documents(platform_paths.lifecycle(), &new_manifest, previous.as_ref())?;
        return Err(InstallerError::AgentStart);
    }
    commit_or_restore(
        || transaction.commit(),
        || restore_documents(platform_paths.lifecycle(), &new_manifest, previous.as_ref()),
    )?;

    request_macos_authorization_if_needed();
    let notification = notification_diagnostic(&config)?;
    Ok(InstallReport {
        upgraded: previous.is_some(),
        agent_running: read_agent_status(config.storage().state_dir()).running,
        notification,
        hook_trust: "review_required",
        approval_notice: capability.approval_installation_notice(),
    })
}

/// Removes exact installed resources while preserving queue state and edits.
///
/// # Errors
///
/// Returns a stable not-installed, shutdown, ownership, platform, or filesystem
/// failure. Managed documents are restored if platform removal fails.
pub async fn uninstall() -> Result<UninstallReport, InstallerError> {
    let config_paths = current_config_paths()?;
    let config = load_config_or_defaults(&config_paths)?;
    let platform_paths = current_platform_paths(&config_paths, config.storage().state_dir())?;
    let manifest =
        read_manifest(platform_paths.lifecycle())?.ok_or(InstallerError::NotInstalled)?;
    let timeout = Duration::from_millis(config.agent().shutdown_timeout_ms())
        .saturating_add(AGENT_STOP_MARGIN);
    request_agent_shutdown(config.storage().state_dir(), timeout).await?;
    let managed = uninstall_managed_documents(platform_paths.lifecycle(), &manifest)?;
    if let Err(error) = uninstall_platform(&platform_paths, &manifest) {
        install_managed_documents(
            platform_paths.lifecycle(),
            manifest.install_version(),
            manifest.hook_group().clone(),
            manifest.platform().clone(),
        )?;
        return Err(error.into());
    }
    Ok(UninstallReport {
        managed,
        state_preserved: true,
    })
}

/// Reads install, startup, agent, queue, and notification status.
///
/// # Errors
///
/// Returns stable path, manifest, configuration, or queue errors.
pub fn status() -> Result<StatusReport, InstallerError> {
    let config_paths = current_config_paths()?;
    let config = load_config_or_defaults(&config_paths)?;
    let platform_paths = current_platform_paths(&config_paths, config.storage().state_dir())?;
    let manifest = read_manifest(platform_paths.lifecycle())?;
    let queue = queue_status(
        config.storage().state_dir(),
        config.storage().max_queue_entries(),
    )?;
    Ok(StatusReport {
        installed: manifest.is_some(),
        version: manifest.map(|value| value.install_version().to_owned()),
        startup_registered: startup_resource_exists(&platform_paths),
        agent: read_agent_status(config.storage().state_dir()),
        queue_pending: queue.pending,
        delivery_receipts: queue.receipts,
        dead_letters: queue.dead_letters,
        notification: notification_diagnostic(&config).ok(),
    })
}

fn restore_documents(
    layout: &crate::lifecycle::LifecycleLayout,
    current: &InstallManifest,
    previous: Option<&InstallManifest>,
) -> Result<(), InstallerError> {
    uninstall_managed_documents(layout, current)?;
    if let Some(previous) = previous {
        install_managed_documents(
            layout,
            previous.install_version(),
            previous.hook_group().clone(),
            previous.platform().clone(),
        )?;
    }
    Ok(())
}

fn commit_or_restore<C, R>(commit: C, restore: R) -> Result<(), InstallerError>
where
    C: FnOnce() -> Result<(), PlatformError>,
    R: FnOnce() -> Result<(), InstallerError>,
{
    if let Err(error) = commit() {
        restore()?;
        return Err(error.into());
    }
    Ok(())
}

async fn wait_for_agent(state_dir: &Path, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if read_agent_status(state_dir).running {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn hook_commands(
    executable: &Path,
    state_dir: &Path,
    profile: &str,
    codex_version: &str,
) -> Result<(String, Option<String>), InstallerError> {
    let executable = executable
        .to_str()
        .ok_or(InstallerError::UnsafeHookCommand)?;
    let state_dir = state_dir
        .to_str()
        .ok_or(InstallerError::UnsafeHookCommand)?;
    let arguments = [
        executable,
        "emit",
        "task-completed",
        "--codex-version",
        codex_version,
        "--state-dir",
        state_dir,
        "--ipc-profile",
        profile,
        "--host-label",
        "desktop",
    ];
    #[cfg(windows)]
    {
        let command = arguments
            .iter()
            .map(|value| windows_quote(value))
            .collect::<Result<Vec<_>, _>>()?
            .join(" ");
        Ok((command.clone(), Some(command)))
    }
    #[cfg(not(windows))]
    {
        let command = arguments
            .iter()
            .map(|value| unix_quote(value))
            .collect::<Result<Vec<_>, _>>()?
            .join(" ");
        Ok((command, None))
    }
}

#[cfg(windows)]
fn windows_quote(value: &str) -> Result<String, InstallerError> {
    if value.is_empty()
        || value.chars().any(char::is_control)
        || value
            .bytes()
            .any(|byte| matches!(byte, b'"' | b'%' | b'!' | b'^' | b'&' | b'|' | b'<' | b'>'))
    {
        return Err(InstallerError::UnsafeHookCommand);
    }
    Ok(format!("\"{value}\""))
}

#[cfg(not(windows))]
fn unix_quote(value: &str) -> Result<String, InstallerError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(InstallerError::UnsafeHookCommand);
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

fn queue_status(state_dir: &Path, limit: usize) -> Result<QueueStatus, InstallerError> {
    let database = database_path(state_dir);
    if !database.exists() {
        return Ok(QueueStatus {
            pending: Some(0),
            receipts: Some(0),
            dead_letters: Some(0),
        });
    }
    let policy = StorePolicy::default()
        .with_queue_limit(limit)
        .map_err(|_| InstallerError::QueueStatus)?;
    let store = SqliteStore::open(&database, policy).map_err(|_| InstallerError::QueueStatus)?;
    Ok(QueueStatus {
        pending: Some(store.queue_len().map_err(|_| InstallerError::QueueStatus)?),
        receipts: Some(
            store
                .receipt_count()
                .map_err(|_| InstallerError::QueueStatus)?,
        ),
        dead_letters: Some(
            store
                .dead_letter_count()
                .map_err(|_| InstallerError::QueueStatus)?,
        ),
    })
}

#[cfg(target_os = "macos")]
fn request_macos_authorization_if_needed() {
    use codex_notifier_native_notification::{
        MacOsNotificationBackend, NotificationBackend, NotificationStatus,
    };

    let backend = MacOsNotificationBackend::codex_notifier();
    if backend.diagnose().status() == NotificationStatus::AuthorizationNotDetermined {
        let _ = backend.request_authorization();
    }
}

#[cfg(not(target_os = "macos"))]
const fn request_macos_authorization_if_needed() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn hook_command_never_contains_unquoted_event_data() {
        let (command, windows) = hook_commands(
            Path::new("/Applications/Codex Notifier.app/Contents/MacOS/codex-notifier"),
            Path::new("/Users/test/Library/Application Support/codex-notifier/state"),
            "default",
            "0.144.5",
        )
        .expect("safe command");
        assert!(command.contains("emit"));
        assert!(command.contains("task-completed"));
        assert!(!command.contains("$()"));
        #[cfg(not(windows))]
        assert!(windows.is_none());
        #[cfg(windows)]
        assert!(windows.is_some());
    }

    #[test]
    fn failed_platform_commit_restores_managed_documents() {
        let restored = AtomicBool::new(false);
        let result = commit_or_restore(
            || Err(PlatformError::FileSystem),
            || {
                restored.store(true, Ordering::SeqCst);
                Ok(())
            },
        );

        assert!(matches!(
            result,
            Err(InstallerError::Platform(PlatformError::FileSystem))
        ));
        assert!(restored.load(Ordering::SeqCst));
    }

    #[cfg(windows)]
    #[test]
    fn windows_hook_rejects_expansion_and_command_metacharacters() {
        for value in ["C:\\Users\\%NAME%", "C:\\bad&path", "C:\\bad!path"] {
            assert!(matches!(
                windows_quote(value),
                Err(InstallerError::UnsafeHookCommand)
            ));
        }
    }
}
