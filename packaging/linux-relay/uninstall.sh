#!/usr/bin/env sh
set -eu

disable=true
hook=true
while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-disable) disable=false; shift ;;
        --no-hook) hook=false; shift ;;
        *)
            echo "usage: uninstall.sh [--no-disable] [--no-hook]" >&2
            exit 2
            ;;
    esac
done

prefix=${CODEX_NOTIFIER_PREFIX:-"$HOME/.local"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
binary="$prefix/bin/codex-notifier"
unit="$config_home/systemd/user/codex-notifier.service"

if [ "$disable" = true ]; then
    systemctl --user disable --now codex-notifier.service 2>/dev/null || true
fi

if [ "$hook" = true ] && [ -x "$binary" ]; then
    "$binary" hook uninstall
fi

rm -f -- "$unit" "$binary"
if [ "$disable" = true ]; then
    systemctl --user daemon-reload
fi
printf 'removed executable=%s unit=%s state_preserved=true\n' "$binary" "$unit"
