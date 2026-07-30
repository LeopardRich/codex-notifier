# codex-notifier

[简体中文](README-zh.md)

`codex-notifier` is a planned cross-platform notification bridge for Codex CLI.
It turns Codex events that need human attention into native Windows or macOS
notifications, including events produced by Codex running on a remote server.

> Status: Stages 01-20 are implemented or audited and platform-verified where the required environment is available: compatibility evidence, architecture
> decisions, the Rust workspace, three-platform quality gates, and the
> canonical event domain model, layered configuration, and cross-platform path
> rules are established, together with structured redacted logging and the
> transactional SQLite outbox/deduplication store and bounded per-user local
> IPC. The role-aware agent lifecycle, durable backpressure, and bounded worker
> drain are also complete. Exact Codex CLI 0.144.5 adapters now cover the CLI
> `Stop` hook and app-server command-approval request, with bounded local
> `emit` paths and read-only capability reporting. The Windows WinRT adapter,
> policy mapping, diagnostics, and automated contracts are implemented;
> product-identity Toast delivery and real disabled, Focus Assist, missing
> identity, and Session 0 states are verified on Windows 10 22H2, and fresh
> first-use delivery is verified on Windows 11. The macOS UserNotifications
> adapter, bundle contract, authorization diagnostics, native CI, and headless
> checks are implemented; first authorization, explicit denial, both event
> banners, and Do Not Disturb suppression are verified on macOS 14.8.7 and the
> current macOS 26.4 runner. The local desktop lifecycle now installs the exact
> verified task-completion hook and per-user startup resources, runs the agent,
> reports status, submits both explicit test event kinds, upgrades idempotently,
> and uninstalls owned resources while preserving event state and user edits.
> The restricted SSH `receive` boundary, redacted acknowledgements, dedicated
> forced-key templates, and SSH security diagnostics are implemented. A real
> system OpenSSH forced-command session and rejection matrix are verified on a
> Linux loopback harness. The durable relay role now invokes system OpenSSH with
> fixed arguments, validates bounded acknowledgements, classifies transport and
> trust failures, and schedules jittered exponential retries with bounded
> dead letters. The same Linux harness verifies offline queueing, automatic
> recovery, at-least-once resend, and desktop deduplication. Windows/macOS
> SSH-server setup remains explicitly unverified. The Stage 17 read-only doctor,
> role-aware status, matching human/JSON output, stable health exit codes, and
> delivery-aware local/remote self-tests pass the Windows, macOS 14, current
> macOS, and Linux relay gates. Stage 18 adds explicit three-window crash
> recovery, 100-retry duplicate suppression, queue/version/offline fault
> coverage, conservative performance/resource budgets, and a four-path
> reliability matrix. Real remote-to-Windows and remote-to-macOS continuous
> native paths remain explicitly unverified; Linux loopback evidence is not
> treated as either desktop-platform result. Stage 19 adds versioned Windows
> x86-64, macOS universal, and Linux x86-64/AArch64 verification archives,
> checksums, SPDX and license materials, package lifecycle gates, and
> fail-closed protected signing/notarization/publication jobs. All branch
> verification packages and the aggregate bundle are green. Production Windows
> signing, Apple Developer ID/notarization, and a protected tag run remain
> unverified, so these unsigned/ad-hoc artifacts are not release candidates.
> Stage 20 completed the evidence-chain, compatibility, threat, secret/config,
> documentation, and rollback audit with a **no-go** result. The protected
> `release` environment does not exist and neither continuous remote desktop
> path can be rerun on a signed candidate, so no formal tag or release exists.

The implementation sequence and acceptance gates are defined in
[`stages.md`](stages.md).

## Product Scope

The first release targets two user-facing events:

- `approval_requested`: Codex is waiting for the user to approve an action.
- `task_completed`: a Codex turn or task has finished.

The original product flow is preserved below:

```mermaid
flowchart TD
    A[Codex CLI waits for approval or completes a task] --> B{Runtime environment}
    B -->|Local Windows| C[Windows system notification]
    B -->|Local macOS| D[macOS system notification]
    B -->|Remote server| E[Forward event to the desktop application over SSH]
    E --> F{Desktop operating system}
    F -->|Windows| C
    F -->|macOS| D
```

### Goals

- Deliver native notifications without exposing a network service publicly.
- Use the same event model for local and remote Codex sessions.
- Keep Codex hooks fast and non-blocking by handing work to a local agent.
- Survive temporary SSH failures with a bounded persistent queue.
- Package one small executable for emitters, relays, receivers, and diagnostics.
- Make installation and removal explicit and reversible.

### Non-goals

- Linux desktop notifications. Linux is supported only as a remote relay host.
- Mobile push, email, chat integrations, or a hosted relay service.
- Remote control of Codex from a notification.
- Transporting complete prompts, model responses, or terminal logs.
- Replacing Codex's own approval UI.

## Codex Integration Boundary

Codex integration is isolated behind source adapters because event availability
can differ by Codex version and interface.

