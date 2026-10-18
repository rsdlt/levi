#!/bin/sh
# The levi plugin for herdr.
#
#   open  (the action) opens levi in a popup over the focused pane.
#   pick  (the popup) runs levi in that pane's agent's repository, and types
#         what you picked into the agent's input, unsent:
#         In commit d3a93ab ("Stack the lists"), src/ui.rs lines 27-40: _
set -u

herdr=${HERDR_BIN_PATH:-herdr}

# One string field from herdr's JSON. Good enough for ids and paths; it doesn't
# unescape, so a path with a quote or backslash in it won't come through.
field() {
    sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"
}

# Shows a message and waits, so the popup doesn't vanish before it's read.
fail() {
    printf '%s\n\nPress Enter to close.' "$1"
    read -r _
    exit 1
}

open() {
    exec "$herdr" plugin pane open --plugin rsdlt.levi --entrypoint pick
}

pick() {
    # Popups get the pane underneath only through the plugin context.
    target=$(printf '%s' "${HERDR_PLUGIN_CONTEXT_JSON:-}" | field focused_pane_id)
    [ -n "$target" ] || target=${HERDR_PANE_ID:-}
    [ -n "$target" ] || fail "levi: no focused pane to send to."

    pane=$("$herdr" pane get "$target" 2>/dev/null)
    printf '%s' "$pane" | grep -q '"agent":"' \
        || fail "levi: the focused pane isn't running an agent. Focus your agent's pane, then try again."

    dir=$(printf '%s' "$pane" | field foreground_cwd)
    [ -n "$dir" ] || dir=$(printf '%s' "$pane" | field cwd)
    cd "$dir" 2>/dev/null || fail "levi: can't open the agent's directory: $dir"

    command -v levi >/dev/null 2>&1 \
        || fail "levi: not found on PATH. Install it: brew install rsdlt/tap/levi"

    picked=$(levi --pick-context)
    case $? in
        0) ;;
        130) exit 0 ;; # quit without picking
        *) fail "" ;;  # levi said why on the screen already
    esac

    "$herdr" pane send-text "$target" "In $picked: " >/dev/null \
        || fail "levi: couldn't type into the agent's pane ($target)."
}

case ${1:-} in
    open) open ;;
    pick) pick ;;
    *) echo "usage: levi-herdr.sh open|pick" >&2; exit 2 ;;
esac
