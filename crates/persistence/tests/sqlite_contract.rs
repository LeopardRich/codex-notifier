//! Crash recovery, deduplication, retention, errors, and migration contracts.

use std::collections::BTreeMap;

use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use codex_notifier_persistence::{
    CURRENT_SCHEMA_VERSION, EnqueueOutcome, PersistenceError, ReceiptOutcome, RetryOutcome,
    SqliteStore, StorePolicy,
};
use rusqlite::{Connection, TransactionBehavior, params};
use tempfile::tempdir;
use time::{Duration, OffsetDateTime};

const NOW_MS: i64 = 1_700_000_000_000;
const ID_1: &str = "01890f4d-e000-7000-8000-000000000001";
const ID_2: &str = "01890f4d-e000-7000-8000-000000000002";
const ID_3: &str = "01890f4d-e000-7000-8000-000000000003";
const ID_4: &str = "01890f4d-e000-7000-8000-000000000004";

fn event(id: &str, kind: EventKind, occurred_at_ms: i64) -> CanonicalEvent {
    let received_at = timestamp(NOW_MS);
    CanonicalEvent::new(
        EventId::parse(id).expect("fixture UUIDv7"),
        kind,
        timestamp(occurred_at_ms),
        EventSource::new("workstation", Some("project".to_owned()), None).expect("fixture source"),
        Presentation::new(
            "Private title",
            "Private body that is stored only in the canonical outbox row",
            Urgency::Normal,
            Privacy::Private,
        )
        .expect("fixture presentation"),
        None,
        Extensions::new(BTreeMap::new()).expect("fixture extensions"),
        received_at,
    )
    .expect("fixture event")
}

fn timestamp(milliseconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000)
        .expect("fixture timestamp")
}

fn id(value: &str) -> EventId {
    EventId::parse(value).expect("fixture UUIDv7")
}

#[test]
fn committed_and_leased_events_recover_after_process_reopen() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("state.sqlite3");
    let policy = StorePolicy::default()
        .with_lease_duration_ms(1_000)
        .expect("policy");
    let fixture = event(ID_1, EventKind::TaskCompleted, NOW_MS - 1_000);

    {
        let mut store = SqliteStore::open(&path, policy).expect("open database");
        assert_eq!(
            store.enqueue(&fixture, NOW_MS).expect("enqueue"),
            EnqueueOutcome::Enqueued
        );
    }
    {
        let mut store = SqliteStore::open(&path, policy).expect("reopen database");
        assert_eq!(store.queue_len().expect("queue length"), 1);
        let leased = store
            .lease_next(NOW_MS, "lease-01")
            .expect("lease")
            .expect("available event");
        assert_eq!(leased.event(), &fixture);
        assert_eq!(leased.attempt(), 1);
    }
    {
        let mut store = SqliteStore::open(&path, policy).expect("reopen leased database");
        assert!(
            store
                .lease_next(NOW_MS + 999, "lease-02")
                .expect("lease before expiry")
                .is_none()
        );
        let recovered = store
            .lease_next(NOW_MS + 1_000, "lease-02")
            .expect("lease at expiry")
            .expect("recovered event");
        assert_eq!(recovered.attempt(), 2);
        store
            .acknowledge(fixture.event_id(), "lease-02", NOW_MS + 1_001)
            .expect("acknowledge");
        assert_eq!(store.queue_len().expect("queue length"), 0);
        assert_eq!(store.receipt_count().expect("receipt count"), 1);
        assert_eq!(
            store
                .enqueue(&fixture, NOW_MS + 1_001)
                .expect("deduplicate"),
            EnqueueOutcome::Duplicate
        );
    }
}

