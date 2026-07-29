//! Reversible per-user installation metadata and Codex hook ownership.

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MANIFEST_SCHEMA_VERSION: u16 = 1;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const DEFAULT_CONFIG: &[u8] = b"config_version = 1\n";
const HOOK_STATUS: &str = "Sending Codex notification";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Stable installation failures that never include a user path or file body.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LifecycleError {
    /// A required path is relative or the manifest points outside its layout.
    #[error("installation layout is invalid")]
    InvalidLayout,
    /// A managed document is too large, malformed, or has an unexpected shape.
    #[error("installation metadata is invalid")]
    InvalidDocument,
    /// A symlink or other unexpected filesystem object occupies an owned path.
    #[error("installation resource ownership is unsafe")]
    UnsafeResource,
    /// Existing installation metadata is newer or belongs to another layout.
    #[error("installation ownership conflicts with existing resources")]
    OwnershipConflict,
    /// A bounded filesystem operation failed.
    #[error("installation filesystem operation failed")]
    FileSystem,
}

impl LifecycleError {
    /// Returns the stable machine-readable failure classification.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidLayout => "install_layout_invalid",
            Self::InvalidDocument => "install_document_invalid",
            Self::UnsafeResource => "install_resource_unsafe",
            Self::OwnershipConflict => "install_ownership_conflict",
            Self::FileSystem => "install_filesystem_failed",
        }
    }
}

/// Exact user-level paths the installer may own or edit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleLayout {
    install_root: PathBuf,
    installed_executable: PathBuf,
    config_file: PathBuf,
    hooks_file: PathBuf,
    manifest_file: PathBuf,
}

impl LifecycleLayout {
    /// Creates a bounded absolute resource layout.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::InvalidLayout`] when any path is relative or
    /// an owned file is not nested under its expected root.
    pub fn new(
        install_root: impl Into<PathBuf>,
        installed_executable: impl Into<PathBuf>,
        config_file: impl Into<PathBuf>,
        hooks_file: impl Into<PathBuf>,
        manifest_file: impl Into<PathBuf>,
    ) -> Result<Self, LifecycleError> {
        let value = Self {
            install_root: install_root.into(),
            installed_executable: installed_executable.into(),
            config_file: config_file.into(),
            hooks_file: hooks_file.into(),
            manifest_file: manifest_file.into(),
        };
        if [
            &value.install_root,
            &value.installed_executable,
            &value.config_file,
            &value.hooks_file,
            &value.manifest_file,
        ]
        .into_iter()
        .any(|path| !path.is_absolute())
            || !value.installed_executable.starts_with(&value.install_root)
        {
            return Err(LifecycleError::InvalidLayout);
        }
        Ok(value)
    }

    /// Returns the owned installation root.
    #[must_use]
    pub fn install_root(&self) -> &Path {
        &self.install_root
    }

    /// Returns the installed executable path.
    #[must_use]
    pub fn installed_executable(&self) -> &Path {
        &self.installed_executable
    }

    /// Returns the user configuration path.
    #[must_use]
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    /// Returns the user Codex hook path.
    #[must_use]
    pub fn hooks_file(&self) -> &Path {
        &self.hooks_file
    }

    /// Returns the ownership manifest path.
    #[must_use]
    pub fn manifest_file(&self) -> &Path {
        &self.manifest_file
    }
}

/// Platform-specific resources recorded without credentials or commands.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlatformOwnership {
    /// Per-user Windows notification and startup resources.
    Windows {
        /// Exact Start Menu shortcut path.
        start_menu_shortcut: PathBuf,
        /// Fixed registry path for the notification identity.
        notification_registry_key: String,
        /// Fixed Run-key value name.
        startup_value_name: String,
    },
    /// Per-user macOS `LaunchAgent` resource.
    MacOs {
        /// Exact `LaunchAgent` plist path.
        launch_agent: PathBuf,
        /// Fixed launchd label.
        launch_agent_label: String,
    },
}

/// Installer-owned state needed for exact uninstall and upgrade.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstallManifest {
    schema_version: u16,
    install_version: String,
    install_root: PathBuf,
    installed_executable: PathBuf,
    config_file: PathBuf,
    config_created: bool,
    config_digest: Option<String>,
    hooks_file: PathBuf,
    hooks_file_created: bool,
    hook_group: Value,
    platform: PlatformOwnership,
}

impl InstallManifest {
    /// Returns the installed application version.
    #[must_use]
    pub fn install_version(&self) -> &str {
        &self.install_version
    }

    /// Returns the recorded platform resources.
    #[must_use]
    pub const fn platform(&self) -> &PlatformOwnership {
        &self.platform
    }

