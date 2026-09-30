#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts
export LIBGL_ALWAYS_SOFTWARE=1
export WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
    if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver"; export VK_ICD_FILENAMES="$driver"; break; fi
done
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/pi-desktop-runtime-$UID}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY
binary="${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target}/debug/pi-desktop}"
"$binary" --demo >artifacts/xvfb-app.log 2>&1 &
app=$!
trap 'kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true' EXIT
window=""
for _ in $(seq 1 100); do
    if ! kill -0 "$app" 2>/dev/null; then
        printf 'App exited before rendering; see artifacts/xvfb-app.log\n' >&2
        exit 1
    fi
    window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
    if [[ -n "$window" ]]; then break; fi
    sleep 0.1
done
[[ -n "$window" ]] || { printf 'No application window appeared\n' >&2; exit 1; }
xdotool windowfocus --sync "$window"
sleep 2
import -window "$window" artifacts/thread-evening.png
xdotool mousemove --window "$window" 1276 26 click 1
sleep 1
import -window "$window" artifacts/thread-moonstone.png
# Tool details are intentionally collapsed by default. Capture each expanded view too.
for tool in 'edit-details 300' 'bash-details 324'; do
    read -r name y <<< "$tool"
    xdotool mousemove --window "$window" 500 "$y" click 1
    sleep 1
    import -window "$window" "artifacts/thread-$name.png"
    xdotool mousemove --window "$window" 500 "$y" click 1
    sleep 0.2
done
for picker in 'model-picker 370' 'thinking-picker 490' 'commands-picker 276'; do
    read -r name x <<< "$picker"
    xdotool mousemove --window "$window" "$x" 678 click 1
    sleep 0.5
    import -window "$window" "artifacts/thread-$name.png"
    xdotool key Escape
    sleep 0.2
done
xdotool key ctrl+a
xdotool type --clearmodifiers 'Xvfb follow-up validation'
xdotool key alt+Return
sleep 1
import -window "$window" artifacts/thread-queued.png
xdotool key Escape
sleep 1
import -window "$window" artifacts/thread-stopped.png
xdotool windowsize "$window" 1000 680
sleep 1
import -window "$window" artifacts/thread-compact.png
xdotool key ctrl+n
sleep 1
import -window "$window" artifacts/thread-new.png
xdotool key ctrl+q
for _ in $(seq 1 50); do
    if ! kill -0 "$app" 2>/dev/null; then
        wait "$app"
        printf 'Captured eleven native GPUI screens, including pickers and expanded tools; app exited cleanly.\n'
        exit 0
    fi
    sleep 0.1
done
printf 'Application did not quit\n' >&2
exit 1
