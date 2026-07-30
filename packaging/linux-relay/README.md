# Linux relay packaging

Stage 19 packages the Stage 16 relay role as x86-64 and AArch64 archives with
checksums, SPDX, license notices, explicit install/uninstall scripts, and a
managed systemd user-service template. Linux desktop notifications are not
supported.

After independently verifying the bundle, extract the archive. Copy
`examples/config.toml.example` to
`${XDG_CONFIG_HOME:-~/.config}/codex-notifier/config.toml`, then configure and
test the pinned SSH host alias. The archive also includes
`examples/authorized_keys-windows.example`,
`examples/authorized_keys-macos.example`, and `examples/ssh-config.example` so
SSH pairing does not depend on a source checkout. Run:

```text
./install.sh --codex-version 0.144.5
```

The script installs the executable under `${CODEX_NOTIFIER_PREFIX:-~/.local}`,
writes the user unit under `${XDG_CONFIG_HOME:-~/.config}/systemd/user`,
structurally installs the verified task-completion Codex hook, and enables
`codex-notifier.service`. The script refuses to enable the service or install a
hook before the relay configuration exists. The unit has no listener, shell
command, credential, or event-derived argument.

Hosts without a live systemd user manager may use `./install.sh --no-enable`
and start `codex-notifier agent` explicitly. `--no-hook` is available only for
deliberately unmanaged integration. `./uninstall.sh` disables the user unit,
removes the exact owned hook, executable, and unit; `--no-disable` and
`--no-hook` skip those respective operations. Configuration, unrelated hooks,
and durable SQLite state remain in place.
