# levi — plan

> Small, swift and precise. One task: browse commits, the files they touched, and the diffs — with fzf-style search on top.

## Layout

```
┌─ / fuzzy query ─────────────────────────────────────────────────────────┐
├─ commits ──────────┬─ files ───────────┬─ diff ─────────────────────────┤
│ a1b2c3d fix parser │ M src/parse.rs    │ @@ -10,6 +10,8 @@               │
│ 9f8e7d6 add tests  │ A tests/p.rs      │ - old line                     │
│ ...                │                   │ + new line                     │
└────────────────────┴───────────────────┴────────────────────────────────┘
```

Widths: ~25% / 20% / 55%. The diff pane gets the room because that's where reading happens.

## Decisions

| # | Topic | Decision |
|---|---|---|
| 1 | Column a source | **Commit log** (`git log`), not the reflog. |
| 2 | Scope | **Current branch only** (`HEAD`), no `--all`. |
| 3 | Merge commits | **Diff against first parent** (`--diff-merges=first-parent` for files and diff). History is *not* first-parent-only: commits from merged branches still show. |
| 4 | Uncommitted changes | **Out for now.** No working-tree pseudo-commit. |
| 5 | Git backend | Shell out to the `git` CLI (no `git2`/`gix`): respects user config, no C build, small binary. |
| 6 | Fuzzy matching | `nucleo-matcher` (fzf-compatible syntax: smart case, `^`, `$`, `'`, `!`). |
| 7 | Diff rendering | Simple coloring (`+` green, `-` red, `@@` cyan, headers dim). No syntax highlighting in v1. |

## Git commands

All commands run as `git --literal-pathspecs` (so paths like `glob*.txt` aren't globbed). Requires git ≥ 2.31 (`--diff-merges`).

- Repo check: `git rev-parse --show-toplevel`
- Log + files, one streaming pass:
  `git log --no-color --no-show-signature --diff-merges=first-parent --name-status -z --format=%x1e%H%x00%h%x00%an%x00%ar%x00%s`
  Each record: `\x1e` + NUL-separated header, then `\n` + `STATUS\0path\0` entries (`R`/`C` carry `old\0new\0`).
- Diff on demand:
  `git show --no-color --no-ext-diff --no-show-signature --diff-merges=first-parent --patch --format= <sha> -- [old] <path>`
  (renames/copies: pass both old and new path so git pairs them)

## Search semantics

- A commit stays in column a if the query matches its **header** (subject + author), **or any file it touched**, **or its sha**.
- Column b shows only the matching files. If the commit matched on header or sha, it shows all its files.
- Results are ranked by score; ties keep log order (newest first). Matched characters are highlighted.
- Typing `parse.rs` answers "which commits touched parse.rs?" right away.

Details settled while building:

- **Sha is prefix-only**, not fuzzy: the whole query, if it's ≥4 hex chars, matches commits whose sha starts with it (ranked first). Fuzzy-matching hashes would make queries like `add` or `bad` match nearly everything.
- **Header is one haystack** (`subject\tauthor`), so a negation like `!wip` drops the commit if either field contains it. **Files are matched one by one** (path, or old path for renames), so `!test` lists a commit's non-test files.
- Every atom of a query must match within the same haystack (the header, or a single file path).
- Files use nucleo's path-aware scoring; the header uses the default config.

### Filtering performance

- Typing only edits the query; the view is refiltered once per frame (or before any key that needs it), so keystroke bursts cost one pass.
- When the new query just extends the old one (and neither adds `!`, `$` or `\`), only the previous hits are rescanned.
- Matching runs on all cores (`std::thread::scope`, one matcher per thread).
- Lists render only their visible rows, so drawing cost is independent of log size.
- git.git, 82.5k commits / 271k file entries, 12 cores: 5–12 ms per refilter (was 25–45 ms single-threaded).

## Performance

- A background thread streams `git log` and sends commits over an `mpsc` channel in batches. The UI is live immediately and fills in as data arrives.
- Diffs load lazily on a worker thread and are cached in an LRU keyed by `(sha, path)`.
- Stale diff responses (selection moved on) are dropped, so fast navigation never stalls.

## Keys

| Key | Action |
|---|---|
| typing | goes into the search bar (always focused, like fzf) |
| `↑/↓`, `C-p/C-n` | move in the focused column |
| `Tab` / `S-Tab`, `←/→` | cycle focus a → b → c |
| `PgUp/PgDn`, `C-u/C-d` | scroll the diff |
| `Enter` | print the selected sha to stdout and quit (`git checkout $(levi)`) |
| `Esc` | clear the query, or quit if it's already empty |
| `C-c` | quit |

## Crates

`ratatui` (bundled `crossterm`), `nucleo-matcher`, `anyhow`, `lru` (or a small hand-rolled cache).

## Code layout (~800 lines)

```
src/
  main.rs    – repo check, terminal setup/restore, panic hook, event loop
  git.rs     – log stream parser, diff fetch + line classification, types
  worker.rs  – input / log / diff threads, all reporting through one Msg channel
  filter.rs  – nucleo matching over commits + files, producing filtered views
  app.rs     – state: query, filtered views, selections, focus, scroll, diff LRU
  ui.rs      – layout and rendering of the 4 widgets
  testutil.rs – throwaway git repos for tests
tests/cli.rs – runs the binary: errors, flags, exit codes
```

`App` is thread-free: it consumes `Msg`s and exposes `take_diff_request()`; `main` forwards
requests to the diff worker. Tests play the worker's role synchronously.

Measured on git.git (82.5k commits, release build): first commits + diff on screen in <0.1s,
full log streamed in ~5s (bounded by `git log` itself), ~75 MB RSS, 1.2 MB binary.

## Build order

1. `git.rs` + tests against a temp repo: NUL-separated parsing, renames (`R100 old new`), merges, root commit.
2. Static three-column UI with navigation; terminal restore on panic.
3. Async log loading + diff cache.
4. Search + filtering + match highlighting.
5. Polish: non-git dir / empty repo errors, status line (`123/4567 commits`), smoke test on a large repo.

All five steps are done.

## CLI behaviour

- `levi` takes no arguments; `-h/--help` and `-V/--version` only. Anything else exits 2.
- Before the TUI starts, a preflight check fails with a one-line `levi: …` on stderr (exit 1):
  `git not found in PATH`, `needs git 2.31 or newer`, `not inside a git repository`, `no commits yet`.
- Bare repositories work (commands run from the bare dir instead of a work tree's top level).
- Exit codes: 0 with a picked sha on stdout, 130 when quit without picking (like fzf), so
  `sha=$(levi) && git show "$sha"` stops cleanly.
- Integration tests in `tests/cli.rs` run the real binary for every pre-TUI path.

## Later (not now)

- Reflog mode (`-r` flag or hotkey) — same pipeline, different source.
- `--all` branches.
- Working-tree pseudo-commit at the top of column a.
- Optional `delta` piping / syntax highlighting.
