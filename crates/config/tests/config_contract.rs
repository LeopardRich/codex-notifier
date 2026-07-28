//! Layering, migration, path, validation, and redaction contract tests.

use std::path::Path;

use codex_notifier_config::{
    CliOverrides, ConfigError, ConfigLoader, ErrorCode, IpcEndpoint, LogLevel, NotificationPrivacy,
    PathEnvironment, Platform, Role, StateDirectoryProbe,
};

#[derive(Clone, Copy)]
struct FixedProbe(bool);

impl StateDirectoryProbe for FixedProbe {
    fn is_writable(&self, _path: &Path) -> bool {
        self.0
    }
}

fn windows_paths() -> codex_notifier_config::ConfigPaths {
    PathEnvironment::new()
        .with_windows_app_data(r"C:\Users\fixture\AppData\Roaming")
        .with_windows_local_app_data(r"C:\Users\fixture\AppData\Local")
        .resolve(Platform::Windows)
        .expect("fixture paths")
}

fn load(
    user_toml: Option<&str>,
    profile_toml: Option<&str>,
    cli: CliOverrides,
) -> Result<codex_notifier_config::Config, ConfigError> {
    ConfigLoader::load(
        &windows_paths(),
        user_toml,
        profile_toml,
        cli,
        &FixedProbe(true),
    )
}

#[test]
fn four_layers_merge_in_documented_order() {
    let defaults = load(None, None, CliOverrides::new()).expect("default configuration");
    assert_eq!(defaults.config_version(), 1);
    assert_eq!(defaults.agent().role(), Role::Desktop);
    assert_eq!(defaults.agent().profile(), "default");
    assert_eq!(defaults.storage().max_queue_entries(), 1_000);
    assert_eq!(defaults.desktop().privacy(), NotificationPrivacy::Private);

    let user = r#"
config_version = 1

[agent]
profile = "user"
shutdown_timeout_ms = 1000

[desktop]
privacy = "public"

[storage]
max_queue_entries = 10
"#;
    let profile = r#"
config_version = 1

[agent]
profile = "profile"

[desktop]
privacy = "private"

[storage]
max_queue_entries = 20
"#;
    let cli = CliOverrides::new()
        .with_profile("cli")
        .with_privacy("public")
        .with_max_queue_entries(30)
        .with_log_level("debug");
    let config = load(Some(user), Some(profile), cli).expect("merged configuration");

    assert_eq!(config.agent().profile(), "cli");
    assert_eq!(config.agent().shutdown_timeout_ms(), 1_000);
    assert_eq!(config.desktop().privacy(), NotificationPrivacy::Public);
    assert_eq!(config.storage().max_queue_entries(), 30);
    assert_eq!(config.logging().level(), LogLevel::Debug);
}

#[test]
fn resolves_windows_and_macos_paths_without_host_environment() {
    let windows = windows_paths();
    assert_path(
        windows.config_file(),
        r"C:\Users\fixture\AppData\Roaming\codex-notifier\config.toml",
    );
    assert_path(
        windows.state_dir(),
        r"C:\Users\fixture\AppData\Local\codex-notifier",
    );
    assert_path(
        windows.log_dir(),
        r"C:\Users\fixture\AppData\Local\codex-notifier\logs",
    );

    let macos = PathEnvironment::new()
        .with_home("/Users/fixture")
        .resolve(Platform::MacOs)
        .expect("macOS paths");
    assert_path(
        macos.config_file(),
        "/Users/fixture/Library/Application Support/codex-notifier/config.toml",
    );
    assert_path(
        macos.state_dir(),
        "/Users/fixture/Library/Application Support/codex-notifier/state",
    );
    assert_path(
        macos.log_dir(),
        "/Users/fixture/Library/Logs/codex-notifier",
    );
}

#[test]
fn resolves_explicit_and_fallback_xdg_paths() {
    let explicit = PathEnvironment::new()
        .with_home("/home/fixture")
        .with_xdg_config_home("/srv/config")
        .with_xdg_state_home("/srv/state")
        .resolve(Platform::Xdg)
        .expect("explicit XDG paths");
    assert_path(
        explicit.config_file(),
        "/srv/config/codex-notifier/config.toml",
    );
    assert_path(explicit.state_dir(), "/srv/state/codex-notifier");

    let fallback = PathEnvironment::new()
        .with_home("/home/fixture")
        .resolve(Platform::Xdg)
        .expect("fallback XDG paths");
    assert_path(
        fallback.config_file(),
        "/home/fixture/.config/codex-notifier/config.toml",
    );
    assert_path(
        fallback.state_dir(),
        "/home/fixture/.local/state/codex-notifier",
    );
    assert_path(
        fallback.log_dir(),
        "/home/fixture/.local/state/codex-notifier/logs",
    );
}

#[test]
fn path_bases_must_be_present_and_target_platform_absolute() {
    assert_eq!(
        PathEnvironment::new().resolve(Platform::Windows),
        Err(ConfigError::MissingPathBase)
    );
    assert_eq!(
        PathEnvironment::new()
            .with_windows_app_data("relative")
            .with_windows_local_app_data(r"C:\state")
            .resolve(Platform::Windows),
        Err(ConfigError::MissingPathBase)
    );
    assert_eq!(
        PathEnvironment::new()
            .with_home("/home/fixture")
            .with_xdg_config_home("relative")
            .resolve(Platform::Xdg),
        Err(ConfigError::MissingPathBase)
    );
}

