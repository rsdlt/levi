#!/bin/sh
# levi for tmux. TPM runs this file; without TPM, add to tmux.conf:
#   run-shell /path/to/levi/levi.tmux
#
# Binds prefix+G (set @levi-key to change it) to open levi in a popup over the
# current pane. Enter types a reference to what you picked into that pane.
dir=$(cd "$(dirname "$0")" && pwd)

key=$(tmux show-option -gqv @levi-key)
[ -n "$key" ] || key=G

# The popup starts in the pane's directory, which is the agent's repository.
tmux bind-key "$key" display-popup -E -w 90% -h 85% -d '#{pane_current_path}' \
    "sh '$dir/tmux/levi-tmux.sh'"
