#!/usr/bin/env sh
set -eu

disable=true
if [ "${1-}" = "--no-disable" ]; then
    disable=false
elif [ "$#" -ne 0 ]; then
    echo "usage: uninstall.sh [--no-disable]" >&2
    exit 2
fi

prefix=${CODEX_NOTIFIER_PREFIX:-"$HOME/.local"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
binary="$prefix/bin/codex-notifier"
unit="$config_home/systemd/user/codex-notifier.service"

if [ "$disable" = true ]; then
    systemctl --user disable --now codex-notifier.service 2>/dev/null || true
    systemctl --user daemon-reload
fi

rm -f -- "$unit" "$binary"
printf 'removed executable=%s unit=%s state_preserved=true\n' "$binary" "$unit"
