//! Fixture-driven Codex approval-request adapter contracts.

use codex_notifier_codex_source::{
    ApprovalInstallation, ApprovalRequestedAdapter, ApprovalRequestedContext,
    CapabilityAvailability, CodexCapabilityReport, CodexCliVersion, CodexInterface, SourceError,
    TaskCompletedAdapter,
};
use codex_notifier_core::{EventId, EventKind, Privacy, Urgency};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const FIXTURE: &[u8] =
    include_bytes!("../../../tests/fixtures/codex-0.144.5-windows-cli-approval-requested.json");
const EVENT_ID: &str = "01983c8d-b800-7000-8000-000000000011";
const THREAD_HASH: &str = "sha256:7f4ca11f15a7b4fe520d4b29cee6d1afd72d3f822613393def4c983180593199";
const REQUIRED_PARAMS: &[&str] = &["threadId", "turnId", "itemId", "startedAtMs"];

fn fixture() -> Value {
    serde_json::from_slice(FIXTURE).expect("valid fixture envelope")
}

fn fixture_payload() -> Value {
    let mut payload = fixture().get("payload").cloned().expect("fixture payload");
    payload
        .as_object_mut()
        .expect("payload object")
        .remove("observed_keys");
    payload["params"]
        .as_object_mut()
        .expect("params object")
        .remove("observed_keys");
    payload
}

fn adapter() -> ApprovalRequestedAdapter {
    ApprovalRequestedAdapter::new(CodexCliVersion::V0_144_5, CodexInterface::AppServer)
        .expect("verified adapter")
}

fn context() -> ApprovalRequestedContext {
    ApprovalRequestedContext::new(
        "workstation",
        Some("codex-noti".to_owned()),
        Some("desktop".to_owned()),
    )
    .expect("trusted context")
}

fn received_at() -> OffsetDateTime {
    OffsetDateTime::parse("2026-07-28T17:27:31.000Z", &Rfc3339).expect("fixture time")
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
fn real_stage_01_fixture_has_verified_shape_and_canonical_snapshot() {
    let fixture = fixture();
    assert_eq!(fixture["source"]["codex_cli_version"], "0.144.5");
    assert_eq!(fixture["source"]["interface"], "codex app-server");
    assert_eq!(fixture["source"]["external_process_invoked"], true);
    assert!(fixture["source"]["schema_verified_at"].is_string());
    assert!(fixture["payload"]["params"]["startedAtMs"].is_i64());
    assert!(fixture["payload"]["params"]["commandActions"].is_array());
    assert!(fixture["payload"]["params"]["proposedExecpolicyAmendment"].is_array());

    let event = normalize(&fixture_payload()).expect("fixture must normalize");
    assert_eq!(adapter().version(), CodexCliVersion::V0_144_5);
    assert_eq!(event.kind(), EventKind::ApprovalRequested);
    assert_eq!(event.source().session_id(), Some(THREAD_HASH));
    assert_eq!(event.presentation().urgency(), Urgency::High);
    assert_eq!(event.presentation().privacy(), Privacy::Private);
    assert_eq!(
        serde_json::from_slice::<Value>(&event.to_json().expect("canonical JSON"))
            .expect("canonical value"),
        json!({
            "schema_version": 1,
            "event_id": EVENT_ID,
            "kind": "approval_requested",
            "occurred_at": "2026-07-28T17:27:30.047Z",
            "source": {
                "host_label": "workstation",
                "project_label": "codex-noti",
                "session_id": THREAD_HASH,
            },
            "presentation": {
                "title": "Codex needs approval",
                "body": "Open Codex to review the request.",
                "urgency": "high",
                "privacy": "private",
            },
            "routing": {"profile": "desktop"},
            "extensions": {},
        })
    );
}

#[test]
fn canonical_event_excludes_commands_decisions_paths_and_actions() {
    let mut payload = fixture_payload();
    payload["params"]["actionUrl"] = json!("codex://approve/request");
    assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));

    let payload = fixture_payload();
    let event = normalize(&payload).expect("fixture must normalize");
    let canonical = String::from_utf8(event.to_json().expect("canonical JSON")).expect("UTF-8");
    for sensitive in [
        "<redacted-request-id>",
        "<redacted-thread-id>",
        "<redacted-turn-id>",
        "<redacted-item-id>",
        "<redacted-environment-id>",
        "<redacted-command>",
        "<redacted-path>",
        "<redacted-amendment>",
        "acceptWithExecpolicyAmendment",
        "codex://",
    ] {
        assert!(!canonical.contains(sensitive));
    }
}

