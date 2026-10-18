# The terminal part of the README's demo. docs/demo.sh records it, puts the
# title card in front and writes docs/demo.gif; run that instead.
#
# It browses levi's own history, so it needs a full clone. The last part runs
# the herdr plugin in a herdr session of its own, levi-demo, so herdr must be
# installed with the plugin and its key (prefix+l) in herdr's config.

Output target/demo-terminal.gif

Set Shell "bash"
Set FontSize 15
Set Width 1400
Set Height 780
Set Padding 16
Set Theme "TokyoNight"
Set WindowBar Colorful
Set TypingSpeed 70ms
Set CursorBlink false

# A clean prompt, this build of levi first on PATH, and no pager for git show.
# It records a throwaway clone whose author emails are replaced, so no real
# address ends up in the GIF.
Hide
# truecolor, so levi asks for the background and tints changed lines.
Type `export REPO="$PWD" PS1='$ ' PATH="$PWD/target/release:$PATH" COLORTERM=truecolor FILTER_BRANCH_SQUELCH_WARNING=1` Enter
# A herdr of its own later, not the one this terminal may be running in.
Type `unset HERDR_ENV HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID HERDR_SOCKET_PATH HERDR_BIN_PATH HERDR_SESSION` Enter
# Ctrl-O's editor: a plain Neovim with line numbers, nobody's own config.
Type `export VISUAL= EDITOR="nvim --clean -c 'set number'" && unset NVIM` Enter
Type `cd "$(mktemp -d)" && git clone -q "$OLDPWD" levi && cd levi` Enter
Type `git filter-branch -f --env-filter 'export GIT_AUTHOR_EMAIL=levi@survey.corps GIT_COMMITTER_EMAIL=levi@survey.corps' HEAD >/dev/null 2>&1; git remote remove origin` Enter
Sleep 8s
Type "clear" Enter
Show

Type "levi"
Sleep 600ms
Enter
Sleep 2s

# Fuzzy search narrows commits and files together.
Type "stack"
Sleep 1.5s
Tab
Sleep 600ms
Down
Sleep 300ms
Down
Sleep 300ms
Down
Sleep 2s

# Ctrl-Y copies the commit's sha and levi stays open; the status line says so.
Ctrl+Y
Sleep 2s

# /c finds the commit whose diff added or removed the text, and jumps to it.
Escape
Sleep 500ms
# Typed in two halves so this file isn't a match itself.
Type "/c fn tok"
Type "ens"
Sleep 3s

# Ctrl-O opens the file in your editor at the line you're reading; quit it to come back.
Ctrl+O
Sleep 2.5s
Type ":q"
Sleep 300ms
Enter
Sleep 1.2s

# Pick it: levi prints its sha and exits, so `git show $(levi)` works.
Enter
Sleep 2s

# --filter runs the same search without the TUI, for scripts and coding agents.
Type "levi --filter '/c fn tok"
Type "ens'"
Sleep 500ms
Enter
Sleep 3s

# In herdr, levi hands what you pick to your agent. A stand-in agent here: an
# input line that never answers (docs/demo-agent.sh), registered as an agent so
# the plugin types into it the way it types into a real one.
Hide
Type "clear" Enter
Type "herdr --session levi-demo" Enter
Sleep 4s
Type `herdr pane report-agent "$HERDR_PANE_ID" --source levi-demo --agent agent --state idle >/dev/null; clear; sh "$REPO/docs/demo-agent.sh"` Enter
Sleep 1.5s
Show
Sleep 1.5s

# prefix+l opens levi over the agent, in its repository.
Ctrl+Space
Type "l"
Sleep 2.5s
Type "stack"
Sleep 1.2s
Tab
Down
Sleep 300ms
Down
Sleep 300ms
Down
Sleep 800ms
PageDown
Sleep 2s

# Enter: the reference goes into the agent's input, unsent, for the question.
Enter
Sleep 2.5s
Type "why 120 and not 100?"
Sleep 800ms
Enter
Sleep 2.5s

# Leave the demo session and remove it.
Hide
Ctrl+Space
Type "d"
Sleep 1s
Type "herdr session stop levi-demo; herdr session delete levi-demo" Enter
Sleep 2s
