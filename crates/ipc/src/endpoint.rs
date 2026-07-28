//! Deterministic per-profile endpoint names and secure runtime directories.

use std::path::{Path, PathBuf};

use interprocess::local_socket::Name;
#[cfg(unix)]
use interprocess::local_socket::{GenericFilePath, ToFsName};
#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ToNsName};

use crate::IpcError;

const MAX_PROFILE_BYTES: usize = 64;

/// Validated per-user local IPC endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpcEndpoint {
    profile: String,
    runtime_dir: PathBuf,
}

impl IpcEndpoint {
    /// Creates an endpoint from an absolute per-user runtime directory and
    /// bounded profile identifier.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::InvalidEndpoint`] for an invalid profile or relative
    /// runtime directory.
    pub fn new(
        runtime_dir: impl Into<PathBuf>,
        profile: impl Into<String>,
    ) -> Result<Self, IpcError> {
        let runtime_dir = runtime_dir.into();
        let profile = profile.into();
        if !runtime_dir.is_absolute() || !valid_profile(&profile) {
            return Err(IpcError::InvalidEndpoint);
        }
        Ok(Self {
            profile,
            runtime_dir,
        })
    }

    /// Returns the logical configuration profile.
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Returns the per-user runtime directory.
    #[must_use]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Returns the Unix socket path or logical Windows pipe label without
    /// attacker-controlled event text.
    #[must_use]
    pub fn display_name(&self) -> String {
        #[cfg(unix)]
        {
            self.socket_path().to_string_lossy().into_owned()
        }
        #[cfg(windows)]
        {
            format!("codex-notifier-{}", self.profile)
        }
    }

    pub(crate) fn name(&self) -> Result<Name<'static>, IpcError> {
        #[cfg(unix)]
        {
            self.socket_path()
                .to_fs_name::<GenericFilePath>()
                .map_err(|_| IpcError::InvalidEndpoint)
        }
        #[cfg(windows)]
        {
            format!("codex-notifier-{}", self.profile)
                .to_ns_name::<GenericNamespaced>()
                .map_err(|_| IpcError::InvalidEndpoint)
        }
    }

    #[cfg(unix)]
    pub(crate) fn socket_path(&self) -> PathBuf {
        self.runtime_dir
            .join(format!("codex-notifier-{}.sock", self.profile))
    }

    #[cfg(windows)]
    pub(crate) fn pipe_path(&self) -> String {
        format!(r"\\.\pipe\codex-notifier-{}", self.profile)
    }

    #[cfg(unix)]
    pub(crate) fn prepare_runtime_dir(&self) -> Result<(), IpcError> {
        use std::fs::{DirBuilder, Permissions};
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

        if self.runtime_dir.exists() {
            let metadata = self
                .runtime_dir
                .symlink_metadata()
                .map_err(|_| IpcError::InsecureEndpoint)?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.uid() != rustix::process::geteuid().as_raw()
            {
                return Err(IpcError::InsecureEndpoint);
            }
        } else {
            let mut builder = DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(&self.runtime_dir)
                .map_err(|_| IpcError::InsecureEndpoint)?;
        }
        std::fs::set_permissions(&self.runtime_dir, Permissions::from_mode(0o700))
            .map_err(|_| IpcError::InsecureEndpoint)?;
        let metadata = self
            .runtime_dir
            .symlink_metadata()
            .map_err(|_| IpcError::InsecureEndpoint)?;
        if metadata.mode() & 0o077 != 0 || metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(IpcError::InsecureEndpoint);
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn validate_existing_socket(&self) -> Result<(), IpcError> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};

        let path = self.socket_path();
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(IpcError::InsecureEndpoint),
        };
        if !metadata.file_type().is_socket()
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(IpcError::InsecureEndpoint);
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn remove_owned_socket(&self) {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};

        let path = self.socket_path();
        let Ok(metadata) = path.symlink_metadata() else {
            return;
        };
        if metadata.file_type().is_socket() && metadata.uid() == rustix::process::geteuid().as_raw()
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn valid_profile(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROFILE_BYTES
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'_' | b'-'))
        })
}