| Product event | Required Codex capability | Current behavior |
| --- | --- | --- |
| Task completed | An external notification/hook event for turn completion | Implemented for the exact Codex CLI 0.144.5 CLI `Stop` hook shape; normalize and enqueue `task_completed`. |
| Approval requested | An external notification/hook event for an approval request | Implemented for the exact Codex CLI 0.144.5 app-server `item/commandExecution/requestApproval` request; the ordinary CLI hook remains unverified. |

The read-only `doctor codex` command and future installer use the same
fixture-gated capability report as adapter selection.
It must not scrape terminal output or private session logs as a silent fallback.
Adapter contracts are gated by sanitized real-event fixtures for each exact
Codex version and interface. This repository intentionally does not claim that
all Codex versions expose both events to external programs.

The initial version floor is Codex CLI 0.144.5. Stage 01 verified task
completion through the `codex exec` `Stop` hook and approval requests through
the app-server JSON-RPC interface on Windows 10 22H2. The ordinary CLI
`PermissionRequest` lifecycle-hook path remains unverified and must be reported
unavailable until real evidence exists. See
[`docs/compatibility.md`](docs/compatibility.md) and
[ADR-0001](docs/decisions/0001-supported-versions.md). The initial OS build
floors are Windows 10 22H2 (19045) and macOS 14. Stages 12 and 13 provide the
required real-state native notification evidence for those platform claims;
Stage 19 implements fail-closed release-package signing and notarization gates,
but the production identities and protected tag path remain unverified.

## Architecture

The project uses a Rust workspace and a hexagonal architecture. Domain and
application logic do not depend on Codex payloads, SSH commands, IPC details,
or operating-system notification APIs.

```mermaid
flowchart LR
    subgraph Host[Machine running Codex]
        Codex[Codex CLI]
        Hook[Codex source adapter]
        Emit[emit command]
        Agent[per-user agent]
        Queue[(SQLite outbox)]
        Codex --> Hook --> Emit -->|local IPC| Agent
        Agent <--> Queue
    end

    subgraph Desktop[User desktop]
        Receive[receive command]
        DesktopAgent[per-user desktop agent]
        Inbox[(deduplication store)]
        Adapter{notification adapter}
        Win[Windows toast]
        Mac[macOS notification]
        Receive -->|local IPC| DesktopAgent
        DesktopAgent <--> Inbox
        DesktopAgent --> Adapter
        Adapter --> Win
        Adapter --> Mac
    end

    Agent -->|desktop role| Adapter
    Agent -->|relay role: OpenSSH| Receive
```

### Runtime Roles

Runtime role is configured explicitly; it is not inferred from whether a
machine appears headless.

| Role | Typical host | Responsibility |
| --- | --- | --- |
| `desktop` | Windows or macOS workstation | Receive events over local IPC, deduplicate them, and show native notifications. |
| `relay` | Remote Linux, Windows, or macOS server | Receive local Codex events, queue them, and forward them to a configured desktop over SSH. |

The same executable supports both roles. A desktop role never needs SSH for
local notifications, and a relay role never calls desktop notification APIs.

### Components

| Component | Responsibility |
| --- | --- |
| Codex source adapter | Convert a version-specific Codex payload into the canonical event model. |
| `emit` command | Validate hook input and submit it to the agent through local IPC; return quickly to Codex. |
| Agent | Own routing, persistence, retry scheduling, deduplication, and graceful shutdown. |
| Local IPC adapter | Use a per-user Windows named pipe or Unix domain socket with user-only access. |
| SQLite store | Persist the relay outbox and desktop delivery receipts with bounded retention. |
| SSH transport | Invoke the system OpenSSH client with an argument array and a configured host alias. |
| `receive` command | Act as the restricted SSH entry point, validate one envelope, and submit it to the desktop agent. |
| Notification adapter | Map canonical events to Windows toast or macOS UserNotifications. |
| Installer | Configure Codex integration, user startup, and optional restricted SSH access. |
| Diagnostics | Report Codex event support, agent health, IPC permissions, SSH reachability, and notification permission. |

### Local Event Flow

```mermaid
sequenceDiagram
    participant C as Codex CLI
    participant E as codex-notifier emit
    participant A as Desktop agent
    participant S as Receipt store
    participant N as Native notification API

    C->>E: Codex event payload
    E->>A: Canonical event over local IPC
    E-->>C: Accepted
    A->>S: Check event ID
    alt New event
        A->>N: Show notification
        A->>S: Record delivery result
    else Duplicate event
        A->>S: Refresh retention metadata
    end
```

### Remote Event Flow

```mermaid
sequenceDiagram
    participant C as Remote Codex CLI
    participant E as Remote emit command
    participant R as Relay agent
    participant Q as SQLite outbox
    participant S as System OpenSSH
    participant X as Desktop receive command
    participant D as Desktop agent
    participant N as Native notification API

    C->>E: Codex event payload
    E->>R: Event over local IPC
    E-->>C: Accepted
    R->>Q: Persist before sending
    R->>S: Send envelope to restricted command
    S->>X: stdin envelope
    X->>D: Event over local IPC
    D->>N: Show notification
    D-->>X: Delivery acknowledgement
    X-->>R: Structured acknowledgement
    R->>Q: Mark delivered
```

