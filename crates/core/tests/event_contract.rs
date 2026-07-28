//! Canonical event protocol version 1 contract and boundary tests.

use std::collections::BTreeMap;

use codex_notifier_core::limits::{
    MAX_BODY_SCALARS, MAX_EVENT_BYTES, MAX_EXTENSION_CONTAINER_ENTRIES, MAX_EXTENSION_ENTRIES,
    MAX_EXTENSION_STRING_SCALARS, MAX_EXTENSIONS_BYTES, MAX_FUTURE_SKEW_SECONDS,
    MAX_HOST_LABEL_SCALARS, MAX_PAST_AGE_SECONDS, MAX_PROJECT_LABEL_SCALARS,
    MAX_ROUTING_PROFILE_BYTES, MAX_SESSION_ID_BYTES, MAX_TITLE_SCALARS,
};
use codex_notifier_core::{
    CanonicalEvent, ErrorCode, EventError, EventId, EventKind, EventSource, Extensions, Field,
    Presentation, Privacy, Routing, Urgency,
};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const UUID_V7: &str = "01890f4d-e000-7000-8000-000000000000";

fn received_at() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("valid fixture timestamp")
}

fn extensions() -> Extensions {
    let mut values = BTreeMap::new();
    values.insert("codex.test".to_owned(), json!({"attempt": 1, "ok": true}));
    Extensions::new(values).expect("valid fixture extensions")
}

fn event(kind: EventKind) -> CanonicalEvent {
    CanonicalEvent::new(
        EventId::parse(UUID_V7).expect("valid UUIDv7 fixture"),
        kind,
        received_at() - Duration::seconds(1),
        EventSource::new(
            "workstation",
            Some("project".to_owned()),
            Some("session-1".to_owned()),
        )
        .expect("valid source"),
        Presentation::new("Title", "Body", Urgency::High, Privacy::Private)
            .expect("valid presentation"),
        Some(Routing::new("desktop_1").expect("valid route")),
        extensions(),
        received_at(),
    )
    .expect("valid canonical event")
}

#[test]
fn both_event_kinds_round_trip_without_field_loss() {
    for kind in [EventKind::ApprovalRequested, EventKind::TaskCompleted] {
        let original = event(kind);
        let encoded = original.to_json().expect("serialize event");
        let decoded = CanonicalEvent::from_json(&encoded, received_at()).expect("parse event");
        assert_eq!(decoded, original);
        assert_eq!(decoded.schema_version(), 1);
        assert_eq!(decoded.event_id().to_string(), UUID_V7);
        assert_eq!(decoded.kind(), kind);
        assert_eq!(decoded.source().host_label(), "workstation");
        assert_eq!(decoded.source().project_label(), Some("project"));
        assert_eq!(decoded.source().session_id(), Some("session-1"));
        assert_eq!(decoded.presentation().title(), "Title");
        assert_eq!(decoded.presentation().body(), "Body");
        assert_eq!(decoded.presentation().urgency(), Urgency::High);
        assert_eq!(decoded.presentation().privacy(), Privacy::Private);
        assert_eq!(decoded.routing().map(Routing::profile), Some("desktop_1"));
        assert_eq!(
            decoded.extensions().as_map(),
            original.extensions().as_map()
        );
    }
}

#[test]
fn serialization_is_compact_and_deterministic() {
    let encoded = event(EventKind::TaskCompleted)
        .to_json()
        .expect("serialize event");
    let text = String::from_utf8(encoded).expect("UTF-8 JSON");
    assert_eq!(
        text,
        concat!(
            r#"{"schema_version":1,"event_id":"01890f4d-e000-7000-8000-000000000000","kind":"task_completed","occurred_at":"2023-11-14T22:13:19.000Z","source":{"host_label":"workstation","project_label":"project","session_id":"session-1"},"presentation":{"title":"Title","body":"Body","urgency":"high","privacy":"private"},"routing":{"profile":"desktop_1"},"extensions":{"codex.test":{"attempt":1,"ok":true}}}"#,
        )
    );
}

#[test]
fn rejects_unknown_schema_kind_and_fields() {
    let mut value: Value = serde_json::from_slice(
        &event(EventKind::TaskCompleted)
            .to_json()
            .expect("serialize event"),
    )
    .expect("parse test JSON");

    value["schema_version"] = json!(2);
    assert_eq!(parse_value(&value), EventError::UnsupportedSchema);

    value["schema_version"] = json!(1);
    value["kind"] = json!("future_kind");
    assert_eq!(parse_value(&value), EventError::UnknownKind);

    value["kind"] = json!("task_completed");
    value["future"] = json!(true);
    assert_eq!(parse_value(&value), EventError::UnknownField);

    value
        .as_object_mut()
        .expect("event object")
        .remove("future");
    value["source"]["future"] = json!(true);
    assert_eq!(parse_value(&value), EventError::UnknownField);
}

