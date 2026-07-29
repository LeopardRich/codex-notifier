//! Operating-system installation, startup, and notification identity resources.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use codex_notifier_config::ConfigPaths;
use thiserror::Error;

use crate::lifecycle::{InstallManifest, LifecycleLayout, PlatformOwnership};

const PRODUCT_NAME: &str = "Codex Notifier";
const PRODUCT_FILE_NAME: &str = "codex-notifier";
#[cfg(target_os = "macos")]
const MACOS_LABEL: &str = "io.github.leopardrich.codex-notifier";
const WINDOWS_APP_ID: &str = "LeopardRich.CodexNotifier";
const WINDOWS_RUN_VALUE: &str = "CodexNotifier";

/// Exact platform paths used by installation and startup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformPaths {
    lifecycle: LifecycleLayout,
    state_dir: PathBuf,
    platform_resource: PathBuf,
}

impl PlatformPaths {
    /// Returns neutral lifecycle paths.
    #[must_use]
    pub const fn lifecycle(&self) -> &LifecycleLayout {
        &self.lifecycle
    }

    /// Returns the persistent state directory, which uninstall preserves.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Returns the Start Menu shortcut or `LaunchAgent` plist path.
    #[must_use]
    pub fn platform_resource(&self) -> &Path {
        &self.platform_resource
    }
}

/// Stable platform installation failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PlatformError {
    /// Current-user environment paths are unavailable or unsafe.
    #[error("platform installation paths are invalid")]
    InvalidPaths,
    /// A resource exists without a matching ownership manifest.
    #[error("platform installation resource ownership conflicts")]
    OwnershipConflict,
    /// The source executable or application bundle is invalid.
    #[error("platform installation source is invalid")]
    InvalidSource,
    /// A filesystem operation failed.
    #[error("platform installation filesystem operation failed")]
    FileSystem,
    /// Native identity or startup registration failed.
    #[error("platform startup registration failed")]
    Registration,
    /// The current host is outside the Windows/macOS desktop scope.
    #[error("platform desktop installation is unsupported")]
    UnsupportedPlatform,
    /// The running installed Windows executable cannot remove itself.
    #[error("run uninstall from the downloaded release executable")]
    ExternalUninstallerRequired,
}

impl PlatformError {
    /// Returns a stable machine-readable classification.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidPaths => "install_platform_paths_invalid",
            Self::OwnershipConflict => "install_platform_ownership_conflict",
            Self::InvalidSource => "install_platform_source_invalid",
            Self::FileSystem => "install_platform_filesystem_failed",
            Self::Registration => "install_platform_registration_failed",
            Self::UnsupportedPlatform => "install_platform_unsupported",
            Self::ExternalUninstallerRequired => "uninstall_external_binary_required",
        }
    }
}

/// Rollback-capable platform installation pending manifest commit.
pub struct PlatformTransaction {
    paths: PlatformPaths,
    ownership: PlatformOwnership,
    backup_root: Option<PathBuf>,
    previous_owned: bool,
    committed: bool,
}

impl PlatformTransaction {
    /// Returns the resources to store in the ownership manifest.
    #[must_use]
    pub const fn ownership(&self) -> &PlatformOwnership {
        &self.ownership
    }

    /// Commits the swap and removes its bounded backup.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the old owned artifact cannot be removed.
    pub fn commit(mut self) -> Result<(), PlatformError> {
        if let Some(backup) = self.backup_root.as_deref() {
            remove_tree(backup)?;
        }
        self.backup_root = None;
        self.committed = true;
        Ok(())
    }

    /// Restores the previous artifact and platform registration.
    ///
    /// # Errors
    ///
    /// Returns a stable filesystem or registration error.
    pub fn rollback(mut self) -> Result<(), PlatformError> {
        rollback_transaction(&mut self)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for PlatformTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = rollback_transaction(self);
        }
    }
}