#[test]
fn migrates_supported_version_zero_documents() {
    let config = load(
        Some("config_version = 0\nrole = \"relay\"\nssh_host = \"desktop-home\""),
        None,
        CliOverrides::new(),
    )
    .expect("explicit v0 migration");
    assert_eq!(config.config_version(), 1);
    assert_eq!(config.agent().role(), Role::Relay);
    assert_eq!(config.relay().ssh_host_alias(), Some("desktop-home"));

    let implicit =
        load(Some("role = \"desktop\""), None, CliOverrides::new()).expect("implicit v0 migration");
    assert_eq!(implicit.agent().role(), Role::Desktop);
}

#[test]
fn classifies_version_and_shape_failures_without_source_values() {
    let cases = [
        ("[agent]\nrole = \"desktop\"", ConfigError::MissingVersion),
        ("config_version = 2", ConfigError::UnsupportedVersion),
        ("config_version = \"one\"", ConfigError::Malformed),
        (
            "config_version = 1\nunknown = \"secret-value\"",
            ConfigError::Malformed,
        ),
        (
            "config_version = 0\nrole = \"desktop\"\nextra = true",
            ConfigError::MigrationFailed,
        ),
    ];
    for (document, expected) in cases {
        let error = load(Some(document), None, CliOverrides::new()).expect_err("must reject");
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("secret-value"));
    }
}

#[test]
fn invalid_role_endpoint_and_relay_settings_have_distinct_errors() {
    assert_error(
        CliOverrides::new().with_role("automatic"),
        &ConfigError::InvalidRole,
    );
    assert_error(
        CliOverrides::new().with_ipc_endpoint("pipe:arbitrary"),
        &ConfigError::InvalidEndpoint,
    );
    assert_error(
        CliOverrides::new().with_role("relay"),
        &ConfigError::MissingRelayHost,
    );
    assert_error(
        CliOverrides::new()
            .with_role("relay")
            .with_relay_host("-invalid"),
        &ConfigError::InvalidRelayHost,
    );

    let relay = load(
        None,
        None,
        CliOverrides::new()
            .with_role("relay")
            .with_relay_host("desktop.example_1")
            .with_ipc_endpoint("name:relay_1"),
    )
    .expect("valid relay configuration");
    assert_eq!(relay.agent().role(), Role::Relay);
    assert_eq!(relay.agent().ipc_endpoint().name(), Some("relay_1"));
    assert!(matches!(
        relay.agent().ipc_endpoint(),
        IpcEndpoint::Named(_)
    ));
}

#[test]
fn rejects_unwritable_state_directory_with_stable_code() {
    let error = ConfigLoader::load(
        &windows_paths(),
        None,
        None,
        CliOverrides::new(),
        &FixedProbe(false),
    )
    .expect_err("unwritable state must fail");
    assert_eq!(error, ConfigError::UnwritableStateDirectory);
    assert_eq!(error.code(), ErrorCode::UnwritableStateDirectory);
    assert_eq!(error.code().as_str(), "config_state_unwritable");
}

#[test]
fn prohibited_sensitive_fields_are_rejected_and_diagnostics_are_redacted() {
    for key in [
        "private_key",
        "private-key",
        "ssh_private_key",
        "access_token",
        "raw_payload",
        "event_payload",
        "prompt",
        "model_output",
    ] {
        let document = format!("config_version = 1\n[logging]\n{key} = \"do-not-log-this\"");
        let error = load(Some(&document), None, CliOverrides::new()).expect_err("must reject");
        assert_eq!(error, ConfigError::SensitiveField, "key {key}");
        assert!(!error.to_string().contains("do-not-log-this"));
    }

    let config = load(
        None,
        None,
        CliOverrides::new()
            .with_role("relay")
            .with_relay_host("private-host-alias")
            .with_state_dir(r"D:\private\state"),
    )
    .expect("configuration for redaction check");
    let debug = format!("{config:?}");
    assert!(!debug.contains("private-host-alias"));
    assert!(!debug.contains("private\\state"));
    assert!(!debug.contains("AppData"));
}

#[test]
fn validates_identifier_numeric_and_path_boundaries() {
    let profile_64 = "a".repeat(64);
    let endpoint_64 = "e".repeat(64);
    let valid = load(
        Some(
            "config_version = 1\n[agent]\nshutdown_timeout_ms = 100\n[relay]\nconnect_timeout_ms = 120000\n[storage]\nmax_queue_entries = 100000",
        ),
        None,
        CliOverrides::new()
            .with_profile(profile_64)
            .with_ipc_endpoint(format!("name:{endpoint_64}")),
    )
    .expect("inclusive boundaries");
    assert_eq!(valid.agent().shutdown_timeout_ms(), 100);
    assert_eq!(valid.storage().max_queue_entries(), 100_000);

    assert_error(
        CliOverrides::new().with_profile("a".repeat(65)),
        &ConfigError::InvalidValue,
    );
    assert_error(
        CliOverrides::new().with_ipc_endpoint(format!("name:{}", "e".repeat(65))),
        &ConfigError::InvalidEndpoint,
    );
    assert_error(
        CliOverrides::new().with_max_queue_entries(0),
        &ConfigError::InvalidValue,
    );
    assert_error(
        CliOverrides::new().with_state_dir("relative/state"),
        &ConfigError::InvalidValue,
    );
}

fn assert_error(cli: CliOverrides, expected: &ConfigError) {
    assert_eq!(&load(None, None, cli).expect_err("must reject"), expected);
}

fn assert_path(actual: &Path, expected: &str) {
    assert_eq!(actual.to_str(), Some(expected));
}
