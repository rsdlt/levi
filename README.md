# levi

> *Tch. Your history is filthy. Let's look at it properly.*

A git diff viewer that does exactly one thing: browse the commits on your branch, the files each
one touched, and their diffs, with fzf-style search on top. Nothing else. Captain Levi doesn't
carry gear he won't use.

Named in honor of humanity's strongest soldier: **small, swift and precise.**

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

## Enlist

Needs Rust 1.88+ and git 2.31+.

```sh
cargo install --path .
```

## Orders

Run `levi` anywhere inside a git repository (bare repositories work too). If you're not in one,
he'll tell you in one line and leave. He doesn't do small talk.

| Key | Action |
|---|---|
| typing | search (the search bar always has focus) |
| `↑` `↓`, `C-p` `C-n` | move in the focused pane |
| `Tab` `S-Tab`, `←` `→` | switch pane: commits → files → diff |
| `PgUp` `PgDn`, `C-u` `C-d` | scroll the diff |
| `Enter` | print the selected commit's sha and quit |
| `Esc` | clear the search, or quit if it's empty |
| `C-c` | quit |

`Enter` prints the sha to stdout, so levi works as a picker. Choose the commit; you won't regret
it.

```sh
git show $(levi)
sha=$(levi) && git revert "$sha"   # levi exits 130 if you quit without picking
```

## Hunting

As you type, the commits and files columns narrow down together, like a squad closing in:

- A commit matches by **subject or author**, by **any file it touched** (including the old name
  of a renamed file), or by **sha prefix** (4+ hex characters).
- If a commit matched only through its files, the files column shows just those files.
  Typing `parse.rs` answers "which commits touched parse.rs?" in a single strike.
- fzf syntax: `'exact`, `^prefix`, `suffix$`, `!not`, smart case, and space-separated terms that
  must all match.

## Field report

- **Swift.** It opens instantly on huge repositories: the log streams in while you're already
  browsing, and diffs load in the background. git.git, with 82k commits, is his Beast Titan.
  The first screen lands in under 0.1 s and every search takes 5–12 ms, so it's over before
  the boulders start flying.
- **Small.** About 1,100 lines of Rust (not counting tests) and a 1.6 MB binary. Short, and
  still the strongest.
- **Precise.** It shows the current branch, newest first. Merge commits are diffed against their
  first parent. A sha search matches only real prefixes, so `add` doesn't fuzzy-match half
  your hashes.
- **Clean.** It uses your own `git`, so your config (rename detection, diff algorithm) applies.
  It doesn't touch your repository, and it leaves your terminal exactly as it found it, even if
  it crashes. He'd never leave a mess.

## License

[MIT](LICENSE). Free to use, like a cup of black tea, held however you like.
