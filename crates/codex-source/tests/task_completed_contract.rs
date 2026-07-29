//! Fixture-driven Codex task-completion adapter contracts.

use codex_notifier_codex_source::{
    CodexCliVersion, CodexInterface, SourceError, TaskCompletedAdapter, TaskCompletedContext,
};
use codex_notifier_core::{EventId, EventKind, Privacy, Urgency};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const FIXTURE: &[u8] =
    include_bytes!("../../../tests/fixtures/codex-0.144.5-windows-cli-task-completed.json");
const EVENT_ID: &str = "01983c8d-b800-7000-8000-000000000010";
const SESSION_HASH: &str =
    "sha256:be11567213447e6ad54c11442de1b3e099169b2f1ef5aa64728a95c935af981f";
const REQUIRED_FIELDS: &[&str] = &[
    "session_id",
    "transcript_path",
    "cwd",
    "hook_event_name",
    "model",
    "turn_id",
    "permission_mode",
    "stop_hook_active",
    "last_assistant_message",
];

fn fixture_payload() -> Value {
    let fixture: Value = serde_json::from_slice(FIXTURE).expect("valid fixture envelope");
    let mut payload = fixture.get("payload").cloned().expect("fixture payload");
    payload
        .as_object_mut()
        .expect("payload object")
        .remove("observed_keys");
    payload
}

fn adapter() -> TaskCompletedAdapter {
    TaskCompletedAdapter::new(CodexCliVersion::V0_144_5, CodexInterface::CliHook)
        .expect("verified adapter")
}

fn context() -> TaskCompletedContext {
    TaskCompletedContext::new(
        "workstation",
        Some("codex-noti".to_owned()),
        Some("desktop".to_owned()),
    )
    .expect("trusted context")
}

fn received_at() -> OffsetDateTime {
    OffsetDateTime::parse("2026-07-29T12:34:56.789Z", &Rfc3339).expect("fixture time")
}

fn normalize(payload: &Value) -> Result<codex_notifier_core::CanonicalEvent, SourceError> {
    adapter().normalize(
        &serde_json::to_vec(payload).expect("payload JSON"),
        &context(),
        EventId::parse(EVENT_ID).expect("UUIDv7"),
        received_at(),
    )
}

#[test]
fn stage_01_fixture_has_the_expected_canonical_snapshot() {
    let event = normalize(&fixture_payload()).expect("fixture must normalize");

    assert_eq!(adapter().version(), CodexCliVersion::V0_144_5);
    assert_eq!(event.kind(), EventKind::TaskCompleted);
    assert_eq!(event.source().session_id(), Some(SESSION_HASH));
    assert_eq!(event.presentation().urgency(), Urgency::Normal);
    assert_eq!(event.presentation().privacy(), Privacy::Private);
    assert_eq!(
        serde_json::from_slice::<Value>(&event.to_json().expect("canonical JSON"))
            .expect("canonical value"),
        json!({
            "schema_version": 1,
            "event_id": EVENT_ID,
            "kind": "task_completed",
            "occurred_at": "2026-07-29T12:34:56.789Z",
            "source": {
                "host_label": "workstation",
                "project_label": "codex-noti",
                "session_id": SESSION_HASH,
            },
            "presentation": {
                "title": "Codex task finished",
                "body": "Open Codex to review the result.",
                "urgency": "normal",
                "privacy": "private",
            },
            "routing": {"profile": "desktop"},
            "extensions": {},
        })
    );
}

#[test]
fn canonical_event_excludes_every_fixture_sensitive_value() {
    let payload = fixture_payload();
    let event = normalize(&payload).expect("fixture must normalize");
    let canonical = String::from_utf8(event.to_json().expect("canonical JSON")).expect("UTF-8");

    for field in [
        "session_id",
        "transcript_path",
        "cwd",
        "model",
        "turn_id",
        "last_assistant_message",
    ] {
        if let Some(value) = payload.get(field).and_then(Value::as_str) {
            assert!(
                !canonical.contains(value),
                "canonical event leaked fixture field {field}"
            );
        }
    }
    assert!(!canonical.contains("<redacted-path>"));
    assert!(!canonical.contains("prompt"));
    assert!(!canonical.contains("response"));
    assert!(!canonical.contains("environment"));
}

#[test]
fn every_missing_or_type_changed_field_is_incompatible() {
    for field in REQUIRED_FIELDS {
        let mut payload = fixture_payload();
        payload
            .as_object_mut()
            .expect("payload object")
            .remove(*field);
        assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));
    }

    for (field, incompatible) in [
        ("session_id", json!(null)),
        ("transcript_path", json!(7)),
        ("cwd", json!(false)),
        ("hook_event_name", json!(1)),
        ("model", json!([])),
        ("turn_id", json!({})),
        ("permission_mode", json!(5)),
        ("stop_hook_active", json!("false")),
        ("last_assistant_message", json!(42)),
    ] {
        let mut payload = fixture_payload();
        payload[field] = incompatible;
        assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));
    }
}

#[test]
fn unknown_and_sensitive_extra_fields_fail_closed() {
    for (field, value) in [
        ("unexpected", json!(true)),
        ("prompt", json!("sensitive prompt")),
        ("response", json!("sensitive response")),
        ("environment", json!({"SECRET": "value"})),
    ] {
        let mut payload = fixture_payload();
        payload[field] = value;
        assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));
    }
}

#[test]
fn invalid_values_and_oversized_input_are_rejected() {
    for (field, value) in [
        ("hook_event_name", json!("PermissionRequest")),
        ("session_id", json!("")),
        ("turn_id", json!("turn\n2")),
        ("cwd", json!("")),
        ("model", json!("model\u{7}")),
        ("permission_mode", json!("unknown")),
    ] {
        let mut payload = fixture_payload();
        payload[field] = value;
        assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));
    }

    assert_eq!(
        adapter().normalize(
            &vec![b' '; 32_769],
            &context(),
            EventId::parse(EVENT_ID).expect("UUIDv7"),
            received_at(),
        ),
        Err(SourceError::PayloadTooLarge)
    );
}

#[test]
fn version_interface_and_context_selection_are_explicit() {
    assert_eq!(
        "0.144.6".parse::<CodexCliVersion>(),
        Err(SourceError::UnsupportedVersion)
    );
    assert_eq!(
        TaskCompletedAdapter::new(CodexCliVersion::V0_144_5, CodexInterface::AppServer),
        Err(SourceError::UnsupportedInterface)
    );
    assert_eq!(
        TaskCompletedContext::new("host", Some("C:\\secret".to_owned()), None),
        Err(SourceError::InvalidContext)
    );
}
