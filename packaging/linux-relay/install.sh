#!/usr/bin/env sh
set -eu

enable=true
hook=true
codex_version=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-enable) enable=false; shift ;;
        --no-hook) hook=false; shift ;;
        --codex-version)
            [ "$#" -ge 2 ] || { echo "--codex-version requires a value" >&2; exit 2; }
            codex_version=$2
            shift 2
            ;;
        *)
            echo "usage: install.sh [--no-enable] [--no-hook] [--codex-version VERSION]" >&2
            exit 2
            ;;
    esac
done

archive_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix=${CODEX_NOTIFIER_PREFIX:-"$HOME/.local"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}
config="$config_home/codex-notifier/config.toml"

if { [ "$enable" = true ] || [ "$hook" = true ]; } && [ ! -f "$config" ]; then
    echo "relay configuration is required at $config" >&2
    echo "copy examples/config.toml.example, set ssh_host_alias, then retry" >&2
    exit 2
fi

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

if [ "$hook" = true ]; then
    if [ -n "$codex_version" ]; then
        "$binary" hook install --codex-version "$codex_version"
    else
        "$binary" hook install
    fi
fi

if [ "$enable" = true ]; then
    systemctl --user daemon-reload
    systemctl --user enable --now codex-notifier.service
fi

printf 'installed executable=%s unit=%s\n' "$binary" "$unit"
