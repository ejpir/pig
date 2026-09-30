#!/usr/bin/env bash
# Offline native layout regression. No prompts/model calls or project mutations.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
export XDG_RUNTIME_DIR="$(mktemp -d /tmp/pi-layout-runtime.XXXXXX)"
unset WAYLAND_DISPLAY
export PI_DESKTOP_DEMO_WORKSPACE=1
"${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}" --demo --light >artifacts/layout-app.log 2>&1 &
app=$!
trap 'kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; rm -rf "$XDG_RUNTIME_DIR"' EXIT
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || { echo 'App exited'; exit 1; }
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep 0.1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
sleep 2
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep 0.2; xdotool click 1; sleep 0.5; }
shot() { import -window "$window" "artifacts/layout-$1.png"; }
# Clear the original thread fixture's queued sample, never send it.
xdotool key Escape
sleep 0.5
shot thread
click 334 72
click 325 224
xdotool mousemove --window "$window" 900 580
sleep 0.3
shot changes
click 1130 432
/usr/bin/xclip -selection clipboard -o > artifacts/layout-diff-copy.txt
grep -q '^--- a/packages/ai/src/providers/openai-completions.ts' artifacts/layout-diff-copy.txt
grep -q '^@@ -205' artifacts/layout-diff-copy.txt
if grep -Eq '^[ +-] +205 +205' artifacts/layout-diff-copy.txt; then
  echo 'Copy diff included display-only line-number columns' >&2
  exit 1
fi
click 250 72
xdotool key ctrl+n
sleep 0.8
shot landing
xdotool mousemove --window "$window" 120 140
sleep 0.4
shot close-hover
xdotool mousemove --window "$window" 120 262
sleep 0.4
shot close-time-hover
click 184 140
shot after-close
xdotool windowsize "$window" 1000 680
sleep 0.5
xdotool key ctrl+n
sleep 0.5
shot narrow-landing
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; echo 'Captured native workspace, changes, landing and close hover; clean exit.'; exit 0; fi
  sleep 0.1
done
echo 'App did not quit' >&2; exit 1
