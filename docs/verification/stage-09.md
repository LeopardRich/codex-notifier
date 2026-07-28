# Stage 09 Verification

- Status: Pending CI
- Date: 2026-07-29
- Host: Windows 10 Pro 22H2 (19045.6466)

## Implemented

- Explicit desktop/relay composition that initializes only the selected
  delivery port after the per-profile IPC endpoint has been bound.
- Monotonic `starting`, `ready`, `draining`, and `stopped` lifecycle with
  durable-before-ack submission and structured duplicate/rejection responses.
- Fixed 1-64 worker policy, durable queue backpressure, cooperative delivery
  cancellation, a 10 ms to 30 second drain deadline, and lease guards that
  return aborted in-flight work to the queue.
- SQLite queue adapter covering enqueue, lease, acknowledge, retry/release, and
  metadata-only dead-letter transitions behind a thread-safe application port;
  cancellation release reverses the lease attempt.
- Host coordination that enters draining before stopping IPC acceptance and
  reports IPC, delivery, retry, dead-letter, and forced-cancellation counts.

## Pending Verification

- Five application agent contract tests cover role initialization isolation,
  readiness and delivery, exact queue-capacity backpressure, fixed worker peak,
  cooperative cancellation, and forced-abort lease recovery.
- Four composition tests cover real IPC plus SQLite, validated relay role
  selection, pre-initialization single-instance rejection, and shutdown cleanup.
- A persistence regression test proves repeated shutdown release cannot consume
  the delivery-attempt budget or dead-letter an otherwise undelivered event.
- `cargo fmt --all -- --check`, application Clippy with `-D warnings`, and all
  12 application tests pass locally.
- Full workspace formatting, Clippy, and tests must pass on the GitHub Actions
  Windows, macOS, and Linux jobs before this stage is complete.

## Local Environment Note

- The executable composition crate cannot rebuild bundled SQLite locally
  because no usable C compiler is available. No machine toolchain, Codex login,
  or global hook was modified as a workaround.

Stage 09 remains incomplete until the three-platform CI evidence is recorded.