/// Resolves exact per-user installation paths for the current desktop OS.
///
/// # Errors
///
/// Returns invalid paths when a required absolute environment base is absent.
pub fn current_platform_paths(
    config: &ConfigPaths,
    state_dir: &Path,
) -> Result<PlatformPaths, PlatformError> {
    #[cfg(windows)]
    {
        let local = required_absolute_env("LOCALAPPDATA")?;
        let roaming = required_absolute_env("APPDATA")?;
        let home = required_absolute_env("USERPROFILE")?;
        let install_root = local.join("Programs").join(PRODUCT_NAME);
        let installed_executable = install_root.join(format!("{PRODUCT_FILE_NAME}.exe"));
        let shortcut = roaming
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(format!("{PRODUCT_NAME}.lnk"));
        let lifecycle = LifecycleLayout::new(
            &install_root,
            installed_executable,
            config.config_file(),
            home.join(".codex").join("hooks.json"),
            config.state_dir().join("install-manifest.json"),
        )
        .map_err(|_| PlatformError::InvalidPaths)?;
        Ok(PlatformPaths {
            lifecycle,
            state_dir: state_dir.to_owned(),
            platform_resource: shortcut,
        })
    }
    #[cfg(target_os = "macos")]
    {
        let home = required_absolute_env("HOME")?;
        let install_root = home
            .join("Applications")
            .join(format!("{PRODUCT_NAME}.app"));
        let installed_executable = install_root
            .join("Contents")
            .join("MacOS")
            .join(PRODUCT_FILE_NAME);
        let launch_agent = home
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{MACOS_LABEL}.plist"));
        let lifecycle = LifecycleLayout::new(
            &install_root,
            installed_executable,
            config.config_file(),
            home.join(".codex").join("hooks.json"),
            config.state_dir().join("install-manifest.json"),
        )
        .map_err(|_| PlatformError::InvalidPaths)?;
        Ok(PlatformPaths {
            lifecycle,
            state_dir: state_dir.to_owned(),
            platform_resource: launch_agent,
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (config, state_dir);
        Err(PlatformError::UnsupportedPlatform)
    }
}

/// Stages and activates platform resources while retaining rollback state.
///
/// # Errors
///
/// Returns a stable conflict, source, filesystem, or registration error.
pub fn begin_platform_install(
    paths: &PlatformPaths,
    previous: Option<&InstallManifest>,
) -> Result<PlatformTransaction, PlatformError> {
    let previous_owned = previous.is_some();
    let ownership = expected_ownership(paths);
    if let Some(manifest) = previous {
        validate_ownership(paths, manifest.platform())?;
    } else {
        ensure_resources_absent(paths)?;
    }
    let install_root = paths.lifecycle.install_root();
    if install_root.exists() && !previous_owned {
        return Err(PlatformError::OwnershipConflict);
    }
    let stage = stage_path(install_root, "stage")?;
    let backup = stage_path(install_root, "backup")?;
    if stage.exists() || backup.exists() {
        return Err(PlatformError::OwnershipConflict);
    }
    fs::create_dir_all(stage.parent().ok_or(PlatformError::InvalidPaths)?)
        .map_err(|_| PlatformError::FileSystem)?;
    stage_artifact(paths, &stage)?;
    let backup_root = if install_root.exists() {
        fs::rename(install_root, &backup).map_err(|_| PlatformError::FileSystem)?;
        Some(backup)
    } else {
        None
    };
    if fs::rename(&stage, install_root).is_err() {
        if let Some(backup) = backup_root.as_deref() {
            let _ = fs::rename(backup, install_root);
        }
        let _ = remove_tree(&stage);
        return Err(PlatformError::FileSystem);
    }
    let mut transaction = PlatformTransaction {
        paths: paths.clone(),
        ownership,
        backup_root,
        previous_owned,
        committed: false,
    };
    if let Err(error) = activate_resources(paths) {
        let _ = rollback_transaction(&mut transaction);
        transaction.committed = true;
        return Err(error);
    }
    Ok(transaction)
}

/// Deactivates exact owned startup/identity resources and removes the artifact.
///
/// # Errors
///
/// Returns a stable ownership, registration, or filesystem error.
pub fn uninstall_platform(
    paths: &PlatformPaths,
    manifest: &InstallManifest,
) -> Result<(), PlatformError> {
    validate_ownership(paths, manifest.platform())?;
    #[cfg(windows)]
    if std::env::current_exe()
        .map_err(|_| PlatformError::InvalidSource)?
        .starts_with(paths.lifecycle.install_root())
    {
        return Err(PlatformError::ExternalUninstallerRequired);
    }
    deactivate_resources(paths)?;
    remove_tree(paths.lifecycle.install_root())
}

/// Returns whether the exact startup resource exists.
#[must_use]
pub fn startup_resource_exists(paths: &PlatformPaths) -> bool {
    #[cfg(windows)]
    {
        windows_resources_exist(paths)
    }
    #[cfg(target_os = "macos")]
    {
        paths.platform_resource.exists()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = paths;
        false
    }
}

fn rollback_transaction(transaction: &mut PlatformTransaction) -> Result<(), PlatformError> {
    let _ = deactivate_resources(&transaction.paths);
    remove_tree(transaction.paths.lifecycle.install_root())?;
    if let Some(backup) = transaction.backup_root.take() {
        fs::rename(&backup, transaction.paths.lifecycle.install_root())
            .map_err(|_| PlatformError::FileSystem)?;
    }
    if transaction.previous_owned {
        activate_resources(&transaction.paths)?;
    }
    Ok(())
}

fn expected_ownership(paths: &PlatformPaths) -> PlatformOwnership {
    #[cfg(windows)]
    {
        PlatformOwnership::Windows {
            start_menu_shortcut: paths.platform_resource.clone(),
            notification_registry_key: format!(r"Software\Classes\AppUserModelId\{WINDOWS_APP_ID}"),
            startup_value_name: WINDOWS_RUN_VALUE.to_owned(),
        }
    }
    #[cfg(target_os = "macos")]
    {
        PlatformOwnership::MacOs {
            launch_agent: paths.platform_resource.clone(),
            launch_agent_label: MACOS_LABEL.to_owned(),
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = paths;
        unreachable!("unsupported platform has no ownership")
    }
}

/// Returns the exact native resources that must be recorded in the manifest.
#[must_use]
pub fn platform_ownership(paths: &PlatformPaths) -> PlatformOwnership {
    expected_ownership(paths)
}

fn validate_ownership(
    paths: &PlatformPaths,
    ownership: &PlatformOwnership,
) -> Result<(), PlatformError> {
    if ownership == &expected_ownership(paths) {
        Ok(())
    } else {
        Err(PlatformError::OwnershipConflict)
    }
}

fn stage_path(root: &Path, purpose: &str) -> Result<PathBuf, PlatformError> {
    let parent = root.parent().ok_or(PlatformError::InvalidPaths)?;
    let stem = root
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or(PlatformError::InvalidPaths)?;
    #[cfg(target_os = "macos")]
    let name = format!("{stem}.{purpose}-{}.app", std::process::id());
    #[cfg(not(target_os = "macos"))]
    let name = format!("{stem}.{purpose}-{}", std::process::id());
    Ok(parent.join(name))
}

fn remove_tree(path: &Path) -> Result<(), PlatformError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(PlatformError::OwnershipConflict)
        }
        Ok(_) => fs::remove_dir_all(path).map_err(|_| PlatformError::FileSystem),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(PlatformError::FileSystem),
    }
}

