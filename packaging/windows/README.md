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
