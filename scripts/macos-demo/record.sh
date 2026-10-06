#!/bin/zsh
# Records a tour of the offline demo (--demo: sample sessions, never a provider)
# and writes docs/pi-desktop-showcase.gif and artifacts/macos-demo/pi-desktop-demo.mp4.
#
# Needs a release build (see the README), ffmpeg, and for the terminal running
# this: Screen Recording and Accessibility in System Settings → Privacy & Security.
# Don't touch the mouse or keyboard while it runs (under two minutes).
set -euo pipefail
ROOT=${0:A:h:h:h}
OUT=$ROOT/artifacts/macos-demo
WORK=$(mktemp -d -t pi-desktop-demo)
trap 'pkill -f "pi-desktop --demo" || true; rm -rf $WORK' EXIT
mkdir -p $OUT $WORK/demo/{desktop,agent} $WORK/zsh
swiftc -O ${0:A:h}/act.swift -o $WORK/act
A=$WORK/act

# The window's top-left corner and size, in screen points. Every point below
# is relative to the corner.
X0=100 Y0=80 W=1440 H=860
click() { $A click $((X0 + $1)) $((Y0 + $2)); }
move() { $A move $((X0 + $1)) $((Y0 + $2)); }
scroll() { $A scroll $((X0 + $1)) $((Y0 + $2)) $3; }
text() { $A type "$1"; }
key() { $A key $1; }
# A shortcut: macOS key code and System Events modifiers.
shortcut() { osascript -e "tell application \"System Events\" to key code $1 using {$2}"; }
cmd() { shortcut $1 "command down"; }
search() { cmd 40; sleep 0.5; text "$1"; sleep 0.6; key return; }
settings_menu() { click 88 810; sleep 0.8; }

# Relative settings folders keep the paths settings pages show short, and the
# terminal gets a prompt without the user or host name.
print -r -- "PROMPT='%F{blue}%1~%f %# '" > $WORK/zsh/.zshrc
cd $WORK
pkill -f "pi-desktop --demo" && sleep 0.5 || true
PI_DESKTOP_CONFIG_DIR=demo/desktop PI_CODING_AGENT_DIR=demo/agent ZDOTDIR=$WORK/zsh \
  $ROOT/target/release/pi-desktop --demo > $WORK/app.log 2>&1 &
sleep 3
app="first process whose unix id is $(pgrep -n -f 'pi-desktop --demo')"
osascript -e "tell application \"System Events\" to tell ($app) to set frontmost to true" \
  -e "tell application \"System Events\" to tell ($app) to set position of window 1 to {$X0, $Y0}" \
  -e "tell application \"System Events\" to tell ($app) to set size of window 1 to {$W, $H}" > /dev/null
move 1200 620
# Interrupting screencapture loses the last seconds, so it records for a fixed
# time, longer than the tour, and the end is trimmed.
screencapture -v -C -k -V 100 -R$X0,$Y0,$W,$H $WORK/raw.mov &
recorder=$!
started=$SECONDS
sleep 2

# A working session, then what it changed.
scroll 720 370 -300; sleep 1.2
click 347 55; sleep 2.2
# The session's branches, and where its context went.
search tree; sleep 1.2
move 600 361; sleep 1.4
search context; sleep 2.4
click 260 55; sleep 1
# The composer: commands, mentions, a follow-up, the model.
click 720 726; cmd 0; sleep 0.2
text /; sleep 1.6
key escape; cmd 0; text @openai; sleep 1.6
key escape; cmd 0; key delete
text "Also add a regression test for the OpenCode path"; sleep 0.6
click 385 782; sleep 1.6
key escape; sleep 0.6
# Another session: a Markdown review.
click 89 200; sleep 1.8
scroll 720 370 -500; sleep 1.4
# A terminal beside the session.
cmd 38; sleep 1.4
text 'echo "Hello from Pi Desktop" && uname -sm'; key return; sleep 1.6
cmd 38; sleep 0.8
# A session on another machine, with the experimental durable engine.
click 1381 17; sleep 1.2
click 571 263; sleep 1
click 720 373; text dev; sleep 0.3
click 720 433; text '~/repos/pi'; sleep 0.4
click 634 470; sleep 1.8
click 802 631; sleep 0.8
# All sessions, models, resources and settings.
search "all ses"; sleep 2
settings_menu; click 73 674; sleep 2.4
settings_menu; click 82 711; sleep 1.2
click 259 62; sleep 2.2
settings_menu; click 75 750; sleep 1.6
click 270 503; sleep 1.2
# The dark theme.
click 915 250; sleep 1.8
# Back to the first session, and its files unfolding.
search qwen; sleep 1.6
click 1371 55; sleep 1.2
click 1200 170; sleep 0.8
click 1200 170; sleep 1.2
click 1282 274; sleep 3

length=$((SECONDS - started))
wait $recorder
ffmpeg -v error -y -ss 1 -t $((length - 1)) -i $WORK/raw.mov -vf "setpts=PTS/1.3,fps=60,scale=1440:-2" \
  -c:v libx264 -crf 24 -preset slow -pix_fmt yuv420p -movflags +faststart -an $OUT/pi-desktop-demo.mp4
ffmpeg -v error -y -ss 1 -t $((length - 1)) -i $WORK/raw.mov -vf "setpts=PTS/1.3,fps=12,scale=1344:-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
  $ROOT/docs/pi-desktop-showcase.gif
print "Wrote $OUT/pi-desktop-demo.mp4 and docs/pi-desktop-showcase.gif"