fn required_absolute_env(name: &str) -> Result<PathBuf, PlatformError> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or(PlatformError::InvalidPaths)?;
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(PlatformError::InvalidPaths)
    }
}

#[cfg(windows)]
fn stage_artifact(paths: &PlatformPaths, stage: &Path) -> Result<(), PlatformError> {
    let source = std::env::current_exe().map_err(|_| PlatformError::InvalidSource)?;
    if source.starts_with(paths.lifecycle.install_root()) {
        return Err(PlatformError::InvalidSource);
    }
    fs::create_dir_all(stage).map_err(|_| PlatformError::FileSystem)?;
    fs::copy(&source, stage.join(format!("{PRODUCT_FILE_NAME}.exe")))
        .map_err(|_| PlatformError::FileSystem)?;
    fs::write(
        stage.join(format!("{PRODUCT_FILE_NAME}.ico")),
        windows_icon(),
    )
    .map_err(|_| PlatformError::FileSystem)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn stage_artifact(_paths: &PlatformPaths, stage: &Path) -> Result<(), PlatformError> {
    let executable = std::env::current_exe().map_err(|_| PlatformError::InvalidSource)?;
    let source = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .filter(|path| path.extension().is_some_and(|value| value == "app"))
        .ok_or(PlatformError::InvalidSource)?;
    let verified = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(source)
        .status()
        .map_err(|_| PlatformError::InvalidSource)?;
    if !verified.success() {
        return Err(PlatformError::InvalidSource);
    }
    copy_tree(source, stage)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn stage_artifact(_paths: &PlatformPaths, _stage: &Path) -> Result<(), PlatformError> {
    Err(PlatformError::UnsupportedPlatform)
}

#[cfg(windows)]
fn ensure_resources_absent(paths: &PlatformPaths) -> Result<(), PlatformError> {
    if windows_resources_exist(paths) {
        Err(PlatformError::OwnershipConflict)
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn ensure_resources_absent(paths: &PlatformPaths) -> Result<(), PlatformError> {
    if paths.platform_resource.exists() {
        Err(PlatformError::OwnershipConflict)
    } else {
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn ensure_resources_absent(_paths: &PlatformPaths) -> Result<(), PlatformError> {
    Err(PlatformError::UnsupportedPlatform)
}

#[cfg(windows)]
fn activate_resources(paths: &PlatformPaths) -> Result<(), PlatformError> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    let executable = paths.lifecycle.installed_executable();
    let icon = paths
        .lifecycle
        .install_root()
        .join(format!("{PRODUCT_FILE_NAME}.ico"));
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let (identity, _) = current_user
        .create_subkey(format!(r"Software\Classes\AppUserModelId\{WINDOWS_APP_ID}"))
        .map_err(|_| PlatformError::Registration)?;
    identity
        .set_value("DisplayName", &PRODUCT_NAME)
        .and_then(|()| identity.set_value("IconBackgroundColor", &"0"))
        .and_then(|()| identity.set_value("IconUri", &icon.to_string_lossy().as_ref()))
        .map_err(|_| PlatformError::Registration)?;
    let (run, _) = current_user
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map_err(|_| PlatformError::Registration)?;
    let startup = format!("\"{}\" agent", executable.display());
    run.set_value(WINDOWS_RUN_VALUE, &startup)
        .map_err(|_| PlatformError::Registration)?;
    create_windows_shortcut(paths.platform_resource(), executable, &icon)?;
    Command::new(executable)
        .arg("agent")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| PlatformError::Registration)
}

#[cfg(windows)]
fn deactivate_resources(paths: &PlatformPaths) -> Result<(), PlatformError> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    if paths.platform_resource.exists() {
        fs::remove_file(paths.platform_resource()).map_err(|_| PlatformError::FileSystem)?;
    }
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    match current_user
        .delete_subkey_all(format!(r"Software\Classes\AppUserModelId\{WINDOWS_APP_ID}"))
    {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(PlatformError::Registration),
    }
    let run = current_user
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            winreg::enums::KEY_SET_VALUE,
        )
        .map_err(|_| PlatformError::Registration)?;
    match run.delete_value(WINDOWS_RUN_VALUE) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(PlatformError::Registration),
    }
}

#[cfg(windows)]
fn windows_resources_exist(paths: &PlatformPaths) -> bool {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let identity = current_user
        .open_subkey(format!(r"Software\Classes\AppUserModelId\{WINDOWS_APP_ID}"))
        .is_ok();
    let startup = current_user
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .and_then(|key| key.get_value::<String, _>(WINDOWS_RUN_VALUE))
        .is_ok();
    paths.platform_resource.exists() || identity || startup
}

#[cfg(windows)]
fn create_windows_shortcut(
    shortcut: &Path,
    executable: &Path,
    icon: &Path,
) -> Result<(), PlatformError> {
    const SCRIPT: &str = r#"
param([string]$ShortcutPath,[string]$TargetPath,[string]$IconPath)
$ErrorActionPreference = 'Stop'
$source = @'
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class CodexNotifierShortcut {
  [ComImport, Guid("00021401-0000-0000-C000-000000000046")] private class ShellLink {}
  [ComImport, Guid("000214F9-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  private interface IShellLinkW {
    [PreserveSig] int GetPath(IntPtr a,int b,IntPtr c,uint d); [PreserveSig] int GetIdList(out IntPtr a);
    [PreserveSig] int SetIdList(IntPtr a); [PreserveSig] int GetDescription(IntPtr a,int b);
    [PreserveSig] int SetDescription([MarshalAs(UnmanagedType.LPWStr)] string a);
    [PreserveSig] int GetWorkingDirectory(IntPtr a,int b);
    [PreserveSig] int SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string a);
    [PreserveSig] int GetArguments(IntPtr a,int b); [PreserveSig] int SetArguments([MarshalAs(UnmanagedType.LPWStr)] string a);
    [PreserveSig] int GetHotkey(out short a); [PreserveSig] int SetHotkey(short a);
    [PreserveSig] int GetShowCommand(out int a); [PreserveSig] int SetShowCommand(int a);
    [PreserveSig] int GetIconLocation(IntPtr a,int b,out int c);
    [PreserveSig] int SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string a,int b);
    [PreserveSig] int SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string a,uint b);
    [PreserveSig] int Resolve(IntPtr a,uint b); [PreserveSig] int SetPath([MarshalAs(UnmanagedType.LPWStr)] string a);
  }
  [StructLayout(LayoutKind.Sequential,Pack=4)] private struct PropertyKey { public Guid F; public uint P; public PropertyKey(Guid f,uint p){F=f;P=p;} }
  [StructLayout(LayoutKind.Explicit)] private struct PropVariant { [FieldOffset(0)] public ushort T; [FieldOffset(8)] public IntPtr V; }
  [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  private interface IPropertyStore { [PreserveSig] int GetCount(out uint a); [PreserveSig] int GetAt(uint a,out PropertyKey b); [PreserveSig] int GetValue(ref PropertyKey a,out PropVariant b); [PreserveSig] int SetValue(ref PropertyKey a,ref PropVariant b); [PreserveSig] int Commit(); }
  [DllImport("ole32.dll",PreserveSig=true)] private static extern int PropVariantClear(ref PropVariant v);
  public static void Create(string shortcut,string target,string icon) {
    var link=(IShellLinkW)new ShellLink();
    try {
      Marshal.ThrowExceptionForHR(link.SetPath(target)); Marshal.ThrowExceptionForHR(link.SetWorkingDirectory(System.IO.Path.GetDirectoryName(target)));
      Marshal.ThrowExceptionForHR(link.SetDescription("Codex Notifier")); Marshal.ThrowExceptionForHR(link.SetArguments("agent")); Marshal.ThrowExceptionForHR(link.SetIconLocation(icon,0));
      var store=(IPropertyStore)link; var key=new PropertyKey(new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"),5);
      var value=new PropVariant{T=31,V=Marshal.StringToCoTaskMemUni("LeopardRich.CodexNotifier")};
      try { Marshal.ThrowExceptionForHR(store.SetValue(ref key,ref value)); Marshal.ThrowExceptionForHR(store.Commit()); } finally { PropVariantClear(ref value); }
      ((IPersistFile)link).Save(shortcut,true);
    } finally { Marshal.FinalReleaseComObject(link); }
  }
}
'@
Add-Type -TypeDefinition $source -Language CSharp
[CodexNotifierShortcut]::Create($ShortcutPath,$TargetPath,$IconPath)
"#;
    let parent = shortcut.parent().ok_or(PlatformError::InvalidPaths)?;
    fs::create_dir_all(parent).map_err(|_| PlatformError::FileSystem)?;
    let script = pathsafe_temporary(executable.parent().ok_or(PlatformError::InvalidPaths)?)?;
    fs::write(&script, SCRIPT).map_err(|_| PlatformError::FileSystem)?;
    let status = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(&script)
        .args(["-ShortcutPath"])
        .arg(shortcut)
        .args(["-TargetPath"])
        .arg(executable)
        .args(["-IconPath"])
        .arg(icon)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| PlatformError::Registration)?;
    let _ = fs::remove_file(script);
    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::Registration)
    }
}

