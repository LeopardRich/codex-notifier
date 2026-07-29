//! macOS `UserNotifications` implementation for a signed application bundle.

use std::fmt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::{MainThreadMarker, runtime::Bool};
use objc2_app_kit::{NSApplication, NSEventMask};
use objc2_foundation::{NSBundle, NSDate, NSError, NSString};
use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
use rustix::process::geteuid;
use usernotifications::{
    AuthorizationStatus, NotificationContent, NotificationInterruptionLevel, NotificationRequest,
    NotificationSetting, NotificationSettings, NotificationSound, USER_NOTIFICATIONS_ERROR_DOMAIN,
    UserNotificationCenter, UserNotificationsError,
};

use crate::{
    FocusStatus, NotificationBackend, NotificationDiagnostic, NotificationError,
    NotificationStatus, PreparedNotification,
};

/// Stable bundle identifier owned by the macOS package.
pub const CODEX_NOTIFIER_BUNDLE_ID: &str = "io.github.leopardrich.codex-notifier";
const MAX_BUNDLE_ID_BYTES: usize = 255;
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);

/// Strict reverse-DNS application bundle identifier.
#[derive(Clone, Eq, PartialEq)]
pub struct MacOsBundleIdentifier(String);

impl MacOsBundleIdentifier {
    /// Validates a packaging-owned bundle identifier.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationError::ApplicationIdentityMissing`] for an empty,
    /// overlong, non-ASCII, path-like, or malformed reverse-DNS identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, NotificationError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_BUNDLE_ID_BYTES
            && value.split('.').count() >= 3
            && value.split('.').all(valid_bundle_segment);
        if !valid {
            return Err(NotificationError::ApplicationIdentityMissing);
        }
        Ok(Self(value))
    }

    /// Returns the validated bundle identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MacOsBundleIdentifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MacOsBundleIdentifier(<product-id>)")
    }
}

fn valid_bundle_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

/// `UserNotifications` backend for the current interactive Aqua session.
pub struct MacOsNotificationBackend {
    bundle_identifier: MacOsBundleIdentifier,
    session: SessionStatus,
}

impl MacOsNotificationBackend {
    /// Creates a backend for a packaging-owned bundle identity.
    #[must_use]
    pub fn new(bundle_identifier: MacOsBundleIdentifier) -> Self {
        Self {
            bundle_identifier,
            session: current_session_status(),
        }
    }

    /// Creates the standard product backend.
    #[must_use]
    pub fn codex_notifier() -> Self {
        Self::new(MacOsBundleIdentifier(CODEX_NOTIFIER_BUNDLE_ID.to_owned()))
    }

    /// Explicitly requests alert and sound authorization from macOS.
    ///
    /// This is intentionally separate from [`NotificationBackend::show`] so a
    /// background event can never trigger a surprise system permission prompt.
    ///
    /// # Errors
    ///
    /// Returns a stable identity, session, denial, or native API error.
    pub fn request_authorization(&self) -> Result<(), NotificationError> {
        self.check_application_identity()?;
        self.check_session()?;
        let main_thread = MainThreadMarker::new().ok_or(NotificationError::Unavailable)?;
        let application = NSApplication::sharedApplication(main_thread);
        application.finishLaunching();
        application.activate();

        let (sender, receiver) = mpsc::sync_channel(1);
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let _ = sender.send((granted.as_bool(), error.is_null()));
        });
        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &completion,
        );
        let deadline = Instant::now() + AUTHORIZATION_TIMEOUT;
        let default_run_loop_mode = NSString::from_str("kCFRunLoopDefaultMode");
        let (granted, error_free) = loop {
            match receiver.try_recv() {
                Ok(result) => break result,
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(NotificationError::Unavailable);
                }
            }
            if Instant::now() >= deadline {
                return Err(NotificationError::Unavailable);
            }
            let next_poll = NSDate::dateWithTimeIntervalSinceNow(0.05);
            if let Some(event) = application.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&next_poll),
                &default_run_loop_mode,
                true,
            ) {
                application.sendEvent(&event);
            }
        };
        let settings = UserNotificationCenter::current()
            .map_err(|_| NotificationError::Unavailable)?
            .notification_settings()
            .map_err(|_| NotificationError::Unavailable)?;
        if granted && error_free {
            settings_error(&settings)
        } else {
            settings_error(&settings).and(Err(NotificationError::DisabledForApplication))
        }
    }

    fn center(&self) -> Result<UserNotificationCenter, NotificationError> {
        self.check_application_identity()?;
        self.check_session()?;
        UserNotificationCenter::current().map_err(|_| NotificationError::Unavailable)
    }

    fn check_session(&self) -> Result<(), NotificationError> {
        match self.session {
            SessionStatus::Interactive => {}
            SessionStatus::NonInteractive => {
                return Err(NotificationError::NoInteractiveSession);
            }
            SessionStatus::Unknown => return Err(NotificationError::Unavailable),
        }
        Ok(())
    }

    fn ready_center(&self) -> Result<UserNotificationCenter, NotificationError> {
        let center = self.center()?;
        let settings = center
            .notification_settings()
            .map_err(|_| NotificationError::Unavailable)?;
        settings_error(&settings)?;
        Ok(center)
    }

    fn check_application_identity(&self) -> Result<(), NotificationError> {
        let bundle = NSBundle::mainBundle();
        let identifier_matches = bundle
            .bundleIdentifier()
            .is_some_and(|identifier| identifier.to_string() == self.bundle_identifier.as_str());
        let bundle_path = bundle.bundlePath().to_string();
        let is_application_bundle = Path::new(&bundle_path)
            .extension()
            .is_some_and(|extension| extension == "app");
        if identifier_matches && is_application_bundle {
            Ok(())
        } else {
            Err(NotificationError::ApplicationIdentityMissing)
        }
    }
}

