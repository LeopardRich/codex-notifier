# Windows packaging

Stage 12 freezes the native notification identity and the resources that the
Stage 14 installer and Stage 19 package must own.

## Application identity

- AppUserModelID: `LeopardRich.CodexNotifier`
- Display name: `Codex Notifier`
- Executable: the signed `codex-notifier.exe` from the same release
- Notification behavior: display-only; no protocol activation, actions, reply
  fields, or COM activation server is registered in version 1

The per-user installer must create one Start Menu shortcut whose
`System.AppUserModel.ID` property exactly matches the AppUserModelID above.
The shortcut target must use an absolute installed executable path and no
event-derived arguments. An arbitrary PowerShell or system application ID is
not an acceptable production fallback.

The installer must also register the unpackaged application under
`HKCU\Software\Classes\AppUserModelId\LeopardRich.CodexNotifier` with the
fixed display name and an absolute product-owned icon path. This registration
is required by the WinRT notification platform on the supported Windows 10
path. The Start Menu shortcut remains required for discoverability and Shell
identity; both resources use the same PascalCase AUMID.

## Owned resources

- The per-user Start Menu shortcut and its application-ID property
- The per-user unpackaged-application registry key and its exact values
- A bounded `.ico` application icon referenced by the shortcut and executable
- The installed executable, license, checksums, and version metadata
- The Stage 14 user startup artifact and ownership manifest
- The application configuration created by the installer, excluding later
  user edits and queued event data

Uninstall must remove only resources named in the ownership manifest. It must
not remove user queue data, unrelated Codex hooks, notification history, SSH
keys, or user-created configuration unless a separate explicit option owns
that deletion.

## Stage 14 lifecycle

`codex-notifier install` copies the invoking external executable to
`%LOCALAPPDATA%\Programs\Codex Notifier\codex-notifier.exe`, creates the fixed
identity and Start Menu shortcut above, and registers the exact
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run\CodexNotifier` value with
the installed executable followed by `agent`. It then starts the agent and
records the owned application layout and platform identifiers in the bounded
manifest under the application state directory.

Reinstallation validates that manifest before replacing the executable and
recreating the same identity, shortcut, and Run value. `status` verifies their
presence, and `test` submits a synthetic event over the normal local IPC path.
Because a running Windows executable cannot delete its own directory,
`uninstall` must be invoked from the external build or downloaded executable,
not the installed copy. Removal stops the agent, removes exact owned platform
resources, and retains the SQLite queue and receipt database.

## Diagnostics

The Windows backend checks the installer-owned per-user registry key before
using an AUMID. Windows 11 can return `ERROR_NOT_FOUND` from
`ToastNotifier.Setting` before its first accepted Toast creates the notification
handler; that state is ready only when the registration key exists. The backend
otherwise reports missing/manifest-disabled identity, per-application
disablement, global user disablement, group-policy disablement, non-interactive
Session 0, native API unavailability, and delivery rejection separately. A
successful `Show` means Windows accepted the Toast; it does not prove the user
saw it. Focus Assist and Do Not Disturb remain higher-priority operating-system
policy and are never bypassed.

Windows Server editions are outside the Windows 10/11 desktop support scope.
An interactive Windows Server process reports `unsupported_platform` before
querying the desktop Toast API. A Session 0 process reports
`no_interactive_session` first so automation and service diagnostics remain
actionable on every Windows host.
