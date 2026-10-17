# levi

[![CI](https://github.com/rsdlt/levi/actions/workflows/ci.yml/badge.svg)](https://github.com/rsdlt/levi/actions/workflows/ci.yml)

> *Tch. Your history is filthy. Let's look at it properly.*

A git diff viewer that does exactly one thing: browse the commits on your branch, the files each
one touched, and their diffs, with fzf-style search on top. Nothing else. Captain Levi doesn't
carry gear he won't use.

Named in honor of humanity's strongest soldier: **small, swift and precise.**

*Why not tig or lazygit?* Those are full squads. Levi is one soldier, two blades and no small talk.

Other tools show you a diff. Levi hunts down which commit put that line there. `/c` searches every
diff on the branch, and he doesn't stop until he finds it.

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
 ↑↓ move  enter pick  /c search code  /r reflog  tab pane  esc clear/quit
```

In a terminal narrower than 120 columns, like a split pane, commits and files sit side by side
above a full-width diff.

## Enlist

Needs git 2.31+.

```sh
brew install rsdlt/tap/levi
```

Or with Rust 1.88+:

```sh
cargo install levi-git
```

The crate is `levi-git` because someone else got to `levi` first. The command is still `levi`.

Or the latest from GitHub: `cargo install --git https://github.com/rsdlt/levi`

## Orders

Run `levi` anywhere inside a git repository (bare repositories work too). If you're not in one,
he'll tell you in one line and leave. He doesn't do small talk.

He patrols the current branch. Name a revision and he covers only that ground:

```sh
levi main..HEAD      # what your branch adds to main, before you open the PR
levi v1.0..          # everything since a tag, for the changelog
levi other-branch    # another branch, without checking it out
```

Anything `git log` understands as a revision works. Search, `/c` and `Enter` act on just those
commits, so `/c` is faster too. `/r` still shows the whole reflog.

| Key | Action |
|---|---|
| typing | search (the search bar always has focus) |
| `↑` `↓`, `C-p` `C-n` | move in the focused pane |
| `Tab` `S-Tab`, `←` `→` | switch pane: commits → files → diff |
| `PgUp` `PgDn`, `C-u` `C-d` | scroll the diff |
| `S-↑` `S-↓` | jump to the previous/next hunk, or `/c` match |
| `S-←` `S-→` | scroll the diff sideways, for lines wider than the pane |
| `Enter` | print the selected commit's sha and quit |
| `Esc` | clear the search, or quit if it's empty |
| `C-c` | quit |
| click | select a commit or file, or focus the diff |
| wheel | move the list under the pointer, or scroll the diff (sideways too: trackpad or `Shift`+wheel) |

levi takes the mouse, so to select text with it, hold a modifier while you drag: `Option` in
iTerm2, `Fn` in Terminal.app, `Shift` in most other terminals.

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

### Commands

Slash commands go in the same search bar. They count only as whole words, so `src/config` is
still a plain search.

| Command | What it does |
|---|---|
| `/r` | Switches to the **reflog**: every place HEAD has been, including commits on branches you've since deleted. The rest of the query filters it, by action too (`/r checkout`). Remove `/r` to go back to the log. |
| `/c text` | Searches the **code**: keeps commits whose diffs add or remove a line containing `text`, and only the files where that happens. Everything after `/c` is the text, spaces included; it's matched literally, ignoring case unless you type a capital. The diff jumps to the first match and highlights it. |

They combine with everything else:

```
/c git_parse_ssize_t           which commits touched this function?
parse.rs /c let x =            ...in parse.rs only
/r /c experiment               that code from a branch you deleted
```

The code search is the one thing that isn't instant: git has to diff every commit it checks.
levi waits until you pause typing, only checks the commits that match the rest of your query
(and in them, only the files that match), splits the work across all your cores and streams
matches in as they're found, with progress in the status line. On git.git a search of all 82k
commits takes 6–9 seconds instead of 55, and `parse.c /c ...` under 0.2; on a normal repository
it's over before you notice. Edit the query
and the old search is dropped on the spot.

## Field report

- **Swift.** It opens instantly on huge repositories: history loads on every core at once
  while you're already browsing, and diffs load in the background. git.git, with 82k commits,
  is his Beast Titan. The first screen lands in 40 ms, the whole history in under a second,
  and every search takes 5–12 ms, so it's over before the boulders start flying.
- **Small.** About 2,300 lines of Rust (not counting tests, comments or blank lines) and a
  1.6 MB binary. Short, and still the strongest.
- **Relentless.** `/c` hunts through every diff in history on every core at once, and stops
  the moment you change your mind.
- **Precise.** It shows the current branch, or the revision you name, newest first. Merge commits are diffed against their
  first parent. A sha search matches only real prefixes, so `add` doesn't fuzzy-match half
  your hashes. Each diff opens with the commit's full message, laid out like `git show`, and in
  an edited line only the words that changed are lit, like git's `diff-highlight`. No squinting
  at two lines to find the one character that moved.
- **Clean.** It uses your own `git`, so your config (rename detection, diff algorithm) applies.
  It doesn't touch your repository, and it leaves your terminal exactly as it found it, even if
  it crashes. He'd never leave a mess.

## Cleaning duty

Levi inspects the barracks regularly. So should you:

- After upgrading dependencies (`cargo update`, or `cargo upgrade` for major versions), run
  `cargo test` and [`cargo deny check`](https://github.com/EmbarkStudios/cargo-deny). It fails
  on any license not allowed in `deny.toml`, and on dependencies with known vulnerabilities.
- To release, list the changes under "Unreleased" in [CHANGELOG.md](CHANGELOG.md), bump
  `version` in `Cargo.toml` following [SemVer](https://semver.org/), commit "Release x.y.z", and
  tag it `vx.y.z`.

## License

[MIT](LICENSE). Free to use, like a cup of black tea, held however you like.