#[test]
fn interrupted_uncommitted_enqueue_leaves_no_partial_event() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("state.sqlite3");
    SqliteStore::open(&path, StorePolicy::default()).expect("create schema");
    let fixture = event(ID_1, EventKind::TaskCompleted, NOW_MS - 1_000);
    let bytes = fixture.to_json().expect("canonical JSON");

    let mut raw = Connection::open(&path).expect("raw connection");
    {
        let transaction = raw
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("begin transaction");
        transaction
            .execute(
                "INSERT INTO outbox (
                    event_id, event_json, kind, occurred_at_ms, enqueued_at_ms,
                    available_at_ms, attempts, state
                 ) VALUES (?1, ?2, 'task_completed', ?3, ?4, ?4, 0, 'queued')",
                params![ID_1, bytes, NOW_MS - 1_000, NOW_MS],
            )
            .expect("uncommitted insert");
    }
    drop(raw);

    let store = SqliteStore::open(&path, StorePolicy::default()).expect("reopen database");
    assert_eq!(store.queue_len().expect("queue length"), 0);
}

#[test]
fn duplicate_submissions_and_delivery_receipts_are_idempotent() {
    let mut store = SqliteStore::open_in_memory(StorePolicy::default()).expect("memory store");
    let fixture = event(ID_1, EventKind::ApprovalRequested, NOW_MS - 1_000);
    assert_eq!(
        store.enqueue(&fixture, NOW_MS).expect("first enqueue"),
        EnqueueOutcome::Enqueued
    );
    assert_eq!(
        store.enqueue(&fixture, NOW_MS).expect("second enqueue"),
        EnqueueOutcome::Duplicate
    );
    assert_eq!(store.queue_len().expect("queue length"), 1);

    assert_eq!(
        store
            .record_delivery(id(ID_2), NOW_MS)
            .expect("first receipt"),
        ReceiptOutcome::Recorded
    );
    assert_eq!(
        store
            .record_delivery(id(ID_2), NOW_MS + 1)
            .expect("duplicate receipt"),
        ReceiptOutcome::Duplicate
    );
    assert_eq!(store.receipt_count().expect("receipt count"), 1);
}

#[test]
fn retries_schedule_then_dead_letter_at_the_attempt_limit() {
    let policy = StorePolicy::default().with_max_attempts(2).expect("policy");
    let mut store = SqliteStore::open_in_memory(policy).expect("memory store");
    let fixture = event(ID_1, EventKind::ApprovalRequested, NOW_MS - 1_000);
    store.enqueue(&fixture, NOW_MS).expect("enqueue");

    store
        .lease_next(NOW_MS, "lease-01")
        .expect("first lease")
        .expect("event");
    assert_eq!(
        store
            .retry(
                fixture.event_id(),
                "lease-01",
                NOW_MS,
                NOW_MS + 100,
                "ssh_unavailable",
            )
            .expect("schedule retry"),
        RetryOutcome::Scheduled
    );
    assert!(
        store
            .lease_next(NOW_MS + 99, "lease-02")
            .expect("before retry")
            .is_none()
    );
    store
        .lease_next(NOW_MS + 100, "lease-02")
        .expect("second lease")
        .expect("event");
    assert_eq!(
        store
            .retry(
                fixture.event_id(),
                "lease-02",
                NOW_MS + 100,
                NOW_MS + 200,
                "ssh_unavailable",
            )
            .expect("exhaust retry"),
        RetryOutcome::DeadLettered
    );
    assert_eq!(store.queue_len().expect("queue length"), 0);
    let dead = store
        .dead_letter_entry(fixture.event_id())
        .expect("dead letter query")
        .expect("dead letter");
    assert_eq!(dead.event_kind(), EventKind::ApprovalRequested);
    assert_eq!(dead.error_code(), "retry_exhausted");
    assert_eq!(dead.failed_at_ms(), NOW_MS + 100);
}

#[test]
fn explicit_dead_letters_store_only_safe_metadata() {
    let mut store = SqliteStore::open_in_memory(StorePolicy::default()).expect("memory store");
    let fixture = event(ID_1, EventKind::TaskCompleted, NOW_MS - 1_000);
    store.enqueue(&fixture, NOW_MS).expect("enqueue");
    store
        .lease_next(NOW_MS, "lease-01")
        .expect("lease")
        .expect("event");
    store
        .dead_letter(
            fixture.event_id(),
            "lease-01",
            "authentication_failed",
            NOW_MS + 1,
        )
        .expect("dead letter");
    let dead = store
        .dead_letter_entry(fixture.event_id())
        .expect("query")
        .expect("dead letter");
    assert_eq!(dead.event_id(), fixture.event_id());
    assert_eq!(dead.error_code(), "authentication_failed");
    assert_eq!(store.queue_len().expect("queue length"), 0);
}

