# Stage 13 Verification

Status: Complete

Date: 2026-07-29

Scope: macOS UserNotifications adapter, application identity, authorization,
and packaging resources.

## Implemented evidence

- The implementation and all native dependencies are selected only under
  `cfg(target_os = "macos")`; other targets do not link or initialize them.
- The backend uses Apple's modern UserNotifications framework through the safe
  `usernotifications-rs` facade. Version `0.2.0` is pinned exactly because
  `0.3.7` references a macOS 15-only SDK type while compiling its Swift bridge
  and fails against the supported macOS 14 SDK. Deprecated
  `NSUserNotification`, `osascript`, bundle-identity spoofing, and hand-written
  unsafe shims are not used.
- The fixed bundle identifier is `io.github.leopardrich.codex-notifier`. The
  backend requires that exact identifier, a `.app` main bundle, and the current
  user's Aqua launch domain.
- Read-only diagnostics distinguish missing identity, authorization not yet
  requested, denial/application disablement, no GUI session, API
  unavailability, and delivery failure. Focus/Do Not Disturb is explicitly
  reported as `system_managed` because the public framework does not expose a
  reliable current Focus state.
- Shared tests cover private/public mapping, both event kinds, stable event IDs,
  Unicode scalar truncation, control removal, urgency, and silent delivery.
  macOS-only tests cover bundle-ID validation, permission classification,
  framework denial mapping, display-only content, and active/passive
  interruption levels that never bypass Focus.
- The ignored `macos_smoke` target is a custom executable so AppKit starts on
  the actual process main thread instead of a libtest worker. It creates and
  ad-hoc signs a temporary product-ID app bundle, registers it with
  LaunchServices, launches it as a foreground application, services AppKit
  events while the product backend explicitly requests permission, and
  verifies an inner success marker after submitting both real event kinds. A
  separate native path proves a bare executable reports missing identity, and
  an ignored headless path relaunches the signed bundle without an Aqua launch
  domain. Normal test runs cannot display notifications or prompt for access.
- Packaging documentation freezes the Info.plist, icon, Developer ID signing,
  notarization, Aqua LaunchAgent, upgrade, uninstall, and ownership resources.

## Automated checks

- `cargo fmt --all -- --check`, locked metadata, and `git diff --check` pass.
- On the final combined Stage 12/13 tree, Rust 1.88 GNU full-workspace Clippy
  passes with all targets, all features, and warnings denied. The workspace
  suite passes 87 automated tests; the four Windows real-state tests stay
  explicitly ignored, and the macOS-only smoke target is empty on Windows.
- On Windows 10 22H2, focused Rust 1.88 GNU `cargo check` and all-targets,
  all-features Clippy with warnings denied pass for the native notification
  crate. Its ten Windows/shared unit tests and documentation tests pass; both
  platform smoke targets remain ignored or empty as intended on this host.
- Target-specific dependency trees prove that Linux includes neither native
  implementation, Windows includes WinRT/sysinfo but no Apple crates, and
  macOS includes Foundation/rustix/`usernotifications-rs` but no Windows or
  deprecated `mac-notification-sys` dependency.
- Cross-target `cargo check` and all-targets/all-features Clippy with warnings
  denied pass for `aarch64-apple-darwin` against the official Rust 1.88 Apple
  standard-library archive, whose SHA-256 matched the published checksum. This
  type-checks the exact Foundation and `usernotifications-rs` APIs used by the
  backend and its tests. Dependency build scripts ran with `DOCS_RS=1`, so the
  Swift bridge and Apple framework linker were intentionally not invoked on
  Windows.
