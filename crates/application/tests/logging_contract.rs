//! Structured logging privacy, injection, filtering, and retention tests.

use std::collections::BTreeMap;

use codex_notifier_application::{
    CorrelationId, EmitOutcome, EventLogRecord, EventOutcome, EventStatus, InMemoryLogSink,
    LogError, LogSeverity, LogSink, LogTiming, RotationPolicy, SafeErrorCode,
};
use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use time::{Duration, OffsetDateTime};

const UUID_V7: &str = "01890f4d-e000-7000-8000-000000000000";

fn event() -> CanonicalEvent {
    let received_at = OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("fixture time");
    CanonicalEvent::new(
        EventId::parse(UUID_V7).expect("fixture UUIDv7"),
        EventKind::ApprovalRequested,
        received_at - Duration::seconds(1),
        EventSource::new(
            "private-host",
            Some("private-project".to_owned()),
            Some("private-session".to_owned()),
        )
        .expect("fixture source"),
        Presentation::new(
            "Secret approval title",
            "password=hunter2 fake_field=forged",
            Urgency::High,
            Privacy::Public,
        )
        .expect("fixture presentation"),
        None,
        Extensions::new(BTreeMap::new()).expect("empty extensions"),
        received_at,
    )
    .expect("fixture event")
}

fn record(timestamp_ms: i64, severity: LogSeverity, correlation: &str) -> EventLogRecord {
    EventLogRecord::for_event(
        &event(),
        severity,
        LogTiming::new(timestamp_ms, Some(25)).expect("fixture timing"),
        CorrelationId::parse(correlation).expect("fixture correlation"),
        EventOutcome::new(EventStatus::Accepted, None).expect("fixture outcome"),
    )
}