The baseline remote topology assumes that the relay host can reach the desktop
through an SSH host alias, usually over a trusted LAN or VPN. Reverse tunnels
may be added later as a separate transport adapter; they are not part of the
first implementation.

## Event Contract

Every adapter produces a versioned canonical envelope. The logical fields are:

| Field | Purpose |
| --- | --- |
| `schema_version` | Allows compatible evolution of the wire format. |
| `event_id` | UUIDv7 generated at first ingestion and reused across retries. |
| `kind` | `approval_requested` or `task_completed`. |
| `occurred_at` | UTC timestamp from the source, bounded by receiver validation. |
| `source` | Sanitized host label, optional project label, and Codex session identifier. |
| `presentation` | Bounded title, body, and urgency intended for display. |
| `routing` | Optional desktop profile name; never an arbitrary command or address. |
| `extensions` | Namespaced, size-limited metadata for forward compatibility. |

Unknown required schema versions, event kinds, and object fields are rejected
in version 1. Prompt text, model output, environment variables, absolute
working directories, and credentials are excluded by default.

Protocol version 1 is frozen by
[ADR-0006](docs/decisions/0006-event-protocol-v1.md) and specified in
[`docs/event-protocol-v1.md`](docs/event-protocol-v1.md). Version 1 rejects
unknown event kinds and unknown object fields, limits an encoded event to
16,384 bytes, and permits forward metadata only through bounded namespaced
extensions.

## Local IPC Contract

Local producers and the per-user agent exchange exactly one canonical event
and one acknowledgement per connection. Each frame uses a four-byte big-endian
length prefix. Requests are limited to 16,384 bytes and acknowledgements to
2,048 bytes. Acknowledgements carry the matching event ID and one of
`accepted`, `duplicate`, `delivered`, or `rejected`; rejection details use a
bounded identifier, retry flag, and single-line safe message.

The client and server apply bounded connection and I/O deadlines. The server
also caps active connection tasks; defaults are two seconds and 32 tasks, with
hard configuration limits of 10 milliseconds to 30 seconds and 1 to 256 tasks.
Windows uses a named pipe with an owner-only DACL and verifies the peer process
belongs to the current user. macOS and Linux use an absolute Unix socket path
inside a current-user-owned `0700` directory, create the socket with mode
`0600`, and compare peer credentials with the effective user ID.

An active endpoint cannot be displaced. An owned stale Unix socket can be
recovered, while symlinks, unrelated files, wrong owners, and unsafe directory
permissions are rejected. Local IPC uses no HTTP client and does not consult
`HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY`.

## Agent Lifecycle Contract

The agent role is taken only from validated configuration. Composition binds
the per-profile IPC endpoint first, then opens the bounded SQLite queue and
initializes exactly one role adapter graph: `desktop` initializes the native
notification port but not SSH, while `relay` initializes the SSH delivery port
but not native notification APIs. The relay port invokes the system OpenSSH
client with fixed arguments; the desktop port selects only the native adapter
for the current supported operating system.

Lifecycle state moves monotonically through `starting`, `ready`, `draining`,
and `stopped`. A local submission is acknowledged only after transactional
enqueue, and duplicate event IDs receive a distinct acknowledgement. The
durable queue is the backpressure boundary, so accepted work does not create an
unbounded in-memory task or channel. The default worker set is four tasks with
a hard maximum of 64.

Shutdown changes state to `draining` before stopping IPC acceptance. New
submissions are rejected with a safe retryable acknowledgement, cooperative
delivery is cancelled, and each lease is acknowledged, retried, dead-lettered,
or returned by a drop guard. Workers that exceed the configured 10 ms to 30
second graceful deadline are aborted only after their lease guard has made the
event durable again. A shutdown release reverses the lease attempt so repeated
agent restarts cannot exhaust delivery retries.

## Delivery Semantics

- Submission from `emit` to the local agent is at-least-once.
- Remote delivery is at-least-once; the desktop deduplicates by `event_id`.
- An event is removed from the outbox only after a structured acknowledgement.
- SQLite schema version 1 applies `IMMEDIATE` transactions to enqueue, lease,
  acknowledge, retry, dead-letter, receipt, retention, and migration changes.
  Expired leases become eligible at the exact expiry boundary. A successful
  acknowledgement writes a deduplication receipt before deleting the canonical
  outbox payload in the same transaction.
- Retries use exponential backoff with jitter and configurable upper bounds.
- Queue size, event size, retry age, and receipt retention are bounded.
- Permanent validation or authentication failures enter a small dead-letter
  record containing the reason and safe metadata, not the full payload.
- Stored outbox rows are revalidated against their indexed event ID and kind
  when leased. Receipts and dead letters contain no canonical JSON or display
  text, and schema migration failures leave the source transaction unchanged.
