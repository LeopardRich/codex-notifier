# macOS packaging

Stage 13 freezes the application identity and resources that the Stage 14
installer and Stage 19 release package must own. A bare CLI executable is not a
valid notification identity and must never impersonate Finder or another app.

## Application bundle

- Bundle path: `Codex Notifier.app`
- Bundle identifier: `io.github.leopardrich.codex-notifier`
- Display name: `Codex Notifier`
- Executable: `Contents/MacOS/codex-notifier`
- Package type: `APPL`
- Minimum system version: `14.0`
- Agent UI policy: `LSUIElement = true`; no Dock icon or interactive approval UI

`Contents/Info.plist` must set `CFBundleExecutable`, `CFBundleIdentifier`,
`CFBundleName`, `CFBundleDisplayName`, `CFBundlePackageType`,
`CFBundleShortVersionString`, `CFBundleVersion`, `LSMinimumSystemVersion`, and
`LSUIElement`. The release version fields must match the archive metadata. The
bundle contains a bounded `Contents/Resources/codex-notifier.icns`, the signed
universal executable, license, checksums, and version metadata.

The native backend rejects an executable outside a `.app`, a missing or
different bundle identifier, and a process without the current user's Aqua
launch domain. Authorization is requested explicitly during setup or a local
test, never while handling an event. Version 1 notifications contain no action,
reply field, URL, command, category, or remote approval control.

## Signing and notarization

Release artifacts use a `Developer ID Application` certificate and hardened
runtime. No notification-bypass, critical-alert, Apple Events, accessibility,
camera, microphone, location, contacts, or keychain entitlement is required.
The complete `.app` is signed from the innermost code outward, verified with
`codesign --verify --deep --strict --verbose=2`, submitted with `notarytool`,
and stapled before archiving. The distributed archive is verified with
`spctl --assess --type execute --verbose=2` after download.

Ad-hoc signing is allowed only for the ignored local smoke test. It is not a
release signature and cannot be cited as notarization evidence. Signing keys,
Apple credentials, API keys, and keychain profiles are never stored in this
repository or printed in logs.

## LaunchAgent

The per-user installer owns
`~/Library/LaunchAgents/io.github.leopardrich.codex-notifier.plist`. Its fixed
label is `io.github.leopardrich.codex-notifier`; `ProgramArguments` contains the
absolute installed `Contents/MacOS/codex-notifier` path followed by `agent`.
It uses `RunAtLoad = true` and `LimitLoadToSessionType = Aqua`, writes logs only
to the documented user log directory, and contains no event text, credentials,
shell command, or network listener. Installation validates the plist with
`plutil` before `launchctl bootstrap gui/<uid>`.

## Stage 14 lifecycle

`codex-notifier install` must be invoked by the executable inside a valid
signed `Codex Notifier.app`. It verifies that source bundle with `codesign`,
copies the complete bundle atomically to `~/Applications/Codex Notifier.app`,
writes the exact LaunchAgent above, bootstraps it in the current Aqua domain,
and records the owned bundle, plist, label, hook, and configuration in the
bounded manifest.

Reinstallation validates the previous manifest, replaces the signed bundle,
and reactivates the same LaunchAgent without adding another job. `status`
checks the installed manifest, startup resource, live agent record, queue, and
UserNotifications diagnostic. `test` submits either synthetic event kind over
the normal local IPC path. `uninstall` boots out the job and removes exact
owned resources while retaining the SQLite queue and receipt database.

## Owned resources

- The installed `.app` bundle and its exact signed contents
- The LaunchAgent plist and its loaded job
- A bounded installer ownership manifest and installation version record
- Application configuration created by the installer, excluding later user
  edits and queued event data

Upgrade replaces the signed bundle atomically and uses `launchctl kickstart`
only after signature and plist validation. Uninstall boots out the owned job and
removes only resources in the ownership manifest. It does not erase queued
events, notification history, SSH keys, unrelated Codex hooks, or user-created
configuration unless a separate explicit option owns that deletion.

## Required release evidence

- First authorization grant and explicit denial diagnostics
- Both event kinds on macOS 14 and the latest supported macOS release
- Focus/Do Not Disturb behavior without time-sensitive or critical bypass
- No-GUI launch-domain rejection
- Install, login restart, upgrade, and uninstall records for the signed package
