# The README's demo. Regenerate it from the repository root with:
#
#   cargo build --release && vhs docs/demo.tape
#
# It browses levi's own history, so it needs a full clone.

Output docs/demo.gif

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
Type `export PS1='$ ' PATH="$PWD/target/release:$PATH" GIT_PAGER=cat FILTER_BRANCH_SQUELCH_WARNING=1` Enter
Type `cd "$(mktemp -d)" && git clone -q "$OLDPWD" levi && cd levi` Enter
Type `git filter-branch -f --env-filter 'export GIT_AUTHOR_EMAIL=levi@survey.corps GIT_COMMITTER_EMAIL=levi@survey.corps' HEAD >/dev/null 2>&1` Enter
Sleep 8s
Type "clear" Enter
Show

# levi is a picker: Enter prints the commit's sha for the command around it.
Type "git show --stat $(levi)"
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
Sleep 2.5s

# /c finds the commit whose diff added or removed the text, and jumps to it.
Escape
Sleep 500ms
# Typed in two halves so this file isn't a match itself.
Type "/c fn tok"
Type "ens"
Sleep 3.5s

# Pick it: levi exits and git show takes the sha.
Enter
Sleep 4s