#[test]
fn queue_age_capacity_and_transition_values_are_bounded() {
    let policy = StorePolicy::default()
        .with_queue_limit(1)
        .expect("queue policy")
        .with_max_event_age_ms(1_000)
        .expect("age policy");
    let mut store = SqliteStore::open_in_memory(policy).expect("memory store");
    let first = event(ID_1, EventKind::TaskCompleted, NOW_MS - 1_000);
    let second = event(ID_2, EventKind::TaskCompleted, NOW_MS - 1_000);
    let expired = event(ID_3, EventKind::TaskCompleted, NOW_MS - 1_001);
    assert_eq!(
        store.enqueue(&first, NOW_MS).expect("exact age"),
        EnqueueOutcome::Enqueued
    );
    assert_eq!(
        store.enqueue(&second, NOW_MS),
        Err(PersistenceError::QueueFull)
    );
    assert_eq!(
        store.enqueue(&expired, NOW_MS),
        Err(PersistenceError::EventExpired)
    );
    assert_eq!(
        store.lease_next(NOW_MS, "bad\nlease"),
        Err(PersistenceError::InvalidValue)
    );
}

#[test]
fn receipt_count_and_age_retention_boundaries_are_deterministic() {
    let policy = StorePolicy::default()
        .with_receipt_retention(2, 1_000)
        .expect("receipt policy");
    let mut store = SqliteStore::open_in_memory(policy).expect("memory store");
    store.record_delivery(id(ID_1), 0).expect("receipt 1");
    store.record_delivery(id(ID_2), 1_000).expect("receipt 2");
    assert_eq!(store.receipt_count().expect("receipt count"), 2);
    store.record_delivery(id(ID_3), 1_000).expect("receipt 3");
    assert_eq!(store.receipt_count().expect("count limited"), 2);
    store.record_delivery(id(ID_4), 1_001).expect("receipt 4");
    assert_eq!(store.receipt_count().expect("age limited"), 2);
    assert_eq!(
        store.record_delivery(id(ID_1), 1_001).expect("expired ID"),
        ReceiptOutcome::Recorded
    );
}

#[test]
fn locked_and_unwritable_databases_have_distinct_errors() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("state.sqlite3");
    let mut store = SqliteStore::open(&path, StorePolicy::default()).expect("open store");
    let raw = Connection::open(&path).expect("second connection");
    raw.execute_batch("BEGIN EXCLUSIVE")
        .expect("exclusive lock");
    let fixture = event(ID_1, EventKind::TaskCompleted, NOW_MS - 1_000);
    assert_eq!(
        store.enqueue(&fixture, NOW_MS),
        Err(PersistenceError::DatabaseLocked)
    );
    raw.execute_batch("ROLLBACK").expect("release lock");

    assert!(matches!(
        SqliteStore::open(directory.path(), StorePolicy::default()),
        Err(PersistenceError::StorageUnwritable)
    ));
}

#[test]
fn migrates_every_schema_fixture_without_losing_pending_events() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("legacy.sqlite3");
    let fixture = event(ID_1, EventKind::TaskCompleted, NOW_MS - 1_000);
    let bytes = fixture.to_json().expect("canonical JSON");
    let raw = Connection::open(&path).expect("legacy database");
    raw.execute_batch(include_str!("fixtures/schema-v0.sql"))
        .expect("legacy schema");
    raw.execute(
        "INSERT INTO outbox (
            event_id, event_json, kind, occurred_at_ms, enqueued_at_ms,
            available_at_ms, attempts
         ) VALUES (?1, ?2, 'task_completed', ?3, ?4, ?4, 0)",
        params![ID_1, bytes, NOW_MS - 1_000, NOW_MS],
    )
    .expect("legacy pending event");
    drop(raw);

    let mut store = SqliteStore::open(&path, StorePolicy::default()).expect("migrate database");
    assert_eq!(
        store.schema_version().expect("schema version"),
        CURRENT_SCHEMA_VERSION
    );
    assert_eq!(store.queue_len().expect("queue length"), 1);
    let leased = store
        .lease_next(NOW_MS, "lease-migrated")
        .expect("lease migrated")
        .expect("pending event");
    assert_eq!(leased.event(), &fixture);
}

