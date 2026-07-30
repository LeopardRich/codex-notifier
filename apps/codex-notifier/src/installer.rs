//! Desktop installation and relay-hook lifecycle orchestration.

use std::path::{Path, PathBuf};
use std::time::Duration;

use codex_notifier_codex_source::{CapabilityAvailability, CodexCapabilityReport, CodexInterface};
use codex_notifier_config::Role;
use codex_notifier_native_notification::NotificationDiagnostic;
use codex_notifier_persistence::SqliteStore;
use thiserror::Error;
use time::OffsetDateTime;

use crate::database_path;
use crate::desktop::{
    AgentStatus, DesktopError, current_config_paths, load_config_or_defaults,
    load_config_or_defaults_read_only, load_current_config, notification_diagnostic,
    read_agent_status, request_agent_shutdown,
};
use crate::lifecycle::{
    InstallManifest, LifecycleError, LifecycleLayout, ManagedRemovalReport, PlatformOwnership,
    RemovalDisposition, install_managed_documents, read_manifest, stop_hook_group,
    uninstall_managed_documents,
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
    /// Relay hook management requires an explicit relay configuration.
    #[error("Codex hook management requires the relay role")]
    RelayRoleRequired,
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
            Self::RelayRoleRequired => "install_relay_role_required",
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

/// Successful relay hook installation result.
#[derive(Clone, Debug)]
pub struct RelayHookInstallReport {
    /// Whether an owned prior hook was upgraded or repaired.
    pub upgraded: bool,
    /// Fixed Codex hook trust action required from the user.
    pub hook_trust: &'static str,
    /// Fixed approval capability notice for the verified CLI interface.
    pub approval_notice: &'static str,
}

/// Successful relay hook removal result.
#[derive(Clone, Copy, Debug)]
pub struct RelayHookUninstallReport {
    /// Exact owned hook removal decision.
    pub hook: RemovalDisposition,
    /// Existing relay configuration is never removed by hook management.
    pub config_preserved: bool,
}

/// Read-only installed state without payload or machine paths.
#[derive(Clone, Debug)]
pub struct StatusReport {
    /// Configured runtime role.
    pub role: Role,
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
    /// Age of the oldest queued event at inspection time, in milliseconds.
    pub oldest_queued_age_ms: Option<u64>,
    /// Most recent successful delivery time in Unix milliseconds.
    pub latest_delivery_at_ms: Option<i64>,
    /// Read-only storage inspection status.
    pub storage: StatusStorage,
    /// Stable storage failure code when inspection failed.
    pub storage_error: Option<&'static str>,
    /// Current native diagnostic when configuration is valid.
    pub notification: Option<NotificationDiagnostic>,
    /// Stable native diagnostic failure when the desktop check could not run.
    pub notification_error: Option<String>,
}

/// Read-only state-directory and database availability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusStorage {
    /// The state directory and optional database are readable.
    Ready,
    /// The state directory does not exist yet.
    Missing,
    /// Existing state could not be inspected safely.
    Unavailable,
}

impl StatusStorage {
    /// Returns the stable machine-readable status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Unavailable => "unavailable",
        }
    }
}