- Notification API success means the OS accepted the notification, not that the
  user saw or opened it.
- A crash after native API acceptance but before the SQLite receipt commits can
  repeat a notification after lease recovery. Recording the receipt first
  would risk silent loss, so version 1 explicitly prefers at-least-once
  recovery in this narrow window.

### Windows native notifications

The Windows adapter is compiled only under `cfg(windows)` and uses the
`windows-rs` WinRT Toast API. Private policy always renders the fixed text from
ADR-0003. Public text requires both explicit application policy and a canonical
event marked public. Native text is independently control-filtered and bounded;
Toast XML is built with DOM nodes, not string interpolation. Application quiet
hours suppress popup and audio while retaining notification-center delivery.
Version 1 emits no actions, launch URI, reply field, command, or remote approval
control.

The backend validates the product AUMID `LeopardRich.CodexNotifier` and its
installer-owned per-user registry identity, rejects Session 0, and
distinguishes missing identity, per-application disablement, global user
disablement, group-policy disablement, API unavailability, and delivery
rejection. A registered first-use identity may submit its initial Toast before
Windows has created a notification-settings record. Windows Focus Assist and
Do Not Disturb remain
operating-system policy; diagnostics report `system_managed` rather than
claiming an unsupported active-state probe. The package resource and reversible
ownership contract is recorded in
[`packaging/windows/README.md`](packaging/windows/README.md).

The adapter's automated contracts pass on Windows 10 22H2. A temporary
per-user unpackaged-app registration using the product AUMID passed the ignored
two-event smoke test: WinRT accepted both real Toasts and the Windows
notification database persisted their exact fixed private payloads. Real
application-disabled and Focus Assist Priority-only states were exercised and
restored. On Windows 11 Enterprise build 26200 Arm64, a fresh install-grade
identity passed the same two-event product smoke without a raw notification or
pre-existing settings record; Notification Center also rendered the product
group under Do Not Disturb. See
[`docs/verification/stage-12.md`](docs/verification/stage-12.md).

### macOS native notifications

The macOS adapter is compiled only under `cfg(target_os = "macos")` and uses
Apple's modern UserNotifications framework through safe Rust bindings. It
pins the binding release that builds against the macOS 14 SDK; native CI now
covers both macOS 14 and the current `macos-latest` image. The adapter
requires the signed application bundle identifier
`io.github.leopardrich.codex-notifier`, verifies that the executable is running
from that `.app`, and requires the current user's Aqua launch domain. A
read-only diagnostic distinguishes missing identity, authorization not yet
requested, explicit denial or application disablement, no GUI session, and
native API unavailability. Authorization is requested only through an explicit
method; displaying an event never opens a permission prompt.

The shared privacy and text-bounding policy is applied before UserNotifications
is called. Each request uses the canonical event ID and contains title/body
only: no category, action, URL, reply field, command, or user-info payload.
Normal delivery uses the default sound and active interruption level;
application quiet hours omit sound and use the passive level. The adapter never
uses time-sensitive or critical levels, so macOS Focus and Do Not Disturb remain
authoritative and diagnostics report `system_managed`.

The bundle, Developer ID signing, notarization, and Aqua LaunchAgent resource
contract is recorded in
[`packaging/macos/README.md`](packaging/macos/README.md). The ignored smoke
harness self-bundles a foreground test application, accepts either ad-hoc or
explicit test signing, registers with LaunchServices, and starts AppKit on the
process main thread. Its grant path submits both event kinds, its fresh-state
denial path verifies `DisabledForApplication`, and its Focus path submits a
stable probe event without requesting a bypass level.

Real hosted-runner checks on macOS 14.8.7 and macOS 26.4 exercised the first
authorization flow and visually confirmed both fixed private banners. Fresh
runners verified explicit denial. Enabling Do Not Disturb through Control
Center suppressed the probe banner on both versions, and native logs tied the
stable event ID to delayed delivery under an active Focus mode; the original
Focus state was restored. The no-Aqua diagnostic is also covered by permanent
CI. These checks used a temporary locally trusted signing chain because the
repository has no Apple signing secret. Stage 19 validates the universal ad-hoc
verification archive and implements a fail-closed protected release path;
Apple-issued Developer ID signing and notarization remain unverified. See
[`docs/verification/stage-13.md`](docs/verification/stage-13.md) and
[`docs/verification/stage-19.md`](docs/verification/stage-19.md).

## Security Model

The trust boundary is the desktop `receive` command.

- Use the operating system's OpenSSH client; do not embed an SSH server.
- Use a dedicated SSH key and host alias for each relay-to-desktop relationship.
- Restrict the authorized key to `codex-notifier receive` with forwarding, PTY,
  shell, and unrelated command access disabled where the SSH server supports it.
- Pin or explicitly enroll the desktop host key; never disable host-key checks.
- Pass envelopes over stdin. Event data must never be interpolated into a shell
  command, command line, notification action, URL, or file path.
- Validate schema, kind, timestamps, string lengths, total size, and rate limits
  before the event reaches persistence or a native API.
