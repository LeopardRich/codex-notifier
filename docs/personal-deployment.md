# Personal deployment on Windows, macOS, and Ubuntu

This procedure is for devices owned and administered by one user. It uses the
unsigned Windows and ad-hoc-signed macOS engineering archives from a green
`main` CI run. They are not public release artifacts. Do not redistribute them
or treat operating-system warnings as satisfied production trust.

## 1. Download and verify one bundle

Open the latest green `CI` run for `main` in GitHub Actions and download the
`codex-notifier-release-bundle` artifact. Extract the outer artifact into an
empty directory. It contains:

- `codex-notifier-v0.1.0-windows-x86_64.zip`;
- `codex-notifier-v0.1.0-macos-universal.zip`;
- Linux x86-64 and AArch64 relay archives;
- `SHA256SUMS`, SBOM, license notices, and engineering release notes.

Verify the archive needed on each device before extracting it. On Ubuntu:

```bash
sha256sum -c SHA256SUMS
```

On macOS:

```bash
shasum -a 256 -c SHA256SUMS
```

On Windows, compare the selected archive with its line in `SHA256SUMS`:

```powershell
(Get-FileHash .\codex-notifier-v0.1.0-windows-x86_64.zip -Algorithm SHA256).Hash.ToLowerInvariant()
```

The metadata inside every selected archive must name the same 40-character
commit as the CI run. Keep the external extracted archive: Windows uninstall
must run from that copy.

## 2. Install the Windows desktop

Extract the Windows archive and run from a normal interactive user session:

```powershell
.\codex-notifier.exe install --codex-version 0.144.5
.\codex-notifier.exe status --format json
.\codex-notifier.exe test task-completed --format json --wait-ms 60000
.\codex-notifier.exe test approval-requested --format json --wait-ms 60000
```

Only proceed when `status` reports `agent_running=true` and
`notification="ready"`, and both tests report `delivery="delivered"`.
Windows may show an unsigned-app or SmartScreen warning. Bypass it only for the
archive whose SHA-256 was verified above.

## 3. Install the macOS desktop

Copy the macOS archive to the Mac, verify it, then extract it with `ditto`:

```bash
mkdir -p "$HOME/Downloads/codex-notifier-personal"
ditto -x -k codex-notifier-v0.1.0-macos-universal.zip \
  "$HOME/Downloads/codex-notifier-personal"
cd "$HOME/Downloads/codex-notifier-personal"
codesign --verify --deep --strict --verbose=2 "Codex Notifier.app"
```

The app has an ad-hoc engineering signature and no notarization ticket. After
verifying the bundle and only when macOS retained a download quarantine flag,
remove that flag from this exact app:

```bash
xattr -dr com.apple.quarantine "Codex Notifier.app"
```

Install and test from the executable inside the app:

```bash
bin="Codex Notifier.app/Contents/MacOS/codex-notifier"
"$bin" install --codex-version 0.144.5
"$bin" status --format json
"$bin" test task-completed --format json --wait-ms 60000
"$bin" test approval-requested --format json --wait-ms 60000
```

Grant notifications when macOS prompts. Require a running agent,
`notification="ready"`, and two delivered self-tests before configuring remote
delivery.

## 4. Prepare one desktop as the SSH receiver

Ubuntu forwards to one configured desktop target at a time. Choose Windows or
macOS, ensure the Ubuntu host can reach it through the LAN or a private VPN, and
enable that desktop's system OpenSSH server. Do not expose SSH directly to the
public Internet solely for this tool.

On Ubuntu, create a dedicated key:

```bash
install -d -m 700 "$HOME/.ssh"
ssh-keygen -t ed25519 -f "$HOME/.ssh/codex-notifier-desktop" \
  -C codex-notifier-relay
```

The extracted Ubuntu relay archive contains the SSH templates needed for this
step. Install only the `.pub` line on the selected desktop using the matching
forced-command template:

- Windows: `examples/authorized_keys-windows.example`;
- macOS: `examples/authorized_keys-macos.example`.

Replace `USERNAME` and `DEDICATED_PUBLIC_KEY`. The SSH login must be the same OS
user that owns the running desktop agent. Apply the exact Windows ACL or Unix
`0700`/`0600` modes in [`restricted-ssh.md`](restricted-ssh.md).

Read the desktop host-key fingerprint locally, compare it over a trusted
channel, and enroll it in Ubuntu `~/.ssh/known_hosts`. Copy
`examples/ssh-config.example` to an owned block in `~/.ssh/config`,
replace every uppercase placeholder, and keep:

```text
StrictHostKeyChecking yes
IdentitiesOnly yes
RequestTTY no
ClearAllForwardings yes
```

## 5. Install the Ubuntu relay and Codex hook

Select the archive using `uname -m`: `x86_64` uses the x86-64 archive;
`aarch64` or `arm64` uses the AArch64 archive. Extract it, then create the relay
configuration before running the installer:

```bash
mkdir -p "$HOME/.config/codex-notifier"
cp examples/config.toml.example "$HOME/.config/codex-notifier/config.toml"
```

The example uses SSH alias `codex-notifier-desktop`, matching the supplied SSH
example. Change both values together if another bounded alias is preferred.
Install the binary, systemd user unit, and verified Codex 0.144.5 task hook:

```bash
./install.sh --codex-version 0.144.5
systemctl --user status codex-notifier.service
codex-notifier doctor ssh
codex-notifier status --format json
```

Review the new `Stop` hook in Codex when requested. `install.sh` refuses to
enable the service or install the hook when the relay configuration is absent.
It preserves existing configuration and unrelated hook groups. Require
`status` to report `role="relay"`, `installed=true`, and
`agent_running=true` before continuing.

If the server does not keep user services alive after logout, enable lingering
deliberately and review that persistent-login change:

```bash
loginctl enable-linger "$USER"
```

## 6. Verify remote and real Codex delivery

Run both explicit remote paths from Ubuntu:

```bash
codex-notifier test task-completed --format json --wait-ms 60000
codex-notifier test approval-requested --format json --wait-ms 60000
```

Both must report `route="remote"` and `delivery="delivered"`, and the selected
desktop must show native notifications. Then run a normal Codex 0.144.5 task on
Ubuntu. Its `Stop` hook should produce a task-completion notification without a
manual notifier command.

The ordinary Codex CLI `PermissionRequest` hook is not yet fixture-verified and
is not installed. The explicit approval self-test proves notification routing,
not automatic CLI approval-event capture.

To switch Ubuntu between Windows and macOS, stop the relay, replace the SSH host
block and verified host-key entry behind the configured alias, run
`codex-notifier doctor ssh`, then restart the service. Do not silently use
`StrictHostKeyChecking accept-new`.

## 7. Reversible removal

On Ubuntu, from the retained extracted relay archive:

```bash
./uninstall.sh
```

This removes the exact owned Codex hook, systemd unit, and installed binary. It
retains relay configuration, unrelated hooks, and SQLite state. Remove the
dedicated key, host block, known-host entry, and desktop public-key line only
after deciding that remote delivery is no longer needed.

On Windows, run from the retained external archive:

```powershell
.\codex-notifier.exe uninstall
```

On macOS, run `uninstall` through the retained external app executable. Both
desktop removals retain SQLite state by design.
