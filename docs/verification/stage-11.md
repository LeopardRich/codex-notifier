# Stage 11 Verification

Status: Complete

Date: 2026-07-29

Scope: Codex CLI 0.144.5 approval-request ingestion through the verified
app-server `item/commandExecution/requestApproval` interface.

## Acceptance evidence

- The sanitized Windows fixture records a real external-process invocation,
  the exact Codex version and interface, and the generated-schema verification
  time. Its command, paths, identifiers, environment, and policy values remain
  redacted.
- The fixture-driven adapter contract normalizes the supported request into a
  private, high-urgency, display-only `approval_requested` canonical event.
  The thread identifier is retained only as a SHA-256 digest.
- Contracts reject missing or type-changed required fields, unknown fields,
  incompatible optional fields, oversized input, unknown versions, and the
  unverified CLI-hook interface.
- Privacy contracts prove that commands, arguments, decisions, paths, policy
  amendments, environment values, and action URLs do not enter the canonical
  event.
- The executable-to-agent integration test submits the real fixture through
  bounded stdin and per-user local IPC within five seconds.
- `doctor codex`, adapter selection, and future installation behavior share
  the same capability report. App-server approval is `supported`; CLI-hook
  approval is `unverified` and selects `report_unavailable` without installing
  a hook or fallback scraper.

## Quality gates

- GitHub Actions run `30419828528` for commit
  `b249c0a50f200572d730d69d314caf3d33990a6d` completed successfully.
- The `windows-desktop`, `macos-desktop`, and `linux-relay` jobs each passed
  formatting, workspace Clippy with warnings denied, and all workspace tests.
- Local `cargo fmt --all -- --check`, `cargo metadata --no-deps`,
  `git diff --check`, and focused all-targets Clippy for `codex-source` passed.
- Full local workspace Clippy and tests remain unverified on this machine
  because the pinned GNU toolchain cannot rebuild bundled SQLite without
  `gcc.exe`; CI provides the complete three-platform gate.

## Explicitly unverified

- A Codex CLI lifecycle `PermissionRequest` hook has no real fixture and is
  not installed or selected.
- No macOS approval-request capture exists, so the fixture proves only the
  Windows Codex 0.144.5 app-server path.
- The notifier does not answer approval requests or expose approval actions.
