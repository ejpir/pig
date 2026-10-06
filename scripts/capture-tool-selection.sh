#!/usr/bin/env bash
# Native clipboard/selection check for an expanded edit detail; offline fixture only.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p artifacts
clipboard="${PI_DESKTOP_XCLIP:-/usr/bin/xclip}"
[[ -x "$clipboard" ]] || { echo 'Install xclip or set PI_DESKTOP_XCLIP' >&2; exit 1; }
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
    if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
binary="$(realpath "${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target}/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-selection.XXXXXX)"; app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" XDG_CACHE_HOME="$tmp/cache"
export XDG_RUNTIME_DIR="$tmp/runtime" PI_DESKTOP_CONFIG_DIR="$tmp/desktop" PI_CODING_AGENT_DIR="$tmp/agent"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project"; chmod 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
"$binary" --demo --light --project "$tmp/project" > artifacts/tool-selection-app.log 2>&1 & app=$!
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
im=Image.open(p).crop((208,92,1016,700))
im.resize((1616,1216)).save(p)
rows=csv.DictReader(io.StringIO(subprocess.check_output(['tesseract',p,'stdout','--psm','6','tsv'],stderr=subprocess.DEVNULL,text=True)),delimiter='\t')
r=next(r for r in rows if r['text'].lower()==sys.argv[2].lower())
print(92+(int(r['top'])+int(r['height'])//2)//2)
PY
}
# Locate a real rendered line, rather than pinning a font-dependent baseline.
# Keep each drag on one X connection: separate xdotool processes can be observed
# out of order by Xvfb under load. Polling also gives GPUI time to own CLIPBOARD.
# The sentinel and one-line assertion prevent stale/full-document copy successes.
select_line() {
    local kind="$1" marker="$2" first="$3" last="$4" y copied x=300
    local output="artifacts/tool-$kind-copy.txt"
    [[ "$kind" != bash ]] || x=253
    for y in $(seq "$first" 4 "$last"); do
        printf sentinel | "$clipboard" -selection clipboard >/dev/null 2>&1
        xdotool mousemove --window "$window" "$x" "$y" \
            mousedown 1 sleep 0.1 \
            mousemove --window "$window" "$(((x + 970) / 2))" "$y" sleep 0.1 \
            mousemove --window "$window" 970 "$y" sleep 0.1 \
            mouseup 1 sleep 0.2 \
            key --clearmodifiers ctrl+c
        for _ in 1 2 3 4 5; do
            sleep 0.1
            if timeout 2 "$clipboard" -selection clipboard -o > "$output" 2>/dev/null; then
                if grep -Fq -- "$marker" "$output" && [[ $(wc -l < "$output") -le 1 ]]; then
                    echo "$y"
                    return 0
                fi
                copied="$(<"$output")"
                # Copy completed, but this is not the target line. Move the
                # pointer instead of waiting out every poll at this baseline.
                [[ "$copied" == sentinel ]] || break
            fi
            xdotool key --clearmodifiers ctrl+c
        done
    done
    echo "Could not select $marker" >&2
    return 1
}
# A run's edits are grouped under "Changed"; open the group, then the edit.
click 500 "$(find_header Changed)"
xdotool mousemove --window "$window" 1300 120
sleep 0.4
click 500 "$(find_header Edit)"
# The diff opens below the fold.
xdotool mousemove --window "$window" 850 300 click --repeat 3 --delay 100 5
sleep 0.4
import -window "$window" artifacts/tool-selection-expanded.png
# Below the rows: a drag that starts on one folds it.
select_line edit MissingSignature "$(($(find_header Edit) + 20))" 500 > /dev/null
import -window "$window" artifacts/thread-edit-selected.png
xdotool key ctrl+q
for _ in $(seq 1 50); do
    if ! kill -0 "$app" 2>/dev/null; then wait "$app"; echo 'PASS: native edit drag selection and keyboard clipboard copy; clean exit.'; exit 0; fi
    sleep 0.1
done
exit 1