#[cfg(windows)]
fn pathsafe_temporary(parent: &Path) -> Result<PathBuf, PlatformError> {
    let path = parent.join(format!("shortcut-helper-{}.ps1", std::process::id()));
    if path.exists() {
        return Err(PlatformError::OwnershipConflict);
    }
    Ok(path)
}

#[cfg(windows)]
fn windows_icon() -> Vec<u8> {
    const SIZE: usize = 32;
    const SIZE_U8: u8 = 32;
    const SIZE_I32: i32 = 32;
    const PIXEL_BYTES: usize = SIZE * SIZE * 4;
    const PIXEL_BYTES_U32: u32 = 4_096;
    const MASK_BYTES: usize = SIZE * 4;
    const IMAGE_BYTES: usize = 40 + PIXEL_BYTES + MASK_BYTES;
    const IMAGE_BYTES_U32: u32 = 4_264;
    let mut bytes = Vec::with_capacity(6 + 16 + IMAGE_BYTES);
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&[SIZE_U8, SIZE_U8, 0, 0]);
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&32_u16.to_le_bytes());
    bytes.extend_from_slice(&IMAGE_BYTES_U32.to_le_bytes());
    bytes.extend_from_slice(&22_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&SIZE_I32.to_le_bytes());
    bytes.extend_from_slice(&(SIZE_I32 * 2).to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&32_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&PIXEL_BYTES_U32.to_le_bytes());
    bytes.extend_from_slice(&[0; 16]);
    for source_y in (0_i32..SIZE_I32).rev() {
        for x in 0_i32..SIZE_I32 {
            let dx = x - 15;
            let dy = source_y - 15;
            let inside = dx * dx + dy * dy <= 14 * 14;
            let bell = (9..=22).contains(&x)
                && ((9..=20).contains(&source_y) && (x - 15).abs() <= (source_y - 6) / 2)
                || ((11..=20).contains(&x) && (20..=22).contains(&source_y));
            let alert = (21..=26).contains(&x) && (5..=10).contains(&source_y);
            let (red, green, blue, alpha) = if !inside {
                (0, 0, 0, 0)
            } else if alert {
                (224, 62, 62, 255)
            } else if bell {
                (250, 250, 250, 255)
            } else {
                (37, 99, 112, 255)
            };
            bytes.extend_from_slice(&[blue, green, red, alpha]);
        }
    }
    bytes.extend_from_slice(&[0; MASK_BYTES]);
    bytes
}