    /// Returns the exact additive hook group owned by this install.
    #[must_use]
    pub const fn hook_group(&self) -> &Value {
        &self.hook_group
    }

    /// Returns the recorded installed executable.
    #[must_use]
    pub fn installed_executable(&self) -> &Path {
        &self.installed_executable
    }

    fn validate(&self, layout: &LifecycleLayout) -> Result<(), LifecycleError> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION
            || self.install_root != layout.install_root
            || self.installed_executable != layout.installed_executable
            || self.config_file != layout.config_file
            || self.hooks_file != layout.hooks_file
        {
            return Err(LifecycleError::OwnershipConflict);
        }
        Ok(())
    }
}

/// Exact disposition of one managed document during uninstall.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RemovalDisposition {
    /// The exact installer-owned content was removed.
    Removed,
    /// User-modified or pre-existing content was preserved.
    Preserved,
    /// The resource was already absent.
    #[default]
    Absent,
}

/// Results that explain which user-modified resources uninstall preserved.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ManagedRemovalReport {
    /// Disposition of the additive `Stop` hook group.
    pub hook: RemovalDisposition,
    /// Disposition of the user configuration file.
    pub config: RemovalDisposition,
}

/// Builds the exact additive `Stop` hook group owned by this installer.
#[must_use]
pub fn stop_hook_group(command: &str, windows_command: Option<&str>) -> Value {
    let mut handler = Map::new();
    handler.insert("type".to_owned(), Value::String("command".to_owned()));
    handler.insert("command".to_owned(), Value::String(command.to_owned()));
    if let Some(command) = windows_command {
        handler.insert(
            "commandWindows".to_owned(),
            Value::String(command.to_owned()),
        );
    }
    handler.insert("timeout".to_owned(), Value::from(3));
    handler.insert(
        "statusMessage".to_owned(),
        Value::String(HOOK_STATUS.to_owned()),
    );
    json!({ "hooks": [Value::Object(handler)] })
}

/// Reads and validates the current ownership manifest.
///
/// # Errors
///
/// Returns a stable document, ownership, or filesystem error.
pub fn read_manifest(layout: &LifecycleLayout) -> Result<Option<InstallManifest>, LifecycleError> {
    let Some(bytes) = read_optional_file(layout.manifest_file())? else {
        return Ok(None);
    };
    let manifest: InstallManifest =
        serde_json::from_slice(&bytes).map_err(|_| LifecycleError::InvalidDocument)?;
    manifest.validate(layout)?;
    Ok(Some(manifest))
}

/// Creates or upgrades configuration, the additive Codex hook, and ownership.
///
/// Existing user configuration is never rewritten. Existing hook JSON is
/// merged structurally. On failure, every touched document is restored.
///
/// # Errors
///
/// Returns a stable validation, ownership, or filesystem error.
pub fn install_managed_documents(
    layout: &LifecycleLayout,
    install_version: &str,
    hook_group: Value,
    platform: PlatformOwnership,
) -> Result<InstallManifest, LifecycleError> {
    if install_version.is_empty() || install_version.len() > 64 || !install_version.is_ascii() {
        return Err(LifecycleError::InvalidDocument);
    }
    validate_hook_group(&hook_group)?;
    let previous_manifest = read_manifest(layout)?;
    let snapshots = [
        FileSnapshot::capture(layout.config_file())?,
        FileSnapshot::capture(layout.hooks_file())?,
        FileSnapshot::capture(layout.manifest_file())?,
    ];

    let operation = || {
        let (config_created, config_digest) =
            install_config(layout.config_file(), previous_manifest.as_ref())?;
        let hooks_file_created =
            install_hook(layout.hooks_file(), previous_manifest.as_ref(), &hook_group)?;
        let manifest = InstallManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            install_version: install_version.to_owned(),
            install_root: layout.install_root.clone(),
            installed_executable: layout.installed_executable.clone(),
            config_file: layout.config_file.clone(),
            config_created,
            config_digest,
            hooks_file: layout.hooks_file.clone(),
            hooks_file_created,
            hook_group,
            platform,
        };
        let bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|_| LifecycleError::InvalidDocument)?;
        write_atomic(layout.manifest_file(), &bytes)?;
        Ok(manifest)
    };

    match operation() {
        Ok(manifest) => Ok(manifest),
        Err(error) => {
            restore_snapshots(&snapshots)?;
            Err(error)
        }
    }
}

