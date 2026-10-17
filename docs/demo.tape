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
Hide
Type `export PS1='$ ' PATH="$PWD/target/release:$PATH" GIT_PAGER=cat` Enter
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
Type "/c BUDGET"
Sleep 3.5s

# Pick it: levi exits and git show takes the sha.
Enter
Sleep 4s
