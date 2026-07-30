//! Relay Codex hook installation and exact removal contracts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

struct IsolatedEnvironment {
    _directory: TempDir,
    home: PathBuf,
    config_base: PathBuf,
    state_base: PathBuf,
    config_file: PathBuf,
    state_dir: PathBuf,
}

impl IsolatedEnvironment {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary directory");
        let home = directory.path().join("home");
        let config_base = directory.path().join("config");
        let state_base = directory.path().join("state");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&config_base).expect("config base");
        fs::create_dir_all(&state_base).expect("state base");

        #[cfg(windows)]
        let (config_file, state_dir) = (
            config_base.join("codex-notifier").join("config.toml"),
            state_base.join("codex-notifier"),
        );
        #[cfg(target_os = "macos")]
        let (config_file, state_dir) = {
            let root = home
                .join("Library")
                .join("Application Support")
                .join("codex-notifier");
            (root.join("config.toml"), root.join("state"))
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let (config_file, state_dir) = (
            config_base.join("codex-notifier").join("config.toml"),
            state_base.join("codex-notifier"),
        );
        fs::create_dir_all(config_file.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_file,
            concat!(
                "config_version = 1\n",
                "[agent]\nrole = \"relay\"\nprofile = \"default\"\n",
                "[relay]\nssh_host_alias = \"codex-notifier-desktop\"\n",
            ),
        )
        .expect("relay config");

        Self {
            _directory: directory,
            home,
            config_base,
            state_base,
            config_file,
            state_dir,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codex-notifier"));
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("APPDATA", &self.config_base)
            .env("LOCALAPPDATA", &self.state_base)
            .env("XDG_CONFIG_HOME", &self.config_base)
            .env("XDG_STATE_HOME", &self.state_base);
        command
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.command()
            .args(arguments)
            .output()
            .expect("codex-notifier command")
    }

    fn hooks_file(&self) -> PathBuf {
        self.home.join(".codex").join("hooks.json")
    }

    fn manifest_file(&self) -> PathBuf {
        self.state_dir.join("relay-hook-manifest.json")
    }
}

fn stop_groups(path: &Path) -> Vec<Value> {
    let document: Value =
        serde_json::from_slice(&fs::read(path).expect("hooks document")).expect("hooks JSON");
    document["hooks"]["Stop"]
        .as_array()
        .expect("Stop groups")
        .clone()
}

#[test]
fn relay_hook_install_is_idempotent_and_uninstall_preserves_user_files() {
    let environment = IsolatedEnvironment::new();
    let original_config = fs::read(&environment.config_file).expect("original config");
    fs::create_dir_all(environment.hooks_file().parent().expect("hooks parent"))
        .expect("hooks directory");
    let unrelated = json!({
        "hooks": [{
            "type": "command",
            "command": "existing-user-hook"
        }]
    });
    fs::write(
        environment.hooks_file(),
        serde_json::to_vec_pretty(&json!({ "hooks": { "Stop": [unrelated] } }))
            .expect("initial hooks JSON"),
    )
    .expect("initial hooks");

    let first = environment.run(&["hook", "install", "--codex-version", "0.144.5"]);
    assert!(
        first.status.success(),
        "first install failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_output = String::from_utf8(first.stdout).expect("install output");
    assert!(first_output.contains("hook_installed=true\nupgraded=false\n"));
    assert!(environment.manifest_file().is_file());
    assert_eq!(
        fs::read(&environment.config_file).expect("config"),
        original_config
    );

    let groups = stop_groups(&environment.hooks_file());
    assert_eq!(groups.len(), 2);
    let owned_command = groups[1]["hooks"][0]["command"]
        .as_str()
        .expect("owned command");
    assert!(owned_command.contains("emit"));
    assert!(owned_command.contains("task-completed"));
    assert!(owned_command.contains("--host-label"));
    assert!(owned_command.contains("remote"));
    assert!(!owned_command.contains("existing-user-hook"));
    #[cfg(windows)]
    assert!(
        groups[1]["hooks"][0]["commandWindows"]
            .as_str()
            .is_some_and(|command| command.starts_with("powershell.exe "))
    );

    let second = environment.run(&["hook", "install", "--codex-version", "0.144.5"]);
    assert!(second.status.success());
    assert!(
        String::from_utf8(second.stdout)
            .expect("upgrade output")
            .contains("upgraded=true")
    );
    assert_eq!(stop_groups(&environment.hooks_file()).len(), 2);

    let status = environment.run(&["status", "--format", "json"]);
    assert_eq!(status.status.code(), Some(12));
    let status: Value = serde_json::from_slice(&status.stdout).expect("status JSON");
    assert_eq!(status["role"], "relay");
    assert_eq!(status["installed"], true);
    assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));

    let uninstall = environment.run(&["hook", "uninstall"]);
    assert!(
        uninstall.status.success(),
        "uninstall failed: {}",
        String::from_utf8_lossy(&uninstall.stderr)
    );
    let uninstall_output = String::from_utf8(uninstall.stdout).expect("uninstall output");
    assert!(uninstall_output.contains("hook_removed=true"));
    assert!(uninstall_output.contains("config_preserved=true"));
    assert!(!environment.manifest_file().exists());
    assert_eq!(
        fs::read(&environment.config_file).expect("config"),
        original_config
    );
    let remaining = stop_groups(&environment.hooks_file());
    assert_eq!(remaining, vec![unrelated]);

    let second_uninstall = environment.run(&["hook", "uninstall"]);
    assert!(second_uninstall.status.success());
    assert!(
        String::from_utf8(second_uninstall.stdout)
            .expect("idempotent uninstall output")
            .contains("hook_removed=false")
    );

    let status = environment.run(&["status", "--format", "json"]);
    let status: Value = serde_json::from_slice(&status.stdout).expect("status JSON");
    assert_eq!(status["installed"], false);
    assert_eq!(status["version"], "none");
}

#[test]
fn relay_hook_install_rejects_a_desktop_configuration() {
    let environment = IsolatedEnvironment::new();
    fs::write(
        &environment.config_file,
        "config_version = 1\n[agent]\nrole = \"desktop\"\n",
    )
    .expect("desktop config");

    let output = environment.run(&["hook", "install", "--codex-version", "0.144.5"]);
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(
        String::from_utf8(output.stderr).expect("error output"),
        "install_relay_role_required\n"
    );
    assert!(!environment.hooks_file().exists());
    assert!(!environment.manifest_file().exists());
}
