//! Configuration parsing, migration, layering, and validation.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml::Value;

use crate::model::{
    AgentConfig, CodexConfig, Config, DesktopConfig, IpcEndpoint, LogLevel, LoggingConfig,
    NotificationPrivacy, RelayConfig, Role, StorageConfig,
};
use crate::paths::is_absolute_any;
use crate::{ConfigError, ConfigPaths};

const CONFIG_VERSION: u16 = 1;
const MAX_PROFILE_BYTES: usize = 64;
const MAX_ENDPOINT_NAME_BYTES: usize = 64;
const MAX_SSH_ALIAS_BYTES: usize = 128;

/// Abstract state-directory writability check used by deterministic tests.
pub trait StateDirectoryProbe {
    /// Returns whether `path` can hold application state.
    fn is_writable(&self, path: &Path) -> bool;
}

/// Real filesystem writability probe.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileSystemStateProbe;

impl StateDirectoryProbe for FileSystemStateProbe {
    fn is_writable(&self, path: &Path) -> bool {
        if fs::create_dir_all(path).is_err() {
            return false;
        }
        let probe_path = path.join(format!(".codex-notifier-write-test-{}", std::process::id()));
        let result = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&probe_path)
            .and_then(|mut file| file.write_all(b"probe"));
        if result.is_ok() {
            let _ = fs::remove_file(probe_path);
            true
        } else {
            false
        }
    }
}

/// Explicit command-line overrides, applied after all file layers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CliOverrides {
    role: Option<String>,
    profile: Option<String>,
    ipc_endpoint: Option<String>,
    privacy: Option<String>,
    relay_host: Option<String>,
    state_dir: Option<PathBuf>,
    max_queue_entries: Option<usize>,
    log_level: Option<String>,
}

impl CliOverrides {
    /// Creates an empty override set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            role: None,
            profile: None,
            ipc_endpoint: None,
            privacy: None,
            relay_host: None,
            state_dir: None,
            max_queue_entries: None,
            log_level: None,
        }
    }

    /// Overrides the runtime role.
    #[must_use]
    pub fn with_role(mut self, value: impl Into<String>) -> Self {
        self.role = Some(value.into());
        self
    }

    /// Overrides the active profile label.
    #[must_use]
    pub fn with_profile(mut self, value: impl Into<String>) -> Self {
        self.profile = Some(value.into());
        self
    }

    /// Overrides the logical IPC endpoint.
    #[must_use]
    pub fn with_ipc_endpoint(mut self, value: impl Into<String>) -> Self {
        self.ipc_endpoint = Some(value.into());
        self
    }

    /// Overrides notification privacy.
    #[must_use]
    pub fn with_privacy(mut self, value: impl Into<String>) -> Self {
        self.privacy = Some(value.into());
        self
    }

    /// Overrides the relay OpenSSH host alias.
    #[must_use]
    pub fn with_relay_host(mut self, value: impl Into<String>) -> Self {
        self.relay_host = Some(value.into());
        self
    }

    /// Overrides the state directory.
    #[must_use]
    pub fn with_state_dir(mut self, value: impl Into<PathBuf>) -> Self {
        self.state_dir = Some(value.into());
        self
    }

    /// Overrides the queue capacity.
    #[must_use]
    pub const fn with_max_queue_entries(mut self, value: usize) -> Self {
        self.max_queue_entries = Some(value);
        self
    }

    /// Overrides the structured log level.
    #[must_use]
    pub fn with_log_level(mut self, value: impl Into<String>) -> Self {
        self.log_level = Some(value.into());
        self
    }

    fn into_layer(self) -> Layer {
        Layer {
            agent: Some(AgentLayer {
                role: self.role,
                profile: self.profile,
                ipc_endpoint: self.ipc_endpoint,
                shutdown_timeout_ms: None,
            }),
            desktop: Some(DesktopLayer {
                privacy: self.privacy,
                quiet_hours: None,
            }),
            relay: Some(RelayLayer {
                ssh_host_alias: self.relay_host,
                target_profile: None,
                connect_timeout_ms: None,
                retry_initial_delay_ms: None,
                retry_max_delay_ms: None,
                retry_max_attempts: None,
            }),
            storage: Some(StorageLayer {
                state_dir: self.state_dir,
                max_queue_entries: self.max_queue_entries,
            }),
            logging: Some(LoggingLayer {
                level: self.log_level,
                directory: None,
            }),
            ..Layer::default()
        }
    }
}

