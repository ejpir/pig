#!/usr/bin/env bash
# Native Models/Resources evidence against an offline metadata-only RPC peer.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-catalog.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
export PI_DESKTOP_PI="$repo/fixtures/catalog-rpc.py" PI_CATALOG_LOG="$tmp/commands.jsonl"
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
appearance=()
[[ "${PI_CATALOG_DARK:-}" == 1 ]] || appearance+=(--light)
"$binary" --project "$tmp/project" "${appearance[@]}" > artifacts/catalog-app.log 2>&1 & app=$!
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || exit 1
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep .1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
sleep 3
shot() { xdotool mousemove --window "$window" 10 52; sleep .15; import -window "$window" "artifacts/catalog-$1.png"; }
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep .15; xdotool click 1; sleep .7; }
# Locate real rendered words, not old hard-coded tool-row positions.
word() {
  shot locate
  python3 - "$@" <<'PY' > "$tmp/point"
from PIL import Image
import csv,io,subprocess,sys
needle=sys.argv[1].lower()
x,y,right,bottom=map(int,sys.argv[2:])
im=Image.open('artifacts/catalog-locate.png').crop((x,y,right,bottom))
im.resize((im.width*3,im.height*3)).save('artifacts/catalog-locate-ocr.png')
data=subprocess.check_output(['tesseract','artifacts/catalog-locate-ocr.png','stdout','--psm','11','tsv'],text=True,stderr=subprocess.DEVNULL)
rows=list(csv.DictReader(io.StringIO(data),delimiter='\t'))
row=next((r for r in rows if r['text'].strip().lower().rstrip('…:.')==needle),None)
assert row is not None,(needle,[r['text'] for r in rows if r['text'].strip()])
print(x+(int(row['left'])+int(row['width'])//2)//3,y+(int(row['top'])+int(row['height'])//2)//3)
PY
  read -r x y < "$tmp/point"
  click "$x" "$y"
}
if [[ "${PI_CATALOG_DARK:-}" == 1 ]]; then
  word Models 0 450 207 715
  shot models-dark
  word Resources 0 450 207 715
  shot resources-dark
  xdotool key ctrl+q
  wait "$app"
  app=""
  echo 'PASS: native dark Models and Resources captured.'
  exit 0
fi
if [[ "${PI_CATALOG_TRUSTED:-}" == 1 ]]; then
  word Resources 0 450 207 715
  shot trust-active
  tesseract artifacts/catalog-trust-active.png stdout --psm 11 2>/dev/null > artifacts/catalog-trust-active.txt
  grep -q 'active in this session' artifacts/catalog-trust-active.txt
  ! grep -q 'restart' artifacts/catalog-trust-active.txt
  xdotool key ctrl+q
  wait "$app"
  app=""
  echo 'PASS: fresh process reports saved trust active, with no restart hint.'
  exit 0
fi
word All 0 450 207 715
shot sessions
word Recent 210 52 720 98
shot sessions-sort
word Name 500 95 720 190
shot sessions-sorted
word Models 0 450 207 715
shot models
word atlas-small 210 190 1015 680
shot model-selected
# Find the filled primary button, not the nearby default action. White-on-blue
# text in a full inspector crop is inconsistently segmented by OCR.
python3 - <<'PY' > "$tmp/point"
from PIL import Image
im=Image.open('artifacts/catalog-model-selected.png').convert('RGB')
runs=[]
start=None
for y in range(350,715):
    r,g,b=im.getpixel((1310,y))
    filled=30<r<150 and b>r+15 and b>g+5
    if filled and start is None: start=y
    if not filled and start is not None:
        if y-start>=16: runs.append((start,y))
        start=None
assert len(runs)==1,runs
print(1200,sum(runs[0])//2)
PY
read -r x y < "$tmp/point"
click "$x" "$y"
shot model-used
xdotool key ctrl+k
xdotool type --clearmodifiers 'local-code'
sleep .5
shot model-search
xdotool key ctrl+a BackSpace
sleep .4
word Resources 0 450 207 715
shot resources
if [[ "${PI_CATALOG_NOTICE:-}" == 1 ]]; then
  # Notice floats over the page. Dismiss it before probing resource tabs.
  shot notice
  click 970 96
  shot notice-dismissed
fi
word Skills 210 150 1015 420
shot skills
word Copy 1020 200 1344 710
/usr/bin/xclip -selection clipboard -o > artifacts/catalog-resource-copy.txt
word Packages 210 220 1015 270
word Remove 1020 200 1344 710
shot remove-confirm
# A modal must block both background navigation and the global search shortcut.
click 75 628
xdotool key ctrl+k
xdotool key Escape
sleep .4
shot remove-cancelled
xdotool windowsize "$window" 1000 740
sleep .5
shot resources-narrow
python3 - "$tmp/commands.jsonl" <<'PY'
from pathlib import Path
from PIL import Image
import json,re,subprocess,sys

def text(name,box=None):
    im=Image.open(f'artifacts/catalog-{name}.png')
    if box: im=im.crop(box)
    im.resize((im.width*2,im.height*2)).save('artifacts/catalog-assert-ocr.png')
    return subprocess.check_output(['tesseract','artifacts/catalog-assert-ocr.png','stdout','--psm','11'],text=True,stderr=subprocess.DEVNULL).lower()
assert 'name' in text('sessions-sort',(505,90,700,190))
assert re.search(r'a\s*named', text('sessions-sorted',(208,95,1016,400)))
assert 'atlas-large' in text('models')
assert re.search(r'atlas\s*small', text('model-selected',(1016,52,1344,715)))
assert re.search(r'current\s*session\s*model', text('model-used',(1016,52,1344,715)))
search=text('model-search',(208,200,1016,715))
assert 'local-code' in search and 'atlas-small' not in search,search
assert 'git-guard' in text('resources')
assert 'review' in text('skills')
assert '/offline/skills/review/SKILL.md' in Path('artifacts/catalog-resource-copy.txt').read_text()
assert 'remove package' in text('remove-confirm')
assert 'remove package' not in text('remove-cancelled')
commands=[json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
allowed={'get_state','get_messages','get_entries','get_settings','get_session_stats','list_sessions','get_commands','get_available_models','get_available_thinking_levels','get_auth_providers','get_project_trust','list_packages','set_model'}
assert {c['type'] for c in commands} <= allowed,commands
selected=[c for c in commands if c['type']=='set_model']
assert len(selected)==1 and selected[0]['modelId']=='atlas-small' and not selected[0].get('persist'),selected
PY
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; echo 'PASS: native model catalog, selection, session-only switch, search, resources/tabs/copy, cancel-safe package removal and narrow layout; no model calls or package operations.'; exit 0; fi
  sleep .1
done
exit 1
