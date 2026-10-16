# Changelog

All notable changes to levi are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and levi uses
[Semantic Versioning](https://semver.org/). Its public surface is the keys, the flags,
the sha printed on stdout and the exit codes.

## [Unreleased]

### Added

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

[Unreleased]: https://github.com/rsdlt/levi/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/rsdlt/levi/releases/tag/v0.1.0