/// Loads deterministic configuration layers.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConfigLoader;

impl ConfigLoader {
    /// Applies defaults, user TOML, profile TOML, then CLI overrides.
    ///
    /// # Errors
    ///
    /// Returns a classified [`ConfigError`] for malformed/versioned input,
    /// unsafe fields, migration failure, invalid values, missing relay settings,
    /// or an unwritable final state directory.
    pub fn load(
        paths: &ConfigPaths,
        user_toml: Option<&str>,
        profile_toml: Option<&str>,
        cli: CliOverrides,
        state_probe: &dyn StateDirectoryProbe,
    ) -> Result<Config, ConfigError> {
        let mut effective = Effective::defaults(paths);
        if let Some(input) = user_toml {
            effective.apply(parse_document(input)?);
        }
        if let Some(input) = profile_toml {
            effective.apply(parse_document(input)?);
        }
        effective.apply(cli.into_layer());
        effective.validate(state_probe)
    }
}

#[derive(Clone)]
struct Effective {
    role: String,
    profile: String,
    ipc_endpoint: String,
    shutdown_timeout_ms: u64,
    source_adapter: String,
    task_completed: bool,
    approval_requested: bool,
    privacy: String,
    quiet_hours: bool,
    ssh_host_alias: Option<String>,
    target_profile: String,
    connect_timeout_ms: u64,
    retry_initial_delay_ms: u64,
    retry_max_delay_ms: u64,
    retry_max_attempts: u32,
    state_dir: PathBuf,
    max_queue_entries: usize,
    log_level: String,
    log_dir: PathBuf,
}

impl Effective {
    fn defaults(paths: &ConfigPaths) -> Self {
        Self {
            role: "desktop".to_owned(),
            profile: "default".to_owned(),
            ipc_endpoint: "default".to_owned(),
            shutdown_timeout_ms: 5_000,
            source_adapter: "auto".to_owned(),
            task_completed: true,
            approval_requested: true,
            privacy: "private".to_owned(),
            quiet_hours: false,
            ssh_host_alias: None,
            target_profile: "default".to_owned(),
            connect_timeout_ms: 10_000,
            retry_initial_delay_ms: 1_000,
            retry_max_delay_ms: 60_000,
            retry_max_attempts: 20,
            state_dir: paths.state_dir().to_owned(),
            max_queue_entries: 1_000,
            log_level: "info".to_owned(),
            log_dir: paths.log_dir().to_owned(),
        }
    }

    fn apply(&mut self, layer: Layer) {
        if let Some(agent) = layer.agent {
            replace(&mut self.role, agent.role);
            replace(&mut self.profile, agent.profile);
            replace(&mut self.ipc_endpoint, agent.ipc_endpoint);
            replace(&mut self.shutdown_timeout_ms, agent.shutdown_timeout_ms);
        }
        if let Some(codex) = layer.codex {
            replace(&mut self.source_adapter, codex.source_adapter);
            replace(&mut self.task_completed, codex.task_completed);
            replace(&mut self.approval_requested, codex.approval_requested);
        }
        if let Some(desktop) = layer.desktop {
            replace(&mut self.privacy, desktop.privacy);
            replace(&mut self.quiet_hours, desktop.quiet_hours);
        }
        if let Some(relay) = layer.relay {
            replace(&mut self.ssh_host_alias, relay.ssh_host_alias.map(Some));
            replace(&mut self.target_profile, relay.target_profile);
            replace(&mut self.connect_timeout_ms, relay.connect_timeout_ms);
            replace(
                &mut self.retry_initial_delay_ms,
                relay.retry_initial_delay_ms,
            );
            replace(&mut self.retry_max_delay_ms, relay.retry_max_delay_ms);
            replace(&mut self.retry_max_attempts, relay.retry_max_attempts);
        }
        if let Some(storage) = layer.storage {
            replace(&mut self.state_dir, storage.state_dir);
            replace(&mut self.max_queue_entries, storage.max_queue_entries);
        }
        if let Some(logging) = layer.logging {
            replace(&mut self.log_level, logging.level);
            replace(&mut self.log_dir, logging.directory);
        }
    }

