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
 ↑↓ move  enter pick  /c search code  /r reflog  tab pane  esc clear/quit
```

Widths: ~25% / 20% / 55%. The diff pane gets the room because that's where reading happens.

Below 120 columns (a split pane, a laptop half-screen) that leaves the diff too narrow to read, so
the layout stacks: commits | files take the top 35% (55% / 45% of the width) and the diff gets the
full width below. It follows the terminal's width on every frame; there's no toggle key, for the
same reason as the hints below. Focus order and keys don't change.

### Resizing

The splits above are defaults; dragging a seam with the mouse moves it.

- **Seams.** Every pane draws its own border, so where two panes meet there are two border
  columns side by side (two rows, between the lists and the diff when stacked). A left press
  on either starts a resize; the outer edges aren't seams. A click on a border used to only
  focus its pane, so no row pick or selection is lost.
- **Dragging** moves the seam with the pointer and changes only the two panes beside it: in
  the wide layout, moving commits|files leaves the diff's width alone. Release ends it. A
  resize doesn't move focus, the selection or any scroll.
- **Floors.** No pane gets narrower than 12 columns or shorter than 5 rows; the seam stops
  there.
- **Proportions, not cells.** Splits are kept as fractions of the body, so a terminal resize
  keeps them. The wide and the stacked layout each keep their own, so crossing 120 columns
  and back doesn't lose either.
- **Double-click a seam** to put its layout back to the defaults.
- **Session only.** Quitting forgets the sizes: keeping them needs a config file, and levi
  has no settings. No keys either: printable keys belong to the search, and a mouse-free
  user has the defaults, which are what levi was laid out for.
- Terminals that report clicks but not drags just don't resize.

A path too long for the files column loses its start, never its end: whole leading directories
go first, so it reads `…/components/frontend/Button.tsx`, and only a file name wider than the
column itself gets cut (`…ton.tsx`). Wrapping was rejected: one row per file keeps clicks, scrolling
and the selection simple. Renames are written like `git diff --stat` does, the shared directories
once: `src/{old.rs → new.rs}`, `src/{a → b}/mod.rs`. When even the part in braces doesn't fit, the
row shows just the new path (the `R` still says it's a rename). Highlights follow the visible text;
the diff's title always has the full path.

The bottom row lists key hints, `/c` and `/r` included, so nobody has to read `--help` to find
them. They're ordered by usefulness; whatever doesn't fit the width is dropped from the end, never
cut mid-hint. One row, always on: no toggle key, since every printable key belongs to the search.

## Decisions

| # | Topic | Decision |
|---|---|---|
| 1 | Column a source | **Commit log** (`git log`) by default; `/r` switches to the reflog (see Commands). |
| 2 | Scope | **Current branch** (`HEAD`) by default, or one revision given as the only argument (`levi main..HEAD`). No `--all`. |
| 3 | Merge commits | **Diff against first parent** (`--diff-merges=first-parent` for files and diff). History is *not* first-parent-only: commits from merged branches still show. |
| 4 | Uncommitted changes | **A row on top of the log**, `working uncommitted changes` in red, while the working tree differs from HEAD (see Uncommitted changes). Out at first; it came in once agents made "what has it changed but not committed?" the most common question. |
| 5 | Git backend | Shell out to the `git` CLI (no `git2`/`gix`): respects user config, no C build, small binary. |
| 6 | Fuzzy matching | `nucleo-matcher` (fzf-compatible syntax: smart case, `^`, `$`, `'`, `!`). |
| 7 | Diff rendering | The commit message on top, code in syntax colors (the terminal's 16, one fixed mapping) with a green/red `+`/`-` bar and removed lines dimmed, changed words in reverse video (see Diff pane). Where the terminal shows truecolor and gives its background, changed rows also get a soft tint mixed from it, and changed words a stronger one. Files in unknown languages keep plain coloring (`+` green, `-` red); `@@` cyan, headers dim. Syntax colors were out of scope at first, and came in once reading a big repo under heavy agent-driven change showed how much they matter. No themes, no settings, no per-language tweaks. |

## Git commands

All commands run as `git --literal-pathspecs` (so paths like `glob*.txt` aren't globbed). Requires git ≥ 2.31 (`--diff-merges`).

- Repo check: `git rev-parse --show-toplevel`
- Log + files, one streaming pass:
  `git log --no-color --no-show-signature --diff-merges=first-parent --name-status -z --format=%x1e%H%x00%h%x00%an%x00%ar%x00%s`
  Each record: `\x1e` + NUL-separated header, then `\n` + `STATUS\0path\0` entries (`R`/`C` carry `old\0new\0`).
- Diff on demand, message included:
  `git show --no-color --no-ext-diff --no-show-signature --diff-merges=first-parent --format=<header + message + NUL> --patch <sha> -- [old] <path>`
  (renames/copies: pass both old and new path so git pairs them; a commit without files gets `--no-patch`)

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

- The branch loads on every core: a producer streams `git rev-list HEAD` and cuts it into chunks (256 commits first, for a fast first frame, then 2048); one worker per core runs `git log --no-walk=unsorted --stdin --name-status` on a chunk at a time; a coordinator forwards finished chunks in log order. Same records as a single `git log`, verified by test. git.git: first batch in ~40 ms, all 82.5k commits in ~0.9 s (single `git log`: 5–9 s).
- The reflog keeps a single streaming `git log --walk-reflogs`: it's short, and only a reflog walk knows each entry's action.
- Diffs load lazily on a worker thread and are cached in an LRU keyed by `(sha, path)`.
- Stale diff responses (selection moved on) are dropped, so fast navigation never stalls.

### Benchmark for the README (0.15.0, 2026-10-18)

git.git at 82,513 commits, Apple M4 Pro (12 cores), git 2.56, the Homebrew release binary;
median of 3 runs after a warm-up, through `levi --filter`, so anyone can rerun them:

| Job | levi | git |
|---|---|---|
| all history with files | `--filter ''` 0.86 s | `git log --name-status` 2.82 s |
| code search, all history | `--filter '/c git_parse_ssize_t'` 6.6 s, 12 commits | `git log -i -G git_parse_ssize_t` 19.1 s, 6 commits |
| code search, one path | `--filter 'parse.c /c git_parse_ssize_t'` 1.12 s, 2 commits | `git log -i -G … -- parse.c` 0.49 s, 1 commit |

The 6 extra commits from `/c` are merges, which `git log -G` skips and levi diffs against their
first parent. git wins the one-path search from cold: `--filter` loads all history first
(0.86 s), and the fuzzy `parse.c` also matches `parse-options.c` and others; in the TUI the
history is already loaded. First screen in the TUI, in a pty that answers the background query
like a real terminal: 48–55 ms (50 median over 7 runs); without an answer it waits out the
100 ms limit (191 ms).

## Keys

| Key | Action |
|---|---|
| typing | goes into the search bar (always focused, like fzf) |
| `↑/↓`, `C-p/C-n` | move in the focused column |
| `Tab` / `S-Tab`, `←/→` | cycle focus a → b → c |
| `PgUp/PgDn`, `C-u/C-d` | scroll the diff |
| `S-↑/S-↓` | jump the diff to the previous/next `@@` hunk, putting it on top; with `/c` on, to the previous/next matching line instead, with 3 lines of context and sideways into view. Past a file's last stop it moves to the next file and lands on its first (backwards: the previous file's last), like vim's `]c`, waiting for that diff to load if needed. Stops at the commit's ends, works from any pane |
| `S-←/S-→` | scroll the diff sideways, 8 columns (Alt-arrows would be word jumps in macOS terminals) |
| `Enter` | print the selected sha to stdout and quit (`git checkout $(levi)`); on the uncommitted changes, which have no sha, the status line says so and levi stays |
| `C-o` | open the selected file in an editor at the new-file line shown at the top of the diff (from the hunk headers; a removed line maps to where it was; above the first hunk, its first line). Inside Neovim's terminal (`$NVIM` set), `nvim --server $NVIM --remote-expr` runs `tabedit +N file` there, so levi keeps its own tab and nothing nests. Otherwise `$VISUAL`/`$EDITOR`/`vi` runs through `sh -c '$EDITOR "$@"'` (like git) with `+N file` (`file:N` for hx/zed/subl, `--goto file:N` for code/cursor), and gets the terminal: the input thread parks first (it polls in 100ms spells and acknowledges a pause, so no key is stolen), levi leaves the alternate screen, and comes back where it was. The file opens from the working tree, which may have moved on since the commit. When it isn't there (deleted, renamed, or a bare repo), `git cat-file blob <sha>:<path>` (the parent's, `<sha>^:`, for a file the commit deletes, with the line taken from the diff's old side) goes to a read-only copy at `$TMPDIR/levi-<full sha>[-parent]/<path>`, which keeps the name for syntax highlighting; the status says it's read-only history. Another editor's copy is deleted when it quits; Neovim's stays, since its buffer may still be open |
| `C-y` | copy the selected commit's full sha to the clipboard and stay ("yank"); the status line says `copied <short>` until the next key, or that the uncommitted changes have no sha |
| `C-w` | delete the query's last word and the spaces after it, as in a shell (`C-u` can't clear the line: it scrolls the diff) |
| `Esc` | clear the query, or quit if it's already empty |
| `C-c` | quit |
| click | select a commit/file and focus its pane; focus the diff |
| wheel | over a list, move its selection; over the diff, scroll 3 lines (sideways: horizontal wheel or Shift+wheel, 8 columns). Focus stays put |
| drag | select and copy on release: the diff copies exactly the text dragged over (char-precise, `+`/`-` column included if covered); the lists copy whole rows as full values, since their rows are shortened: paths, and commits as `short subject` (`git log --oneline`). Past the diff's top or bottom edge the diff scrolls along |

