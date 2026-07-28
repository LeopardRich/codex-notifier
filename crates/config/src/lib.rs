//! Layered configuration, platform paths, and configuration migrations.

mod error;
mod loader;
mod model;
mod paths;

pub use error::{ConfigError, ErrorCode};
pub use loader::{CliOverrides, ConfigLoader, FileSystemStateProbe, StateDirectoryProbe};
pub use model::{
    AgentConfig, CodexConfig, Config, DesktopConfig, IpcEndpoint, LogLevel, LoggingConfig,
    NotificationPrivacy, RelayConfig, Role, SafeConfigSummary, StorageConfig,
};
pub use paths::{ConfigPaths, PathEnvironment, Platform};
