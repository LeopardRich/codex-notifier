//! Native desktop notification policy and adapter boundary.

use std::future::ready;
use std::sync::Arc;

use codex_notifier_application::{
    CancellationToken, DeliveryFailure, DeliveryFuture, DeliveryOutcome, EventDelivery,
    SafeErrorCode,
};
use codex_notifier_core::{CanonicalEvent, EventKind, Privacy, Urgency};
use thiserror::Error;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{CODEX_NOTIFIER_APP_ID, WindowsApplicationId, WindowsNotificationBackend};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{CODEX_NOTIFIER_BUNDLE_ID, MacOsBundleIdentifier, MacOsNotificationBackend};

const PRIVATE_TASK_TITLE: &str = "Codex task finished";
const PRIVATE_TASK_BODY: &str = "Open Codex to review the result.";
const PRIVATE_APPROVAL_TITLE: &str = "Codex needs approval";
const PRIVATE_APPROVAL_BODY: &str = "Open Codex to review the request.";
const MAX_NATIVE_TITLE_SCALARS: usize = 64;
const MAX_NATIVE_BODY_SCALARS: usize = 200;

/// Application-level notification privacy policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationContentPolicy {
    /// Always render fixed generic text for the event kind.
    Private,
    /// Permit canonical display text when the event also marks it public.
    Public,
}

/// Validated presentation settings shared by desktop platforms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationPolicy {
    content: NotificationContentPolicy,
    quiet_hours: bool,
}

impl NotificationPolicy {
    /// Creates an explicit privacy and application quiet-hours policy.
    #[must_use]
    pub const fn new(content: NotificationContentPolicy, quiet_hours: bool) -> Self {
        Self {
            content,
            quiet_hours,
        }
    }

    /// Returns the configured content policy.
    #[must_use]
    pub const fn content(self) -> NotificationContentPolicy {
        self.content
    }

    /// Returns whether delivery should be silent but still enter history.
    #[must_use]
    pub const fn quiet_hours(self) -> bool {
        self.quiet_hours
    }
}

impl Default for NotificationPolicy {
    fn default() -> Self {
        Self::new(NotificationContentPolicy::Private, false)
    }
}

/// Bounded, platform-neutral input to one native notification API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedNotification {
    identifier: String,
    title: String,
    body: String,
    urgency: Urgency,
    silent: bool,
}

impl PreparedNotification {
    /// Returns the canonical event identifier used for native replacement.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Returns the bounded native title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the bounded native body.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Returns the platform-independent urgency.
    #[must_use]
    pub const fn urgency(&self) -> Urgency {
        self.urgency
    }

    /// Returns whether popup and sound should be suppressed.
    #[must_use]
    pub const fn silent(&self) -> bool {
        self.silent
    }
}

/// Stable native notification readiness classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NotificationStatus {
    /// The native API is available and application notifications are enabled.
    Ready,
    /// The operating system is outside the supported desktop platforms.
    UnsupportedPlatform,
    /// No usable application identity is registered.
    ApplicationIdentityMissing,
    /// The application has not requested notification authorization yet.
    AuthorizationNotDetermined,
    /// The application is disabled in system notification settings.
    DisabledForApplication,
    /// The current user disabled notifications globally.
    DisabledForUser,
    /// Notification delivery is disabled by system policy.
    DisabledByPolicy,
    /// The process has no interactive user session.
    NoInteractiveSession,
    /// The native notification API could not be queried.
    Unavailable,
}

impl NotificationStatus {
    /// Returns a stable machine-readable status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::ApplicationIdentityMissing => "application_identity_missing",
            Self::AuthorizationNotDetermined => "authorization_not_determined",
            Self::DisabledForApplication => "disabled_for_application",
            Self::DisabledForUser => "disabled_for_user",
            Self::DisabledByPolicy => "disabled_by_policy",
            Self::NoInteractiveSession => "no_interactive_session",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Current interruption-policy visibility exposed by the native backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FocusStatus {
    /// The operating system owns focus/do-not-disturb suppression.
    SystemManaged,
    /// Notifications are currently accepted without known suppression.
    AcceptsNotifications,
    /// The user is not present in the active session.
    UserNotPresent,
    /// The user is busy and popup delivery may be suppressed.
    Busy,
    /// A full-screen application may suppress popup delivery.
    FullScreen,
    /// Presentation mode may suppress popup delivery.
    Presentation,
    /// System quiet time may suppress popup delivery.
    QuietTime,
    /// A Windows Store application is active.
    ApplicationRunning,
    /// The current focus state could not be queried.
    Unknown,
}

impl FocusStatus {
    /// Returns a stable machine-readable focus status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SystemManaged => "system_managed",
            Self::AcceptsNotifications => "accepts_notifications",
            Self::UserNotPresent => "user_not_present",
            Self::Busy => "busy",
            Self::FullScreen => "full_screen",
            Self::Presentation => "presentation",
            Self::QuietTime => "quiet_time",
            Self::ApplicationRunning => "application_running",
            Self::Unknown => "unknown",
        }
    }
}