struct QueueStatus {
    pending: Option<usize>,
    receipts: Option<usize>,
    dead_letters: Option<usize>,
    oldest_queued_age_ms: Option<u64>,
    latest_delivery_at_ms: Option<i64>,
    storage: StatusStorage,
    error: Option<&'static str>,
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
        "desktop",
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

/// Installs or upgrades the verified task-completion hook for a relay agent.
///
/// Existing relay configuration and unrelated Codex hooks are preserved. The
/// ownership manifest is separate from desktop installation state.
///
/// # Errors
///
/// Returns a stable capability, role, path, or structural document failure.
pub fn install_relay_hook(codex_version: &str) -> Result<RelayHookInstallReport, InstallerError> {
    let capability = CodexCapabilityReport::inspect(codex_version, CodexInterface::CliHook);
    if capability.task_completed() != CapabilityAvailability::Supported {
        return Err(InstallerError::UnsupportedCodex);
    }
    let (config_paths, config) = load_current_config()?;
    if config.agent().role() != Role::Relay {
        return Err(InstallerError::RelayRoleRequired);
    }
    let layout = relay_hook_layout(config_paths.config_file(), config.storage().state_dir())?;
    let previous = read_manifest(&layout)?;
    if previous
        .as_ref()
        .is_some_and(|manifest| manifest.platform() != &PlatformOwnership::Relay)
    {
        return Err(LifecycleError::OwnershipConflict.into());
    }
    let (command, windows_command) = hook_commands(
        layout.installed_executable(),
        config.storage().state_dir(),
        config.agent().profile(),
        codex_version,
        "remote",
    )?;
    install_managed_documents(
        &layout,
        env!("CARGO_PKG_VERSION"),
        stop_hook_group(&command, windows_command.as_deref()),
        PlatformOwnership::Relay,
    )?;
    Ok(RelayHookInstallReport {
        upgraded: previous.is_some(),
        hook_trust: "review_required",
        approval_notice: capability.approval_installation_notice(),
    })
}

/// Removes only the exact relay-owned task-completion hook.
///
/// # Errors
///
/// Returns a stable role, ownership, path, or structural document failure.
pub fn uninstall_relay_hook() -> Result<RelayHookUninstallReport, InstallerError> {
    let (config_paths, config) = load_current_config()?;
    if config.agent().role() != Role::Relay {
        return Err(InstallerError::RelayRoleRequired);
    }
    let layout = relay_hook_layout(config_paths.config_file(), config.storage().state_dir())?;
    let Some(manifest) = read_manifest(&layout)? else {
        return Ok(RelayHookUninstallReport {
            hook: RemovalDisposition::Absent,
            config_preserved: true,
        });
    };
    if manifest.platform() != &PlatformOwnership::Relay {
        return Err(LifecycleError::OwnershipConflict.into());
    }
    let managed = uninstall_managed_documents(&layout, &manifest)?;
    Ok(RelayHookUninstallReport {
        hook: managed.hook,
        config_preserved: managed.config != RemovalDisposition::Removed,
    })
}

/// Reads install, startup, agent, queue, and notification status.
///
/// # Errors
///
/// Returns stable path, manifest, configuration, or queue errors.
pub fn status() -> Result<StatusReport, InstallerError> {
    let config_paths = current_config_paths()?;
    let config = load_config_or_defaults_read_only(&config_paths)?;
    let (manifest, startup_registered) = match config.agent().role() {
        Role::Desktop => {
            let platform_paths =
                current_platform_paths(&config_paths, config.storage().state_dir())?;
            (
                read_manifest(platform_paths.lifecycle())?,
                startup_resource_exists(&platform_paths),
            )
        }
        Role::Relay => {
            let layout =
                relay_hook_layout(config_paths.config_file(), config.storage().state_dir())?;
            (read_manifest(&layout)?, false)
        }
    };
    let queue = queue_status(config.storage().state_dir());
    let (notification, notification_error) = if config.agent().role() == Role::Desktop {
        match notification_diagnostic(&config) {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error.code().to_owned())),
        }
    } else {
        (None, None)
    };
    Ok(StatusReport {
        role: config.agent().role(),
        installed: manifest.is_some(),
        version: manifest.map(|value| value.install_version().to_owned()),
        startup_registered,
        agent: read_agent_status(config.storage().state_dir()),
        queue_pending: queue.pending,
        delivery_receipts: queue.receipts,
        dead_letters: queue.dead_letters,
        oldest_queued_age_ms: queue.oldest_queued_age_ms,
        latest_delivery_at_ms: queue.latest_delivery_at_ms,
        storage: queue.storage,
        storage_error: queue.error,
        notification,
        notification_error,
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
    host_label: &str,
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
        host_label,
    ];
    #[cfg(windows)]
    {
        let invocation = arguments
            .iter()
            .map(|value| powershell_quote(value))
            .collect::<Result<Vec<_>, _>>()?
            .join(" ");
        // Codex 0.144.5 cannot launch a command whose first Windows token is a
        // quoted path. A fixed system executable keeps the first token
        // unquoted while PowerShell safely invokes the installed path.
        let command = format!(
            "powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command \
             \"$OutputEncoding=[Text.UTF8Encoding]::new($false); \
             [Console]::In.ReadToEnd() | & {invocation}; exit $LASTEXITCODE\""
        );
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

fn relay_hook_layout(
    config_file: &Path,
    state_dir: &Path,
) -> Result<LifecycleLayout, InstallerError> {
    let executable = std::env::current_exe().map_err(|_| InstallerError::UnsafeHookCommand)?;
    let install_root = executable
        .parent()
        .map(Path::to_owned)
        .ok_or(InstallerError::UnsafeHookCommand)?;
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    let home = home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(LifecycleError::InvalidLayout)?;
    LifecycleLayout::new(
        install_root,
        executable,
        config_file,
        home.join(".codex").join("hooks.json"),
        state_dir.join("relay-hook-manifest.json"),
    )
    .map_err(InstallerError::from)
}

#[cfg(windows)]
fn powershell_quote(value: &str) -> Result<String, InstallerError> {
    if value.is_empty()
        || value.chars().any(char::is_control)
        || value
            .bytes()
            .any(|byte| matches!(byte, b'"' | b'%' | b'!' | b'^' | b'&' | b'|' | b'<' | b'>'))
    {
        return Err(InstallerError::UnsafeHookCommand);
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

#[cfg(not(windows))]
fn unix_quote(value: &str) -> Result<String, InstallerError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(InstallerError::UnsafeHookCommand);
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

fn queue_status(state_dir: &Path) -> QueueStatus {
    let database = database_path(state_dir);
    let state_exists = match state_dir.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return unavailable_queue("storage_unwritable");
        }
        Ok(metadata) if metadata.permissions().readonly() => {
            return unavailable_queue("storage_unwritable");
        }
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return unavailable_queue("storage_unwritable"),
    };
    match database.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return QueueStatus {
                pending: Some(0),
                receipts: Some(0),
                dead_letters: Some(0),
                oldest_queued_age_ms: None,
                latest_delivery_at_ms: None,
                storage: if state_exists {
                    StatusStorage::Ready
                } else {
                    StatusStorage::Missing
                },
                error: None,
            };
        }
        Err(_) => return unavailable_queue("storage_unwritable"),
        Ok(_) => {}
    }
    if !state_exists {
        return unavailable_queue("storage_unwritable");
    }
    match SqliteStore::inspect_read_only(&database) {
        Ok(snapshot) => {
            let now_ms = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
            QueueStatus {
                pending: Some(snapshot.queue_entries()),
                receipts: Some(snapshot.receipt_entries()),
                dead_letters: Some(snapshot.dead_letter_entries()),
                oldest_queued_age_ms: snapshot.oldest_enqueued_at_ms().map(|timestamp| {
                    u64::try_from(now_ms.saturating_sub(i128::from(timestamp))).unwrap_or(u64::MAX)
                }),
                latest_delivery_at_ms: snapshot.latest_delivered_at_ms(),
                storage: StatusStorage::Ready,
                error: None,
            }
        }
        Err(error) => unavailable_queue(error.code().as_str()),
    }
}

