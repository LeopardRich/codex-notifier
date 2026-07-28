//! Deterministic platform path resolution.

use std::path::{Path, PathBuf};

use crate::ConfigError;

const PRODUCT_DIRECTORY: &str = "codex-notifier";

/// Path convention to resolve without consulting the current host implicitly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    /// `%APPDATA%` and `%LOCALAPPDATA%` Windows conventions.
    Windows,
    /// macOS `Library` conventions.
    MacOs,
    /// XDG base-directory conventions for relay hosts.
    Xdg,
}

/// Explicit environment snapshot used for deterministic path resolution.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PathEnvironment {
    home: Option<PathBuf>,
    windows_app_data: Option<PathBuf>,
    windows_local_app_data: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    xdg_state_home: Option<PathBuf>,
}

impl PathEnvironment {
    /// Creates an empty environment snapshot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            home: None,
            windows_app_data: None,
            windows_local_app_data: None,
            xdg_config_home: None,
            xdg_state_home: None,
        }
    }

    /// Sets the user home directory.
    #[must_use]
    pub fn with_home(mut self, value: impl Into<PathBuf>) -> Self {
        self.home = Some(value.into());
        self
    }

    /// Sets `%APPDATA%`.
    #[must_use]
    pub fn with_windows_app_data(mut self, value: impl Into<PathBuf>) -> Self {
        self.windows_app_data = Some(value.into());
        self
    }

    /// Sets `%LOCALAPPDATA%`.
    #[must_use]
    pub fn with_windows_local_app_data(mut self, value: impl Into<PathBuf>) -> Self {
        self.windows_local_app_data = Some(value.into());
        self
    }

    /// Sets `$XDG_CONFIG_HOME`.
    #[must_use]
    pub fn with_xdg_config_home(mut self, value: impl Into<PathBuf>) -> Self {
        self.xdg_config_home = Some(value.into());
        self
    }

    /// Sets `$XDG_STATE_HOME`.
    #[must_use]
    pub fn with_xdg_state_home(mut self, value: impl Into<PathBuf>) -> Self {
        self.xdg_state_home = Some(value.into());
        self
    }

    /// Resolves configuration, state, and log paths for `platform`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingPathBase`] when required explicit
    /// environment paths are absent or not absolute.
    pub fn resolve(&self, platform: Platform) -> Result<ConfigPaths, ConfigError> {
        match platform {
            Platform::Windows => {
                let config_base = required_absolute(self.windows_app_data.as_deref(), platform)?;
                let state_base =
                    required_absolute(self.windows_local_app_data.as_deref(), platform)?;
                let config_dir = config_base.join(PRODUCT_DIRECTORY);
                let state_dir = state_base.join(PRODUCT_DIRECTORY);
                Ok(ConfigPaths {
                    config_file: config_dir.join("config.toml"),
                    log_dir: state_dir.join("logs"),
                    state_dir,
                })
            }
            Platform::MacOs => {
                let home = required_absolute(self.home.as_deref(), platform)?;
                let application_support = home
                    .join("Library")
                    .join("Application Support")
                    .join(PRODUCT_DIRECTORY);
                Ok(ConfigPaths {
                    config_file: application_support.join("config.toml"),
                    state_dir: application_support.join("state"),
                    log_dir: home.join("Library").join("Logs").join(PRODUCT_DIRECTORY),
                })
            }
            Platform::Xdg => {
                let home = required_absolute(self.home.as_deref(), platform)?;
                let config_base = match self.xdg_config_home.as_deref() {
                    Some(path) => required_absolute(Some(path), platform)?.to_owned(),
                    None => home.join(".config"),
                };
                let state_base = match self.xdg_state_home.as_deref() {
                    Some(path) => required_absolute(Some(path), platform)?.to_owned(),
                    None => home.join(".local").join("state"),
                };
                let state_dir = state_base.join(PRODUCT_DIRECTORY);
                Ok(ConfigPaths {
                    config_file: config_base.join(PRODUCT_DIRECTORY).join("config.toml"),
                    log_dir: state_dir.join("logs"),
                    state_dir,
                })
            }
        }
    }
}

/// Resolved per-user configuration and state paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigPaths {
    config_file: PathBuf,
    state_dir: PathBuf,
    log_dir: PathBuf,
}

impl ConfigPaths {
    /// Returns the user configuration file path.
    #[must_use]
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    /// Returns the state directory path.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Returns the log directory path.
    #[must_use]
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }
}

fn required_absolute(path: Option<&Path>, platform: Platform) -> Result<&Path, ConfigError> {
    path.filter(|value| is_absolute_for(value, platform))
        .ok_or(ConfigError::MissingPathBase)
}

pub(crate) fn is_absolute_any(path: &Path) -> bool {
    is_absolute_for(path, Platform::Windows)
        || is_absolute_for(path, Platform::MacOs)
        || is_absolute_for(path, Platform::Xdg)
}

fn is_absolute_for(path: &Path, platform: Platform) -> bool {
    let value = path.to_string_lossy();
    match platform {
        Platform::Windows => {
            let bytes = value.as_bytes();
            value.starts_with("\\\\")
                || (bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && matches!(bytes[2], b'\\' | b'/'))
        }
        Platform::MacOs | Platform::Xdg => value.starts_with('/'),
    }
}
