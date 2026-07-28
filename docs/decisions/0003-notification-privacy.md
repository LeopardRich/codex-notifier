# ADR-0003: Private notifications by default

- Status: Accepted
- Date: 2026-07-29
- Owners: Project maintainers

## Context

System notifications can appear on lock screens, shared displays, and remote
screen captures. Approval payloads may contain commands or sensitive context.

## Decision

The default `desktop.privacy` value is `private`. Default notification text is:

| Event | Title | Body |
| --- | --- | --- |
| `task_completed` | `Codex task finished` | `Open Codex to review the result.` |
| `approval_requested` | `Codex needs approval` | `Open Codex to review the request.` |

Private mode never displays project labels, host labels, commands, arguments,
prompts, model output, paths, or raw adapter text. An explicit `public` setting
may display the bounded canonical title/body but still cannot display commands,
credentials, prompts, model output, or paths.

Application quiet hours are disabled by default. Native notifications always
respect the operating system's focus/do-not-disturb policy. If application
quiet hours are explicitly enabled, events are delivered silently rather than
queued or discarded, preventing stale approval notifications after the quiet
period. Version 1 notifications are display-only and contain no approval
button, action URL, reply field, or other remote-control action.

## Alternatives

Showing canonical text by default was rejected because OS lock-screen controls
are inconsistent. Deferring all events during quiet hours was rejected because
approval requests rapidly become stale.

## Consequences

The default is useful but intentionally generic. Users who opt into public text
accept additional shoulder-surfing risk. Quiet hours reduce interruption but
not notification-center entries.

## Security Impact

Privacy is enforced when mapping the domain event to the native adapter, not by
logs or OS settings. Native payloads are escaped and length-bounded.

## Compatibility Impact

Both desktop platforms expose the same semantic settings even if their native
focus modes and permission diagnostics differ.

## Verification

- Snapshot-test both events in private and public modes.
- Assert forbidden source fields never reach native adapter calls.
- Smoke-test silent delivery and OS focus behavior on Windows and macOS.