- GitHub Actions run
  [`30438113181`](https://github.com/LeopardRich/codex-notifier/actions/runs/30438113181)
  for commit `69143203a1bd7754eb4fbd2bde55cdd83aa132cd` passed formatting,
  warnings-as-errors Clippy, and all normal workspace tests on a native
  `macos-latest` runner. This exercised the Swift bridge build and Apple
  framework linking. The ignored interactive smoke test did not run, so this is
  not authorization or notification-display evidence.
- GitHub Actions run
  [`30439550334`](https://github.com/LeopardRich/codex-notifier/actions/runs/30439550334)
  passed the permanent headless diagnostic gate. The runner first built and
  ad-hoc signed the exact temporary product bundle, then launched only the
  inner test as `nobody`; the resulting process had no Aqua launch domain and
  reported `no_gui_session`. The same native run also passed the non-bundled
  missing-identity test. No notification permission was requested.
- GitHub Actions run
  [`30445395390`](https://github.com/LeopardRich/codex-notifier/actions/runs/30445395390)
  for commit `fc88b38be573326c37b0620c7ddcf048562b4379` passed all four
  blocking jobs. Native macOS 14 and current `macos-latest` runners both built
  the pinned Swift bridge and Apple frameworks, passed formatting,
  warnings-as-errors Clippy and normal workspace tests, and passed the signed
  no-Aqua diagnostic. This closes the minimum-SDK build gap but is not an
  interactive authorization or display claim.
- GitHub Actions run
  [`30448479812`](https://github.com/LeopardRich/codex-notifier/actions/runs/30448479812)
  passed the four permanent Windows, Linux, macOS 14, and current
  `macos-latest` jobs after the main-thread smoke harness was added. Both macOS
  jobs compiled the custom harness and passed all normal tests plus the signed
  no-Aqua diagnostic. The ignored interactive path did not run.
- A bounded hosted interactive probe in run
  [`30444848179`](https://github.com/LeopardRich/codex-notifier/actions/runs/30444848179)
  used real console/Aqua sessions on macOS 14.8.7 and macOS 26.4. Moving the
  ad-hoc signed bundle under `/Applications` and registering it changed the
  native result from immediate `UNErrorDomain:1` rejection to an authorization
  request waiting for user input. The hosted desktop exposed neither the system
  prompt nor an automatable application UI, so the bounded probe terminated
  the inner test without a grant. Its green workflow conclusion represents
  artifact collection only and is not authorization, denial, Focus, or display
  evidence.
- Follow-up bounded probes
  [`30446598148`](https://github.com/LeopardRich/codex-notifier/actions/runs/30446598148),
  [`30446973757`](https://github.com/LeopardRich/codex-notifier/actions/runs/30446973757),
  [`30447470640`](https://github.com/LeopardRich/codex-notifier/actions/runs/30447470640),
  and
  [`30448189220`](https://github.com/LeopardRich/codex-notifier/actions/runs/30448189220)
  isolated the test-runner thread, application activation, synchronous bridge,
  and AppKit event-dispatch hypotheses. On macOS 14.8.7 and macOS 26.4, the
  final application was a foreground LaunchServices app with the exact product
  identity. Its authorization requests reached UserNotifications, and the
  latest host also established a ViewBridge connection. Neither host started a
  visible Notification Center permission UI; screenshots remained unchanged,
  automation found no `Allow` button, the completion callback did not run, and
  no success marker was written. These runs establish a hosted-session
  limitation, not a successful authorization or notification display.
- Session-service probes
  [`30449966627`](https://github.com/LeopardRich/codex-notifier/actions/runs/30449966627)
  and
  [`30451095267`](https://github.com/LeopardRich/codex-notifier/actions/runs/30451095267)
  separated the hosted runner's screen-capture permission from notification
  permission, started `UserNotificationCenter`, and invoked
  `UNUserNotificationCenter` directly from the AppKit main thread. No prompt or
  callback appeared on macOS 14.8.7 or macOS 26.4.
- The repository Actions API reported zero configured repository secrets, so
  the probe had no Apple Development or Developer ID signing identity. A
  bounded alternative generated a temporary code-signing certificate and
  root, imported them into an ephemeral keychain, and added the root to the
  ephemeral host's system trust domain. In final run
  [`30454957673`](https://github.com/LeopardRich/codex-notifier/actions/runs/30454957673),
  both hosts reported `1 valid identities found`; `codesign` recorded the
  expected bundle identifier and certificate authorities. This was a locally
  trusted diagnostic signature, not an Apple-issued release identity, and its
  `TeamIdentifier` remained unset.
- The same final probe found `com.apple.notificationcenterui.agent` disabled by
  the hosted image, reversibly enabled it in the current user's launch domain,
  and verified the service was running and the override was `enabled` before
  requesting authorization. Both bundle processes reached the real
  UserNotifications listener, but neither produced a prompt, callback, or
  success marker. Unified logs recorded an invalid `usernoted` connection on
  macOS 14 and macOS latest. The probe restored the launch-agent override and
  bounded deletion of the temporary signing material.

## Final real-state verification

- GitHub Actions run
  [`30470133739`](https://github.com/LeopardRich/codex-notifier/actions/runs/30470133739)
  for commit `2c7f362222b8b6a3df42c13972595e207ebea0b3` passed all five
  probe jobs: the Windows 11 regression control, grant/display/Focus on macOS
  14.8.7 and macOS 26.4, and fresh denial on both macOS versions. The temporary
  branch workflow was an evidence harness, not a permanent CI or product
  component.
- Both grant jobs started from fresh hosted runners, built the exact product-ID
  app bundle, registered it with LaunchServices, and signed it with a temporary
  certificate chain trusted only on that runner. The real permission
  notification and the System Settings `Allow notifications` control were
  exercised. The product diagnostic then reported `Ready`. Evidence is in the
  `macos-14-grant-session-services` and
  `macos-latest-grant-session-services` artifacts.
- On each macOS version, native screenshots visibly confirm both fixed private
  notifications: `Codex task finished` / `Open Codex to review the result.` and
  `Codex needs approval` / `Open Codex to review the request.` The events used
  stable canonical IDs ending in `0014` and `0015`; UserNotifications accepted
  both through the product adapter. The corresponding files are
  `macos-task-completed.png` and `macos-approval-requested.png` in each grant
  artifact.
- Each denial job used a separate fresh runner and selected the native
  notification denial path. The authorization callback reported
  `granted=false, error_free=false`, and the product diagnostic reported
  `DisabledForApplication` with Focus `SystemManaged`. Unified logs recorded
  `didGrant: 0` with an authorization error on both versions. The
  `macos-14-denial-session-services` and
  `macos-latest-denial-session-services` artifacts preserve the prompt,
  callback, diagnostic, and unified-log evidence.
- Each grant job enabled `Control Center -> Focus -> Do Not Disturb`, submitted
  event `01983c8d-b800-7000-8000-000000000016`, and continuously inspected the
  real Notification Center UI. All 17 macOS 14 polls and all seven macOS 26.4
  polls reported no matching banner. Unified logs tied that event ID to
  `interruptionSuppression: delay delivery` and a non-null `activeModeUUID`;
  macOS 26.4 also recorded `outcome: suppressed`. Screenshots show Do Not
  Disturb active and no probe banner. Each grant artifact preserves the AX
  polling in `macos-focus-ui-history.txt`, native resolution in
  `macos-focus-unified.log`, and final screen in
  `macos-focus-after-delivery.png`.
- The same native Control Center sequence restored Do Not Disturb to off on
  both runners; `macos-focus-enable.txt` and `macos-focus-disable.txt` record
  both transitions. Cleanup also restored the hosted image's original
  notification UI-agent state and removed the temporary keychain, certificate,
  app bundle, and probe material.
- Permanent CI run
  [`30470133228`](https://github.com/LeopardRich/codex-notifier/actions/runs/30470133228)
  passed all Windows, Linux, macOS 14, and current `macos-latest` gates for the
  same commit. This includes formatting, warnings-as-errors Clippy, normal
  workspace tests, macOS smoke-target compilation, the non-bundled
  missing-identity check, and the signed no-Aqua `no_gui_session` diagnostic.

## Completion assessment

- The minimum and latest supported macOS versions displayed both real event
  kinds and exercised first authorization, explicit denial, Focus suppression,
  restoration, missing identity, and no-GUI diagnostics. Unicode/control/text
  bounds and non-macOS dependency isolation remain covered by automated tests.
- Stage 13 is complete. The locally trusted probe identity establishes native
  adapter behavior but is not an Apple-issued release identity; Developer ID
  signing, notarization, and validation of distributable artifacts remain
  required by Stage 19.
