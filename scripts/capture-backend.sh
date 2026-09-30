#!/usr/bin/env bash
# Actual native desktop + released Pi SDK. Isolated metadata/history only.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-backend-native.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
export PI_CODING_AGENT_DIR="$tmp/agent" PI_OFFLINE=1 PI_SKIP_VERSION_CHECK=1
export PI_BACKEND_TEST_ROOT="$tmp" PI_BACKEND_TEST_REPO="$repo"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
# Never use inherited provider secrets for this validation.
while IFS= read -r key; do
  case "$key" in *API_KEY*|*ACCESS_TOKEN*|*AUTH_TOKEN*) unset "$key";; esac
done < <(compgen -e)
unset WAYLAND_DISPLAY PI_DESKTOP_PI PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
export PI_DESKTOP_RPC_ENTRY="$tmp/entry.mjs"
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
python3 - <<'PY'
import json, os
from pathlib import Path
root = Path(os.environ['PI_BACKEND_TEST_ROOT'])
records = [
    {'type':'session','version':3,'id':'offline-backend','cwd':str(root/'project'),'timestamp':'2026-09-29T12:00:00Z'},
    {'type':'thinking_level_change','id':'think','parentId':None,'timestamp':'2026-09-29T12:00:00Z','thinkingLevel':'off'},
    {'type':'message','id':'user','parentId':'think','timestamp':'2026-09-29T12:00:00Z','message':{'role':'user','content':'Offline backend history','timestamp':1790683200000}},
    {'type':'session_info','id':'name','parentId':'user','timestamp':'2026-09-29T12:00:00Z','name':'Offline backend'},
]
(root/'history.jsonl').write_text(''.join(json.dumps(record)+'\n' for record in records))
(root/'entry.mjs').write_text('''import {spawn} from 'node:child_process';
import {appendFileSync} from 'node:fs';
const root=process.env.PI_BACKEND_TEST_ROOT, repo=process.env.PI_BACKEND_TEST_REPO;
const plumbing=/^(PATH|HOME|PI_CODING_AGENT_DIR|PI_OFFLINE|PI_SKIP_VERSION_CHECK|PI_DESKTOP_LSP(?:_TOKEN)?|SystemRoot|ComSpec|TEMP|TMP|TMPDIR|LANG|LC_.*|TZ|SSL_CERT_FILE|NODE_EXTRA_CA_CERTS|HTTPS?_PROXY|ALL_PROXY|NO_PROXY|https?_proxy|all_proxy|no_proxy)$/;
const env=Object.fromEntries(Object.entries(process.env).filter(([key])=>plumbing.test(key)));
const child=spawn(process.execPath,[repo+'/packages/pi-desktop-backend/src/cli.mjs',
  '--offline','-e',repo+'/packages/pi-desktop-backend/test/fixtures/extension.ts',
  '--session',root+'/history.jsonl',...process.argv.slice(2)],{env,stdio:['pipe','inherit','inherit']});
let pending='';
process.stdin.setEncoding('utf8');
process.stdin.on('data',chunk=>{
  pending+=chunk; let index;
  while((index=pending.indexOf('\\n'))>=0){
    const line=pending.slice(0,index); pending=pending.slice(index+1);
    appendFileSync(root+'/commands.jsonl',JSON.stringify({type:JSON.parse(line).type})+'\\n');
    child.stdin.write(line+'\\n');
  }
});
process.stdin.on('end',()=>child.stdin.end());
child.on('error',error=>{console.error(error.message);process.exit(1);});
child.on('exit',code=>process.exit(code??1));
''')
PY
"$binary" --project "$tmp/project" --light > artifacts/backend-native-app.log 2>&1 & app=$!
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || exit 1
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep .1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
shot() { xdotool mousemove --window "$window" 10 52; sleep .2; import -window "$window" "artifacts/backend-native-$1.png"; }
for _ in $(seq 1 50); do
  shot thread
  tesseract artifacts/backend-native-thread.png stdout --psm 11 2>/dev/null > artifacts/backend-native-thread.txt
  if grep -qi 'Offline backend history' artifacts/backend-native-thread.txt && grep -qi 'Ready' artifacts/backend-native-thread.txt; then break; fi
  sleep .5
done
grep -qi 'Offline backend history' artifacts/backend-native-thread.txt
grep -qi 'Ready' artifacts/backend-native-thread.txt
word() {
  shot locate
  python3 - "$1" <<'PY' > "$tmp/point"
from PIL import Image
import csv, io, subprocess, sys
im=Image.open('artifacts/backend-native-locate.png').crop((0,450,207,715))
im.resize((621,795)).save('artifacts/backend-native-ocr.png')
rows=list(csv.DictReader(io.StringIO(subprocess.check_output(['tesseract','artifacts/backend-native-ocr.png','stdout','--psm','11','tsv'],text=True,stderr=subprocess.DEVNULL)),delimiter='\t'))
row=next(r for r in rows if r['text'].lower()==sys.argv[1].lower())
print((int(row['left'])+int(row['width'])//2)//3,450+(int(row['top'])+int(row['height'])//2)//3)
PY
  read -r x y < "$tmp/point"
  xdotool mousemove --window "$window" "$x" "$y" click 1
  sleep 1
}
word Models
shot models
word Resources
shot resources
word All
shot sessions
python3 - "$tmp/commands.jsonl" <<'PY'
from pathlib import Path
import json, subprocess, sys
for name, expected in [('models','offline'),('resources','packages'),('sessions','offline')]:
    text=subprocess.check_output(['tesseract',f'artifacts/backend-native-{name}.png','stdout','--psm','11'],text=True,stderr=subprocess.DEVNULL).lower()
    assert expected in text,(name,text)
    assert 'setup failed' not in text and 'did not resume' not in text,(name,text)
allowed={'get_state','get_messages','get_entries','get_session_stats','get_settings','get_available_models','get_available_thinking_levels','get_commands','get_auth_providers','get_project_trust','list_packages','list_sessions'}
commands={json.loads(line)['type'] for line in Path(sys.argv[1]).read_text().splitlines()}
assert commands<=allowed,commands
assert {'get_settings','get_auth_providers','get_project_trust','list_packages'}<=commands,commands
Path('artifacts/backend-native-commands.json').write_text(json.dumps(sorted(commands),indent=2)+'\n')
PY
xdotool key ctrl+q
for _ in $(seq 1 75); do
  if ! kill -0 "$app" 2>/dev/null; then
    wait "$app"; app=""
    echo 'PASS: actual desktop bootstraps saved history and Models/Resources/Sessions against the released Pi SDK; metadata-only commands; clean exit.'
    exit 0
  fi
  sleep .1
done
exit 1