- Limit local IPC and state files to the current OS user.
- Redact payloads from normal logs and never store SSH private keys in project
  configuration.
- Notifications are display-only in the first release and have no action button
  that can approve a Codex operation.

The audited key restrictions, host-key enrollment procedure, permission
requirements, diagnostics, and revocation steps are in
[`docs/restricted-ssh.md`](docs/restricted-ssh.md).

## Configuration Model

Configuration is layered in this order: built-in defaults, user configuration,
profile-specific configuration, and explicit CLI overrides. Environment
variables are reserved for deployment integration and must not carry event
payloads or private keys.

Configuration schema version 1 implements these groups:

| Group | Examples of owned settings |
| --- | --- |
| `agent` | Explicit desktop/relay role, profile, logical IPC endpoint, shutdown timeout. |
| `codex` | Source-adapter selector and accepted task-completion/approval event kinds. |
| `desktop` | Quiet-hours behavior and private/public notification content policy. |
| `relay` | Preconfigured OpenSSH host alias, destination profile, connection timeout, and bounded retry schedule. |
| `storage` | Absolute state path and bounded queue capacity. |
| `logging` | Level and absolute log directory; configuration diagnostics are redacted. |

The default notification privacy mode uses generic title/body text with no
host, project, command, prompt, response, or path. Application quiet hours are
off by default while OS focus/do-not-disturb remains authoritative; explicitly
enabled application quiet hours deliver silently rather than deferring stale
events. See [ADR-0003](docs/decisions/0003-notification-privacy.md).

User configuration and state follow platform conventions: `%APPDATA%` and
`%LOCALAPPDATA%` on Windows, and `~/Library/Application Support` on macOS.
Relay hosts follow the XDG base directory specification when available.

The main file is `%APPDATA%\codex-notifier\config.toml` on Windows,
`~/Library/Application Support/codex-notifier/config.toml` on macOS, and
`${XDG_CONFIG_HOME:-~/.config}/codex-notifier/config.toml` on XDG relay hosts.
State and logs use `%LOCALAPPDATA%`, the corresponding macOS Application
Support/Logs directories, or `${XDG_STATE_HOME:-~/.local/state}`. Explicit path
bases and configured state/log directories must be absolute, and the final
state directory must be writable.

Every current TOML file requires `config_version = 1`. The loader can migrate
the bounded legacy version 0 `role` and optional `ssh_host` keys. Unknown
settings, unsupported versions, invalid roles/endpoints, and prohibited
sensitive keys fail with stable safe error classifications. Private keys,
access tokens, passwords, prompts, model output, and raw event payloads are
not valid configuration values and are never included in configuration debug
output.

## Command Surface

The local desktop lifecycle, two low-level Codex ingestion entries, restricted
SSH receiver, durable relay sender, and complete read-only operational
diagnostics are implemented:

| Command | Availability | Purpose |
| --- | --- | --- |
| `--version` | Implemented | Print the package version used by archive and upgrade verification. |
| `emit task-completed` | Implemented | Codex-facing, bounded local ingestion for the verified `Stop` payload. |
| `emit approval-requested` | Implemented | Bounded local ingestion for a verified app-server command-approval request. |
| `doctor codex` | Implemented | Read-only version/interface capability and installation reporting. |
| `doctor ssh` | Implemented | Read-only host-key enrollment and authorized-file permission reporting. |
| `agent` | Implemented for desktop and relay | Run the configured per-user role process. |
| `receive` | Implemented | Accept exactly one bounded canonical event from a restricted SSH session and forward it over local IPC. |
| `install` / `uninstall` | Implemented for desktop | Reversibly manage the verified Codex hook and per-user Windows/macOS startup artifacts. |
| `doctor` | Implemented for desktop and relay | Read-only configuration, Codex, agent, IPC, storage, notification, OpenSSH, and target checks. |
| `test` | Implemented for desktop and relay | Submit either explicit synthetic event and wait for its local or remote desktop delivery receipt. |
| `status` | Implemented for desktop and relay | Show role, installation, agent, queue age/counts, delivery time, storage, and native status without event content. |

### Package verification and setup

There is no formal `v0.1.0` release. Current CI archives are unsigned/ad-hoc
engineering artifacts and must not be installed as trusted release candidates.
The eventual candidate must first pass checksums, the matching platform
verifier, production desktop trust, and every item in
[`docs/release-checklist.md`](docs/release-checklist.md).

After those gates report go, extract the matching verified archive and use its
own executable or scripts:

| Target | Install and check | Remove |
| --- | --- | --- |
| Windows x86-64 | `codex-notifier.exe install --codex-version 0.144.5`, then `codex-notifier.exe doctor` and `codex-notifier.exe status` | Run `codex-notifier.exe uninstall` from the external extracted archive, not the installed copy. |
| macOS 14+ | Run `Codex Notifier.app/Contents/MacOS/codex-notifier install --codex-version 0.144.5`, then `doctor` and `status` through the same executable | Run `uninstall` through the verified external app executable. |
| Linux relay | Run `./install.sh`, create the relay configuration/SSH host block, then run the installed `codex-notifier doctor` | Run `./uninstall.sh`; configuration and SQLite state remain. |