#[test]
fn missing_or_type_changed_required_fields_are_incompatible() {
    for field in ["id", "method", "params"] {
        let mut payload = fixture_payload();
        payload
            .as_object_mut()
            .expect("payload object")
            .remove(field);
        assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));
    }
    for field in REQUIRED_PARAMS {
        let mut payload = fixture_payload();
        payload["params"]
            .as_object_mut()
            .expect("params object")
            .remove(*field);
        assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));
    }

    for (field, incompatible) in [
        ("threadId", json!(null)),
        ("turnId", json!(7)),
        ("itemId", json!([])),
        ("startedAtMs", json!("1785259650047")),
    ] {
        let mut payload = fixture_payload();
        payload["params"][field] = incompatible;
        assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));
    }
}

#[test]
fn optional_types_unknown_fields_and_size_fail_closed() {
    for (field, incompatible) in [
        ("command", json!(false)),
        ("commandActions", json!("read")),
        ("environmentId", json!(9)),
        ("availableDecisions", json!({})),
        ("proposedExecpolicyAmendment", json!("allow")),
        ("reason", json!(42)),
    ] {
        let mut payload = fixture_payload();
        payload["params"][field] = incompatible;
        assert_eq!(normalize(&payload), Err(SourceError::IncompatiblePayload));
    }
    for (field, value) in [
        ("prompt", json!("sensitive prompt")),
        ("arguments", json!(["--secret"])),
        ("environment", json!({"TOKEN": "secret"})),
    ] {
        let mut payload = fixture_payload();
        payload["params"][field] = value;
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
fn capability_report_matches_adapter_selection_and_installation_behavior() {
    let app_server = CodexCapabilityReport::inspect("0.144.5", CodexInterface::AppServer);
    assert_eq!(app_server.version(), Some(CodexCliVersion::V0_144_5));
    assert_eq!(app_server.interface(), CodexInterface::AppServer);
    assert_eq!(
        app_server.task_completed(),
        CapabilityAvailability::UnsupportedInterface
    );
    assert_eq!(
        app_server.approval_requested(),
        CapabilityAvailability::Supported
    );
    assert_eq!(
        app_server.approval_installation(),
        ApprovalInstallation::ConfigureAppServer
    );
    assert!(
        app_server
            .approval_installation_notice()
            .contains("display-only")
    );
    assert!(
        ApprovalRequestedAdapter::new(CodexCliVersion::V0_144_5, app_server.interface()).is_ok()
    );
    assert_eq!(
        TaskCompletedAdapter::new(CodexCliVersion::V0_144_5, app_server.interface()),
        Err(SourceError::UnsupportedInterface)
    );

    let cli_hook = CodexCapabilityReport::inspect("0.144.5", CodexInterface::CliHook);
    assert_eq!(cli_hook.task_completed(), CapabilityAvailability::Supported);
    assert_eq!(
        cli_hook.approval_requested(),
        CapabilityAvailability::Unverified
    );
    assert_eq!(
        cli_hook.approval_installation(),
        ApprovalInstallation::ReportUnavailable
    );
    assert!(
        cli_hook
            .approval_installation_notice()
            .contains("no approval hook")
    );
    assert_eq!(
        ApprovalRequestedAdapter::new(CodexCliVersion::V0_144_5, cli_hook.interface()),
        Err(SourceError::UnsupportedInterface)
    );

    let unknown = CodexCapabilityReport::inspect("0.144.6", CodexInterface::AppServer);
    assert_eq!(unknown.version(), None);
    assert_eq!(
        unknown.approval_requested(),
        CapabilityAvailability::UnsupportedVersion
    );
    assert_eq!(
        unknown.approval_installation(),
        ApprovalInstallation::ReportUnavailable
    );
    assert!(!unknown.approval_installation_notice().contains("0.144.6"));
}