/// Removes only exact installer-owned documents and the manifest.
///
/// The platform adapter must deactivate its resources before this function is
/// called. Modified configuration and hook groups are preserved.
///
/// # Errors
///
/// Returns a stable validation, ownership, or filesystem error. Touched files
/// are restored when any operation fails.
pub fn uninstall_managed_documents(
    layout: &LifecycleLayout,
    manifest: &InstallManifest,
) -> Result<ManagedRemovalReport, LifecycleError> {
    manifest.validate(layout)?;
    let snapshots = [
        FileSnapshot::capture(layout.config_file())?,
        FileSnapshot::capture(layout.hooks_file())?,
        FileSnapshot::capture(layout.manifest_file())?,
    ];
    let operation = || {
        let hook = uninstall_hook(manifest)?;
        let config = uninstall_config(manifest)?;
        remove_owned_file(layout.manifest_file())?;
        Ok(ManagedRemovalReport { hook, config })
    };
    match operation() {
        Ok(report) => Ok(report),
        Err(error) => {
            restore_snapshots(&snapshots)?;
            Err(error)
        }
    }
}

fn install_config(
    path: &Path,
    previous: Option<&InstallManifest>,
) -> Result<(bool, Option<String>), LifecycleError> {
    match read_optional_file(path)? {
        None => {
            write_atomic(path, DEFAULT_CONFIG)?;
            Ok((true, Some(digest(DEFAULT_CONFIG))))
        }
        Some(bytes) => {
            let remains_owned = previous.is_some_and(|manifest| {
                manifest.config_created
                    && manifest.config_digest.as_deref() == Some(digest(&bytes).as_str())
            });
            Ok(if remains_owned {
                (true, Some(digest(&bytes)))
            } else {
                (false, None)
            })
        }
    }
}

fn install_hook(
    path: &Path,
    previous: Option<&InstallManifest>,
    hook_group: &Value,
) -> Result<bool, LifecycleError> {
    let existing = read_optional_file(path)?;
    let created = previous.map_or(existing.is_none(), |manifest| manifest.hooks_file_created);
    let mut document = match existing.as_deref() {
        Some(bytes) => {
            serde_json::from_slice(bytes).map_err(|_| LifecycleError::InvalidDocument)?
        }
        None => Value::Object(Map::new()),
    };

    if let Some(manifest) = previous {
        let removed = remove_hook_group(&mut document, &manifest.hook_group)?;
        if !removed && manifest.hook_group != *hook_group {
            return Err(LifecycleError::OwnershipConflict);
        }
    } else if contains_hook_group(&document, hook_group)? {
        return Err(LifecycleError::OwnershipConflict);
    }
    if !contains_hook_group(&document, hook_group)? {
        stop_groups_mut(&mut document)?.push(hook_group.clone());
    }
    let bytes =
        serde_json::to_vec_pretty(&document).map_err(|_| LifecycleError::InvalidDocument)?;
    write_atomic(path, &bytes)?;
    Ok(created)
}

fn uninstall_hook(manifest: &InstallManifest) -> Result<RemovalDisposition, LifecycleError> {
    let Some(bytes) = read_optional_file(&manifest.hooks_file)? else {
        return Ok(RemovalDisposition::Absent);
    };
    let mut document: Value =
        serde_json::from_slice(&bytes).map_err(|_| LifecycleError::InvalidDocument)?;
    if !remove_hook_group(&mut document, &manifest.hook_group)? {
        return Ok(RemovalDisposition::Preserved);
    }
    if manifest.hooks_file_created && hook_document_is_empty(&document) {
        remove_owned_file(&manifest.hooks_file)?;
    } else {
        let bytes =
            serde_json::to_vec_pretty(&document).map_err(|_| LifecycleError::InvalidDocument)?;
        write_atomic(&manifest.hooks_file, &bytes)?;
    }
    Ok(RemovalDisposition::Removed)
}

fn uninstall_config(manifest: &InstallManifest) -> Result<RemovalDisposition, LifecycleError> {
    if !manifest.config_created {
        return Ok(if manifest.config_file.exists() {
            RemovalDisposition::Preserved
        } else {
            RemovalDisposition::Absent
        });
    }
    let Some(bytes) = read_optional_file(&manifest.config_file)? else {
        return Ok(RemovalDisposition::Absent);
    };
    if manifest.config_digest.as_deref() == Some(digest(&bytes).as_str()) {
        remove_owned_file(&manifest.config_file)?;
        Ok(RemovalDisposition::Removed)
    } else {
        Ok(RemovalDisposition::Preserved)
    }
}

fn validate_hook_group(value: &Value) -> Result<(), LifecycleError> {
    let handlers = value
        .as_object()
        .and_then(|object| object.get("hooks"))
        .and_then(Value::as_array)
        .ok_or(LifecycleError::InvalidDocument)?;
    if handlers.len() != 1
        || handlers[0]
            .as_object()
            .and_then(|handler| handler.get("type"))
            .and_then(Value::as_str)
            != Some("command")
    {
        return Err(LifecycleError::InvalidDocument);
    }
    Ok(())
}

