# Stage 07 Verification

- Status: Complete
- Date: 2026-07-29
- Host: Windows 10 Pro 22H2 (19045.6466)

## Implemented

- SQLite schema version 1 with transactional enqueue, expiring leases,
  acknowledgement, retries, metadata-only dead letters, delivery receipts,
  deduplication, and maintenance.
- Canonical event row revalidation against indexed event ID/kind on every
  lease, with bound SQL parameters and stable safe errors.
- Bounded queue count, event age, attempts, lease duration, receipt retention,
  and dead-letter retention.
- Transactional version 0 migration, newer-version rejection, schema shape
  validation, and SQLite quick integrity checks.
- File-type/symlink rejection and classified locked, unwritable, corrupt,
  migration, capacity, expiry, and lease-transition failures.

## Verified

- Twelve persistence contract tests cover committed and uncommitted crash
  recovery, lease expiry recovery, acknowledgement tombstones, duplicate
  submissions/receipts, retry scheduling and exhaustion, metadata-only dead
  letters, exact capacity/age/retention bounds, lock and unwritable failures,
  version 0 migration preservation, migration rollback, newer schemas, corrupt
  payload rollback, and hostile transition inputs.
- `cargo fmt --all -- --check` exits 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` exits 0.
- `cargo test --workspace` exits 0 with all 43 Stage 04-07 contract tests
  passing.
- GitHub Actions run
  [`30394129954`](https://github.com/LeopardRich/codex-notifier/actions/runs/30394129954)
  completed successfully for `windows-desktop`, `macos-desktop`, and
  `linux-relay` from commit `7d9c6e4`.

## Local environment note

- Local compile, Clippy, and test execution require a C compiler for bundled
  SQLite. The installed Rust GNU toolchain has no usable `cc1`, and no system
  clang/MSVC compiler is present; no machine toolchain was installed or
  modified as a workaround.

Stage 07 is complete. Per-user local IPC remains assigned to Stage 08.
