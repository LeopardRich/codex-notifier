//! Validated effective configuration.

use std::fmt;
use std::path::{Path, PathBuf};

/// Agent runtime role selected explicitly by configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// Deliver native notifications on a Windows or macOS workstation.
    Desktop,
    /// Persist and forward events to a configured desktop over SSH.
    Relay,
}

/// A bounded logical local IPC endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IpcEndpoint {
    /// Use the platform-derived endpoint for the selected profile.
    Default,
    /// Use a validated logical endpoint name.
    Named(String),
}

impl IpcEndpoint {
    /// Returns the optional validated logical name.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Default => None,
            Self::Named(value) => Some(value),
        }
    }
}

/// Notification content privacy setting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationPrivacy {
    /// Always use generic private title and body text.
    Private,
    /// Allow bounded canonical title and body text.
    Public,
}

/// Structured logging level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    /// Error records only.
    Error,
    /// Warnings and errors.
    Warn,
    /// Normal operational status.
    Info,
    /// Detailed diagnostics with mandatory redaction.
    Debug,
    /// Maximum diagnostics with mandatory redaction.
    Trace,
}

/// Agent-owned settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentConfig {
    pub(crate) role: Role,
    pub(crate) profile: String,
    pub(crate) ipc_endpoint: IpcEndpoint,
    pub(crate) shutdown_timeout_ms: u64,
}

impl AgentConfig {
    /// Returns the explicit runtime role.
    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    /// Returns the active configuration profile.
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Returns the logical IPC endpoint.
    #[must_use]
    pub const fn ipc_endpoint(&self) -> &IpcEndpoint {
        &self.ipc_endpoint
    }

    /// Returns the graceful-shutdown timeout in milliseconds.
    #[must_use]
    pub const fn shutdown_timeout_ms(&self) -> u64 {
        self.shutdown_timeout_ms
    }
}

/// Codex source-adapter settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexConfig {
    pub(crate) source_adapter: String,
    pub(crate) task_completed: bool,
    pub(crate) approval_requested: bool,
}

impl CodexConfig {
    /// Returns the configured source-adapter selector.
    #[must_use]
    pub fn source_adapter(&self) -> &str {
        &self.source_adapter
    }

    /// Returns whether task-completion ingestion is requested.
    #[must_use]
    pub const fn task_completed(&self) -> bool {
        self.task_completed
    }

    /// Returns whether approval-request ingestion is requested.
    #[must_use]
    pub const fn approval_requested(&self) -> bool {
        self.approval_requested
    }
}

/// Desktop notification settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopConfig {
    pub(crate) privacy: NotificationPrivacy,
    pub(crate) quiet_hours: bool,
}

impl DesktopConfig {
    /// Returns the notification privacy policy.
    #[must_use]
    pub const fn privacy(&self) -> NotificationPrivacy {
        self.privacy
    }

    /// Returns whether application quiet hours are enabled.
    #[must_use]
    pub const fn quiet_hours(&self) -> bool {
        self.quiet_hours
    }
}

/// Relay SSH settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayConfig {
    pub(crate) ssh_host_alias: Option<String>,
    pub(crate) target_profile: String,
    pub(crate) connect_timeout_ms: u64,
    pub(crate) retry_initial_delay_ms: u64,
    pub(crate) retry_max_delay_ms: u64,
    pub(crate) retry_max_attempts: u32,
}

impl RelayConfig {
    /// Returns the configured OpenSSH host alias.
    #[must_use]
    pub fn ssh_host_alias(&self) -> Option<&str> {
        self.ssh_host_alias.as_deref()
    }

    /// Returns the destination configuration profile.
    #[must_use]
    pub fn target_profile(&self) -> &str {
        &self.target_profile
    }

    /// Returns the SSH connection timeout in milliseconds.
    #[must_use]
    pub const fn connect_timeout_ms(&self) -> u64 {
        self.connect_timeout_ms
    }

    /// Returns the initial relay retry delay before random jitter.
    #[must_use]
    pub const fn retry_initial_delay_ms(&self) -> u64 {
        self.retry_initial_delay_ms
    }

    /// Returns the maximum relay retry delay before random jitter.
    #[must_use]
    pub const fn retry_max_delay_ms(&self) -> u64 {
        self.retry_max_delay_ms
    }

    /// Returns the maximum number of consumed relay delivery attempts.
    #[must_use]
    pub const fn retry_max_attempts(&self) -> u32 {
        self.retry_max_attempts
    }
}

/// Persistent state settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageConfig {
    pub(crate) state_dir: PathBuf,
    pub(crate) max_queue_entries: usize,
}

impl StorageConfig {
    /// Returns the state directory.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Returns the bounded queue entry limit.
    #[must_use]
    pub const fn max_queue_entries(&self) -> usize {
        self.max_queue_entries
    }
}

/// Structured log settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoggingConfig {
    pub(crate) level: LogLevel,
    pub(crate) directory: PathBuf,
}

impl LoggingConfig {
    /// Returns the structured logging level.
    #[must_use]
    pub const fn level(&self) -> LogLevel {
        self.level
    }

    /// Returns the log directory.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

/// Fully merged and validated configuration version 1.
#[derive(Clone, Eq, PartialEq)]
pub struct Config {
    pub(crate) version: u16,
    pub(crate) agent: AgentConfig,
    pub(crate) codex: CodexConfig,
    pub(crate) desktop: DesktopConfig,
    pub(crate) relay: RelayConfig,
    pub(crate) storage: StorageConfig,
    pub(crate) logging: LoggingConfig,
}

impl Config {
    /// Returns the effective configuration version.
    #[must_use]
    pub const fn config_version(&self) -> u16 {
        self.version
    }

    /// Returns agent settings.
    #[must_use]
    pub const fn agent(&self) -> &AgentConfig {
        &self.agent
    }

    /// Returns Codex source settings.
    #[must_use]
    pub const fn codex(&self) -> &CodexConfig {
        &self.codex
    }

    /// Returns desktop settings.
    #[must_use]
    pub const fn desktop(&self) -> &DesktopConfig {
        &self.desktop
    }

    /// Returns relay settings.
    #[must_use]
    pub const fn relay(&self) -> &RelayConfig {
        &self.relay
    }

    /// Returns storage settings.
    #[must_use]
    pub const fn storage(&self) -> &StorageConfig {
        &self.storage
    }

    /// Returns logging settings.
    #[must_use]
    pub const fn logging(&self) -> &LoggingConfig {
        &self.logging
    }

    /// Creates a redacted summary suitable for logs and diagnostics.
    #[must_use]
    pub fn safe_summary(&self) -> SafeConfigSummary<'_> {
        SafeConfigSummary {
            version: self.version,
            role: self.agent.role,
            profile: &self.agent.profile,
            privacy: self.desktop.privacy,
            log_level: self.logging.level,
            max_queue_entries: self.storage.max_queue_entries,
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.safe_summary().fmt(formatter)
    }
}

/// Redacted configuration view safe for normal diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SafeConfigSummary<'a> {
    /// Configuration schema version.
    pub version: u16,
    /// Explicit runtime role.
    pub role: Role,
    /// Non-sensitive profile label.
    pub profile: &'a str,
    /// Notification privacy setting.
    pub privacy: NotificationPrivacy,
    /// Structured log level.
    pub log_level: LogLevel,
    /// Bounded queue capacity.
    pub max_queue_entries: usize,
}