Verify `SHA256SUMS` before extraction and follow the signature/notarization and
per-archive commands in [`docs/release.md`](docs/release.md). Configuration
paths and keys are defined above; remote setup is in
[`docs/restricted-ssh.md`](docs/restricted-ssh.md) and
[`docs/relay-ssh.md`](docs/relay-ssh.md); diagnostics are in
[`docs/diagnostics.md`](docs/diagnostics.md). Uninstall removes owned resources
but deliberately preserves durable event state.

### Local desktop lifecycle

Stage 14 provides the current source-built Windows and macOS interface:

```text
codex-notifier install [--codex-version 0.144.5]
codex-notifier status [--format human|json]
codex-notifier test [task-completed|approval-requested] [--format human|json] [--wait-ms 100..180000]
codex-notifier uninstall
codex-notifier agent
```

When `--codex-version` is omitted, `install` invokes `codex --version` and
accepts only the exact verified `codex-cli 0.144.5` result. Installation adds
one structurally merged `Stop` hook, records its exact ownership, creates the
platform notification identity and per-user startup resource, starts the
desktop agent, and reports the native notification diagnostic. Codex still
requires the user to review hook trust. The ordinary CLI approval hook is not
installed: `approval_installation=report_unavailable` remains explicit even
though `test approval-requested` can verify the local notification route.

Reinstallation replaces the previously owned hook and application atomically,
without adding duplicate hooks or startup entries. `status` reports only
bounded metadata and uses read-only SQLite inspection. `uninstall` removes the exact managed hook, startup identity,
application, and unchanged installer-created configuration; it preserves
pre-existing or modified hook/configuration content and always retains the
SQLite queue, receipts, and notification history. On Windows, run `uninstall`
from an external build or downloaded executable because the installed process
cannot remove its own directory. On macOS, `install` must run from a valid
signed `Codex Notifier.app`. Stage 19 verification archives exercise this
lifecycle on both desktop platforms; production Developer ID signing and
notarization remain release-candidate blockers.

See the platform-specific ownership details in
[`packaging/windows/README.md`](packaging/windows/README.md) and
[`packaging/macos/README.md`](packaging/macos/README.md), and the executed
evidence in [`docs/verification/stage-14.md`](docs/verification/stage-14.md).

### Restricted SSH receive

Stage 15 adds the desktop-side trust boundary:

```text
codex-notifier receive
codex-notifier doctor ssh [--ssh-config <absolute-path>] [--known-hosts <absolute-path>] [--authorized-keys <absolute-path>]
```

`receive` is not a general local ingestion command. It requires OpenSSH's
`SSH_ORIGINAL_COMMAND` to equal `codex-notifier receive`, requires an SSH
connection marker, rejects any PTY marker, and accepts no CLI argument. It
reads at most 16,384 bytes and parses exactly one canonical protocol-v1 JSON
object. A valid event is submitted to the configured desktop agent through the
same per-user IPC used by local producers; its bounded agent acknowledgement is
written as the only stdout object.

Session, input, configuration, and IPC failures become fixed structured
rejections. They never include the original command, event text, host alias,
path, key, or stack trace. Invalid input that has no trustworthy event ID gets
a fresh UUIDv7 response correlation ID. Shell metacharacters in valid display
fields remain JSON/notification data and cannot select an executable,
argument, endpoint, path, URL, or action.

The system OpenSSH server and a dedicated Ed25519 key must be configured
separately. Forced-key and relay config templates, mandatory host-key
verification, Windows/macOS permission rules, diagnostic meanings, and
reversible removal are documented in
[`docs/restricted-ssh.md`](docs/restricted-ssh.md). The project never changes a
user's SSH configuration or key files automatically.

### Relay SSH delivery

Stage 16 adds the source-built relay agent path. A relay configuration uses a
fixed system OpenSSH host alias and bounded delivery policy:

```toml
config_version = 1

[agent]
role = "relay"
profile = "default"

[relay]
ssh_host_alias = "codex-notifier-desktop"
connect_timeout_ms = 10000
retry_initial_delay_ms = 1000
retry_max_delay_ms = 60000
retry_max_attempts = 20
```

Run `codex-notifier agent` under the remote user's session or service manager.
The existing `emit` commands submit to its local IPC endpoint exactly as they
do for a desktop role. Stage 19 provides Linux x86-64 and AArch64 relay
archives, reversible scripts, and a managed systemd user service; no Linux
notification adapter is created.

For every leased event, the relay starts the system `ssh` executable with a
fixed argument array. It forces batch mode, no PTY, no agent or configured port
forwarding, one connection attempt, strict host-key checking, and the exact
remote command `codex-notifier receive`. The canonical event is written only
to stdin. Stdout is limited to the 2,048-byte acknowledgement bound and stderr
to 8 KiB; neither diagnostic content nor event data is logged.

