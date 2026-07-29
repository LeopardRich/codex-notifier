# Stage 13 Verification

Status: In progress

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

## Hosted-runner blocker

- The hosted path has exhausted application-main-thread, AppKit-run-loop,
  LaunchServices, UI-agent, screen-capture, and locally valid signing
  hypotheses. The remaining required evidence needs an interactive Mac where
  Notification Center and `usernoted` are supported session services, plus a
  release-appropriate Apple signing identity when validating the distributable
  package.
- Hosted artifacts cannot substitute for the first grant, explicit denial,
  Focus/Do Not Disturb, and two-event visual checks required by this stage.

## Required before completion

- Run the ignored smoke test from an interactive signed bundle and visually
  confirm both event kinds on macOS 14.
- Repeat the same confirmation on the latest supported macOS release.
- Record first authorization, denial, and Focus/Do Not Disturb behavior in real
  interactive system states.
- Until these interactive items are complete, Stage 13 is not a support claim.