fn unavailable_queue(error: &'static str) -> QueueStatus {
    QueueStatus {
        pending: None,
        receipts: None,
        dead_letters: None,
        oldest_queued_age_ms: None,
        latest_delivery_at_ms: None,
        storage: StatusStorage::Unavailable,
        error: Some(error),
    }
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
            "desktop",
        )
        .expect("safe command");
        assert!(command.contains("emit"));
        assert!(command.contains("task-completed"));
        assert!(!command.contains("$()"));
        #[cfg(not(windows))]
        assert!(windows.is_none());
        #[cfg(windows)]
        {
            assert!(command.starts_with("powershell.exe "));
            assert!(command.contains("[Console]::In.ReadToEnd()"));
            assert_eq!(windows.as_deref(), Some(command.as_str()));
        }
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

    #[test]
    fn status_rejects_an_unsafe_database_type_without_modifying_it() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let state_dir = directory.path().join("state");
        std::fs::create_dir_all(database_path(&state_dir)).expect("database-shaped directory");

        let report = queue_status(&state_dir);

        assert_eq!(report.storage, StatusStorage::Unavailable);
        assert_eq!(report.error, Some("storage_unwritable"));
        assert!(database_path(&state_dir).is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn windows_hook_rejects_expansion_and_command_metacharacters() {
        for value in ["C:\\Users\\%NAME%", "C:\\bad&path", "C:\\bad!path"] {
            assert!(matches!(
                powershell_quote(value),
                Err(InstallerError::UnsafeHookCommand)
            ));
        }
        assert_eq!(
            powershell_quote("C:\\Users\\O'Brien\\notifier.exe").expect("quoted path"),
            "'C:\\Users\\O''Brien\\notifier.exe'"
        );
    }
}
