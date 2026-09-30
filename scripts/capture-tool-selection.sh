#!/usr/bin/env bash
# Native clipboard/selection check for expanded tool details; offline fixture only.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts
clipboard="${PI_DESKTOP_XCLIP:-/usr/bin/xclip}"
[[ -x "$clipboard" ]] || { echo 'Install xclip or set PI_DESKTOP_XCLIP' >&2; exit 1; }
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
    if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
export XDG_RUNTIME_DIR="$(mktemp -d /tmp/pi-selection-runtime.XXXXXX)"
unset WAYLAND_DISPLAY PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
binary="${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target}/debug/pi-desktop}"
"$binary" --demo --light > artifacts/tool-selection-app.log 2>&1 & app=$!
trap 'kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; rm -rf "$XDG_RUNTIME_DIR"' EXIT
window=""
for _ in $(seq 1 100); do
    kill -0 "$app" 2>/dev/null || exit 1
    window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
    [[ -n "$window" ]] && break
    sleep 0.1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
sleep 2
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep 0.15; xdotool click "${3:-1}"; sleep 0.4; }
find_header() {
    import -window "$window" "$XDG_RUNTIME_DIR/header.png"
    python3 - "$XDG_RUNTIME_DIR/header.png" "$1" <<'PY'
from PIL import Image
import sys,subprocess,io,csv
p=sys.argv[1]
im=Image.open(p).crop((208,92,1016,555))
im.resize((1616,926)).save(p)
rows=csv.DictReader(io.StringIO(subprocess.check_output(['tesseract',p,'stdout','--psm','6','tsv'],stderr=subprocess.DEVNULL,text=True)),delimiter='\t')
r=next(r for r in rows if r['text'].lower()==sys.argv[2].lower())
print(92+(int(r['top'])+int(r['height'])//2)//2)
PY
}
# Locate a real rendered line, rather than pinning a font-dependent baseline.
# The sentinel and one-line assertion prevent stale/full-document copy successes.
select_line() {
    local kind="$1" marker="$2" first="$3" last="$4" y x=260
    [[ "$kind" != bash ]] || x=253
    for y in $(seq "$first" 4 "$last"); do
        printf sentinel | "$clipboard" -selection clipboard >/dev/null 2>&1
        xdotool mousemove --window "$window" "$x" "$y"
        xdotool mousedown 1
        sleep 0.05
        xdotool mousemove --window "$window" 970 "$y"
        sleep 0.05
        xdotool mouseup 1
        sleep 0.08
        xdotool key --clearmodifiers ctrl+c
        sleep 0.08
        timeout 5 "$clipboard" -selection clipboard -o > "artifacts/tool-$kind-copy.txt"
        if grep -q "$marker" "artifacts/tool-$kind-copy.txt" && [[ $(wc -l < "artifacts/tool-$kind-copy.txt") -le 1 ]]; then echo "$y"; return 0; fi
    done
    echo "Could not select $marker" >&2; return 1
}
edit_y="$(find_header Edit)"
click 500 "$edit_y"
import -window "$window" artifacts/tool-selection-expanded.png
select_line edit MissingSignature "$((edit_y+30))" 550 > /dev/null
import -window "$window" artifacts/thread-edit-selected.png
click 500 "$edit_y"
bash_y="$(find_header Bash)"
click 500 "$bash_y"
import -window "$window" artifacts/tool-bash-expanded.png
y="$(select_line bash 'Checked 1,284' "$((bash_y+60))" 550)"
import -window "$window" artifacts/thread-bash-selected.png
printf sentinel | "$clipboard" -selection clipboard
click 340 "$y" 3
sleep 0.4
xdotool key Home Return
sleep 0.4
timeout 5 "$clipboard" -selection clipboard -o > artifacts/tool-context-copy.txt
grep -q 'Checked 1,284' artifacts/tool-context-copy.txt
xdotool key ctrl+q
for _ in $(seq 1 50); do
    if ! kill -0 "$app" 2>/dev/null; then wait "$app"; echo 'PASS: native edit/bash drag selection, keyboard clipboard copy, context-menu copy, clean exit.'; exit 0; fi
    sleep 0.1
done
exit 1
