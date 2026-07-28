# persistence

Implements the transactional SQLite outbox, expiring leases, acknowledgements,
cancellation release, retry scheduling, delivery receipts, deduplication,
bounded metadata-only dead letters, retention, integrity checks, and forward
schema migration.

Only validated canonical event JSON is stored in the outbox. Receipt and dead
letter rows never contain display text or event payloads. Every state change
and migration uses an immediate transaction, and callers receive stable safe
error classifications for locks, unwritable storage, full queues, corrupt
rows, and migration failures.
