# Relay SSH Delivery

Stage 16 implements source-built relay delivery through the operating system's
OpenSSH client. It does not install OpenSSH, create keys, edit SSH files, open a
firewall, add a reverse tunnel, or create a Linux desktop notification path.
Release archives and a managed systemd user service remain Stage 19 work.

## 1. Prepare the desktop trust boundary

Complete the dedicated key, pinned host key, forced command, permissions, and
diagnostic procedure in [`restricted-ssh.md`](restricted-ssh.md). The relay
must be able to run this controlled probe successfully:

```text
ssh codex-notifier-desktop "codex-notifier receive"
```

The host alias is setup data. Do not replace it with an event field, raw
hostname from a hook, shell fragment, URL, or command.

## 2. Configure the relay role

Create the normal user configuration file for the relay host. On an XDG host
the default path is
`${XDG_CONFIG_HOME:-~/.config}/codex-notifier/config.toml`:

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

[storage]
max_queue_entries = 1000
```

The host alias accepts only a bounded ASCII identifier. Connection timeout is
100 to 120,000 milliseconds. Initial retry delay is 100 to 60,000
milliseconds, maximum retry delay is 100 to 3,600,000 milliseconds, and the
attempt limit is 1 to 1,000. The initial delay cannot exceed the maximum.

Do not put a private key, password, token, event payload, prompt, model output,
or machine-specific proxy setting in this file. `IdentityFile`, destination
address, port, user, and `UserKnownHostsFile` remain in the user's OpenSSH host
block, as shown by
[`packaging/ssh/config.relay.example`](../packaging/ssh/config.relay.example).

## 3. Run and submit

Start the per-user relay process:

```text
codex-notifier agent
```

The configured Codex hook or low-level `emit` command submits through local IPC
and returns after the canonical event is transactionally durable. The four
bounded workers lease queued events independently. The relay starts one
`ssh` process per leased event with fixed options that enforce:

- batch mode and no password prompts;
- no PTY, agent forwarding, or configured port forwarding;
- one connection attempt and the configured connection timeout;
- strict host-key checking;
- the configured host alias and exact command `codex-notifier receive`.

The event is serialized once and sent only through stdin. No event value is an
executable, argument, alias, command, path, environment value, or URL. Stdout
is bounded to one 2,048-byte acknowledgement and diagnostic stderr to 8 KiB.
Both are parsed or discarded internally and are never copied into normal logs.
The durable lease is the connection timeout plus ten seconds, which remains
five seconds longer than the maximum child-process lifetime; another worker
cannot recover the same row while its SSH attempt is still active.

## 4. Acknowledgement and retry policy

An acknowledgement must be valid protocol-v1 JSON and carry the same event ID.
`accepted`, `duplicate`, and `delivered` all prove that the destination no
longer needs this outbox copy. A `rejected` acknowledgement retains only its
validated code and retry flag; its message is not stored.

| Classification | Queue action |
| --- | --- |
| Network unavailable, timeout, missing OpenSSH, generic process failure | Retry |
| Receiver rejection with `retryable=true` | Retry |
| Authentication failure | Dead letter |
| Changed, unknown, or otherwise rejected host key | Dead letter |
| Malformed, oversized, or mismatched acknowledgement | Dead letter |
| Receiver rejection with `retryable=false` | Dead letter |
| Cooperative agent shutdown | Release immediately without consuming an attempt |

Retry base delay doubles after each consumed attempt and stops growing at
`retry_max_delay_ms`. Each scheduled delay is randomly selected between 75 and
100 percent of that base. SQLite stores the future availability time, so the
agent wakes without another submission and resumes the same schedule after a
restart. The event remains bounded by queue capacity, event age, attempt count,
and metadata retention. Attempt exhaustion records `retry_exhausted` and
deletes the canonical payload.

At-least-once delivery means a connection can fail after the desktop accepted
the event but before the relay read its response. The relay then sends the same
stable event ID again. The desktop receipt returns `duplicate` and does not
display a second notification.

## 5. Diagnose and recover

Run the read-only SSH checks before starting or after changing keys:

```text
codex-notifier doctor ssh
```

`host_key=insecure`, `missing`, or `unavailable` must be corrected in the SSH
configuration or enrollment files. Authentication and host-key failures are
permanent for the affected queued attempt because retrying unchanged
credentials cannot repair them. Restore the correct dedicated key or verify
and deliberately enroll the new desktop host key before submitting a new event.

Network and timeout errors remain queued until the desktop, LAN, or VPN is
reachable, or until the configured age/attempt limit moves them to dead
letters. Normal output reports only stable codes and counts; it never prints
the alias, username, key, path, event text, or captured SSH diagnostic.

## 6. Stop and remove

Stop the user process through the service manager or the normal cooperative
agent shutdown mechanism. Unacknowledged leases return to SQLite and are not
discarded.

To remove a relay relationship reversibly:

1. Stop the relay agent.
2. Remove only the dedicated relay host block if it is not shared.
3. Remove only the verified host-key entry for that relationship.
4. Remove the dedicated public-key line from the desktop authorized-key file.
5. Delete the dedicated private/public relay key pair.
6. Retain or deliberately remove the relay state directory according to the
   user's data-retention choice.

The project does not own OpenSSH installation, VPN, firewall, or SSH-server
state, so those changes must be reversed separately by whoever created them.