/// Read-only native notification diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationDiagnostic {
    status: NotificationStatus,
    focus: FocusStatus,
}

impl NotificationDiagnostic {
    /// Creates a stable status and focus-policy result.
    #[must_use]
    pub const fn new(status: NotificationStatus, focus: FocusStatus) -> Self {
        Self { status, focus }
    }

    /// Returns native notification readiness.
    #[must_use]
    pub const fn status(self) -> NotificationStatus {
        self.status
    }

    /// Returns interruption-policy visibility.
    #[must_use]
    pub const fn focus(self) -> FocusStatus {
        self.focus
    }
}

/// Safe native delivery errors with stable retry behavior.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum NotificationError {
    /// The current operating system has no supported desktop adapter.
    #[error("native notifications are unsupported on this platform")]
    UnsupportedPlatform,
    /// The required native application identity is absent or invalid.
    #[error("native notification application identity is missing")]
    ApplicationIdentityMissing,
    /// Notification authorization has not been requested yet.
    #[error("native notification authorization has not been requested")]
    AuthorizationNotDetermined,
    /// The application is disabled in native notification settings.
    #[error("native notifications are disabled for the application")]
    DisabledForApplication,
    /// The current user disabled native notifications globally.
    #[error("native notifications are disabled for the user")]
    DisabledForUser,
    /// Native notifications are disabled by system policy.
    #[error("native notifications are disabled by policy")]
    DisabledByPolicy,
    /// The process is not running in an interactive desktop session.
    #[error("native notifications require an interactive user session")]
    NoInteractiveSession,
    /// The native API could not be queried.
    #[error("native notification service is unavailable")]
    Unavailable,
    /// The native API rejected a bounded notification.
    #[error("native notification delivery failed")]
    DeliveryFailed,
}

impl NotificationError {
    /// Returns a stable machine-readable error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "notification_platform_unsupported",
            Self::ApplicationIdentityMissing => "notification_identity_missing",
            Self::AuthorizationNotDetermined => "notification_authorization_not_determined",
            Self::DisabledForApplication => "notification_disabled_application",
            Self::DisabledForUser => "notification_disabled_user",
            Self::DisabledByPolicy => "notification_disabled_policy",
            Self::NoInteractiveSession => "notification_no_interactive_session",
            Self::Unavailable => "notification_unavailable",
            Self::DeliveryFailed => "notification_delivery_failed",
        }
    }

    /// Returns whether a later attempt in the same user session may succeed.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::NoInteractiveSession | Self::Unavailable | Self::DeliveryFailed
        )
    }
}

/// Platform adapter interface used by the role-aware application runtime.
pub trait NotificationBackend: Send + Sync {
    /// Performs a read-only native readiness check.
    fn diagnose(&self) -> NotificationDiagnostic;

    /// Submits one bounded display-only notification to the operating system.
    ///
    /// # Errors
    ///
    /// Returns a stable identity, permission, session, availability, or
    /// delivery classification without retaining native payload text.
    fn show(&self, notification: &PreparedNotification) -> Result<(), NotificationError>;
}

/// Policy-enforcing adapter from canonical events to a native backend.
pub struct NativeNotificationAdapter {
    backend: Arc<dyn NotificationBackend>,
    policy: NotificationPolicy,
}

impl NativeNotificationAdapter {
    /// Composes a native backend behind an explicit privacy policy.
    #[must_use]
    pub fn new(backend: Arc<dyn NotificationBackend>, policy: NotificationPolicy) -> Self {
        Self { backend, policy }
    }

    /// Runs the backend's read-only diagnostic.
    #[must_use]
    pub fn diagnose(&self) -> NotificationDiagnostic {
        self.backend.diagnose()
    }

