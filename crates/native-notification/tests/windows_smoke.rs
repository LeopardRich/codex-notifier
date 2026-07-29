//! Explicit interactive Windows Toast smoke test.

#![cfg(windows)]

use std::collections::BTreeMap;
use std::sync::Arc;

use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use codex_notifier_native_notification::{
    NativeNotificationAdapter, NotificationBackend, NotificationContentPolicy, NotificationPolicy,
    NotificationStatus, WindowsApplicationId, WindowsNotificationBackend,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn event(kind: EventKind, id: &str, title: &str, body: &str, urgency: Urgency) -> CanonicalEvent {
    let occurred_at =
        OffsetDateTime::parse("2026-07-29T05:00:00.000Z", &Rfc3339).expect("valid smoke-test time");
    CanonicalEvent::new(
        EventId::parse(id).expect("UUIDv7 smoke-test ID"),
        kind,
        occurred_at,
        EventSource::new("smoke-test", None, None).expect("safe smoke-test source"),
        Presentation::new(title, body, urgency, Privacy::Private)
            .expect("safe smoke-test presentation"),
        None,
        Extensions::new(BTreeMap::new()).expect("empty extensions"),
        occurred_at,
    )
    .expect("valid smoke-test event")
}

#[test]
#[ignore = "requires an interactive Windows session and displays two real Toast notifications"]
fn displays_task_completion_and_approval_request_toasts() {
    let backend = Arc::new(WindowsNotificationBackend::codex_notifier());
    let diagnostic = backend.diagnose();
    assert_eq!(
        diagnostic.status(),
        NotificationStatus::Ready,
        "Windows notification diagnostic: {diagnostic:?}"
    );
    let adapter = NativeNotificationAdapter::new(
        backend,
        NotificationPolicy::new(NotificationContentPolicy::Private, false),
    );
    adapter
        .deliver_now(&event(
            EventKind::TaskCompleted,
            "01983c8d-b800-7000-8000-000000000012",
            "not displayed",
            "not displayed",
            Urgency::Normal,
        ))
        .expect("Windows accepted the task-completion Toast");
    adapter
        .deliver_now(&event(
            EventKind::ApprovalRequested,
            "01983c8d-b800-7000-8000-000000000013",
            "not displayed",
            "not displayed",
            Urgency::High,
        ))
        .expect("Windows accepted the approval-request Toast");
}

#[test]
#[ignore = "requires temporarily disabling product notifications in Windows settings"]
fn reports_real_application_disabled_state() {
    let backend = WindowsNotificationBackend::codex_notifier();
    let diagnostic = backend.diagnose();
    assert_eq!(
        diagnostic.status(),
        NotificationStatus::DisabledForApplication,
        "Windows notification diagnostic: {diagnostic:?}"
    );
}

#[test]
#[ignore = "requires an interactive Windows session and an unregistered diagnostic identity"]
fn reports_real_unregistered_application_identity() {
    let identity = WindowsApplicationId::new("LeopardRich.CodexNotifier.MissingIdentityProbe")
        .expect("valid diagnostic AUMID");
    let backend = WindowsNotificationBackend::new(identity);
    let diagnostic = backend.diagnose();
    assert_eq!(
        diagnostic.status(),
        NotificationStatus::ApplicationIdentityMissing,
        "Windows notification diagnostic: {diagnostic:?}"
    );
}

#[test]
#[ignore = "requires a non-interactive Windows Session 0 process"]
fn reports_real_non_interactive_session() {
    let backend = WindowsNotificationBackend::codex_notifier();
    let diagnostic = backend.diagnose();
    assert_eq!(
        diagnostic.status(),
        NotificationStatus::NoInteractiveSession,
        "Windows notification diagnostic: {diagnostic:?}"
    );
}
