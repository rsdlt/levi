#!/bin/sh
# The levi plugin for tmux: runs levi in a popup over a pane, in that pane's
# directory, and types what you picked into the pane, unsent:
#   In commit d3a93ab ("Stack the lists"), src/ui.rs lines 27-40: _
#
# levi.tmux binds it, with the popup around it.
set -u

# Shows a message and waits, so the popup doesn't vanish before it's read.
fail() {
    printf '%s\n\nPress Enter to close.' "$1"
    read -r _
    exit 1
}

# The pane under the popup. tmux doesn't expand #{pane_id} in a popup's
# command, but it answers for the client the popup is on.
target=$(tmux display-message -p '#{pane_id}')
[ -n "$target" ] || fail "levi: no pane to send to."

command -v levi >/dev/null 2>&1 \
    || fail "levi: not found on PATH. Install it: brew install rsdlt/tap/levi"

picked=$(levi --pick-context)
case $? in
    0) ;;
    130) exit 0 ;; # quit without picking
    *) fail "" ;;  # levi said why on the screen already
esac

# -l types the text as it is, so nothing in it is read as a key name.
tmux send-keys -t "$target" -l "In $picked: " \
    || fail "levi: couldn't type into the pane ($target)."