    fn validate(self, state_probe: &dyn StateDirectoryProbe) -> Result<Config, ConfigError> {
        let role = parse_role(&self.role)?;
        let profile = validate_profile(self.profile)?;
        let ipc_endpoint = parse_endpoint(&self.ipc_endpoint)?;
        if !(100..=60_000).contains(&self.shutdown_timeout_ms) {
            return Err(ConfigError::InvalidValue);
        }
        if !valid_selector(&self.source_adapter) {
            return Err(ConfigError::InvalidValue);
        }
        let privacy = parse_privacy(&self.privacy)?;
        let ssh_host_alias = self.ssh_host_alias.map(validate_ssh_alias).transpose()?;
        if role == Role::Relay && ssh_host_alias.is_none() {
            return Err(ConfigError::MissingRelayHost);
        }
        let target_profile = validate_profile(self.target_profile)?;
        if !(100..=120_000).contains(&self.connect_timeout_ms)
            || !(100..=60_000).contains(&self.retry_initial_delay_ms)
            || !(100..=3_600_000).contains(&self.retry_max_delay_ms)
            || self.retry_initial_delay_ms > self.retry_max_delay_ms
            || !(1..=1_000).contains(&self.retry_max_attempts)
            || !(1..=100_000).contains(&self.max_queue_entries)
            || !is_absolute_any(&self.state_dir)
            || !is_absolute_any(&self.log_dir)
        {
            return Err(ConfigError::InvalidValue);
        }
        if !state_probe.is_writable(&self.state_dir) {
            return Err(ConfigError::UnwritableStateDirectory);
        }
        let log_level = parse_log_level(&self.log_level)?;

        Ok(Config {
            version: CONFIG_VERSION,
            agent: AgentConfig {
                role,
                profile,
                ipc_endpoint,
                shutdown_timeout_ms: self.shutdown_timeout_ms,
            },
            codex: CodexConfig {
                source_adapter: self.source_adapter,
                task_completed: self.task_completed,
                approval_requested: self.approval_requested,
            },
            desktop: DesktopConfig {
                privacy,
                quiet_hours: self.quiet_hours,
            },
            relay: RelayConfig {
                ssh_host_alias,
                target_profile,
                connect_timeout_ms: self.connect_timeout_ms,
                retry_initial_delay_ms: self.retry_initial_delay_ms,
                retry_max_delay_ms: self.retry_max_delay_ms,
                retry_max_attempts: self.retry_max_attempts,
            },
            storage: StorageConfig {
                state_dir: self.state_dir,
                max_queue_entries: self.max_queue_entries,
            },
            logging: LoggingConfig {
                level: log_level,
                directory: self.log_dir,
            },
        })
    }
}

fn replace<T>(destination: &mut T, source: Option<T>) {
    if let Some(value) = source {
        *destination = value;
    }
}

fn parse_document(input: &str) -> Result<Layer, ConfigError> {
    let mut value: Value = toml::from_str(input).map_err(|_| ConfigError::Malformed)?;
    reject_sensitive_keys(&value)?;
    let table = value.as_table_mut().ok_or(ConfigError::Malformed)?;
    let version_value = table.get("config_version");
    let version = version_value.and_then(Value::as_integer);
    match version {
        Some(1) => {
            table.remove("config_version");
        }
        Some(0) => migrate_v0(table)?,
        Some(_) => return Err(ConfigError::UnsupportedVersion),
        None if version_value.is_some() => return Err(ConfigError::Malformed),
        None if table.contains_key("role") || table.contains_key("ssh_host") => {
            migrate_v0(table)?;
        }
        None => return Err(ConfigError::MissingVersion),
    }
    value.try_into().map_err(|_| ConfigError::Malformed)
}

fn migrate_v0(table: &mut toml::Table) -> Result<(), ConfigError> {
    let allowed = ["config_version", "role", "ssh_host"];
    if table.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ConfigError::MigrationFailed);
    }
    table.remove("config_version");
    let role = table.remove("role").ok_or(ConfigError::MigrationFailed)?;
    if !role.is_str() {
        return Err(ConfigError::MigrationFailed);
    }
    let mut agent = toml::Table::new();
    agent.insert("role".to_owned(), role);
    table.insert("agent".to_owned(), Value::Table(agent));
    if let Some(host) = table.remove("ssh_host") {
        if !host.is_str() {
            return Err(ConfigError::MigrationFailed);
        }
        let mut relay = toml::Table::new();
        relay.insert("ssh_host_alias".to_owned(), host);
        table.insert("relay".to_owned(), Value::Table(relay));
    }
    Ok(())
}

