#!/usr/bin/env bash
# Native offline views only. Never sends a prompt or calls a model.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
export XDG_RUNTIME_DIR="$(mktemp -d /tmp/pi-views-runtime.XXXXXX)"
unset WAYLAND_DISPLAY
prefix="${PI_CAPTURE_PREFIX:-views}"
"${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target/linux}/debug/pi-desktop}" --demo --light >"artifacts/$prefix-app.log" 2>&1 &
app=$!
trap 'kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; rm -rf "$XDG_RUNTIME_DIR"' EXIT
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || { printf 'App exited; inspect %s\n' "artifacts/$prefix-app.log"; exit 1; }
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep 0.1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
sleep 2
xdotool key Escape
sleep 0.5
import -window "$window" "artifacts/$prefix-thread.png"
xdotool mousemove --window "$window" 420 72 click 1
sleep 1
import -window "$window" "artifacts/$prefix-tree.png"
xdotool key Up
sleep 0.2
xdotool key Down
sleep 0.2
import -window "$window" "artifacts/$prefix-tree-keyboard.png"
xdotool mousemove --window "$window" 334 115 click 1
sleep 0.4
import -window "$window" "artifacts/$prefix-tree-no-tools.png"
xdotool mousemove --window "$window" 265 115 click 1
sleep 0.2
xdotool mousemove --window "$window" 480 72 click 1
sleep 1
import -window "$window" "artifacts/$prefix-context.png"
xdotool mousemove --window "$window" 335 72 click 1
sleep 1
import -window "$window" "artifacts/$prefix-changes.png"
if [[ "${PI_CAPTURE_FILES:-0}" == 1 ]]; then
  xdotool mousemove --window "$window" 1130 384 click 1
  sleep 3
  kill -0 "$app" 2>/dev/null || { echo 'File preview crashed; inspect app log' >&2; exit 1; }
  import -window "$window" "artifacts/$prefix-editor.png"
fi
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; echo 'Captured native offline views; clean exit.'; exit 0; fi
  sleep 0.1
done
echo 'App did not quit' >&2
exit 1