A matching `accepted`, `duplicate`, or `delivered` acknowledgement commits the
relay receipt and removes the outbox payload. Retryable receiver failures,
network loss, connection timeout, missing OpenSSH, and generic process failures
remain queued. Authentication failure, changed/untrusted host key, malformed or
mismatched acknowledgement, oversized output, and non-retryable receiver
rejection become metadata-only dead letters. The receiver's bounded retry flag
is preserved without retaining its message.

Retries use exponential base delays with random jitter between 75 and 100
percent of each interval, capped by `retry_max_delay_ms`. Configuration accepts
initial delays from 100 to 60,000 ms, maximum delays from 100 to 3,600,000 ms,
1 to 1,000 attempts, and connection timeouts from 100 to 120,000 ms. Defaults are
1 second, 60 seconds, 20 attempts, and 10 seconds. A future-dated retry wakes
without a new IPC submission, including after agent restart; reaching the age
or attempt bound cannot consume queue resources indefinitely.

Setup, host-key enrollment, source-built operation, error meanings, recovery,
and reversible removal are documented in
[`docs/relay-ssh.md`](docs/relay-ssh.md). The executed evidence is in
[`docs/verification/stage-16.md`](docs/verification/stage-16.md).

### Diagnostics, status, and self-test

Stage 17 adds the role-aware operational surface:

```text
codex-notifier doctor [--format human|json]
codex-notifier status [--format human|json]
codex-notifier test [task-completed|approval-requested] [--format human|json] [--wait-ms 100..180000]
```

`doctor` checks configuration, fixture-gated Codex support, agent state,
same-user IPC, read-only SQLite state, and the role-specific native notification
or OpenSSH path. It does not create or migrate state, change Codex/SSH/startup
configuration, request notification permission, or send an event. The relay
target probe sends empty stdin and accepts only the receiver's fixed
`malformed_json` rejection, so reachability testing cannot enter either queue.

`test` submits generic private text through normal IPC and waits for its stable
event ID to reach a receipt or dead letter. A local receipt follows native
adapter acceptance. For a relay, the restricted receiver waits for the desktop
native receipt and returns `delivered`; pending desktop work remains retryable.
Human lines and compact schema-v1 JSON use the same typed status, code, exit
code, and fixed remediation. They exclude event text, source/user/host labels,
keys, raw SSH errors, profiles, and machine paths.

The complete check order, field contract, self-test semantics, remediation, and
exit-code table are in [`docs/diagnostics.md`](docs/diagnostics.md). Stage 17
execution evidence is in
[`docs/verification/stage-17.md`](docs/verification/stage-17.md).

### Codex event emit

The Stage 10 executable entry reads one raw Codex `Stop` hook JSON object from
stdin and accepts at most 32 KiB. Its current low-level invocation is:

```text
codex-notifier emit task-completed --codex-version 0.144.5 --state-dir <absolute-state-directory> --ipc-profile <agent-ipc-profile> --host-label <display-label> [--project-label <display-label>] [--routing-profile <profile>]
```

The state directory and IPC profile must match the running agent. Host,
project, and route labels are trusted setup values and are never copied from
the hook working directory. The command accepts only the exact verified
version, reports source compatibility separately from IPC failures, and emits
no payload text. The Stage 14 installer places this exact invocation in its
owned `Stop` hook group; direct use remains available for controlled adapters.

The approval entry accepts one raw
`item/commandExecution/requestApproval` JSON-RPC request from the verified
app-server interface with the same bounded endpoint/context options:

```text
codex-notifier emit approval-requested --codex-version 0.144.5 --state-dir <absolute-state-directory> --ipc-profile <agent-ipc-profile> --host-label <display-label> [--project-label <display-label>] [--routing-profile <profile>]
```

This command only emits a display-only notification event. The app-server
client remains responsible for replying to Codex through its existing approval
UI; `codex-notifier` does not approve, decline, execute, or expose the command.
The desktop installer does not configure this app-server path and reports CLI
approval notifications as unavailable.

The minimal read-only capability check is:

```text
codex-notifier doctor codex --codex-version 0.144.5 --interface <cli-hook|app-server>
```

It reports stable support and installation states without reading transcripts,
terminal output, credentials, or user configuration. Unknown version text is
not echoed. The comprehensive `doctor` command composes this capability with
the Stage 17 operational checks.

## Repository Layout

The workspace packages now exist. Packages whose implementation belongs to a
later stage intentionally contain only a documented Rust module boundary.

