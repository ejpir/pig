#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts
binary="$(realpath "${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target}/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-first-screen.XXXXXX)"; app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" XDG_CACHE_HOME="$tmp/cache"
export XDG_RUNTIME_DIR="$tmp/runtime" PI_DESKTOP_CONFIG_DIR="$tmp/desktop" PI_CODING_AGENT_DIR="$tmp/agent"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$PI_DESKTOP_CONFIG_DIR" "$tmp/project"
chmod 700 "$XDG_RUNTIME_DIR"
# Startup defaults to the OS theme. Pin the named capture, not the user's settings.
printf '%s\n' '{"appearance":{"theme":"evening"}}' > "$PI_DESKTOP_CONFIG_DIR/settings.json"
unset WAYLAND_DISPLAY PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
export LIBGL_ALWAYS_SOFTWARE=1
export WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
    if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver"; export VK_ICD_FILENAMES="$driver"; break; fi
done
"$binary" --demo --project "$tmp/project" >artifacts/xvfb-app.log 2>&1 &
app=$!
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
xdotool key --clearmodifiers ctrl+shift+t
sleep 1
import -window "$window" artifacts/thread-moonstone.png
# Locate rendered controls: tool grouping and the new-session chooser move them.
locate_word() {
    import -window "$window" "$tmp/control.png"
    python3 - "$tmp/control.png" "$1" "${2:-}" <<'PY'
from PIL import Image
import csv, io, subprocess, sys
im = Image.open(sys.argv[1])
left, top, right, bottom = (map(int, sys.argv[3].split(',')) if sys.argv[3]
                           else (0, 0, im.width, im.height))
im = im.crop((left, top, right, bottom))
im.resize((im.width * 2, im.height * 2)).save(sys.argv[1])
result = subprocess.check_output(['tesseract', sys.argv[1], 'stdout', '--psm', '11', 'tsv'],
                                 stderr=subprocess.DEVNULL, text=True)
rows = [row for row in csv.DictReader(io.StringIO(result), delimiter='\t')
        if row['text'].lower() == sys.argv[2].lower()]
assert len(rows) == 1, (sys.argv[2], rows)
row = rows[0]
print(left + (int(row['left']) + int(row['width']) // 2) // 2,
      top + (int(row['top']) + int(row['height']) // 2) // 2)
PY
}
# Tool details are intentionally collapsed by default. Capture each expanded view too.
for tool in 'edit-details Edit' 'bash-details Bash'; do
    read -r name word <<< "$tool"
    read -r x y < <(locate_word "$word" '230,270,1000,555')
    xdotool mousemove --window "$window" "$x" "$y" click 1
    sleep 1
    import -window "$window" "artifacts/thread-$name.png"
    xdotool mousemove --window "$window" "$x" "$y" click 1
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
sleep 0.5
# White text on the accent button is unreliable in full-frame OCR. Locate its
# undimmed native fill; the old session's controls are behind the prompt scrim.
import -window "$window" "$tmp/create.png"
read -r x y < <(python3 - "$tmp/create.png" <<'PY'
from PIL import Image
import sys
im = Image.open(sys.argv[1]).convert('RGB')
rows = []
for y in range(im.height // 2, im.height - 45):
    xs = [x for x in range(im.width // 2, im.width - 30)
          if max(abs(a - b) for a, b in zip(im.getpixel((x, y)), (75, 96, 124))) <= 2]
    rows.append((len(xs), xs, y))
count, xs, y = max(rows, key=lambda row: row[0])
assert count > 70, 'No enabled Create session button in the native prompt'
print((min(xs) + max(xs)) // 2, y)
PY
)
xdotool mousemove --window "$window" "$x" "$y" click 1
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
