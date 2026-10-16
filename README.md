# levi

A git diff viewer that does one thing: browse the commits on your branch, the files each one
touched, and their diffs, with fzf-style search on top.

Named in honor of Captain Levi: small, swift and precise.

```
┌ levi ───────────────────────────────────────────────────────────────────┐
│> parse.rs█                                                               │
└────────────────────────────────────────────────────────── 12/4567 commits┘
┌ commits ───────────────┐┌ files ───────────┐┌ src/parse.rs ──────────────┐
│a1b2c3d fix parser      ││M src/parse.rs    ││@@ -10,6 +10,8 @@           │
│9f8e7d6 split tokenizer ││                  ││ fn parse(input: &str) {    │
│...                     ││                  ││-    let x = 1;             │
│                        ││                  ││+    let x = 2;             │
└────────────────────────┘└──────────────────┘└────────────────────────────┘
```

## Install

Needs Rust 1.88+ and git 2.31+.

```sh
cargo install --path .
```

## Use

Run `levi` anywhere inside a git repository (bare repositories work too).

| Key | Action |
|---|---|
| typing | search (the search bar always has focus) |
| `↑` `↓`, `C-p` `C-n` | move in the focused pane |
| `Tab` `S-Tab`, `←` `→` | switch pane: commits → files → diff |
| `PgUp` `PgDn`, `C-u` `C-d` | scroll the diff |
| `Enter` | print the selected commit's sha and quit |
| `Esc` | clear the search, or quit if it's empty |
| `C-c` | quit |

`Enter` prints to stdout, so levi works as a picker:

```sh
git show $(levi)
sha=$(levi) && git revert "$sha"   # levi exits 130 if you quit without picking
```

## Search

As you type, both the commits and the files columns narrow down:

- A commit matches by **subject or author**, by **any file it touched** (including the old name
  of a renamed file), or by **sha prefix** (4+ hex characters).
- If a commit matched only through its files, the files column shows just those files.
  Typing `parse.rs` answers "which commits touched parse.rs?".
- fzf syntax: `'exact`, `^prefix`, `suffix$`, `!not`, smart case, and space-separated terms that
  must all match.

## How it behaves

- Shows the current branch (`git log`), newest first. Merge commits are diffed against their
  first parent.
- Opens instantly on large repositories: the log streams in while you browse, and diffs load in
  the background. On git.git (82k commits) the first screen appears in under 0.1 s and each
  search takes 5–12 ms.
- Uses your `git`, so your config (rename detection, diff algorithm) applies.

## License

[MIT](LICENSE)