#[test]
fn migration_failure_preserves_source_and_newer_schema_is_rejected() {
    let directory = tempdir().expect("temporary directory");
    let broken_path = directory.path().join("broken.sqlite3");
    let raw = Connection::open(&broken_path).expect("broken database");
    raw.execute_batch(
        "CREATE TABLE outbox(event_id TEXT PRIMARY KEY);
         INSERT INTO outbox(event_id) VALUES ('preserve-me');
         PRAGMA user_version = 0;",
    )
    .expect("broken legacy schema");
    drop(raw);
    assert!(matches!(
        SqliteStore::open(&broken_path, StorePolicy::default()),
        Err(PersistenceError::MigrationFailed)
    ));
    let raw = Connection::open(&broken_path).expect("reopen source");
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("source version");
    let count: i64 = raw
        .query_row("SELECT COUNT(*) FROM outbox", [], |row| row.get(0))
        .expect("source row");
    assert_eq!(version, 0);
    assert_eq!(count, 1);

    let newer_path = directory.path().join("newer.sqlite3");
    let raw = Connection::open(&newer_path).expect("newer database");
    raw.pragma_update(None, "user_version", 99)
        .expect("newer version");
    drop(raw);
    assert!(matches!(
        SqliteStore::open(&newer_path, StorePolicy::default()),
        Err(PersistenceError::UnsupportedSchema)
    ));
}

#[test]
fn corrupted_indexed_payload_is_rejected_without_deleting_the_row() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("state.sqlite3");
    let fixture = event(ID_1, EventKind::TaskCompleted, NOW_MS - 1_000);
    let mut store = SqliteStore::open(&path, StorePolicy::default()).expect("store");
    store.enqueue(&fixture, NOW_MS).expect("enqueue");
    drop(store);
    let raw = Connection::open(&path).expect("raw connection");
    raw.execute(
        "UPDATE outbox SET event_json = '{\"schema_version\":1}' WHERE event_id = ?1",
        [ID_1],
    )
    .expect("corrupt payload");
    drop(raw);

    let mut store = SqliteStore::open(&path, StorePolicy::default()).expect("reopen store");
    assert_eq!(
        store.lease_next(NOW_MS, "lease-corrupt"),
        Err(PersistenceError::CorruptData)
    );
    assert_eq!(store.queue_len().expect("row retained"), 1);
}

#[test]
fn policy_and_safe_error_boundaries_reject_untrusted_values() {
    assert_eq!(
        StorePolicy::default().with_queue_limit(0),
        Err(PersistenceError::InvalidValue)
    );
    assert_eq!(
        StorePolicy::default().with_max_attempts(0),
        Err(PersistenceError::InvalidValue)
    );
    let mut store = SqliteStore::open_in_memory(StorePolicy::default()).expect("memory store");
    let fixture = event(ID_1, EventKind::TaskCompleted, NOW_MS - 1_000);
    store.enqueue(&fixture, NOW_MS).expect("enqueue");
    store
        .lease_next(NOW_MS, "lease-01")
        .expect("lease")
        .expect("event");
    assert_eq!(
        store.retry(
            fixture.event_id(),
            "lease-01",
            NOW_MS,
            NOW_MS + 1,
            "error\nforged",
        ),
        Err(PersistenceError::InvalidValue)
    );
    assert_eq!(
        store.retry(
            fixture.event_id(),
            "lease-01",
            NOW_MS,
            NOW_MS - 1,
            "ssh_unavailable",
        ),
        Err(PersistenceError::InvalidValue)
    );
}