impl NotificationBackend for MacOsNotificationBackend {
    fn diagnose(&self) -> NotificationDiagnostic {
        let status = match self.ready_center() {
            Ok(_) => NotificationStatus::Ready,
            Err(error) => status_for_error(error),
        };
        NotificationDiagnostic::new(status, FocusStatus::SystemManaged)
    }

    fn show(&self, notification: &PreparedNotification) -> Result<(), NotificationError> {
        let center = self.ready_center()?;
        let content = notification_content(notification);
        let request = NotificationRequest::new(notification.identifier(), content, None);
        center
            .add_notification_request(&request)
            .map_err(|error| delivery_error(&error))
    }
}

fn notification_content(notification: &PreparedNotification) -> NotificationContent {
    let mut content = NotificationContent::new(notification.title(), notification.body())
        .with_interruption_level(if notification.silent() {
            NotificationInterruptionLevel::Passive
        } else {
            NotificationInterruptionLevel::Active
        });
    if !notification.silent() {
        content = content.with_sound(NotificationSound::Default);
    }
    content
}

fn settings_error(settings: &NotificationSettings) -> Result<(), NotificationError> {
    match settings.authorization_status {
        AuthorizationStatus::NotDetermined => Err(NotificationError::AuthorizationNotDetermined),
        AuthorizationStatus::Denied => Err(NotificationError::DisabledForApplication),
        AuthorizationStatus::Authorized => display_setting_error(settings.alert_setting),
        AuthorizationStatus::Provisional => {
            display_setting_error(settings.notification_center_setting)
        }
    }
}

const fn display_setting_error(setting: NotificationSetting) -> Result<(), NotificationError> {
    match setting {
        NotificationSetting::Enabled => Ok(()),
        NotificationSetting::Disabled => Err(NotificationError::DisabledForApplication),
        NotificationSetting::NotSupported => Err(NotificationError::Unavailable),
    }
}

fn delivery_error(error: &UserNotificationsError) -> NotificationError {
    let mut parts = error.message().splitn(3, ':');
    if parts.next() == Some(USER_NOTIFICATIONS_ERROR_DOMAIN) && parts.next() == Some("1") {
        NotificationError::DisabledForApplication
    } else {
        NotificationError::DeliveryFailed
    }
}