#[test]
fn snapshot_contains_only_typed_safe_event_metadata() {
    let record = record(1_700_000_000_000, LogSeverity::Info, "corr-01");
    let encoded = record.to_json_line().expect("encode record");
    assert_eq!(
        encoded,
        concat!(
            r#"{"timestamp_ms":1700000000000,"severity":"info","event_id":"01890f4d-e000-7000-8000-000000000000","event_kind":"approval_requested","status":"accepted","duration_ms":25,"correlation_id":"corr-01","error_code":null}"#,
        )
    );
    for forbidden in [
        "Secret approval title",
        "hunter2",
        "fake_field",
        "private-host",
        "private-project",
        "private-session",
    ] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn rejects_line_control_terminal_and_field_injection_inputs() {
    for value in [
        "corr\nforged=true",
        "corr\rforged=true",
        "corr\u{1b}[31m",
        "corr\0tail",
        "corr\",\"body\":\"leak",
    ] {
        let error = CorrelationId::parse(value).expect_err("must reject correlation injection");
        assert_eq!(error, LogError::InvalidCorrelationId);
        assert!(!error.to_string().contains(value));
    }
    for value in [
        "denied\nstatus=delivered",
        "denied\u{1b}[2J",
        "denied,body=leak",
        "DENIED",
    ] {
        let error = SafeErrorCode::parse(value).expect_err("must reject code injection");
        assert_eq!(error, LogError::InvalidErrorCode);
        assert!(!error.to_string().contains(value));
    }
}

#[test]
fn every_enabled_log_level_uses_the_same_redacted_schema() {
    let levels = [
        LogSeverity::Error,
        LogSeverity::Warn,
        LogSeverity::Info,
        LogSeverity::Debug,
        LogSeverity::Trace,
    ];
    for level in levels {
        let sink = InMemoryLogSink::new(level, RotationPolicy::default());
        assert_eq!(
            sink.emit(&record(100, level, "corr-level"))
                .expect("emit record"),
            EmitOutcome::Recorded
        );
        let snapshot = sink.records().expect("memory snapshot").join("\n");
        assert!(!snapshot.contains("Secret approval title"));
        assert!(!snapshot.contains("hunter2"));
        assert!(!snapshot.contains("private-host"));
        assert!(snapshot.contains("\"event_id\""));
        assert!(snapshot.contains("\"event_kind\""));
    }

    let info = InMemoryLogSink::new(LogSeverity::Info, RotationPolicy::default());
    assert_eq!(
        info.emit(&record(100, LogSeverity::Debug, "corr-filter"))
            .expect("filter record"),
        EmitOutcome::Filtered
    );
    assert!(info.records().expect("memory snapshot").is_empty());
}

#[test]
fn failure_diagnostics_use_fixed_text_and_safe_codes() {
    let code = SafeErrorCode::parse("ssh_authentication_failed").expect("safe code");
    let outcome = EventOutcome::new(EventStatus::Rejected, Some(code)).expect("failure outcome");
    let record = EventLogRecord::for_event(
        &event(),
        LogSeverity::Error,
        LogTiming::new(100, Some(5)).expect("timing"),
        CorrelationId::parse("corr-error").expect("correlation"),
        outcome,
    );
    let diagnostic = record.safe_diagnostic();
    assert_eq!(diagnostic.message(), "Event rejected.");
    assert_eq!(
        diagnostic.to_human_line(),
        "Event rejected. error_code=ssh_authentication_failed"
    );
    assert_eq!(
        diagnostic.to_json().expect("diagnostic JSON"),
        r#"{"status":"rejected","error_code":"ssh_authentication_failed","message":"Event rejected."}"#
    );
    assert!(
        !diagnostic
            .to_json()
            .expect("diagnostic JSON")
            .contains("hunter2")
    );

    assert_eq!(
        EventOutcome::new(EventStatus::Rejected, None),
        Err(LogError::InvalidOutcome)
    );
    assert_eq!(
        EventOutcome::new(
            EventStatus::Delivered,
            Some(SafeErrorCode::parse("unexpected").expect("safe code")),
        ),
        Err(LogError::InvalidOutcome)
    );
}

#[test]
fn rotation_size_and_count_boundaries_are_exact() {
    let first = record(100, LogSeverity::Info, "corr-01");
    let line_bytes = first.to_json_line().expect("encode").len() + 1;
    let policy = RotationPolicy::new(line_bytes * 2, 2, 10_000).expect("rotation policy");
    let sink = InMemoryLogSink::new(LogSeverity::Info, policy);

    sink.emit(&first).expect("first record");
    sink.emit(&record(101, LogSeverity::Info, "corr-02"))
        .expect("exact-boundary record");
    assert_eq!(sink.segment_count().expect("segment count"), 1);

    sink.emit(&record(102, LogSeverity::Info, "corr-03"))
        .expect("rotated record");
    assert_eq!(sink.segment_count().expect("segment count"), 2);

    sink.emit(&record(103, LogSeverity::Info, "corr-04"))
        .expect("second segment exact boundary");
    sink.emit(&record(104, LogSeverity::Info, "corr-05"))
        .expect("retention record");
    let records = sink.records().expect("retained records").join("\n");
    assert_eq!(sink.segment_count().expect("segment count"), 2);
    assert!(!records.contains("corr-01"));
    assert!(!records.contains("corr-02"));
    assert!(records.contains("corr-03"));
    assert!(records.contains("corr-05"));
}

#[test]
fn age_retention_is_inclusive_at_the_exact_boundary() {
    let first = record(0, LogSeverity::Info, "corr-age-1");
    let line_bytes = record(1_001, LogSeverity::Info, "corr-age-3")
        .to_json_line()
        .expect("encode")
        .len()
        + 1;
    let policy = RotationPolicy::new(line_bytes, 10, 1_000).expect("rotation policy");
    let sink = InMemoryLogSink::new(LogSeverity::Info, policy);

    sink.emit(&first).expect("first record");
    sink.emit(&record(1_000, LogSeverity::Info, "corr-age-2"))
        .expect("exact-age record");
    assert_eq!(sink.segment_count().expect("segment count"), 2);
    assert!(
        sink.records()
            .expect("records")
            .join("\n")
            .contains("corr-age-1")
    );

    sink.emit(&record(1_001, LogSeverity::Info, "corr-age-3"))
        .expect("past-age record");
    let records = sink.records().expect("records").join("\n");
    assert!(!records.contains("corr-age-1"));
    assert!(records.contains("corr-age-2"));
    assert!(records.contains("corr-age-3"));
}

#[test]
fn duration_rotation_and_record_size_limits_are_bounded() {
    assert_eq!(
        LogTiming::new(0, Some(7 * 24 * 60 * 60 * 1_000 + 1)),
        Err(LogError::InvalidDuration)
    );
    assert_eq!(
        RotationPolicy::new(0, 1, 1),
        Err(LogError::InvalidRotationPolicy)
    );
    assert_eq!(
        RotationPolicy::new(1, 0, 1),
        Err(LogError::InvalidRotationPolicy)
    );
    assert_eq!(
        RotationPolicy::new(1, 1, 0),
        Err(LogError::InvalidRotationPolicy)
    );

    let sink = InMemoryLogSink::new(
        LogSeverity::Trace,
        RotationPolicy::new(1, 1, 1).expect("minimal valid policy"),
    );
    assert_eq!(
        sink.emit(&record(0, LogSeverity::Info, "corr-large")),
        Err(LogError::RecordTooLarge)
    );
}