#[cfg(target_os = "macos")]
fn activate_resources(paths: &PlatformPaths) -> Result<(), PlatformError> {
    let parent = paths
        .platform_resource
        .parent()
        .ok_or(PlatformError::InvalidPaths)?;
    fs::create_dir_all(parent).map_err(|_| PlatformError::FileSystem)?;
    fs::create_dir_all(paths.state_dir.join("logs")).map_err(|_| PlatformError::FileSystem)?;
    let plist = launch_agent_plist(paths.lifecycle.installed_executable(), &paths.state_dir);
    fs::write(&paths.platform_resource, plist).map_err(|_| PlatformError::FileSystem)?;
    let valid = Command::new("/usr/bin/plutil")
        .args(["-lint"])
        .arg(&paths.platform_resource)
        .status()
        .map_err(|_| PlatformError::Registration)?;
    if !valid.success() {
        return Err(PlatformError::Registration);
    }
    let domain = launch_domain()?;
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &format!("{domain}/{MACOS_LABEL}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let loaded = Command::new("/bin/launchctl")
        .args(["bootstrap", &domain])
        .arg(&paths.platform_resource)
        .status()
        .map_err(|_| PlatformError::Registration)?;
    if !loaded.success() {
        return Err(PlatformError::Registration);
    }
    let started = Command::new("/bin/launchctl")
        .args(["kickstart", "-k", &format!("{domain}/{MACOS_LABEL}")])
        .status()
        .map_err(|_| PlatformError::Registration)?;
    if started.success() {
        Ok(())
    } else {
        Err(PlatformError::Registration)
    }
}

#[cfg(target_os = "macos")]
fn deactivate_resources(paths: &PlatformPaths) -> Result<(), PlatformError> {
    let domain = launch_domain()?;
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &format!("{domain}/{MACOS_LABEL}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match fs::symlink_metadata(&paths.platform_resource) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(PlatformError::OwnershipConflict)
        }
        Ok(_) => fs::remove_file(&paths.platform_resource).map_err(|_| PlatformError::FileSystem),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(PlatformError::FileSystem),
    }
}