fn contains_hook_group(document: &Value, group: &Value) -> Result<bool, LifecycleError> {
    let Some(hooks) = document.as_object().and_then(|root| root.get("hooks")) else {
        return Ok(false);
    };
    let hooks = hooks.as_object().ok_or(LifecycleError::InvalidDocument)?;
    let Some(groups) = hooks.get("Stop") else {
        return Ok(false);
    };
    let groups = groups.as_array().ok_or(LifecycleError::InvalidDocument)?;
    Ok(groups.iter().any(|candidate| candidate == group))
}

fn remove_hook_group(document: &mut Value, group: &Value) -> Result<bool, LifecycleError> {
    let Some(hooks) = document
        .as_object_mut()
        .ok_or(LifecycleError::InvalidDocument)?
        .get_mut("hooks")
    else {
        return Ok(false);
    };
    let hooks = hooks
        .as_object_mut()
        .ok_or(LifecycleError::InvalidDocument)?;
    let (changed, empty) = {
        let Some(groups) = hooks.get_mut("Stop") else {
            return Ok(false);
        };
        let groups = groups
            .as_array_mut()
            .ok_or(LifecycleError::InvalidDocument)?;
        let original = groups.len();
        groups.retain(|candidate| candidate != group);
        (groups.len() != original, groups.is_empty())
    };
    if empty {
        hooks.remove("Stop");
    }
    Ok(changed)
}

fn stop_groups_mut(document: &mut Value) -> Result<&mut Vec<Value>, LifecycleError> {
    let root = document
        .as_object_mut()
        .ok_or(LifecycleError::InvalidDocument)?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(LifecycleError::InvalidDocument)?;
    hooks
        .entry("Stop")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or(LifecycleError::InvalidDocument)
}

fn hook_document_is_empty(document: &Value) -> bool {
    let Some(root) = document.as_object() else {
        return false;
    };
    root.is_empty()
        || (root.len() == 1
            && root
                .get("hooks")
                .and_then(Value::as_object)
                .is_some_and(Map::is_empty))
}

fn digest(bytes: &[u8]) -> String {
    let value = Sha256::digest(bytes);
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, LifecycleError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(LifecycleError::UnsafeResource);
        }
        Ok(metadata) if metadata.len() > MAX_DOCUMENT_BYTES as u64 => {
            return Err(LifecycleError::InvalidDocument);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(LifecycleError::FileSystem),
    }
    fs::read(path)
        .map(Some)
        .map_err(|_| LifecycleError::FileSystem)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), LifecycleError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(LifecycleError::InvalidDocument);
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(LifecycleError::UnsafeResource);
    }
    let parent = path.parent().ok_or(LifecycleError::InvalidLayout)?;
    fs::create_dir_all(parent).map_err(|_| LifecycleError::FileSystem)?;
    let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".codex-notifier-{}-{sequence}.tmp",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| LifecycleError::FileSystem)?;
    let write_result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| LifecycleError::FileSystem);
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    replace_file(&temporary, path)
}

fn replace_file(temporary: &Path, target: &Path) -> Result<(), LifecycleError> {
    if !target.exists() {
        return fs::rename(temporary, target).map_err(|_| LifecycleError::FileSystem);
    }
    let backup = target.with_extension(format!("codex-notifier-backup-{}", std::process::id()));
    if backup.exists() {
        return Err(LifecycleError::OwnershipConflict);
    }
    fs::rename(target, &backup).map_err(|_| LifecycleError::FileSystem)?;
    if fs::rename(temporary, target).is_err() {
        let _ = fs::rename(&backup, target);
        let _ = fs::remove_file(temporary);
        return Err(LifecycleError::FileSystem);
    }
    fs::remove_file(backup).map_err(|_| LifecycleError::FileSystem)
}

fn remove_owned_file(path: &Path) -> Result<(), LifecycleError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(LifecycleError::UnsafeResource)
        }
        Ok(_) => fs::remove_file(path).map_err(|_| LifecycleError::FileSystem),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(LifecycleError::FileSystem),
    }
}

struct FileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

impl FileSnapshot {
    fn capture(path: &Path) -> Result<Self, LifecycleError> {
        Ok(Self {
            path: path.to_owned(),
            bytes: read_optional_file(path)?,
        })
    }

    fn restore(&self) -> Result<(), LifecycleError> {
        match self.bytes.as_deref() {
            Some(bytes) => write_atomic(&self.path, bytes),
            None => remove_owned_file(&self.path),
        }
    }
}

