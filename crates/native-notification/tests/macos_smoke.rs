//! Explicit macOS `UserNotifications` smoke-test executable.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {

    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use codex_notifier_core::{
        CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
    };
    use codex_notifier_native_notification::{
        CODEX_NOTIFIER_BUNDLE_ID, MacOsNotificationBackend, NativeNotificationAdapter,
        NotificationBackend, NotificationContentPolicy, NotificationPolicy, NotificationStatus,
    };
    use objc2::{MainThreadMarker, runtime::Bool};
    use objc2_app_kit::{NSApplication, NSEventMask};
    use objc2_foundation::{NSDate, NSError, NSString};
    use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
    use tempfile::Builder;
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    const RESULT_PATH_ENV: &str = "CODEX_NOTIFIER_MACOS_SMOKE_RESULT";
    const RECOVER_AUTHORIZATION_ENV: &str = "CODEX_NOTIFIER_MACOS_SMOKE_RECOVER_AUTHORIZATION";
    const SMOKE_ROOT_ENV: &str = "CODEX_NOTIFIER_MACOS_SMOKE_ROOT";
    const SIGNING_IDENTITY_ENV: &str = "CODEX_NOTIFIER_MACOS_SIGNING_IDENTITY";
    const SIGNING_KEYCHAIN_ENV: &str = "CODEX_NOTIFIER_MACOS_SIGNING_KEYCHAIN";
    const TEST_NAME: &str = "displays_task_completion_and_approval_request_notifications";
    const DENIAL_TEST_NAME: &str = "reports_real_denied_authorization";
    const NO_GUI_TEST_NAME: &str = "reports_real_no_gui_session";
    const EXECUTABLE_NAME: &str = "codex-notifier-macos-smoke";
    const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);

    pub(super) fn run() {
        let arguments = std::env::args().skip(1).collect::<Vec<_>>();
        if arguments.iter().any(|argument| argument == "--list") {
            println!("{TEST_NAME}: test");
            println!("{DENIAL_TEST_NAME}: test");
            println!("{NO_GUI_TEST_NAME}: test");
            println!("reports_real_missing_application_identity: test");
            return;
        }
        if arguments.iter().any(|argument| argument == TEST_NAME) {
            displays_task_completion_and_approval_request_notifications();
        } else if arguments
            .iter()
            .any(|argument| argument == DENIAL_TEST_NAME)
        {
            reports_real_denied_authorization();
        } else if arguments
            .iter()
            .any(|argument| argument == NO_GUI_TEST_NAME)
        {
            reports_real_no_gui_session();
        } else {
            reports_real_missing_application_identity();
        }
    }

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

    fn displays_task_completion_and_approval_request_notifications() {
        if !running_from_application_bundle() {
            relaunch_in_product_bundle(TEST_NAME, None);
            return;
        }

        request_authorization_with_application_run_loop(AuthorizationExpectation::Grant);
        let backend = Arc::new(MacOsNotificationBackend::codex_notifier());
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
        thread::sleep(Duration::from_secs(4));
        adapter
            .deliver_now(&event(
                EventKind::ApprovalRequested,
                "01983c8d-b800-7000-8000-000000000015",
                Urgency::High,
            ))
            .expect("macOS accepted the approval-request notification");
        thread::sleep(Duration::from_secs(6));
        if let Some(path) = std::env::var_os(RESULT_PATH_ENV) {
            fs::write(path, b"ok").expect("write successful macOS smoke result");
        }
    }

    fn reports_real_missing_application_identity() {
        let backend = MacOsNotificationBackend::codex_notifier();
        let diagnostic = backend.diagnose();
        assert_eq!(
            diagnostic.status(),
            NotificationStatus::ApplicationIdentityMissing,
            "macOS notification diagnostic: {diagnostic:?}"
        );
    }

    fn reports_real_denied_authorization() {
        if !running_from_application_bundle() {
            relaunch_in_product_bundle(DENIAL_TEST_NAME, None);
            return;
        }

        request_authorization_with_application_run_loop(AuthorizationExpectation::Denial);
        if let Some(path) = std::env::var_os(RESULT_PATH_ENV) {
            fs::write(path, b"ok").expect("write successful macOS denial result");
        }
    }

    fn reports_real_no_gui_session() {
        if !running_from_application_bundle() {
            relaunch_in_product_bundle(NO_GUI_TEST_NAME, Some("nobody"));
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

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AuthorizationExpectation {
        Grant,
        Denial,
    }

    fn request_authorization_with_application_run_loop(expectation: AuthorizationExpectation) {
        let main_thread = MainThreadMarker::new().expect("smoke app must start on the main thread");
        let application = NSApplication::sharedApplication(main_thread);
        application.finishLaunching();
        application.activate();

        let (sender, receiver) = mpsc::sync_channel(1);
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            sender
                .send((granted.as_bool(), error.is_null()))
                .expect("authorization result receiver must remain connected");
        });
        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &completion,
        );

        let deadline = Instant::now() + AUTHORIZATION_TIMEOUT;
        let default_run_loop_mode = NSString::from_str("kCFRunLoopDefaultMode");
        let recover_authorization = std::env::var_os(RECOVER_AUTHORIZATION_ENV).is_some();
        let mut awaiting_settings_grant = false;
        let mut next_diagnostic_at = Instant::now();
        loop {
            if awaiting_settings_grant && Instant::now() >= next_diagnostic_at {
                let diagnostic = MacOsNotificationBackend::codex_notifier().diagnose();
                if diagnostic.status() == NotificationStatus::Ready {
                    eprintln!(
                        "macOS notification authorization recovered through System Settings; diagnostic={diagnostic:?}"
                    );
                    break;
                }
                next_diagnostic_at = Instant::now() + Duration::from_millis(500);
            }
            match receiver.try_recv() {
                Ok((granted, error_free)) => {
                    if !error_free || !granted {
                        let diagnostic = MacOsNotificationBackend::codex_notifier().diagnose();
                        if expectation == AuthorizationExpectation::Denial {
                            assert_eq!(
                                diagnostic.status(),
                                NotificationStatus::DisabledForApplication,
                                "macOS denial diagnostic: {diagnostic:?}"
                            );
                            eprintln!(
                                "macOS notification authorization denied as expected; granted={granted}, error_free={error_free}, diagnostic={diagnostic:?}"
                            );
                            break;
                        }
                        if recover_authorization {
                            eprintln!(
                                "macOS notification authorization requires System Settings recovery; granted={granted}, error_free={error_free}, diagnostic={diagnostic:?}"
                            );
                            awaiting_settings_grant = true;
                        } else {
                            panic!(
                                "macOS notification authorization failed; granted={granted}, error_free={error_free}, diagnostic={diagnostic:?}"
                            );
                        }
                    }
                    if granted && error_free {
                        assert_eq!(
                            expectation,
                            AuthorizationExpectation::Grant,
                            "macOS unexpectedly granted authorization in the denial smoke test"
                        );
                        break;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("authorization callback disconnected without a result");
                }
            }
            assert!(
                Instant::now() < deadline,
                "macOS notification authorization timed out"
            );
            let next_poll = NSDate::dateWithTimeIntervalSinceNow(0.05);
            if let Some(event) = application.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&next_poll),
                &default_run_loop_mode,
                true,
            ) {
                application.sendEvent(&event);
            }
        }
    }

    fn running_from_application_bundle() -> bool {
        std::env::current_exe()
            .ok()
            .and_then(|executable| {
                executable
                    .parent()
                    .and_then(std::path::Path::parent)
                    .and_then(std::path::Path::parent)
                    .map(std::path::Path::to_path_buf)
            })
            .is_some_and(|bundle| bundle.extension() == Some(OsStr::new("app")))
    }

    fn relaunch_in_product_bundle(test_name: &str, run_as_user: Option<&str>) {
        let smoke_root = std::path::PathBuf::from(
            std::env::var_os(SMOKE_ROOT_ENV).unwrap_or_else(|| "/tmp".into()),
        );
        let directory = Builder::new()
            .prefix("codex-notifier-smoke-")
            .tempdir_in(&smoke_root)
            .expect("temporary smoke bundle directory");
        if run_as_user.is_some() {
            let mut permissions = fs::metadata(directory.path())
                .expect("read smoke directory metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(directory.path(), permissions)
                .expect("make smoke directory traversable");
        }
        let (bundle, bundled) = create_signed_product_bundle(directory.path());

        if let Some(user) = run_as_user {
            let mut command = Command::new("/usr/bin/sudo");
            command.args(["-u", user, "env", "HOME=/tmp", "TMPDIR=/tmp"]);
            command.arg(&bundled);
            let status = command
                .args(["--exact", test_name, "--ignored", "--nocapture"])
                .status()
                .expect("launch bundled smoke test without Aqua");
            assert!(status.success(), "bundled smoke test failed: {status}");
            return;
        }

        let result_path = directory.path().join("smoke-result");
        let stdout_path = directory.path().join("smoke-stdout.log");
        let stderr_path = directory.path().join("smoke-stderr.log");
        let mut command = Command::new("/usr/bin/open");
        command
            .args(["-W", "-n", "--stdout"])
            .arg(&stdout_path)
            .arg("--stderr")
            .arg(&stderr_path)
            .arg("--env")
            .arg(format!("{RESULT_PATH_ENV}={}", result_path.display()));
        if let Some(value) = std::env::var_os(RECOVER_AUTHORIZATION_ENV) {
            command.arg("--env").arg(format!(
                "{RECOVER_AUTHORIZATION_ENV}={}",
                value.to_string_lossy()
            ));
        }
        command
            .arg(&bundle)
            .args(["--args", "--exact", test_name, "--ignored", "--nocapture"]);
        let status = command
            .status()
            .expect("launch bundled smoke test through LaunchServices");
        let result = fs::read(&result_path).unwrap_or_default();
        let stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        assert!(
            status.success() && result == b"ok",
            "LaunchServices smoke failed: status={status}, result={result:?}, stdout={stdout:?}, stderr={stderr:?}"
        );
    }

    fn create_signed_product_bundle(
        directory: &std::path::Path,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let bundle = directory.join("Codex Notifier.app");
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

        let signing_identity =
            std::env::var(SIGNING_IDENTITY_ENV).unwrap_or_else(|_| "-".to_owned());
        let mut signing_command = Command::new("/usr/bin/codesign");
        signing_command
            .args(["--force", "--sign"])
            .arg(&signing_identity);
        if let Some(keychain) = std::env::var_os(SIGNING_KEYCHAIN_ENV) {
            signing_command.arg("--keychain").arg(keychain);
        }
        let signed = signing_command
            .args(["--identifier", CODEX_NOTIFIER_BUNDLE_ID])
            .arg(&bundle)
            .status()
            .expect("run codesign for smoke bundle");
        assert!(signed.success(), "codesign failed: {signed}");
        let inspected = Command::new("/usr/bin/codesign")
            .args(["--display", "--verbose=4"])
            .arg(&bundle)
            .status()
            .expect("inspect smoke bundle signature");
        assert!(
            inspected.success(),
            "codesign inspection failed: {inspected}"
        );
        let registered = Command::new(
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
    )
    .arg("-f")
    .arg(&bundle)
    .status()
    .expect("register smoke bundle with LaunchServices");
        assert!(
            registered.success(),
            "LaunchServices registration failed: {registered}"
        );
        (bundle, bundled)
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
</dict>
</plist>
"#
        )
    }
}