Mouse capture is on for the whole session and off on every exit (panics too). Motion and
right/middle drags are dropped in the input thread, since capture reports every movement and
each would cost a redraw. The renderer records the three pane rects in `App::panes` for
hit-testing. A selection lives in content coordinates (`Spot`: list item, or diff line and
display column), so scrolling while it's shown keeps it on the same text. It needs the pointer
to move: a click selects nothing. It stays shown after release, with `copied N lines` in the
status, until the next key or click, and a newly selected commit or file clears it. Native
text selection still works with a modifier (Option / Fn / Shift, per terminal).

## Clipboard

Copying (`C-y`, and dragging with the mouse) goes two ways at once, since neither reports back:
OSC 52 written to the terminal, which works over SSH and in Ghostty, iTerm2, kitty, WezTerm,
Alacritty and Windows Terminal; and the platform's tool (`pbcopy`, or `wl-copy`/`xclip`/`xsel`),
for terminals without OSC 52 such as macOS Terminal.app. `App` only records what to copy; the
main loop, which owns the terminal, sends it.

## Crates

`ratatui` (bundled `crossterm`), `nucleo-matcher`, `anyhow`, and `syntect` for syntax colors
(parsing only: its bundled grammars and the pure-Rust regex engine, no themes, no C library).
Its grammar bundle is decoded with `bincode` 1.3.3, which RustSec flags as unmaintained but
its authors call complete; `deny.toml` records the exception.

