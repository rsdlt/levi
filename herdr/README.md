# levi for herdr

> *Tch. You found the commit. Now tell your agent what to do about it.*

This plugin opens [levi](https://github.com/rsdlt/levi) over your agent's pane in
[herdr](https://herdr.dev). Find the commit, the file and the lines you mean, press `Enter`,
and levi types a reference to them into the agent's input. He doesn't send it. You finish the
question.

```
In commit d3a93ab ("Stack the lists above the diff on narrow terminals"), src/ui.rs lines 27-40: why 120?
```

The agent gets the commit, the file and the lines in one go, and reads the rest with
`git show`. No copying shas, no pasting paths.

## Enlist

You need levi 0.16 or newer and herdr 0.8 or newer, on macOS or Linux.

```sh
brew install rsdlt/tap/levi        # or: cargo install levi-git
herdr plugin install rsdlt/levi/herdr
```

Plugins can't bind keys, so add one to herdr's `config.toml`, then reload:

```toml
[[keys.command]]
key = "prefix+l"
type = "plugin_action"
command = "rsdlt.levi.pick"
```

```sh
herdr server reload-config
```

## Orders

1. In your agent's pane, press `prefix` `l`. levi opens over it, in the agent's repository.
2. Search, pick the commit and the file, and scroll to the lines you mean, or drag over them.
3. Press `Enter`. The reference lands in the agent's input.
4. Type your question and send it.

`Esc` in levi closes the popup and types nothing.

## Fine print

- The reference goes to the pane you pressed the key in. It has to be an agent's pane; levi
  says so if it isn't, and waits for you to read it.
- The lines are the ones in view, or the ones under your drag selection.
- levi runs the `levi` on your `PATH`, from the agent's working directory.
- Under the hood it's `levi --pick-context` and `herdr pane send-text`, so you can build your
  own variations: see [levi-herdr.sh](levi-herdr.sh).

The rest of levi's orders, keys and search syntax are in the
[main README](https://github.com/rsdlt/levi#readme).