```text
codex-notifier/
|-- Cargo.toml
|-- README.md
|-- README-zh.md
|-- LICENSE
|-- crates/
|   |-- core/                 # Event types, validation, routing, policies
|   |-- application/          # Use cases and ports
|   |-- codex-source/         # Version-specific Codex payload adapters
|   |-- ipc/                  # Named-pipe and Unix-socket adapters
|   |-- persistence/          # SQLite queue and receipt adapters
|   |-- ssh-transport/        # System OpenSSH process adapter
|   |-- native-notification/  # Windows and macOS adapters
|   `-- config/               # Layered configuration and migrations
|-- apps/
|   `-- codex-notifier/       # Binary, commands, agent lifecycle
|-- tests/
|   |-- contract/             # Event and acknowledgement compatibility
|   |-- integration/          # IPC, persistence, SSH process boundaries
|   `-- fixtures/             # Sanitized Codex payload fixtures
|-- packaging/
|   |-- windows/              # Packaging and per-user startup assets
|   |-- macos/                # Bundle, signing, and LaunchAgent assets
|   |-- linux-relay/          # systemd user service assets
|   `-- ssh/                  # Restricted-key and relay SSH config templates
`-- docs/
    |-- decisions/            # Architecture decision records
    |-- event-protocol-v1.md  # Frozen canonical event and acknowledgement contract
    |-- threat-model.md
    `-- compatibility.md
```

`core` and `application` must compile without platform notification or SSH
dependencies. Platform code lives in adapters selected at the binary boundary.

## Development

Install [rustup](https://rustup.rs/) once. The checked-in
[`rust-toolchain.toml`](rust-toolchain.toml) selects Rust 1.88.0 and installs
`rustfmt` and Clippy; no additional global Cargo package is required.

```text
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

CI runs the same quality gates on Windows, macOS, and a Linux relay runner.

## Observability and Operations

- Structured event logs use a fixed allowlist: timestamp, severity, event ID,
  event kind, typed state, bounded duration, validated correlation ID, and an
  optional validated safe error code. Display text, source labels, paths,
  commands, and raw payloads are not log fields at any level.
- Records use compact one-object-per-line JSON. Correlation IDs and error codes
  accept only bounded identifier grammars, so newlines, controls, terminal
  escapes, quotes, and forged fields cannot alter the log structure. Human and
  JSON diagnostics use the same typed status and fixed non-interpolated text.
- The default rotation policy is 1 MiB per segment, five retained segments,
  and seven days. Hard limits cap a segment at 64 MiB, retained segments at 64,
  and age at 365 days; exact size and age boundaries are inclusive.
- `status` reports queue depth, oldest queued age, and the most recent successful
  delivery time.
- Health checks are local-only and do not open an HTTP port.
- Database migrations are transactional and backward compatible for at least
  one released minor version.
- Shutdown stops accepting IPC, checkpoints in-flight work, and leaves
  unacknowledged events queued.

## Testing Strategy

- Unit tests cover validation, redaction, routing, retries, and deduplication.
- Contract tests freeze canonical event and acknowledgement compatibility.
- Integration tests exercise real local IPC, SQLite, the receive executable,
  and a temporary restricted system OpenSSH server.
- Adapter tests use captured, sanitized Codex payload fixtures by version.
- Windows and macOS CI compile and test their native adapters.
- Manual release smoke tests verify real notifications and OS permissions on
  both supported desktop platforms.
- Stage 18 reliability gates and the four-path evidence matrix are defined in
  [`docs/reliability.md`](docs/reliability.md); unavailable real platform paths
  stay unverified rather than being inferred from component tests.
- Security tests cover oversized input, malformed JSON, shell metacharacters,
  replayed event IDs, permission boundaries, and log redaction.
- Release CI blocks advisories, disallowed dependency licenses/sources,
  mismatched archive versions/checksums, invalid package layouts, and missing
  production signing or notarization; see [`docs/release.md`](docs/release.md).

## Release Plan

1. Freeze the canonical event contract and verify target Codex event support.
2. Implement local ingestion, desktop agent, persistence, and native adapters.
3. Add diagnostics and reversible per-user installation.
4. Implement restricted SSH receive and relay outbox delivery.
5. Add packaging, signing/notarization, cross-platform CI, and upgrade tests.
6. Run Windows/macOS end-to-end release validation and publish compatibility
   notes for tested Codex versions.

## Accepted Architecture Decisions

| Area | Decision |
| --- | --- |
| Versions | [ADR-0001](docs/decisions/0001-supported-versions.md): Codex 0.144.5, Windows 10 22H2, and macOS 14 initial floors with evidence-gated support. |
| License | [ADR-0002](docs/decisions/0002-license.md): MIT. |
| Privacy | [ADR-0003](docs/decisions/0003-notification-privacy.md): generic private notifications by default; OS focus policy remains authoritative. |
| SSH | [ADR-0004](docs/decisions/0004-ssh-topology.md): direct system OpenSSH over a reachable LAN/VPN path; no reverse tunnel in version 1. |
| Release | [ADR-0005](docs/decisions/0005-release-channel.md): signed/notarized GitHub Release artifacts with checksums and SBOM. |
| Protocol | [ADR-0006](docs/decisions/0006-event-protocol-v1.md): strict bounded JSON envelope version 1. |
| Reliability | [ADR-0007](docs/decisions/0007-reliability-budgets.md): explicit crash semantics and conservative release-gating resource budgets. |

Signing identity identifiers remain external secrets owned by the release
maintainer and must be fixed before the first release candidate. This is a
release gate, not an unresolved protocol or product behavior decision.