## Diff pane

- **Message on top.** The diff request's `git show` also prints the header and message
  (`commit`, `Author:`, `Date:`, then `%w(0,4,4)%B`, like `git show`; the date honours
  `log.date`), then a NUL, then the patch. Same process, same cache entry, so no extra latency.
  The NUL goes after `%w(0,0,0)`: inside `%w` git drops it. A commit without files shows its
  message and "no file changes".
- **Syntax colors** (`src/syntax.rs`). syntect parses each hunk's old side (context + removed
  lines) and new side (context + added) separately, in order, as each version of the file
  reads; context lines take the new side's colors. Scopes map to the terminal's 16 colors
  (comments dark gray, strings green, keywords magenta, functions and tags blue, types and
  constants yellow), so they follow its theme. A hunk starting inside a block comment or
  multi-line string is miscolored until it ends: the price of not reading whole files. The
  language comes from the extension or file name; unknown files and plain text keep the plain
  green/red look. The diff worker sends each diff plain at once, then colored to replace it in
  place (same lines, so scroll and selection stay); coloring checks every 32 lines for a newer
  request and gives way, keeping up to 16 interrupted diffs to finish when idle. A 1 MiB budget
  per diff (about 30,000 lines, a second) and a 1,000-byte cap per line (minified code) bound
  the work. Grammars load in the background at startup.
