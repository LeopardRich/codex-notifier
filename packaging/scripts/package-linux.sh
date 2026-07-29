#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 7 ]]; then
    echo "usage: package-linux.sh BINARY OUTPUT_DIR VERSION COMMIT TARGET LICENSE_NOTICES SBOM" >&2
    exit 2
fi

binary=$1
output=$2
version=$3
commit=$4
target=$5
notices=$6
sbom=$7
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || { echo "invalid version" >&2; exit 2; }
[[ $commit =~ ^[0-9a-f]{40}$ ]] || { echo "invalid commit" >&2; exit 2; }
[[ $target == x86_64-unknown-linux-gnu || $target == aarch64-unknown-linux-gnu ]] || { echo "invalid target" >&2; exit 2; }
for path in "$binary" "$notices" "$sbom" LICENSE packaging/linux-relay/codex-notifier.service.in packaging/linux-relay/install.sh packaging/linux-relay/uninstall.sh packaging/ssh/config.relay.example; do
    [[ -f $path ]] || { echo "missing release input" >&2; exit 2; }
done

arch=${target%%-*}
name="codex-notifier-v${version}-linux-${arch}"
mkdir -p "$output"
stage=$(mktemp -d "$output/.${name}.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
root="$stage/$name"
mkdir -p "$root/systemd" "$root/examples"
install -m 0755 "$binary" "$root/codex-notifier"
install -m 0755 packaging/linux-relay/install.sh "$root/install.sh"
install -m 0755 packaging/linux-relay/uninstall.sh "$root/uninstall.sh"
install -m 0644 packaging/linux-relay/codex-notifier.service.in "$root/systemd/codex-notifier.service.in"
install -m 0644 packaging/ssh/config.relay.example "$root/examples/ssh-config.example"
install -m 0644 LICENSE "$root/LICENSE"
install -m 0644 "$notices" "$root/THIRD-PARTY-LICENSES.html"
install -m 0644 "$sbom" "$root/codex-notifier.spdx.json"
cat > "$root/RELEASE-METADATA.json" <<EOF
{"schema_version":1,"product":"codex-notifier","version":"$version","target":"$target","commit":"$commit","signature":"checksums-and-provenance"}
EOF

epoch=${SOURCE_DATE_EPOCH:-0}
[[ $epoch =~ ^[0-9]+$ ]] || { echo "invalid SOURCE_DATE_EPOCH" >&2; exit 2; }
archive="$output/$name.tar.gz"
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner -C "$stage" -czf "$archive" "$name"
sha256sum "$archive" | sed "s|  .*/|  |" > "$archive.sha256"
printf '%s\n' "$archive"
