# levi

[![CI](https://github.com/rsdlt/levi/actions/workflows/ci.yml/badge.svg)](https://github.com/rsdlt/levi/actions/workflows/ci.yml)

> *Tch. Your history is filthy. Let's look at it properly.*

levi browses the commits on your branch, the files each one touched, and their diffs. Type to
search commits and files fzf style, and add `/c` to search the code itself: levi finds the
commit that added or removed any line you name.

![levi browsing its own history: a fuzzy search, a diff in syntax colors with tinted changes, Ctrl-Y copying a sha, /c finding the commit that added a line, Ctrl-O opening it in Neovim, Enter printing the sha, and levi --filter running the same search without the TUI](docs/demo.gif)

## Why levi

You and your agents made forty commits since lunch, and now the parser is broken. Finding the
commit that did it takes git a `git log -p` scrolled by hand, or a `git log -G` that leaves you
waiting.

levi does that one job: find the change and show it. Named after humanity's strongest soldier,
he's small, swift and precise. Captain Levi doesn't carry gear he won't use.

*Why not tig or lazygit?* Those are full squads. Levi is one soldier, two blades and no small talk.

## Speed

levi loads history on every core at once and searches it in memory. On
[git.git](https://github.com/git/git), with 82,513 commits, on an Apple M4 Pro (12 cores),
against the git command that does the same job:

| Job | levi | git |
|---|---|---|
| Load all history, with the files each commit touched | **0.86 s** | 2.8 s, `git log --name-status` |
| Search every diff for a line of code | **6.6 s** | 19.1 s, `git log -G` |

levi searched more, too: `git log -G` skips merge commits, and levi checks those against their
first parent. You never wait for the full load: the first screen lands in 50 ms, and history
streams in behind it while you type. Each keystroke refilters all 82k commits in 5 to 12 ms.

Rerun it on any big repository: time `levi --filter ''` against `git log --name-status`, and
`levi --filter '/c some_function'` against `git log -i -G some_function`.

## Enlist

You need git 2.31 or newer.

```sh
brew install rsdlt/tap/levi
```

With Rust, run `cargo install levi-git`, or `cargo binstall levi-git` for a prebuilt binary.
Someone else took the name `levi` on crates.io; the command is still `levi`. The
[latest release](https://github.com/rsdlt/levi/releases/latest) has binaries for macOS, Linux
and Windows.

## Orders

Run `levi` inside a git repository. He patrols the current branch. Name a revision and he
covers that ground instead:

```sh
levi main..HEAD    # what your branch adds to main
levi v1.0..        # everything since a tag
```

`Enter` prints the selected commit's sha and quits, so levi works as a picker:
`git show $(levi)`.

| Key | Action |
|---|---|
| type | search |
| `↑` `↓` | move |
| `Tab` | switch pane: commits, files, diff |
| `PgUp` `PgDn` | scroll the diff |
| `S-↑` `S-↓` | jump between hunks, across files |
| `S-←` `S-→` | scroll the diff sideways |
| `Enter` | print the sha and quit |
| `C-y` | copy the sha |
| `C-o` | open the file in your editor at the line you're reading |
| `Esc` | clear the search, or quit |

The mouse works too: click to select, scroll with the wheel, drag to copy. `levi --help` lists
every key. Set `NO_COLOR` for plain black and white.

## Hunting

The search narrows commits and files together. It matches subjects, authors, sha prefixes and
the paths each commit touched, so `parse.rs` lists the commits that changed parse.rs. fzf
syntax works: `'exact`, `^prefix`, `suffix$`, `!not`.

Two commands share the search bar:

- `/c text` searches the code: the lines each commit added or removed. levi keeps the commits
  where `text` appears as you typed it, spaces included, and jumps to the first match. Lowercase
  matches any case; a capital makes it exact. Fuzzy terms before `/c` pick which commits and
  files levi searches, which also makes it faster.
- `/r` switches to the reflog, with commits from branches you deleted.

```
/c git_parse_ssize_t    which commits touched this function?
parse.rs /c let x =     the code `let x =`, in commits that touched parse.rs
/r /c experiment        that code, from a branch you deleted
```

## Reconnaissance: levi and coding agents

> *Brats. They change forty files before lunch and can't tell you which commit broke the build.*

Agents like Claude Code can't work a TUI, so levi reports to them in plain text.
`levi --filter` runs the same search without the screen and prints one `<sha> <subject>` line
per match, best first. Ask your agent which commit made the layout stack on narrow terminals,
and it runs:

```sh
$ levi --filter 'ui.rs /c STACK_BELOW'
d3a93ab5f5c98621cbaf9079d0c952786239d221 Stack the lists above the diff on narrow terminals
```

Then it reads that commit with `git show`. levi exits 1 when nothing matches, so the agent can
tell an empty search from a broken one. Add a revision to search only its ground:
`levi --filter 'login' main..HEAD`.

Brief your agent once, in `CLAUDE.md` or `AGENTS.md`:

```markdown
To find the commits that touched some code, run `levi --filter '<query>'`. The query is fuzzy
over commit subject, author, sha and file paths; `/c <text>` keeps only commits whose diffs add
or remove `<text>` (literal, smart case), and `/r` searches the reflog. It prints
`<sha> <subject>`, best match first. Then read one with `git show <sha>`.
```

Orders go the other way too. Found the culprit in levi? `C-y` copies its sha and a drag over
the diff copies the code, ready to paste into the agent's chat with whatever you want done
about it.

## Joint operations

`C-o` opens the file in your editor at the line you're reading. Quit the editor and levi waits
where you left him. Inside Neovim's terminal, the file opens in a new tab of that Neovim. A
file you deleted since then opens read-only, as it was in that commit.

To run levi in a floating window inside Neovim, see [docs/neovim.md](docs/neovim.md).

## Field report

- **Precise.** levi lights the words that changed in each line, like git's `diff-highlight`.
  Code takes syntax colors from your terminal's palette. On terminals like Ghostty, changed
  lines also get a soft tint that suits your theme, dark or light.
- **Small.** About 3,500 lines of Rust and a 4 MB binary.
- **Clean.** levi runs your own `git`, never writes to your repository, and leaves your
  terminal as he found it, even after a crash.

## License

[MIT](LICENSE). Free to use, like a cup of black tea, held however you like.