#[test]
fn rejects_missing_fields_wrong_shapes_and_duplicate_keys() {
    assert_eq!(
        CanonicalEvent::from_json(br#"{"schema_version":1}"#, received_at())
            .expect_err("missing fields must fail")
            .code(),
        ErrorCode::MalformedJson
    );
    assert_eq!(
        CanonicalEvent::from_json(br#"{"schema_version":1,"schema_version":1}"#, received_at(),)
            .expect_err("duplicate key must fail"),
        EventError::MalformedJson
    );
    assert_eq!(
        CanonicalEvent::from_json(
            br#"{"extensions":{"codex.test":{"key":1,"key":2}}}"#,
            received_at(),
        )
        .expect_err("nested duplicate key must fail"),
        EventError::MalformedJson
    );
}

#[test]
fn event_ids_must_be_lowercase_canonical_uuid_v7() {
    assert_eq!(
        EventId::parse(UUID_V7)
            .expect("valid UUIDv7")
            .as_uuid()
            .get_version_num(),
        7
    );
    assert_eq!(
        EventId::parse("550e8400-e29b-41d4-a716-446655440000"),
        Err(EventError::InvalidEventId)
    );
    assert_eq!(
        EventId::parse(&UUID_V7.to_uppercase()),
        Err(EventError::InvalidEventId)
    );
    let v4 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("valid UUID");
    assert_eq!(EventId::from_uuid(v4), Err(EventError::InvalidEventId));
}

#[test]
fn timestamp_boundaries_are_inclusive() {
    for occurred_at in [
        received_at() - Duration::seconds(MAX_PAST_AGE_SECONDS),
        received_at() + Duration::seconds(MAX_FUTURE_SKEW_SECONDS),
    ] {
        assert!(event_at(occurred_at).is_ok());
    }
    assert_eq!(
        event_at(received_at() - Duration::seconds(MAX_PAST_AGE_SECONDS + 1)),
        Err(EventError::TimestampOutOfRange)
    );
    assert_eq!(
        event_at(received_at() + Duration::seconds(MAX_FUTURE_SKEW_SECONDS + 1)),
        Err(EventError::TimestampOutOfRange)
    );
}

#[test]
fn timestamps_require_canonical_utc_milliseconds() {
    let valid = event(EventKind::TaskCompleted)
        .to_json()
        .expect("serialize");
    let valid_text = String::from_utf8(valid).expect("UTF-8");
    for invalid in [
        valid_text.replace(".000Z", "Z"),
        valid_text.replace(".000Z", ".000000Z"),
        valid_text.replace(".000Z", ".000+00:00"),
    ] {
        assert_eq!(
            CanonicalEvent::from_json(invalid.as_bytes(), received_at())
                .expect_err("non-canonical time must fail"),
            EventError::InvalidTimestamp
        );
    }
}

#[test]
fn source_string_boundaries_and_paths_are_enforced() {
    assert!(
        EventSource::new(
            "h".repeat(MAX_HOST_LABEL_SCALARS),
            Some("p".repeat(MAX_PROJECT_LABEL_SCALARS)),
            Some("s".repeat(MAX_SESSION_ID_BYTES)),
        )
        .is_ok()
    );
    assert_eq!(
        EventSource::new("", None, None),
        Err(EventError::InvalidField(Field::HostLabel))
    );
    assert_eq!(
        EventSource::new("host", Some(String::new()), None),
        Err(EventError::InvalidField(Field::ProjectLabel))
    );
    assert_eq!(
        EventSource::new("host", None, Some(String::new())),
        Err(EventError::InvalidField(Field::SessionId))
    );
    assert_eq!(
        EventSource::new("h".repeat(MAX_HOST_LABEL_SCALARS + 1), None, None),
        Err(EventError::FieldTooLong(Field::HostLabel))
    );
    assert_eq!(
        EventSource::new(
            "host",
            Some("p".repeat(MAX_PROJECT_LABEL_SCALARS + 1)),
            None,
        ),
        Err(EventError::FieldTooLong(Field::ProjectLabel))
    );
    assert_eq!(
        EventSource::new("host", None, Some("s".repeat(MAX_SESSION_ID_BYTES + 1))),
        Err(EventError::FieldTooLong(Field::SessionId))
    );
    assert_eq!(
        EventSource::new("host", Some("C:\\private".to_owned()), None),
        Err(EventError::InvalidField(Field::ProjectLabel))
    );
    assert_eq!(
        EventSource::new("host/name", None, None),
        Err(EventError::InvalidField(Field::HostLabel))
    );
}

#[test]
fn presentation_boundaries_normalization_and_controls_are_enforced() {
    assert!(
        Presentation::new(
            "t".repeat(MAX_TITLE_SCALARS),
            "b".repeat(MAX_BODY_SCALARS),
            Urgency::Normal,
            Privacy::Public,
        )
        .is_ok()
    );
    assert_eq!(
        Presentation::new("", "body", Urgency::Normal, Privacy::Private),
        Err(EventError::InvalidField(Field::Title))
    );
    assert_eq!(
        Presentation::new("title", "", Urgency::Normal, Privacy::Private),
        Err(EventError::InvalidField(Field::Body))
    );
    assert_eq!(
        Presentation::new(
            "t".repeat(MAX_TITLE_SCALARS + 1),
            "body",
            Urgency::Normal,
            Privacy::Private,
        ),
        Err(EventError::FieldTooLong(Field::Title))
    );
    assert_eq!(
        Presentation::new(
            "title",
            "b".repeat(MAX_BODY_SCALARS + 1),
            Urgency::Normal,
            Privacy::Private,
        ),
        Err(EventError::FieldTooLong(Field::Body))
    );
    let normalized = Presentation::new("e\u{301}", "body", Urgency::Normal, Privacy::Private)
        .expect("NFC normalization");
    assert_eq!(normalized.title(), "é");
    assert_eq!(
        Presentation::new("bad\nlog", "body", Urgency::Normal, Privacy::Private),
        Err(EventError::InvalidField(Field::Title))
    );
}

#[test]
fn routing_boundaries_are_enforced() {
    assert!(Routing::new("A").is_ok());
    let maximum = format!("A{}", "_".repeat(MAX_ROUTING_PROFILE_BYTES - 1));
    assert!(Routing::new(maximum).is_ok());
    assert_eq!(
        Routing::new(format!("A{}", "_".repeat(MAX_ROUTING_PROFILE_BYTES))),
        Err(EventError::InvalidRouting)
    );
    for invalid in ["", "_profile", "profile.name", "profile/path", "配置"] {
        assert_eq!(Routing::new(invalid), Err(EventError::InvalidRouting));
    }
}

#[test]
fn extension_entry_namespace_and_container_boundaries_are_enforced() {
    assert!(Extensions::default().as_map().is_empty());
    assert!(single_named_extension("a.b", Value::Null).is_ok());
    assert!(single_named_extension(&format!("a.{}", "b".repeat(62)), Value::Null).is_ok());
    assert_eq!(
        single_named_extension(&format!("a.{}", "b".repeat(63)), Value::Null),
        Err(EventError::InvalidExtension)
    );
    let maximum_entries = (0..MAX_EXTENSION_ENTRIES)
        .map(|index| (format!("ns.key{index}"), json!(index)))
        .collect();
    assert!(Extensions::new(maximum_entries).is_ok());

    let too_many_entries = (0..=MAX_EXTENSION_ENTRIES)
        .map(|index| (format!("ns.key{index}"), json!(index)))
        .collect();
    assert_eq!(
        Extensions::new(too_many_entries),
        Err(EventError::InvalidExtension)
    );

    for invalid in ["plain", "UPPER.key", ".key", "ns.", "ns.key.dot!"] {
        let mut values = BTreeMap::new();
        values.insert(invalid.to_owned(), Value::Null);
        assert_eq!(Extensions::new(values), Err(EventError::InvalidExtension));
    }

    assert!(single_extension(json!(vec![0; MAX_EXTENSION_CONTAINER_ENTRIES])).is_ok());
    assert_eq!(
        single_extension(json!(vec![0; MAX_EXTENSION_CONTAINER_ENTRIES + 1])),
        Err(EventError::InvalidExtension)
    );

    let maximum_object = (0..MAX_EXTENSION_CONTAINER_ENTRIES)
        .map(|index| (format!("key{index}"), json!(index)))
        .collect::<serde_json::Map<_, _>>();
    assert!(single_extension(Value::Object(maximum_object)).is_ok());
    let too_large_object = (0..=MAX_EXTENSION_CONTAINER_ENTRIES)
        .map(|index| (format!("key{index}"), json!(index)))
        .collect::<serde_json::Map<_, _>>();
    assert_eq!(
        single_extension(Value::Object(too_large_object)),
        Err(EventError::InvalidExtension)
    );
}

#[test]
fn extension_string_depth_numeric_and_size_boundaries_are_enforced() {
    assert!(single_extension(json!("")).is_ok());
    assert!(single_extension(json!("x".repeat(MAX_EXTENSION_STRING_SCALARS))).is_ok());
    assert_eq!(
        single_extension(json!("x".repeat(MAX_EXTENSION_STRING_SCALARS + 1))),
        Err(EventError::InvalidExtension)
    );
    assert!(single_extension(json!({"a": {"b": {"c": null}}})).is_ok());
    assert_eq!(
        single_extension(json!({"a": {"b": {"c": {"d": null}}}})),
        Err(EventError::InvalidExtension)
    );
    assert_eq!(
        single_extension(json!(1.5)),
        Err(EventError::InvalidExtension)
    );

    let maximum_key =
        serde_json::Map::from_iter([("k".repeat(MAX_EXTENSION_STRING_SCALARS), Value::Null)]);
    assert!(single_extension(Value::Object(maximum_key)).is_ok());
    let overlong_key =
        serde_json::Map::from_iter([("k".repeat(MAX_EXTENSION_STRING_SCALARS + 1), Value::Null)]);
    assert_eq!(
        single_extension(Value::Object(overlong_key)),
        Err(EventError::InvalidExtension)
    );

    let at_byte_limit = sized_extension_map(253);
    assert_eq!(
        serde_json::to_vec(&at_byte_limit)
            .expect("serialize extensions")
            .len(),
        MAX_EXTENSIONS_BYTES
    );
    assert!(Extensions::new(at_byte_limit).is_ok());
    let oversized = sized_extension_map(254);
    assert_eq!(
        serde_json::to_vec(&oversized)
            .expect("serialize extensions")
            .len(),
        MAX_EXTENSIONS_BYTES + 1
    );
    assert_eq!(
        Extensions::new(oversized),
        Err(EventError::InvalidExtension)
    );
}

#[test]
fn raw_payload_byte_boundary_is_checked_before_parsing() {
    let at_limit = vec![b' '; MAX_EVENT_BYTES];
    assert_eq!(
        CanonicalEvent::from_json(&at_limit, received_at())
            .expect_err("invalid JSON at exact byte limit"),
        EventError::MalformedJson
    );
    let over_limit = vec![b' '; MAX_EVENT_BYTES + 1];
    assert_eq!(
        CanonicalEvent::from_json(&over_limit, received_at())
            .expect_err("oversized input must fail before parsing"),
        EventError::PayloadTooLarge
    );
}

#[test]
fn all_errors_have_stable_safe_codes() {
    let cases = [
        (EventError::MalformedJson, "malformed_json"),
        (EventError::PayloadTooLarge, "payload_too_large"),
        (EventError::UnsupportedSchema, "unsupported_schema"),
        (EventError::UnknownField, "unknown_field"),
        (EventError::InvalidShape, "malformed_json"),
        (EventError::InvalidEventId, "invalid_event_id"),
        (EventError::UnknownKind, "unknown_kind"),
        (EventError::InvalidTimestamp, "timestamp_out_of_range"),
        (EventError::TimestampOutOfRange, "timestamp_out_of_range"),
        (EventError::FieldTooLong(Field::Body), "field_too_long"),
        (EventError::InvalidField(Field::Body), "malformed_json"),
        (EventError::InvalidRouting, "invalid_routing"),
        (EventError::InvalidExtension, "invalid_extension"),
    ];
    for (error, expected) in cases {
        assert_eq!(error.code().as_str(), expected);
        assert!(!error.to_string().contains(['\n', '\r', '\u{1b}']));
    }
}

fn parse_value(value: &Value) -> EventError {
    let bytes = serde_json::to_vec(value).expect("serialize modified JSON");
    CanonicalEvent::from_json(&bytes, received_at()).expect_err("event must be rejected")
}

fn event_at(occurred_at: OffsetDateTime) -> Result<CanonicalEvent, EventError> {
    CanonicalEvent::new(
        EventId::parse(UUID_V7).expect("valid UUIDv7 fixture"),
        EventKind::TaskCompleted,
        occurred_at,
        EventSource::new("host", None, None).expect("valid source"),
        Presentation::new("title", "body", Urgency::Normal, Privacy::Private)
            .expect("valid presentation"),
        None,
        Extensions::default(),
        received_at(),
    )
}

fn single_extension(value: Value) -> Result<Extensions, EventError> {
    single_named_extension("codex.test", value)
}

fn single_named_extension(key: &str, value: Value) -> Result<Extensions, EventError> {
    let mut values = BTreeMap::new();
    values.insert(key.to_owned(), value);
    Extensions::new(values)
}

fn sized_extension_map(last_value_length: usize) -> BTreeMap<String, Value> {
    (0..MAX_EXTENSION_CONTAINER_ENTRIES)
        .map(|index| {
            let length = if index == MAX_EXTENSION_CONTAINER_ENTRIES - 1 {
                last_value_length
            } else {
                244
            };
            (format!("ns.k{index}"), json!("x".repeat(length)))
        })
        .collect()
}
