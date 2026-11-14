#!/bin/sh
# Linux equivalent of the PopClip / SnipDo clip extensions.
#
# Upstream only ships clip extensions for macOS (PopClip) and Windows (SnipDo),
# but both do the same trivial thing: hand the selected text to the running app
# over its IPC socket. The macOS one is literally
#
#   curl -d "$POPCLIP_TEXT" --unix-socket /tmp/openai-translator.sock http://nextai-translator
#
# and that socket exists on Linux too, so the same integration works here — the
# only missing piece is a way to read the selection, which is what this script
# adds (Wayland PRIMARY selection first, X11 fallback).
#
# Bind it in Hyprland, e.g. ~/.config/hypr/bindings.lua:
#
#   o.bind("SUPER + T", "Translate selection", "nextai-translate-selection")
#
# Environment:
#   NEXTAI_SOCK  override the socket path (default /tmp/openai-translator.sock)
#   NEXTAI_BIN   override the app binary  (default: nextai-translator on PATH)

set -u

sock=${NEXTAI_SOCK:-/tmp/openai-translator.sock}
url=http://nextai-translator
bin=${NEXTAI_BIN:-nextai-translator}

read_selection() {
    if [ -n "${WAYLAND_DISPLAY:-}" ] && command -v wl-paste >/dev/null 2>&1; then
        # Same MIME-type order as src-tauri/src/selected_text.rs.
        for mime in 'text/plain;charset=utf-8' text/plain UTF8_STRING; do
            text=$(wl-paste --primary --no-newline --type "$mime" 2>/dev/null) || continue
            if [ -n "$text" ]; then
                printf '%s' "$text"
                return 0
            fi
        done
    fi
    if command -v xclip >/dev/null 2>&1; then
        xclip -selection primary -o 2>/dev/null && return 0
    fi
    return 1
}

send() {
    printf '%s' "$1" | curl -s -m 5 -d @- --unix-socket "$sock" "$url" >/dev/null 2>&1
}

text=$(read_selection) || exit 0

# Nothing selected (or only whitespace): stay quiet.
case $text in
    *[![:space:]]*) ;;
    *) exit 0 ;;
esac

if send "$text"; then
    exit 0
fi

# App is not running yet: start it, then retry while the socket comes up.
if command -v "$bin" >/dev/null 2>&1; then
    setsid "$bin" --silently >/dev/null 2>&1 &
fi
i=0
while [ "$i" -lt 60 ]; do
    sleep 0.1
    i=$((i + 1))
    if send "$text"; then
        exit 0
    fi
done

echo "nextai-translator: IPC socket $sock did not come up" >&2
exit 1