    /// Maps and submits one canonical event immediately.
    ///
    /// # Errors
    ///
    /// Returns the backend's safe native failure classification.
    pub fn deliver_now(&self, event: &CanonicalEvent) -> Result<(), NotificationError> {
        self.backend.show(&prepare(event, self.policy))
    }

    /// Applies privacy, urgency, and quiet-hours policy without native I/O.
    #[must_use]
    pub fn prepare(&self, event: &CanonicalEvent) -> PreparedNotification {
        prepare(event, self.policy)
    }
}

impl EventDelivery for NativeNotificationAdapter {
    fn deliver<'a>(
        &'a self,
        event: &'a CanonicalEvent,
        cancellation: CancellationToken,
    ) -> DeliveryFuture<'a> {
        let outcome = if cancellation.is_cancelled() {
            DeliveryOutcome::Cancelled
        } else {
            match self.deliver_now(event) {
                Ok(()) => DeliveryOutcome::Delivered,
                Err(error) => DeliveryOutcome::Failed(DeliveryFailure::new(
                    SafeErrorCode::parse(error.code()).expect("notification codes are valid"),
                    error.retryable(),
                )),
            }
        };
        Box::pin(ready(outcome))
    }
}

fn prepare(event: &CanonicalEvent, policy: NotificationPolicy) -> PreparedNotification {
    let presentation = event.presentation();
    let use_public = policy.content() == NotificationContentPolicy::Public
        && presentation.privacy() == Privacy::Public;
    let (title, body) = if use_public {
        (presentation.title(), presentation.body())
    } else {
        private_text(event.kind())
    };
    PreparedNotification {
        identifier: event.event_id().to_string(),
        title: sanitize_native_text(title, MAX_NATIVE_TITLE_SCALARS),
        body: sanitize_native_text(body, MAX_NATIVE_BODY_SCALARS),
        urgency: presentation.urgency(),
        silent: policy.quiet_hours(),
    }
}

const fn private_text(kind: EventKind) -> (&'static str, &'static str) {
    match kind {
        EventKind::TaskCompleted => (PRIVATE_TASK_TITLE, PRIVATE_TASK_BODY),
        EventKind::ApprovalRequested => (PRIVATE_APPROVAL_TITLE, PRIVATE_APPROVAL_BODY),
    }
}

