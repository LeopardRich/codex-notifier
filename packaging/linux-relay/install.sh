#!/usr/bin/env sh
set -eu

enable=true
if [ "${1-}" = "--no-enable" ]; then
    enable=false
elif [ "$#" -ne 0 ]; then
    echo "usage: install.sh [--no-enable]" >&2
    exit 2
fi

archive_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix=${CODEX_NOTIFIER_PREFIX:-"$HOME/.local"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}

case "$prefix" in
    /*) ;;
    *) echo "CODEX_NOTIFIER_PREFIX must be absolute" >&2; exit 2 ;;
esac
case "$prefix" in
    *'"'*|*'|'*|*'&'*|*'\'*|*'
'*) echo "CODEX_NOTIFIER_PREFIX contains unsupported characters" >&2; exit 2 ;;
esac

binary="$prefix/bin/codex-notifier"
unit_dir="$config_home/systemd/user"
unit="$unit_dir/codex-notifier.service"
install -d -m 0755 "$prefix/bin" "$unit_dir"
install -d -m 0700 "$config_home/codex-notifier" "$state_home/codex-notifier"
install -m 0755 "$archive_dir/codex-notifier" "$binary"
sed "s|@EXECUTABLE@|$binary|g" \
    "$archive_dir/systemd/codex-notifier.service.in" > "$unit"
chmod 0644 "$unit"

if [ "$enable" = true ]; then
    systemctl --user daemon-reload
    systemctl --user enable --now codex-notifier.service
fi

printf 'installed executable=%s unit=%s\n' "$binary" "$unit"
