#!/bin/sh
# A stand-in for a coding agent in the demo GIF: an input line and nothing
# else. It never answers, so the GIF doesn't pass off a made-up reply as an
# agent's. docs/demo.tape runs it in a herdr pane registered as an agent, so
# the levi plugin types into it the way it types into a real one.
printf '\033[2J\033[H'
printf '\n  \033[1;36m●\033[0m \033[1magent\033[0m  \033[2mwaiting for your question\033[0m\n\n'
printf '  \033[1;36m>\033[0m '
IFS= read -r question
printf '\n  \033[2msent.\033[0m\n'
sleep 30
