//! Explicit interactive macOS `UserNotifications` smoke test.

#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use codex_notifier_native_notification::{
    CODEX_NOTIFIER_BUNDLE_ID, MacOsNotificationBackend, NativeNotificationAdapter,
    NotificationBackend, NotificationContentPolicy, NotificationPolicy, NotificationStatus,
};
use tempfile::tempdir;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const BUNDLED_SMOKE_ENV: &str = "CODEX_NOTIFIER_BUNDLED_MACOS_SMOKE";
const TEST_NAME: &str = "displays_task_completion_and_approval_request_notifications";
const NO_GUI_TEST_NAME: &str = "reports_real_no_gui_session";
const EXECUTABLE_NAME: &str = "codex-notifier-macos-smoke";

fn event(kind: EventKind, id: &str, urgency: Urgency) -> CanonicalEvent {
    let occurred_at =
        OffsetDateTime::parse("2026-07-29T06:00:00.000Z", &Rfc3339).expect("valid smoke time");
    CanonicalEvent::new(
        EventId::parse(id).expect("UUIDv7 smoke ID"),
        kind,
        occurred_at,
        EventSource::new("smoke-test", None, None).expect("safe source"),
        Presentation::new("not displayed", "not displayed", urgency, Privacy::Private)
            .expect("safe presentation"),
        None,
        Extensions::new(BTreeMap::new()).expect("empty extensions"),
        occurred_at,
    )
    .expect("valid smoke event")
}

#[test]
#[ignore = "requires an interactive Aqua session, prompts for authorization, and displays two notifications"]
fn displays_task_completion_and_approval_request_notifications() {
    if std::env::var_os(BUNDLED_SMOKE_ENV).is_none() {
        relaunch_in_product_bundle(TEST_NAME);
        return;
    }

    let backend = Arc::new(MacOsNotificationBackend::codex_notifier());
    backend
        .request_authorization()
        .expect("macOS notification authorization must be granted");
    let diagnostic = backend.diagnose();
    assert_eq!(
        diagnostic.status(),
        NotificationStatus::Ready,
        "macOS notification diagnostic: {diagnostic:?}"
    );

    let adapter = NativeNotificationAdapter::new(
        backend,
        NotificationPolicy::new(NotificationContentPolicy::Private, false),
    );
    adapter
        .deliver_now(&event(
            EventKind::TaskCompleted,
            "01983c8d-b800-7000-8000-000000000014",
            Urgency::Normal,
        ))
        .expect("macOS accepted the task-completion notification");
    thread::sleep(Duration::from_millis(750));
    adapter
        .deliver_now(&event(
            EventKind::ApprovalRequested,
            "01983c8d-b800-7000-8000-000000000015",
            Urgency::High,
        ))
        .expect("macOS accepted the approval-request notification");
}

#[test]
fn reports_real_missing_application_identity() {
    let backend = MacOsNotificationBackend::codex_notifier();
    let diagnostic = backend.diagnose();
    assert_eq!(
        diagnostic.status(),
        NotificationStatus::ApplicationIdentityMissing,
        "macOS notification diagnostic: {diagnostic:?}"
    );
}

#[test]
#[ignore = "requires a signed product bundle running without an Aqua launch domain"]
fn reports_real_no_gui_session() {
    if std::env::var_os(BUNDLED_SMOKE_ENV).is_none() {
        relaunch_in_product_bundle(NO_GUI_TEST_NAME);
        return;
    }

    let backend = MacOsNotificationBackend::codex_notifier();
    let diagnostic = backend.diagnose();
    assert_eq!(
        diagnostic.status(),
        NotificationStatus::NoInteractiveSession,
        "macOS notification diagnostic: {diagnostic:?}"
    );
}

fn relaunch_in_product_bundle(test_name: &str) {
    let directory = tempdir().expect("temporary smoke bundle directory");
    let bundle = directory.path().join("Codex Notifier.app");
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    fs::create_dir_all(&macos).expect("create smoke app bundle");
    fs::write(contents.join("Info.plist"), info_plist()).expect("write smoke Info.plist");

    let current = std::env::current_exe().expect("locate smoke test executable");
    let bundled = macos.join(EXECUTABLE_NAME);
    fs::copy(current, &bundled).expect("copy smoke executable into app bundle");
    let mut permissions = fs::metadata(&bundled)
        .expect("read smoke executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&bundled, permissions).expect("make smoke executable runnable");

    let signed = Command::new("/usr/bin/codesign")
        .args([
            "--force",
            "--sign",
            "-",
            "--identifier",
            CODEX_NOTIFIER_BUNDLE_ID,
        ])
        .arg(&bundle)
        .status()
        .expect("run ad-hoc codesign for smoke bundle");
    assert!(signed.success(), "ad-hoc codesign failed: {signed}");

    let status = Command::new(&bundled)
        .args(["--exact", test_name, "--ignored", "--nocapture"])
        .env(BUNDLED_SMOKE_ENV, "1")
        .status()
        .expect("launch bundled smoke test");
    assert!(status.success(), "bundled smoke test failed: {status}");
}

fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDisplayName</key>
    <string>Codex Notifier</string>
    <key>CFBundleExecutable</key>
    <string>{EXECUTABLE_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>{CODEX_NOTIFIER_BUNDLE_ID}</string>
    <key>CFBundleName</key>
    <string>Codex Notifier</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
"#
    )
}
