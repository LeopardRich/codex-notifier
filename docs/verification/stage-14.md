# Stage 14 Verification

Status: Complete

Date: 2026-07-29

Scope: reversible local desktop installation, per-user agent startup, status,
synthetic tests, upgrade, and uninstall on Windows and macOS.

## Implemented evidence

- The executable implements `install`, `uninstall`, `agent`, `status`, and
  `test [task-completed|approval-requested]`. Linux rejects desktop
  installation as unsupported; no Linux notification path was added.
- Installation accepts only the fixture-verified Codex CLI 0.144.5 capability.
  It structurally adds one exact `Stop` hook group for `emit task-completed`,
  never interprets hook JSON or event text as a shell command, and explicitly
  reports that the ordinary CLI approval hook is unavailable.
- A bounded ownership manifest records the application root, executable,
  configuration, hooks document, exact hook group, and platform startup and
  notification identity. Absolute layout validation prevents a modified
  manifest from redirecting uninstall outside the product layout.
- Hook/configuration writes are atomic and snapshot-backed. Reinstall removes
  the previous exact hook before adding its replacement. Transaction failures
  restore both managed documents and the previous platform application.
- Uninstall removes an exact unmodified managed hook and installer-created
  configuration. Missing or user-modified content is preserved, and unrelated
  hooks remain structurally unchanged. SQLite queue, receipt, and dead-letter
  state is always retained.
- Windows installs a per-user executable and icon, the fixed product AUMID
  registry identity, an indexed Start Menu shortcut, and one Run value. macOS
  verifies and copies the complete signed application bundle and installs one
  Aqua LaunchAgent. Both paths start the installed desktop agent.
- `status` reports bounded installation, startup, live/stale agent, profile,
  queue, receipt, dead-letter, notification, and Focus metadata. `test` sends a
  synthetic canonical event through the same local IPC, durable queue, routing,
  and native adapter used by a Codex hook.

## Automated checks

- On Windows 10 22H2, Rust 1.88 GNU `cargo fmt --all -- --check`, full-workspace
  all-targets/all-features Clippy with warnings denied, and all 100 automated
  workspace tests passed. Four interactive Windows notification tests remain
  explicitly ignored during the normal suite.
- Lifecycle unit tests cover idempotent structural hook merging, exact
  uninstall, preservation of modified configuration and hooks, hostile
  manifest paths, failed platform-commit restoration, strict CLI parsing,
  bounded notification diagnostics, stale agent records, platform host
  classification, and the generated Windows icon structure.
- Permanent CI run
  [`30479936618`](https://github.com/LeopardRich/codex-notifier/actions/runs/30479936618)
  passed formatting, warnings-as-errors Clippy, all normal workspace tests,
  the real Windows Session 0 diagnostic, macOS smoke-target builds, and the
  signed no-Aqua diagnostic on Windows, Linux, macOS 14, and macOS latest.

## Windows real-state verification

- A local Windows 10 Pro 22H2 interactive-session run started with an existing
  user hooks document. Fresh install reached a running agent with native
  notification status `ready`; `test task-completed` produced the fixed private
  Windows Toast and one durable delivery receipt.
- Reinstall left exactly one managed `Stop` hook and one startup registration.
  A build with changed binary metadata replaced the installed executable,
  restarted the agent, and delivered an explicit approval-request test through
  the same local path.
- External uninstall removed the installed executable, icon, shortcut, AUMID,
  Run value, exact managed hook, unchanged installer configuration, and
  ownership manifest. The pre-existing hooks document matched its original
  semantic JSON, and SQLite state and receipts remained. The live user profile
  was left uninstalled. The ignored local evidence directory preserves only
  bounded lifecycle outputs and a hash of the pre-install hooks; it is not part
  of the repository.
- GitHub Actions lifecycle run
  [`30481210674`](https://github.com/LeopardRich/codex-notifier/actions/runs/30481210674)
  repeated install, identity indexing, startup restart, changed-binary upgrade,
  both synthetic event submissions, uninstall, hook cleanup, and state
  preservation on the hosted Windows runner. That runner is Windows Server, so
  native delivery correctly reported `unsupported_platform`; this run is
  lifecycle evidence, not Windows desktop Toast evidence.

## macOS real-state verification

- The same lifecycle run used fresh macOS 14.8.7 and current macOS 26.4 hosted
  user sessions. Each source application was signed with a temporary
  certificate chain trusted only on that runner. The installer verified the
  signature, atomically copied the bundle under `~/Applications`, validated and
  bootstrapped the LaunchAgent, installed one task-completion hook, and reached
  a running agent.
- Each fresh runner exercised the real notification permission banner and the
  application-specific System Settings control. `status` changed from
  `disabled_for_application` while authorization was pending to
  `notification=ready` after the native control was enabled.
- `test task-completed` produced a visible fixed private notification and one
  delivery receipt. Each artifact contains `macos-task-completed.png`, which
  shows `Codex task finished` / `Open Codex to review the result.` under the
  `Codex Notifier` application identity.
- Each job stopped and reactivated the owned LaunchAgent in the current Aqua
  login domain, verified the agent returned, installed a changed signed binary,
  and submitted `test approval-requested`. The final status reported two
  delivery receipts and no dead letters.
- Uninstall booted out the LaunchAgent and removed the installed bundle, plist,
  managed hook/configuration, and manifest. The SQLite database and its two
  receipts remained. Cleanup restored the hosted image's original Notification
  Center UI-agent setting and removed all temporary signing material.
- The evidence harness and UI helpers were temporary branch-only verification
  assets. They were removed after the artifacts were captured and are not part
  of permanent CI or the shipped product.

## Explicit limits

- The macOS probe identity is not an Apple Developer ID. Release signing,
  notarization, stapling, archives, checksums, and installed release-package
  verification remain Stage 19 gates.
- The Windows hosted runner does not extend support to Windows Server. Native
  Windows 10/11 notification support remains grounded in Stage 12 and the
  local Windows 10 Stage 14 run.
- Codex CLI approval hook installation remains unavailable because no verified
  CLI `PermissionRequest` fixture exists. The app-server approval adapter and
  explicit local test event do not change that installation claim.
- SSH receive and relay delivery are not implemented and remain Stage 15 and
  Stage 16 work.

## Completion decision

- Fresh local installation produces a native task-completion notification on
  both supported desktop operating systems. Reinstall and changed-binary
  upgrade are idempotent, startup registrations can restart the installed
  agent, and uninstall removes only owned application/configuration resources
  while preserving user edits and event state.
- Stage 14 is complete. Stage 15 may begin only after these changes are merged
  to `main` and permanent CI is green there.
