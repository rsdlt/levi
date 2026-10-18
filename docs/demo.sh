#!/bin/sh
# Makes docs/demo.gif, the README's demo: a title card made from docs/logo.svg,
# then levi browsing its own history (docs/demo.tape). Run it from the
# repository root. Needs vhs, rsvg-convert and ffmpeg.
set -eu

cargo build --release
vhs docs/demo.tape
rsvg-convert docs/demo-card.svg -o target/demo-card.png

# Two seconds of the card fading into the recording, then one palette for the
# whole GIF so both parts keep their colors.
ffmpeg -v error -y \
    -loop 1 -framerate 25 -t 2.4 -i target/demo-card.png \
    -i target/demo-terminal.gif \
    -filter_complex "\
[0:v]fps=25,format=rgb24,setsar=1[card];\
[1:v]fps=25,format=rgb24,setsar=1[term];\
[card][term]xfade=transition=fade:duration=0.4:offset=2,split[a][b];\
[a]palettegen=stats_mode=diff[p];\
[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
    -loop 0 docs/demo.gif

echo "docs/demo.gif: $(du -h docs/demo.gif | cut -f1)"