fn restore_snapshots(snapshots: &[FileSnapshot]) -> Result<(), LifecycleError> {
    for snapshot in snapshots.iter().rev() {
        snapshot.restore()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn layout(directory: &TempDir) -> LifecycleLayout {
        let root = directory.path().join("install");
        LifecycleLayout::new(
            &root,
            root.join("codex-notifier"),
            directory.path().join("config/config.toml"),
            directory.path().join("codex/hooks.json"),
            directory.path().join("state/install-manifest.json"),
        )
        .expect("layout")
    }

    fn platform(directory: &TempDir) -> PlatformOwnership {
        PlatformOwnership::MacOs {
            launch_agent: directory.path().join("LaunchAgents/product.plist"),
            launch_agent_label: "io.github.leopardrich.codex-notifier".to_owned(),
        }
    }

    #[test]
    fn install_is_idempotent_and_preserves_unrelated_hooks() {
        let directory = TempDir::new().expect("temporary directory");
        let layout = layout(&directory);
        let existing = json!({
            "description": "user hooks",
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "user-command"}]}]}
        });
        write_atomic(
            layout.hooks_file(),
            &serde_json::to_vec_pretty(&existing).expect("JSON"),
        )
        .expect("existing hooks");
        let group = stop_hook_group("product-command", None);

        let first =
            install_managed_documents(&layout, "0.1.0", group.clone(), platform(&directory))
                .expect("first install");
        let second =
            install_managed_documents(&layout, "0.1.0", group.clone(), platform(&directory))
                .expect("repeat install");

        assert_eq!(first.hook_group, second.hook_group);
        let hooks: Value = serde_json::from_slice(
            &read_optional_file(layout.hooks_file())
                .expect("read hooks")
                .expect("hooks exist"),
        )
        .expect("hook JSON");
        let groups = hooks["hooks"]["Stop"].as_array().expect("Stop groups");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.iter().filter(|value| *value == &group).count(), 1);
        assert_eq!(hooks["description"], "user hooks");
    }

    #[test]
    fn uninstall_removes_only_exact_owned_content() {
        let directory = TempDir::new().expect("temporary directory");
        let layout = layout(&directory);
        let group = stop_hook_group("product-command", None);
        let manifest = install_managed_documents(&layout, "0.1.0", group, platform(&directory))
            .expect("install");

        let report = uninstall_managed_documents(&layout, &manifest).expect("uninstall");

        assert_eq!(report.hook, RemovalDisposition::Removed);
        assert_eq!(report.config, RemovalDisposition::Removed);
        assert!(!layout.hooks_file().exists());
        assert!(!layout.config_file().exists());
        assert!(!layout.manifest_file().exists());
    }

    #[test]
    fn uninstall_preserves_user_modified_config_and_hook_group() {
        let directory = TempDir::new().expect("temporary directory");
        let layout = layout(&directory);
        let group = stop_hook_group("product-command", None);
        let manifest = install_managed_documents(&layout, "0.1.0", group, platform(&directory))
            .expect("install");
        write_atomic(
            layout.config_file(),
            b"config_version = 1\n[desktop]\nquiet_hours = true\n",
        )
        .expect("modify config");
        let mut hooks: Value = serde_json::from_slice(
            &read_optional_file(layout.hooks_file())
                .expect("read hooks")
                .expect("hooks exist"),
        )
        .expect("hook JSON");
        hooks["hooks"]["Stop"][0]["hooks"][0]["timeout"] = Value::from(4);
        write_atomic(
            layout.hooks_file(),
            &serde_json::to_vec_pretty(&hooks).expect("JSON"),
        )
        .expect("modify hooks");

        let report = uninstall_managed_documents(&layout, &manifest).expect("uninstall");

        assert_eq!(report.hook, RemovalDisposition::Preserved);
        assert_eq!(report.config, RemovalDisposition::Preserved);
        assert!(layout.hooks_file().exists());
        assert!(layout.config_file().exists());
        assert!(!layout.manifest_file().exists());
    }

    #[test]
    fn manifest_cannot_redirect_uninstall_outside_layout() {
        let directory = TempDir::new().expect("temporary directory");
        let layout = layout(&directory);
        let group = stop_hook_group("product-command", None);
        let mut manifest = install_managed_documents(&layout, "0.1.0", group, platform(&directory))
            .expect("install");
        manifest.config_file = directory.path().join("unrelated");
        assert!(matches!(
            uninstall_managed_documents(&layout, &manifest),
            Err(LifecycleError::OwnershipConflict)
        ));
    }
}