#[cfg(target_os = "macos")]
fn launch_domain() -> Result<String, PlatformError> {
    let output = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|_| PlatformError::Registration)?;
    if !output.status.success() {
        return Err(PlatformError::Registration);
    }
    let uid = String::from_utf8(output.stdout).map_err(|_| PlatformError::Registration)?;
    let uid = uid.trim();
    if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PlatformError::Registration);
    }
    Ok(format!("gui/{uid}"))
}

#[cfg(target_os = "macos")]
fn launch_agent_plist(executable: &Path, state_dir: &Path) -> String {
    let log_dir = state_dir.join("logs");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{MACOS_LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>agent</string></array>
  <key>RunAtLoad</key><true/>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        xml_escape(&executable.to_string_lossy()),
        xml_escape(&log_dir.join("agent.stdout.log").to_string_lossy()),
        xml_escape(&log_dir.join("agent.stderr.log").to_string_lossy()),
    )
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(target_os = "macos")]
fn copy_tree(source: &Path, target: &Path) -> Result<(), PlatformError> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = fs::symlink_metadata(source).map_err(|_| PlatformError::InvalidSource)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PlatformError::InvalidSource);
    }
    fs::create_dir(target).map_err(|_| PlatformError::FileSystem)?;
    fs::set_permissions(
        target,
        fs::Permissions::from_mode(metadata.permissions().mode()),
    )
    .map_err(|_| PlatformError::FileSystem)?;
    for entry in fs::read_dir(source).map_err(|_| PlatformError::FileSystem)? {
        let entry = entry.map_err(|_| PlatformError::FileSystem)?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(|_| PlatformError::FileSystem)?;
        if metadata.file_type().is_symlink() {
            return Err(PlatformError::InvalidSource);
        }
        if metadata.is_dir() {
            copy_tree(&source_path, &target_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &target_path).map_err(|_| PlatformError::FileSystem)?;
            fs::set_permissions(
                &target_path,
                fs::Permissions::from_mode(metadata.permissions().mode()),
            )
            .map_err(|_| PlatformError::FileSystem)?;
        } else {
            return Err(PlatformError::InvalidSource);
        }
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn activate_resources(_paths: &PlatformPaths) -> Result<(), PlatformError> {
    Err(PlatformError::UnsupportedPlatform)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn deactivate_resources(_paths: &PlatformPaths) -> Result<(), PlatformError> {
    Err(PlatformError::UnsupportedPlatform)
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn generated_icon_has_one_bounded_32_bit_image() {
        let icon = windows_icon();
        assert_eq!(icon.len(), 4_286);
        assert_eq!(&icon[0..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(&icon[6..10], &[32, 32, 0, 0]);
        assert_eq!(
            u32::from_le_bytes(icon[14..18].try_into().expect("size")),
            4_264
        );
        assert_eq!(
            u32::from_le_bytes(icon[18..22].try_into().expect("offset")),
            22
        );
        assert!(
            icon[22 + 40..22 + 40 + 4_096]
                .chunks_exact(4)
                .any(|pixel| pixel[3] == 255)
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    #[test]
    fn launch_agent_escapes_paths_and_uses_fixed_arguments() {
        let plist = launch_agent_plist(
            Path::new("/Users/test/Applications/A&B.app/Contents/MacOS/codex-notifier"),
            Path::new("/Users/test/Library/Application Support/codex-notifier"),
        );
        assert!(plist.contains("A&amp;B.app"));
        assert!(plist.contains("<string>agent</string>"));
        assert!(plist.contains(MACOS_LABEL));
        assert!(!plist.contains("A&B.app"));
    }
}
