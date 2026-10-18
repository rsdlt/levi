# Changelog

All notable changes to levi are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and levi uses
[Semantic Versioning](https://semver.org/). Its public surface is the keys, the flags,
the search commands, the sha printed on stdout and the exit codes.

## [Unreleased]

## [0.14.1] - 2026-10-17

### Fixed

- `Ctrl-O` inside `$(levi)`, as in `git show $(levi)`: the editor drew into the pipe meant for the
  sha instead of on the screen, and the redraw afterwards put a terminal query in front of the sha.
  The editor now uses the terminal levi draws on, and the sha comes out clean.

## [0.14.0] - 2026-10-17

### Added

- `levi --filter '<query>' [revision]` runs the search without the TUI, like `fzf --filter`, and
  prints `<sha> <subject>` per match, best first. For scripts and coding agents: the full query
  syntax, `/c` and `/r` included. It exits 1 when nothing matches.
- `Ctrl-O` opens the selected file in your editor at the line you're reading, and levi carries on
  when you quit. Inside Neovim's terminal it opens in a new tab of that Neovim instead of nesting
  one. `$VISUAL`, then `$EDITOR`, then `vi`; Helix, Zed, Sublime and VS Code get the line their way.
  A file that's no longer in the working tree opens read-only as it was in that commit, or just
  before it for a file the commit deletes.
- A README recipe for Neovim: levi in a floating window, the picked commit shown in a new tab.

### Changed

- `Shift-↑/↓` hunk jumps carry on across files: past a file's last hunk, the next file opens at
  its first, and back the same way, so a whole commit reads with one key.

## [0.13.0] - 2026-10-17

### Added

- `Ctrl-Y` copies the selected commit's full sha to the clipboard and keeps levi open. It goes
  through the terminal (OSC 52, which works over SSH) and through `pbcopy` or
  `wl-copy`/`xclip`/`xsel`.
- Drag with the mouse to copy. In the diff, exactly the text you drag over; past the top or
  bottom edge it scrolls along. In the lists, whole rows as full values: complete paths, and
  commits as `sha subject` lines like `git log --oneline`.

### Changed

- Long paths in the files column keep their end in view: leading directories give way first, as in
  `…/components/frontend/Button.tsx`, so the file name always shows. Renames are written like
  `git diff --stat`, with the shared directories once: `src/{old.rs → new.rs}`.

## [0.12.0] - 2026-10-17

### Added

- `levi -v` prints the version, like `-V` and `--version`.
- Without a terminal, as in a script, levi says it needs one instead of failing with an OS error.
- `Ctrl-W` deletes the last word of the query, as in a shell.
- `NO_COLOR` turns colors off. The selection and code matches show in reverse video instead.
- Prebuilt binaries on every GitHub release: macOS (Apple Silicon and Intel), static Linux
  (x86_64 and ARM64) and Windows. `cargo binstall levi-git` downloads them too.

## [0.11.2] - 2026-10-17

### Changed

- The search bar uses the terminal's own cursor instead of drawing one, so it takes your cursor
  style and input methods know where typing goes.

## [0.11.1] - 2026-10-17

### Changed

- Published on crates.io as `levi-git`, since `levi` was taken: `cargo install levi-git`. The
  command is still `levi`.

## [0.11.0] - 2026-10-17

### Added

- Shift-↑/↓ jumps the diff to the previous or next hunk, from any pane. With `/c` on, it steps
  through the matching lines instead, scrolling sideways when a match is past the edge.

## [0.10.0] - 2026-10-17

### Added

- Narrow terminals (under 120 columns, like a split pane) stack commits and files above a
  full-width diff instead of squeezing three columns side by side. It switches by itself as the
  terminal resizes.

## [0.9.0] - 2026-10-16

### Added

- An optional revision argument: `levi main..HEAD` shows only what your branch adds, `levi v1.0..`
  everything since a tag, `levi other-branch` another branch. The status line names it, and a bad
  revision or an empty range fails before the TUI opens. `/r` still shows the whole reflog.

## [0.8.0] - 2026-10-16

### Added

- A row of key hints at the bottom, including the `/c` and `/r` commands. On narrow terminals
  the least used hints are left out.

## [0.7.0] - 2026-10-16

### Added

- Lines wider than the diff pane scroll sideways with `Shift-←`/`Shift-→`, horizontal mouse
  scrolling or `Shift`+wheel. The pane title shows the column while shifted, and `/c` scrolls
  to a match past the right edge.

## [0.6.0] - 2026-10-16

### Added

- Mouse support: click a commit or file to select it, and use the wheel to move through a
  list or scroll the diff. To select text, hold your terminal's modifier while dragging
  (`Option` in iTerm2, `Fn` in Terminal.app, `Shift` in most others).

### Fixed

- levi no longer crashes when the terminal is too small to show any list rows.

## [0.5.0] - 2026-10-16

### Added

- The diff pane opens with the commit's header and full message, laid out like `git show`.
  Commits without file changes show their message too.
- In edited lines, only the words that changed are highlighted (reverse video), like git's
  `diff-highlight`.

## [0.4.0] - 2026-10-16

### Changed

- When the rest of the query matched a commit through its file names, `/c` only diffs those
  files. Big merges no longer cost a full diff: `parse.c /c git_parse_ssize_t` on git.git takes
  about 0.16 seconds instead of 1.5, with the same results.

## [0.3.0] - 2026-10-16

### Changed

- History loads on every core at once. On git.git (82k commits) the whole log, file lists
  included, is ready in under a second instead of 5–9 seconds, so file search covers all of
  history sooner and `/c` starts sooner.
- `/c` only searches the commits that match the rest of the query, so `parse.c /c foo` on
  git.git takes about 1.5 seconds instead of 9. Narrowing the query keeps the results; widening
  it searches again once typing pauses.

## [0.2.0] - 2026-10-16

### Added

- `/r` in the search bar switches to the reflog: every place HEAD has been, including commits
  on deleted branches. The rest of the query filters it, by reflog action too.
- `/c text` searches the code: it keeps commits whose diffs add or remove a line containing
  `text`, and only the files where that happens. The match is literal with smart case; the diff
  jumps to it and highlights it. The search starts when typing pauses, runs on every core,
  streams results with progress, and is cancelled as soon as the query changes.
- GitHub Actions CI: tests on Linux and macOS, rustfmt, clippy, the minimum Rust version
  (1.88), and `cargo deny`.

## [0.1.0] - 2026-10-15

First release.

### Added

- Three panes: the current branch's commits, the files each one touched, and the selected
  file's diff. Merge commits are diffed against their first parent.
- fzf-style search over commit subject, author, touched files (including a rename's old path)
  and sha prefix. Both the commits and files columns narrow as you type, with matches
  highlighted. Supports `'exact`, `^prefix`, `suffix$`, `!not` and smart case.
- `Enter` prints the selected sha to stdout. Quitting without picking exits with 130.
- The log streams in the background and diffs load off the UI thread, with a small cache.
  On git.git (82k commits) the first screen appears in under 0.1 s.
- One-line errors outside a repository, in a repository with no commits, without `git`, or
  with git older than 2.31. Bare repositories are supported.
- `--help` and `--version`.
- `cargo deny` configuration for license, advisory and source checks.

[Unreleased]: https://github.com/rsdlt/levi/compare/v0.14.1...HEAD
[0.14.1]: https://github.com/rsdlt/levi/compare/v0.14.0...v0.14.1
[0.14.0]: https://github.com/rsdlt/levi/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/rsdlt/levi/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/rsdlt/levi/compare/v0.11.2...v0.12.0
[0.11.2]: https://github.com/rsdlt/levi/compare/v0.11.1...v0.11.2
[0.11.1]: https://github.com/rsdlt/levi/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/rsdlt/levi/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/rsdlt/levi/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/rsdlt/levi/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/rsdlt/levi/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/rsdlt/levi/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/rsdlt/levi/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/rsdlt/levi/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/rsdlt/levi/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/rsdlt/levi/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/rsdlt/levi/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rsdlt/levi/releases/tag/v0.1.0
