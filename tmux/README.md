# levi for tmux

> *Tch. You found the commit. Now tell your agent what to do about it.*

This plugin opens [levi](https://github.com/rsdlt/levi) in a popup over your agent's pane in
[tmux](https://github.com/tmux/tmux). Find the commit, the file and the lines you mean, press
`Enter`, and levi types a reference to them into the agent's input. He doesn't send it. You
finish the question.

```
In commit d3a93ab ("Stack the lists above the diff on narrow terminals"), src/ui.rs lines 27-40: why 120?
```

The agent gets the commit, the file and the lines in one go, and reads the rest with
`git show`. No copying shas, no pasting paths.

## Enlist

You need levi 0.16 or newer on your `PATH` and tmux 3.2 or newer, for popups.

```sh
brew install rsdlt/tap/levi        # or: cargo install levi-git
```

With [TPM](https://github.com/tmux-plugins/tpm), add the plugin to `tmux.conf`, then press
`prefix I` to install it:

```tmux
set -g @plugin 'rsdlt/levi'
```

Without TPM, clone the repository anywhere and run its `levi.tmux` from `tmux.conf`:

```tmux
run-shell ~/src/levi/levi.tmux
```

The key is `prefix G`. For another one, set `@levi-key` above the plugin line:

```tmux
set -g @levi-key 'C-l'
```

## Orders

1. In your agent's pane, press `prefix` `G`. levi opens over it, in the pane's directory.
2. Search, pick the commit and the file, and scroll to the lines you mean, or drag over them.
3. Press `Enter`. The reference lands in the agent's input.
4. Type your question and send it.

`Esc` in levi closes the popup and types nothing.

## Fine print

- The reference goes to the pane you pressed the key in. tmux can't tell an agent from a
  shell, so it types into whatever runs there. It's never sent, so a wrong pane costs a
  `C-u`.
- levi runs in the pane's current directory, so the pane has to be inside the repository.
- The lines are the ones in view, or the ones under your drag selection.
- Under the hood it's `levi --pick-context` and `tmux send-keys -l`, so you can build your own
  variations: see [levi-tmux.sh](levi-tmux.sh).

The rest of levi's orders, keys and search syntax are in the
[main README](https://github.com/rsdlt/levi#readme).