- **Tints** (`src/tint.rs`). On a truecolor terminal (`COLORTERM=truecolor`/`24bit`) without
  `NO_COLOR`, levi asks for the background at startup, after raw mode and before the input
  thread (the answer comes back as input): OSC 11, then DA1, which every terminal answers, so
  a terminal that ignores OSC 11 costs no wait; 100 ms at most. Both go out on stderr, never
  stdout (`$(levi)`). The reply is read from `/dev/tty` with `select`: macOS's `poll` doesn't
  support terminals and reports them readable at once, which blocked the read until a key came
  and swallowed it. Added and removed rows get 13% green or red mixed into that background,
  across the whole row; changed words get 32% instead of reverse video. No answer, no tints:
  the look is as without them.
- **Changed words** (`src/words.rs`). In a block of removed lines followed by added ones, pairs
  are taken in order (1st `-` with 1st `+`, …) and compared by token (words, whitespace runs,
  single punctuation): shared ends are trimmed, the middle goes through an LCS. Changed tokens
  render in reverse video, as `diff-highlight` does; a `/c` match still wins. A pair whose kept
  (non-whitespace) bytes are under half the line is left unmarked, since lighting most of a line
  says nothing.
- **Sideways.** Lines aren't wrapped; the diff scrolls horizontally instead (`Paragraph::scroll`,
  so wide characters are cut correctly). It stops once the widest line's end is in view, resets
  with the vertical scroll when the selection moves, and the title shows `· col N` while shifted.
  A `/c` jump also shifts right when its match ends past the pane's edge.
- **Bounded.** The LCS is quadratic, so each diff gets a budget of 1M table cells. Past it, or
  for a pair that alone would exceed it, the whole middle between the shared ends is marked
  (what `diff-highlight` does), which is linear. Runs on the diff worker, once per cached diff.
  5,000 pairs of 200-word lines: 2 s unbounded, 39 ms now; 20k typical code pairs: 18 ms.

## Code layout