const fn status_for_error(error: NotificationError) -> NotificationStatus {
    match error {
        NotificationError::UnsupportedPlatform => NotificationStatus::UnsupportedPlatform,
        NotificationError::ApplicationIdentityMissing => {
            NotificationStatus::ApplicationIdentityMissing
        }
        NotificationError::AuthorizationNotDetermined => {
            NotificationStatus::AuthorizationNotDetermined
        }
        NotificationError::DisabledForApplication => NotificationStatus::DisabledForApplication,
        NotificationError::DisabledForUser => NotificationStatus::DisabledForUser,
        NotificationError::DisabledByPolicy => NotificationStatus::DisabledByPolicy,
        NotificationError::NoInteractiveSession => NotificationStatus::NoInteractiveSession,
        NotificationError::Unavailable | NotificationError::DeliveryFailed => {
            NotificationStatus::Unavailable
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionStatus {
    Interactive,
    NonInteractive,
    Unknown,
}

fn current_session_status() -> SessionStatus {
    let user_id = geteuid().as_raw();
    let domain = format!("gui/{user_id}");
    match Command::new("/bin/launchctl")
        .args(["print", &domain])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => SessionStatus::Interactive,
        Ok(_) => SessionStatus::NonInteractive,
        Err(_) => SessionStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use codex_notifier_core::{
        CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
    };
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    use super::*;
    use crate::{NotificationContentPolicy, NotificationPolicy};

    fn prepared(quiet: bool) -> PreparedNotification {
        let time = OffsetDateTime::parse("2026-07-29T04:00:00.000Z", &Rfc3339).expect("valid time");
        let event = CanonicalEvent::new(
            EventId::parse("01983c8d-b800-7000-8000-000000000012").expect("UUIDv7"),
            EventKind::ApprovalRequested,
            time,
            EventSource::new("workstation", None, None).expect("source"),
            Presentation::new("title", "body", Urgency::High, Privacy::Public)
                .expect("presentation"),
            None,
            Extensions::new(BTreeMap::new()).expect("extensions"),
            time,
        )
        .expect("event");
        crate::prepare(
            &event,
            NotificationPolicy::new(NotificationContentPolicy::Public, quiet),
        )
    }

    #[test]
    fn bundle_identity_is_strict_and_redacted() {
        let identity =
            MacOsBundleIdentifier::new(CODEX_NOTIFIER_BUNDLE_ID).expect("product bundle ID");
        assert_eq!(identity.as_str(), CODEX_NOTIFIER_BUNDLE_ID);
        assert_eq!(
            format!("{identity:?}"),
            "MacOsBundleIdentifier(<product-id>)"
        );
        for invalid in [
            "",
            "two.parts",
            ".invalid.bundle",
            "invalid.bundle.",
            "has space.bundle.id",
            "path\\bundle.id",
            "bad!.bundle.id",
        ] {
            assert_eq!(
                MacOsBundleIdentifier::new(invalid),
                Err(NotificationError::ApplicationIdentityMissing)
            );
        }
        assert_eq!(
            MacOsBundleIdentifier::new("a".repeat(MAX_BUNDLE_ID_BYTES + 1)),
            Err(NotificationError::ApplicationIdentityMissing)
        );
    }

    #[test]
    fn authorization_and_display_settings_are_classified() {
        assert_eq!(
            status_for_error(NotificationError::AuthorizationNotDetermined),
            NotificationStatus::AuthorizationNotDetermined
        );
        assert_eq!(
            status_for_error(NotificationError::DisabledForApplication),
            NotificationStatus::DisabledForApplication
        );
        assert_eq!(display_setting_error(NotificationSetting::Enabled), Ok(()));
        assert_eq!(
            display_setting_error(NotificationSetting::Disabled),
            Err(NotificationError::DisabledForApplication)
        );
        assert_eq!(
            display_setting_error(NotificationSetting::NotSupported),
            Err(NotificationError::Unavailable)
        );
    }

    #[test]
    fn content_is_display_only_and_never_bypasses_focus() {
        let active = notification_content(&prepared(false));
        assert_eq!(active.title, "title");
        assert_eq!(active.body, "body");
        assert_eq!(active.sound, Some(NotificationSound::Default));
        assert_eq!(
            active.interruption_level,
            Some(NotificationInterruptionLevel::Active)
        );
        assert!(active.category_identifier.is_empty());
        assert!(active.user_info.is_none());

        let silent = notification_content(&prepared(true));
        assert!(silent.sound.is_none());
        assert_eq!(
            silent.interruption_level,
            Some(NotificationInterruptionLevel::Passive)
        );
    }

    #[test]
    fn framework_denial_is_distinct_from_delivery_failure() {
        assert_eq!(
            delivery_error(&UserNotificationsError::FrameworkError(
                "UNErrorDomain:1:notifications are not allowed".to_owned()
            )),
            NotificationError::DisabledForApplication
        );
        assert_eq!(
            delivery_error(&UserNotificationsError::FrameworkError(
                "OtherDomain:9:rejected".to_owned()
            )),
            NotificationError::DeliveryFailed
        );
    }
}
