CREATE TABLE outbox (
    event_id TEXT PRIMARY KEY NOT NULL,
    event_json BLOB NOT NULL,
    kind TEXT NOT NULL,
    occurred_at_ms INTEGER NOT NULL,
    enqueued_at_ms INTEGER NOT NULL,
    available_at_ms INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0
);
PRAGMA user_version = 0;
