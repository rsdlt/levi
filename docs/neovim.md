# levi in Neovim

> *You live in that editor. Fine. levi comes to you.*

This plugin opens [levi](https://github.com/rsdlt/levi) in a floating window over Neovim, in
the repository of the file you're in. It has two orders:

- **`:Levi`** shows the commit you pick, in a new tab: with
  [diffview.nvim](https://github.com/sindrets/diffview.nvim) when it's installed, as
  `git show` otherwise.
- **`:LeviSend`** hands what you found to your coding agent. Run the agent in a Neovim
  terminal, find the commit, the file and the lines you mean, and press `Enter`. levi types a
  reference to them into the agent's input. He doesn't send it. You finish the question.

```
In commit d3a93ab ("Stack the lists above the diff on narrow terminals"), src/ui.rs lines 27-40: why 120?
```

## Enlist

You need levi 0.16 or newer on your `PATH` and Neovim 0.11 or newer.

```sh
brew install rsdlt/tap/levi        # or: cargo install levi-git
```

The plugin is in levi's own repository. With [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{
  "rsdlt/levi",
  cmd = { "Levi", "LeviSend" },
  keys = {
    { "<leader>gl", "<cmd>Levi<cr>", desc = "levi: browse commits" },
    { "<leader>ga", "<cmd>LeviSend<cr>", desc = "levi: hand a commit to the agent" },
  },
}
```

With Neovim's built-in `vim.pack` (0.12), add `vim.pack.add({ "https://github.com/rsdlt/levi" })`
and the same keys with `vim.keymap.set`. The plugin binds no keys of its own.

## Orders

`:Levi` and `:LeviSend` take one revision, like levi does: `:Levi main..HEAD` shows only what
your branch adds to main.

In the window, levi works as everywhere else: type to search, `Tab` between panes, `C-o` to
open a file at the line you're reading, in a new tab of this same Neovim. `Esc` closes the
window and does nothing.

`:LeviSend` types into the terminal you're in, or into the one you used last when you're in a
file. It works with any agent that runs in a Neovim terminal, and with plugins that open the
agent in one. When that terminal is on screen, levi puts you in it, ready to type; when it's
hidden, he types anyway and tells you.

From Lua, `require("levi").open(rev)` and `require("levi").send(rev)` do the same, `rev`
optional.

## Fine print

- levi runs in the repository of the file you're in, or in Neovim's working directory when
  you're not in a file.
- A reference is never sent, so a wrong terminal costs a `C-u`.
- The lines are the ones in view, or the ones under your drag selection.
- If levi can't start, say outside a repository, the window stays up with his reason. `q`
  closes it.
- Under the hood it's `levi --pick-context` and `chansend()`: see
  [lua/levi/init.lua](../lua/levi/init.lua) to build your own variations.

The rest of levi's orders, keys and search syntax are in the
[main README](../README.md).