fn reject_sensitive_keys(value: &Value) -> Result<(), ConfigError> {
    match value {
        Value::Table(table) => {
            for (key, value) in table {
                let key = key.to_ascii_lowercase().replace('-', "_");
                if matches!(
                    key.as_str(),
                    "private_key"
                        | "private_key_pem"
                        | "ssh_private_key"
                        | "access_token"
                        | "api_key"
                        | "password"
                        | "raw_payload"
                        | "event_payload"
                        | "prompt"
                        | "model_output"
                ) {
                    return Err(ConfigError::SensitiveField);
                }
                reject_sensitive_keys(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_sensitive_keys(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_role(value: &str) -> Result<Role, ConfigError> {
    match value {
        "desktop" => Ok(Role::Desktop),
        "relay" => Ok(Role::Relay),
        _ => Err(ConfigError::InvalidRole),
    }
}

fn parse_endpoint(value: &str) -> Result<IpcEndpoint, ConfigError> {
    if value == "default" {
        return Ok(IpcEndpoint::Default);
    }
    let name = value
        .strip_prefix("name:")
        .ok_or(ConfigError::InvalidEndpoint)?;
    if valid_identifier(name, MAX_ENDPOINT_NAME_BYTES) {
        Ok(IpcEndpoint::Named(name.to_owned()))
    } else {
        Err(ConfigError::InvalidEndpoint)
    }
}

fn parse_privacy(value: &str) -> Result<NotificationPrivacy, ConfigError> {
    match value {
        "private" => Ok(NotificationPrivacy::Private),
        "public" => Ok(NotificationPrivacy::Public),
        _ => Err(ConfigError::InvalidValue),
    }
}

fn parse_log_level(value: &str) -> Result<LogLevel, ConfigError> {
    match value {
        "error" => Ok(LogLevel::Error),
        "warn" => Ok(LogLevel::Warn),
        "info" => Ok(LogLevel::Info),
        "debug" => Ok(LogLevel::Debug),
        "trace" => Ok(LogLevel::Trace),
        _ => Err(ConfigError::InvalidValue),
    }
}

fn validate_profile(value: String) -> Result<String, ConfigError> {
    if valid_identifier(&value, MAX_PROFILE_BYTES) {
        Ok(value)
    } else {
        Err(ConfigError::InvalidValue)
    }
}

fn validate_ssh_alias(value: String) -> Result<String, ConfigError> {
    if value.is_empty()
        || value.len() > MAX_SSH_ALIAS_BYTES
        || !value.is_ascii()
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        })
    {
        Err(ConfigError::InvalidRelayHost)
    } else {
        Ok(value)
    }
}

fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'_' | b'-'))
        })
}

fn valid_selector(value: &str) -> bool {
    value == "auto" || valid_identifier(value, MAX_PROFILE_BYTES)
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layer {
    agent: Option<AgentLayer>,
    codex: Option<CodexLayer>,
    desktop: Option<DesktopLayer>,
    relay: Option<RelayLayer>,
    storage: Option<StorageLayer>,
    logging: Option<LoggingLayer>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentLayer {
    role: Option<String>,
    profile: Option<String>,
    ipc_endpoint: Option<String>,
    shutdown_timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexLayer {
    source_adapter: Option<String>,
    task_completed: Option<bool>,
    approval_requested: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesktopLayer {
    privacy: Option<String>,
    quiet_hours: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayLayer {
    ssh_host_alias: Option<String>,
    target_profile: Option<String>,
    connect_timeout_ms: Option<u64>,
    retry_initial_delay_ms: Option<u64>,
    retry_max_delay_ms: Option<u64>,
    retry_max_attempts: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageLayer {
    state_dir: Option<PathBuf>,
    max_queue_entries: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoggingLayer {
    level: Option<String>,
    directory: Option<PathBuf>,
}