```
src/
  main.rs    – repo check, terminal setup/restore, panic hook, event loop
  git.rs     – log stream parser, diff fetch + line classification, types
  worker.rs  – input / log / diff threads, all reporting through one Msg channel
  filter.rs  – nucleo matching over commits + files, producing filtered views
  app.rs     – state: query, filtered views, selections, focus, scroll, diff LRU
  layout.rs  – where the panes go, and the seams a drag moves
  ui.rs      – rendering of the 4 widgets
  words.rs   – word-level marks for edited lines
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

- `levi [revision]`: one optional revision (`main..HEAD`, `v1.0..`, a branch), handed to
  `git rev-list --end-of-options <rev> --`, which feeds the parallel loader in place of `HEAD`.
  `--` keeps git from reading it as a path, so `levi src/` is a bad revision rather than a path
  filter (typing `src/` already filters by path). `-h/--help` and `-V/-v/--version` aside,
  arguments starting with `-` and any second argument exit 2.
- `/r` ignores the revision: the reflog is every place HEAD has been, not a commit range. The
  status line names the revision (`12 commits in main..HEAD`) only outside the reflog.
- Before the TUI starts, a preflight check fails with a one-line `levi: …` on stderr (exit 1):
  `git not found in PATH`, `needs git 2.31 or newer`, `not inside a git repository`, `no commits yet`,
  and with a revision `bad revision '<rev>'` or `no commits in '<rev>'` (an empty range).
  Then, since the UI draws on stderr, `needs a terminal` when stderr isn't one (scripts, `2>file`).
- `NO_COLOR` (non-empty, per no-color.org) drops every color after each frame is drawn; cells
  that had a background (selection, code matches) get reverse video so they still stand out.
- Bare repositories work (commands run from the bare dir instead of a work tree's top level).
- Exit codes: 0 with a picked sha on stdout, 130 when quit without picking (like fzf), so
  `sha=$(levi) && git show "$sha"` stops cleanly.
- `--filter <query>` (`-f`, `--filter=`) runs the same search headless, like `fzf --filter`:
  `App` is driven by the log loader and code searcher with no terminal and no input thread, a
  `/c` search starting as soon as the log is in (nobody is typing, so no debounce), until both
  are done. Then each hit prints as `<full sha> <subject>` (the reflog action under `/r`), best
  first. Exit 0 with matches, 1 without, 2 on errors, like grep, so agents and scripts can tell
  "nothing" from "broken". A closed pipe ends the output quietly.
- `--pick-context` (with or without a revision) changes only what Enter prints: one line,
  `commit <short> ("<subject or reflog action>")`, then `, <path>` when a file is selected,
  then ` lines a-b` (or ` line n`): the file lines of the code rows in view, or of the drag
  selection, on the new side (the old side for a deleted file), mapped from the hunk headers.
  Rows with no code (message, headers) add no lines. A key clears the selection after it's
  handled, so Enter still sees it.
- Integration tests in `tests/cli.rs` run the real binary for every pre-TUI path.

## Refresh

levi follows the repository while it's open: a commit, pull, amend or rebase in another
terminal shows up without a restart, and without a key or a setting.

- **Noticing.** A watcher thread asks git every 2 s what the shown ground resolves to:
  `git rev-parse <rev> --` (one line per tip, `^` for excluded ones; `HEAD` by default), or
  under `/r` the newest reflog entry (`git log -g -1 --date=raw`, whose selector carries a
  timestamp, so a checkout of the same commit still counts). A few ms of git each time; no
  file watcher, which would add a dependency and miss worktrees, packed refs and bare repos.
  It also checks at once when the terminal regains focus (focus reporting, ignored by
  terminals without it) and when `C-o`'s editor returns. `--filter` doesn't watch.
- **The baseline** is taken in the main thread right before each full load starts, so
  nothing committed between the two goes unseen. A commit landing after the baseline but
  before the load's `rev-list` is loaded twice at worst: prepends skip shas already shown.
- **Growing** (every old tip is an ancestor of its new one, excluded tips unchanged: commit,
  pull, fast-forward): only `<new tips> ^<old tips> <excluded>` loads, in one `git log`, and
  goes on top of the list. On top, not in date order: a pull that brings older-dated commits
  shows them first, as what just arrived.
- **Anything else** (amend, rebase, reset, another branch checked out, and every reflog
  change) reloads in full, the way switching to `/r` does. A change during any load also
  reloads in full, rather than stacking partial loads.
- **What stays:** the query, focus and diff cache (commits never change, so `(sha, path)`
  stays valid). The selected commit stays selected by sha: after a prepend at once, after a
  reload as soon as its batch arrives, with its file and diff scroll. If a rewrite dropped it,
  the selection rests on the top commit.
- **`/c`:** after a prepend, a finished search runs on the new commits only and adds their
  hits; an unfinished one starts over on its whole scope, keeping the hits it had. A reload
  reruns it.
- **The status line** says what happened until the next key or click: `2 new commits`,
  `history changed, reloaded`, `reflog updated`, `working tree changed`.

## Uncommitted changes

While the working tree differs from HEAD, the commits column starts with a row for it:
`working uncommitted changes`, all in red, as `git status` shows what isn't committed. Its
files keep their usual colors, plus `?` for untracked ones; red on them would read as
deletions. It answers what an agent changed but hasn't committed, with the same search, diff
and keys as any commit.

- **One row.** Staged and unstaged changes count as one, against HEAD: the question is what
  changed since the last commit, and two rows would bring in git's index. Untracked files
  that git doesn't ignore are in it, since agents create files all the time; at most 1,000,
  so a big directory nobody ignored can't flood the files column.
- **Where.** Only on ground that ends at HEAD: no revision, or one whose tip is HEAD's commit
  when levi starts (`main..HEAD`, `main..` while on a branch off main). Not in the reflog, not
  in a bare repository, not in `--filter`, whose lines are `<sha> <subject>` that `git show`
  can read. Gone when the tree is clean.
- **Never writes.** `git status` and `git diff` refresh the index, which writes to `.git`.
  The list comes from `git --no-optional-locks status --porcelain=v2 -z
  --untracked-files=all`, which then doesn't; diffs and `/c` use the plumbing `git
  diff-index`, which never refreshes (porcelain `git diff` does, `--no-optional-locks` or
  not). A test checks the index is byte for byte the same after all of them.
- **The status letter** is X and Y combined against HEAD: `A` if staged as new, `D` if gone
  from the index or the tree, `R`/`C` for a staged rename or copy (paired with `-M`), `T` for a
  type change, `U` for a conflict, `M` otherwise. A file added and then deleted again isn't
  listed: against HEAD, nothing changed.
- **The diff** starts with `Uncommitted changes on <branch>` and `On top of <short>
  <subject>` (Meta lines, so the colors are a commit header's), then `git diff-index --patch
  -M HEAD -- [old] <path>`, or `git diff --no-index /dev/null <path>` for an untracked file
  (which exits 1 when they differ, always here).
- **Following.** The watcher (see Refresh) also runs the status every 2 s, at once at
  startup, and when poked, and reports when a fingerprint changes: the status output plus
  each listed file's size and mtime, since editing an already modified file doesn't change
  the status. git.git's tree (4,857 files): 17 ms a look.
- **The row's sha** is `working:<n>`, `n` counting the reports. Not hex, so no sha prefix
  matches it. A new state is a new sha, so diffs and `/c` results of an older one have other
  keys: the diff cache needs no eviction, and a late result can't be taken for the current
  one. While the new diff loads, the last one stays shown, so the pane doesn't flicker as an
  agent writes; the selection, file and scroll stay too.
- **Selection.** The first report selects the row if the selection is still on top, so levi
  opens on what changed since the last commit; otherwise the selected commit stays. When the
  row goes (all committed), the top commit is selected. New commits a refresh finds go below
  the row.
- **`/c`** searches it on its own, beside the commits' search: the tree can change every few
  seconds, and restarting a 6 s search each time would never let it finish. `git diff-index
  -G` (same literal, smart-case pattern) finds tracked files; untracked ones are read and
  matched line by line, skipping binary files and files over 8 MiB. It searches when the
  commits' search starts and again whenever the tree changes, keeping the last result until
  the new one is in; a result for an older state is dropped.
- **Keys.** Enter and `C-y` say the row has no sha, rather than print or copy one that
  `git show` can't read. With `--pick-context`, Enter prints `the uncommitted changes,
  src/ui.rs lines 27-40`. `C-o` opens the working tree's file; one deleted in the tree opens
  read-only as HEAD has it.

## Commands

Slash commands live in the search bar (`src/query.rs`). They're whole words only, so `src/config`
and `/cache` stay fuzzy text; git paths never start with `/`.

- **`/r` — reflog.** A switch, anywhere in the query. The commits column reloads from
  `git log --walk-reflogs` (same record format: `%gd`/`%gs` are empty outside reflog mode), rows show
  the reflog action, and the fuzzy header haystack includes it. Entries can repeat a commit.
- **`/c` — code search.** Everything after it, spaces included, is the text the diffs must
  contain; what's before it stays fuzzy. `/r /c foo` searches the code of reflog commits.
  - Engine: `git log -G` (added/removed lines), fed explicit shas with `--stdin --no-walk`,
    `--name-only` for the matching paths. The text is regex-escaped (git speaks POSIX ERE) so it
    matches literally; `--regexp-ignore-case` unless the text has an uppercase letter.
  - Parallel: the loaded commits (deduplicated for the reflog) are cut into chunks of 500, newest
    first; one thread per core pulls chunks and runs its own git. Matches stream back as
    `CodeHits`, progress as chunks finish.
  - Starts 300 ms after typing pauses, and only once the log is fully loaded. A changed pattern
    or source bumps the code generation: running gits are killed at once and late results are
    dropped.
  - Scoped: it searches only the commits that pass the fuzzy part, and remembers that fuzzy
    query. Changing the fuzzy part keeps the results if the new query only narrows it
    (`filter::narrows`: an empty query, or appending without `!`, `$`, `\`); otherwise it
    searches again after the pause. Chunks are sized to give every core ~4 of them (max 500
    commits), since scoped sets are smaller and skew toward expensive merges.
  - A commit shows if it passes both filters; its files are the intersection (fuzzy file matches,
    or all files when the header matched, ∩ files whose diff matched).
  - The diff scrolls to the first added/removed line containing the text (3 lines of context)
    and highlights every occurrence.
  - git.git (82.5k commits, 12 cores): ~8.5 s for a full search vs ~55 s single-process; first
    hit for a common identifier in ~0.1 s; `parse /c …` (22.6k commits) ~3.2 s, `parse.c /c …`
    (2.7k commits) ~1.5 s. Cancelling kills all gits within 0.3 s.
  - File-scoped: a commit the fuzzy query matched through its files is searched with a pathspec
    of just those files (plus old names of renames, so git still pairs them — a pure rename
    must not match). Commits matched by header are diffed whole. `plan_chunks` keeps the two
    kinds in separate chunks; scoped chunks share the union of their paths, falling back to a
    whole diff past 512 paths. Same visible results, since levi shows only fuzzy-matched files
    anyway. git.git: `parse.c /c git_parse_ssize_t` 1.5 s → 0.16 s (big merges were the cost).
  - Measured and rejected: `-S` (2.3× faster, misses edited lines: 34 of 48 hits), scanning
    blobs ourselves via `cat-file --batch` (no faster, misses merges/moves), a bigger
    `core.deltaBaseCacheLimit` (~4%), commit-graph (no change), `--no-renames` (~3%), diff
    prefetching (a diff opens in ~3 ms already), merging the preflight git calls (~4 ms).

Each load has a generation number: switching source starts a new one, the old loader stops at its
next commit (killing its git), and late messages from old generations are dropped.

## Later (not now)

- `--all` branches.
- Optional `delta` piping / syntax highlighting.
- Sending what's on screen to a coding agent. Done for herdr users: `levi --pick-context` plus
  the herdr plugin in `herdr/`, which types the pick into the agent's input (herdr can send
  text to an agent pane, which no agent offers on its own). Kept for the record, the shapes
  considered before:
  - **`levi --focus` (agents pull).** Superseded by the herdr plugin. A running levi keeps a small note of what it shows: commit,
    file, the lines in view and any drag selection, rewritten only when they change (e.g. under
    `.git/`) and removed on exit. `levi --focus` prints it, so "look at what I have open in levi"
    works with any agent that runs commands, in the session you already have. Same model as
    `--filter`; the CLAUDE.md snippet grows by a line. With two levis on one repo, the last one
    touched wins.
  - **Copy a reference.** One key copies `<sha> <path>:<lines> ("<subject>")` instead of raw text,
    for agents that can't run commands; they `git show` the rest. Small, but still a paste.
  - **Launch the agent** like `C-o` launches the editor, with a prompt naming the commit, file and
    lines. Starts a new session each time (losing the running one), needs a question typed into
    levi and agent-specific command lines: the least levi-like. Pushing a message into an
    already-running agent isn't possible: Claude Code has no public way for another program to
    do it.
