# Restricted SSH Receive Setup

Stage 15 uses an existing system OpenSSH server on the Windows or macOS
desktop. It does not install or configure `sshd`, open a firewall port, create
a listener, or copy a private key. The local desktop notification path remains
usable when SSH is absent or disabled.

The SSH login must be the same operating-system user that runs the desktop
`codex-notifier agent`. Local IPC rejects a receiver running as another user.
Use a reachable LAN or VPN address; this design does not add a public relay or
reverse tunnel.

## 1. Create a dedicated relay key

Create a new Ed25519 key on the relay. Do not reuse an interactive login key.

```text
ssh-keygen -t ed25519 -f ~/.ssh/codex-notifier-desktop -C codex-notifier-relay
```

Keep the private file on the relay with user-only permissions. Never add it to
this repository, `config.toml`, a command argument, or a diagnostic bundle.
Only the single-line `.pub` value is installed on the desktop.

## 2. Enroll and verify the desktop host key

Obtain the desktop OpenSSH host-key fingerprint through a separate trusted
channel before accepting it. `ssh-keyscan` retrieves a key but does not
authenticate it:

```text
ssh-keyscan -p PORT DESKTOP_HOST_OR_VPN_ADDRESS > codex-notifier-host-key.scan
ssh-keygen -lf codex-notifier-host-key.scan
```

Compare that fingerprint with a value read locally on the desktop or supplied
by its administrator. Only after it matches, append the scan output to the
relay user's `~/.ssh/known_hosts`, set that file to user-only write access, and
remove the temporary scan file.

Copy [`config.relay.example`](../packaging/ssh/config.relay.example) into the
relay user's SSH configuration and replace every uppercase placeholder. The
application `relay.ssh_host_alias` is the fixed `Host` name from this block,
not an event field. `StrictHostKeyChecking yes` is required. Do not use `no`,
`off`, or `accept-new`; a changed or absent key must stop delivery.

## 3. Install the forced authorized key

Enable the operating system's OpenSSH server separately and review that
change using the operating-system documentation. On the desktop, copy the
appropriate single line into the SSH account's authorized-key file:

- [macOS forced-key template](../packaging/ssh/authorized_keys-macos.example)
- [Windows forced-key template](../packaging/ssh/authorized_keys-windows.example)

Replace `USERNAME` and `DEDICATED_PUBLIC_KEY`; do not change `receive` or add
another command. The `restrict` option disables PTY allocation, agent/X11/port
forwarding, and user RC execution. `command="... receive"` is the only process
the key may start. The receiver also requires the client's original command to
be exactly `codex-notifier receive`; a shell request or extra argument receives
`ssh_session_rejected` before stdin is read.

On macOS and other Unix OpenSSH hosts, use mode `0700` for `~/.ssh` and `0600`
for `authorized_keys`, with both owned by the desktop user. Reject symlinks.

Windows OpenSSH normally uses `%USERPROFILE%\.ssh\authorized_keys`. Its stock
`Match Group administrators` rule may instead select
`%PROGRAMDATA%\ssh\administrators_authorized_keys` for an administrator. Apply
an explicit protected ACL to the file: only the target user, `SYSTEM`, and
`Administrators` may have write-capable allow entries. Do not grant `Users`,
`Authenticated Users`, or `Everyone` write access. Keep whichever `sshd_config`
choice is already active documented so uninstall or key rotation is reversible.

## 4. Diagnose without changing SSH state

Run the read-only diagnostic on the relay and desktop as applicable:

```text
codex-notifier doctor ssh
codex-notifier doctor ssh --ssh-config ABSOLUTE_PATH
codex-notifier doctor ssh --known-hosts ABSOLUTE_PATH
codex-notifier doctor ssh --authorized-keys ABSOLUTE_PATH
```

The optional paths support a custom OpenSSH client configuration,
`UserKnownHostsFile`, and the Windows administrator key file. The command
never prints a host alias, path, key, username, or file content.

`host_key=ready` means the system `ssh -G` result uses
`StrictHostKeyChecking yes` and `ssh-keygen -F` finds the resolved host/port or
`HostKeyAlias` in the selected file. `insecure` means strict checking is not
`yes`; `missing`, `unavailable`, and `not_configured` are distinct states.

`authorized_keys=ready` means the selected file passed the platform permission
check. On Unix this requires an owned nonsymlink directory/file at exact modes
`0700`/`0600`. On Windows it requires protected inheritance, an owner in the
target user/`SYSTEM`/`Administrators` set, and no other write-capable allow
entry. Other results are `missing`, `insecure`, or `unavailable`.

## 5. Exercise and revoke

Stage 16 normal queued sending uses this same boundary. A controlled probe
requests the exact remote command and sends one canonical JSON envelope through
stdin:

```text
ssh codex-notifier-desktop "codex-notifier receive"
```

The response is one compact protocol-v1 acknowledgement. Valid events retain
their event ID. If the input is invalid before an ID can be trusted, the
rejection contains a fresh UUIDv7 correlation ID and a fixed safe error. No
payload, path, key, or stack trace is returned.

Relay role configuration, bounded retry policy, failure classifications,
offline recovery, and source-built operation are documented separately in
[`relay-ssh.md`](relay-ssh.md).

To revoke access, remove only this dedicated public-key line from the active
authorized-key file and delete the relay private/public key pair. Remove the
dedicated SSH `Host` block and its verified `known_hosts` entry only if they are
not shared. Disabling the OpenSSH server or reverting firewall/VPN changes is a
separate operating-system action because this project did not create them.
