# persistence

Implements the transactional SQLite outbox, expiring leases, acknowledgements,
cancellation release, retry scheduling, delivery receipts, deduplication,
bounded metadata-only dead letters, retention, integrity checks, and forward
schema migration.

Status and self-test readers use separate read-only snapshot and per-event
inspection APIs. They never create, migrate, prune, or repair a database and
detect an event identifier that appears in more than one state table.

Only validated canonical event JSON is stored in the outbox. Receipt and dead
letter rows never contain display text or event payloads. Every state change
and migration uses an immediate transaction, and callers receive stable safe
error classifications for locks, unwritable storage, full queues, corrupt
rows, and migration failures.
