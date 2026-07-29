# Linux relay packaging

Stage 19 packages the Stage 16 relay role as x86-64 and AArch64 archives with
checksums, SPDX, license notices, explicit install/uninstall scripts, and a
managed systemd user-service template. Linux desktop notifications are not
supported.

After independently verifying the release bundle, extract the archive and run:

```text
./install.sh
```

The script installs the executable under `${CODEX_NOTIFIER_PREFIX:-~/.local}`,
writes the user unit under `${XDG_CONFIG_HOME:-~/.config}/systemd/user`, and
enables `codex-notifier.service`. Create the versioned relay configuration from
[`docs/relay-ssh.md`](../../docs/relay-ssh.md) and the pinned OpenSSH host block
before expecting delivery. The unit has no listener, shell command, credential,
or event-derived argument.

Hosts without a live systemd user manager may use `./install.sh --no-enable`
and start `codex-notifier agent` explicitly. `./uninstall.sh` disables the user
unit and removes only the installed executable and unit; `--no-disable` skips
systemd operations. Configuration and durable SQLite state remain in place.
