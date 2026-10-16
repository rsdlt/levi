# Changelog

All notable changes to levi are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and levi uses
[Semantic Versioning](https://semver.org/). Its public surface is the keys, the flags,
the search commands, the sha printed on stdout and the exit codes.

## [Unreleased]

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

[Unreleased]: https://github.com/rsdlt/levi/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/rsdlt/levi/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/rsdlt/levi/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/rsdlt/levi/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/rsdlt/levi/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/rsdlt/levi/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rsdlt/levi/releases/tag/v0.1.0