fn sanitize_native_text(value: &str, max_scalars: usize) -> String {
    value
        .chars()
        .filter(|character| {
            let code = u32::from(*character);
            code > 0x1f && !(0x7f..=0x9f).contains(&code)
        })
        .take(max_scalars)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use codex_notifier_core::{EventId, EventSource, Extensions, Presentation, Routing};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    use super::*;

    const EVENT_ID: &str = "01983c8d-b800-7000-8000-000000000012";

    fn event(kind: EventKind, privacy: Privacy, title: &str, body: &str) -> CanonicalEvent {
        let occurred_at =
            OffsetDateTime::parse("2026-07-29T04:00:00.000Z", &Rfc3339).expect("valid time");
        CanonicalEvent::new(
            EventId::parse(EVENT_ID).expect("UUIDv7"),
            kind,
            occurred_at,
            EventSource::new("workstation", Some("project".to_owned()), None)
                .expect("valid source"),
            Presentation::new(title, body, Urgency::High, privacy).expect("valid presentation"),
            Some(Routing::new("desktop").expect("valid route")),
            Extensions::new(BTreeMap::new()).expect("valid extensions"),
            occurred_at,
        )
        .expect("valid event")
    }

    #[test]
    fn private_policy_uses_fixed_text_for_both_events() {
        let backend = Arc::new(FakeBackend::default());
        let adapter = NativeNotificationAdapter::new(backend, NotificationPolicy::default());
        let task = adapter.prepare(&event(
            EventKind::TaskCompleted,
            Privacy::Public,
            "project details",
            "model response",
        ));
        assert_eq!(task.title(), PRIVATE_TASK_TITLE);
        assert_eq!(task.body(), PRIVATE_TASK_BODY);
        assert_eq!(task.identifier(), EVENT_ID);
        assert_eq!(task.urgency(), Urgency::High);
        assert!(!task.silent());

        let approval = adapter.prepare(&event(
            EventKind::ApprovalRequested,
            Privacy::Public,
            "run command",
            "secret arguments",
        ));
        assert_eq!(approval.title(), PRIVATE_APPROVAL_TITLE);
        assert_eq!(approval.body(), PRIVATE_APPROVAL_BODY);
    }

    #[test]
    fn public_policy_requires_event_consent_and_preserves_xml_text_as_data() {
        let backend = Arc::new(FakeBackend::default());
        let adapter = NativeNotificationAdapter::new(
            backend,
            NotificationPolicy::new(NotificationContentPolicy::Public, false),
        );
        let public = adapter.prepare(&event(
            EventKind::TaskCompleted,
            Privacy::Public,
            "A <tag> & value",
            "quotes: \" ' >",
        ));
        assert_eq!(public.title(), "A <tag> & value");
        assert_eq!(public.body(), "quotes: \" ' >");

        let private = adapter.prepare(&event(
            EventKind::TaskCompleted,
            Privacy::Private,
            "must not display",
            "must not display",
        ));
        assert_eq!(private.title(), PRIVATE_TASK_TITLE);
        assert_eq!(private.body(), PRIVATE_TASK_BODY);
    }

    #[test]
    fn native_defense_removes_controls_and_truncates_by_scalar() {
        let mut value = "a\n\u{1b}\u{80}".to_owned();
        value.push_str(&"界".repeat(MAX_NATIVE_BODY_SCALARS + 10));
        let sanitized = sanitize_native_text(&value, MAX_NATIVE_BODY_SCALARS);
        assert_eq!(sanitized.chars().count(), MAX_NATIVE_BODY_SCALARS);
        assert!(!sanitized.chars().any(char::is_control));
        assert!(sanitized.starts_with('a'));
        assert!(sanitized.ends_with('界'));
    }

    #[test]
    fn quiet_hours_is_silent_without_discarding_delivery() {
        let backend = Arc::new(FakeBackend::default());
        let adapter = NativeNotificationAdapter::new(
            backend.clone(),
            NotificationPolicy::new(NotificationContentPolicy::Private, true),
        );
        let event = event(
            EventKind::ApprovalRequested,
            Privacy::Private,
            "unused",
            "unused",
        );
        adapter.deliver_now(&event).expect("fake delivery");
        let delivered = backend.delivered.lock().expect("fake lock");
        assert_eq!(delivered.len(), 1);
        assert!(delivered[0].silent());
    }

    #[test]
    fn diagnostic_and_failures_remain_structured() {
        let backend = Arc::new(FakeBackend {
            diagnostic: NotificationDiagnostic::new(
                NotificationStatus::DisabledByPolicy,
                FocusStatus::QuietTime,
            ),
            result: Err(NotificationError::DisabledByPolicy),
            delivered: Mutex::new(Vec::new()),
        });
        let adapter = NativeNotificationAdapter::new(backend, NotificationPolicy::default());
        assert_eq!(
            adapter.diagnose().status(),
            NotificationStatus::DisabledByPolicy
        );
        assert_eq!(adapter.diagnose().focus(), FocusStatus::QuietTime);
        assert_eq!(
            adapter.deliver_now(&event(
                EventKind::TaskCompleted,
                Privacy::Private,
                "unused",
                "unused",
            )),
            Err(NotificationError::DisabledByPolicy)
        );
        assert_eq!(
            NotificationError::DisabledByPolicy.code(),
            "notification_disabled_policy"
        );
        assert_eq!(
            NotificationError::AuthorizationNotDetermined.code(),
            "notification_authorization_not_determined"
        );
        assert!(!NotificationError::DisabledByPolicy.retryable());
    }

    struct FakeBackend {
        diagnostic: NotificationDiagnostic,
        result: Result<(), NotificationError>,
        delivered: Mutex<Vec<PreparedNotification>>,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                diagnostic: NotificationDiagnostic::new(
                    NotificationStatus::Ready,
                    FocusStatus::AcceptsNotifications,
                ),
                result: Ok(()),
                delivered: Mutex::new(Vec::new()),
            }
        }
    }

    impl NotificationBackend for FakeBackend {
        fn diagnose(&self) -> NotificationDiagnostic {
            self.diagnostic
        }

        fn show(&self, notification: &PreparedNotification) -> Result<(), NotificationError> {
            self.delivered
                .lock()
                .expect("fake lock")
                .push(notification.clone());
            self.result
        }
    }
}
