#!/usr/bin/env bash
# Native study 02/04 and shell copy icons; published SDK, isolated local fixtures.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-projects-ui.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
export PI_CODING_AGENT_DIR="$tmp/agent" PI_OFFLINE=1 PI_SKIP_VERSION_CHECK=1
export PI_PROJECTS_TEST_ROOT="$tmp" PI_PROJECTS_TEST_REPO="$repo"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/pi/.pi/extensions" "$tmp/zed" "$tmp/minivm" "$tmp/agent" "$tmp/sessions" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY PI_DESKTOP_PI PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
export PI_DESKTOP_RPC_ENTRY="$tmp/entry.mjs" LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
python3 - <<'PY'
import json, os
from pathlib import Path
r=Path(os.environ['PI_PROJECTS_TEST_ROOT'])
(r/'agent/settings.json').write_text(json.dumps({'defaultProjectTrust':'always'}))
(r/'pi/.pi/settings.json').write_text(json.dumps({'defaultThinkingLevel':'medium'}))
(r/'pi/AGENTS.md').write_text('Isolated offline UI fixture. Do not execute model or tool calls.\n')
(r/'pi/.pi/extensions/release-notes.ts').write_text("export default function(pi) { pi.registerCommand('release-notes', { description: 'Offline fixture command', handler: async () => {} }); }\n")
(r/'pi/.pi/extensions/rpc-demo.ts').write_text("throw new Error('Synthetic load error for native UI validation');\n")
for name in ['pi','zed','minivm']:
 records=[{'type':'session','version':3,'id':'offline-'+name,'cwd':str(r/name),'timestamp':'2026-09-30T01:00:00Z'},
          {'type':'message','id':'user-'+name,'parentId':None,'timestamp':'2026-09-30T01:00:00Z','message':{'role':'user','content':'Offline UI validation','timestamp':1790730000000}},
          {'type':'session_info','id':'name-'+name,'parentId':'user-'+name,'timestamp':'2026-09-30T01:00:00Z','name':'Offline '+name}]
 (r/'sessions'/f'{name}.jsonl').write_text(''.join(json.dumps(record)+'\n' for record in records))
(r/'entry.mjs').write_text('''import {spawn} from 'node:child_process';
import {appendFileSync} from 'node:fs';
const root=process.env.PI_PROJECTS_TEST_ROOT, repo=process.env.PI_PROJECTS_TEST_REPO;
const plumbing=/^(PATH|HOME|PI_CODING_AGENT_DIR|PI_OFFLINE|PI_SKIP_VERSION_CHECK|PI_DESKTOP_LSP(?:_TOKEN)?|SystemRoot|ComSpec|TEMP|TMP|TMPDIR|LANG|LC_.*|TZ|SSL_CERT_FILE|NODE_EXTRA_CA_CERTS|HTTPS?_PROXY|ALL_PROXY|NO_PROXY|https?_proxy|all_proxy|no_proxy)$/;
const env=Object.fromEntries(Object.entries(process.env).filter(([key])=>plumbing.test(key)));
const child=spawn(process.execPath,[repo+'/packages/pi-desktop-backend/src/cli.mjs','--offline',
 '-e',repo+'/packages/pi-desktop-backend/test/fixtures/extension.ts','--session-dir',root+'/sessions',
 '--session',root+'/sessions/pi.jsonl',...process.argv.slice(2)],{env,stdio:['pipe','pipe','inherit']});
let input='',output=''; process.stdin.setEncoding('utf8'); child.stdout.setEncoding('utf8');
process.stdin.on('data',chunk=>{input+=chunk;let n;while((n=input.indexOf('\\n'))>=0){
 const line=input.slice(0,n);input=input.slice(n+1);const command=JSON.parse(line);
 if(command.type==='prompt'&&!['/backend-dialog-queue','/backend-dialog-timeout'].includes(command.message)) throw new Error('Native fixture forbids ordinary prompts');
 appendFileSync(root+'/commands.jsonl',JSON.stringify({type:command.type,...(command.type==='bash'?{excludeFromContext:command.excludeFromContext}:{}),...(command.type==='prompt'?{message:command.message}:{}),...(command.type==='extension_ui_response'?{id:command.id,cancelled:command.cancelled,value:command.value,confirmed:command.confirmed}:{})})+'\\n');
 child.stdin.write(line+'\\n');}});
child.stdout.on('data',chunk=>{process.stdout.write(chunk);output+=chunk;let n;while((n=output.indexOf('\\n'))>=0){
 const line=output.slice(0,n);output=output.slice(n+1);const record=JSON.parse(line);
 appendFileSync(root+'/events.jsonl',JSON.stringify({type:record.type,method:record.method,...(record.type==='extension_ui_request'&&record.method!=='notify'||record.type==='extension_ui_cancel'?{id:record.id}: {})})+'\\n');
 if(record.type==='response'&&record.command==='get_project_trust'&&record.success)
  appendFileSync(root+'/resources.jsonl',JSON.stringify(record.data)+'\\n');}});
process.stdin.on('end',()=>child.stdin.end());child.on('error',e=>{console.error(e.message);process.exit(1);});child.on('exit',code=>process.exit(code??1));
''')
PY
"$binary" --project "$tmp/pi" --light > artifacts/projects-shell-native-app.log 2>&1 & app=$!
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || exit 1
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep .1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
shot() { xdotool mousemove --window "$window" 10 52; sleep .25; import -window "$window" "artifacts/projects-shell-$1.png"; }
ocr() {
  python3 - "$1" <<'PY'
from PIL import Image
import sys
im=Image.open(f'artifacts/projects-shell-{sys.argv[1]}.png')
im.resize((im.width*3,im.height*3)).save('artifacts/projects-shell-ocr.png')
PY
  tesseract artifacts/projects-shell-ocr.png stdout --psm 11 2>/dev/null > "artifacts/projects-shell-$1.txt"
}
# Locate real painted text, not presumed dialog coordinates.
word() {
  shot locate
  python3 - "$1" "${2:-0}" "${3:-0}" "${4:-1344}" "${5:-740}" <<'PY' > "$tmp/point"
from PIL import Image
import csv,io,subprocess,sys
x,y,w,h=map(int,sys.argv[2:]); im=Image.open('artifacts/projects-shell-locate.png').crop((x,y,x+w,y+h))
im.resize((w*2,h*2)).save('artifacts/projects-shell-ocr.png')
rows=csv.DictReader(io.StringIO(subprocess.check_output(['tesseract','artifacts/projects-shell-ocr.png','stdout','--psm','11','tsv'],text=True,stderr=subprocess.DEVNULL)),delimiter='\t')
r=next(r for r in rows if r['text'].lower().strip('….,:')==sys.argv[1].lower())
print(x+(int(r['left'])+int(r['width'])//2)//2,y+(int(r['top'])+int(r['height'])//2)//2)
PY
  read -r x y < "$tmp/point"
  xdotool mousemove --window "$window" "$x" "$y" click 1
  sleep .5
}
for _ in $(seq 1 60); do
  shot startup; ocr startup
  grep -qi 'Ready' artifacts/projects-shell-startup.txt && grep -qi 'minivm' artifacts/projects-shell-startup.txt && break
  sleep .3
done
grep -qi 'Ready' artifacts/projects-shell-startup.txt
xdotool key ctrl+n
sleep .3
word worktree 420 50 510 650
shot new-session-light; ocr new-session-light
grep -qi 'BRANCH' artifacts/projects-shell-new-session-light.txt
grep -qi 'New session' artifacts/projects-shell-new-session-light.txt
word configured 420 350 510 300
shot new-model-picker-light
xdotool key Escape
# Preview and cancel must start no additional subprocess or mutation.
xdotool key Escape
xdotool key ctrl+shift+t ctrl+n
sleep .3
word worktree 420 50 510 650
shot new-session-dark
word configured 420 350 510 300
shot new-model-picker-dark
xdotool key Escape Escape ctrl+shift+t
word Resources 0 450 207 265
sleep 1
shot resources-light; ocr resources-light
grep -qi 'PROJECT SETTINGS' artifacts/projects-shell-resources-light.txt
grep -qi 'release-notes' artifacts/projects-shell-resources-light.txt
# Explicitly dismiss only the displayed fixture notice before inspecting other cards.
xdotool mousemove --window "$window" 973 94 click 1; sleep .3
word pi 345 55 250 45
shot resource-projects
xdotool key Escape
xdotool mousemove --window "$window" 251 78 click 1; sleep .3
shot resources-user; ocr resources-user
! grep -qi 'USER SETTINGS' artifacts/projects-shell-resources-user.txt
xdotool mousemove --window "$window" 460 118 click 1; sleep .3
shot resources-user-skills; ocr resources-user-skills
! grep -qi 'USER SETTINGS' artifacts/projects-shell-resources-user-skills.txt
xdotool mousemove --window "$window" 303 78 click 1; sleep .3
xdotool mousemove --window "$window" 372 172 click 1; sleep .3
xdotool windowsize "$window" 1000 740
sleep .5
shot resources-narrow
xdotool windowsize "$window" 1344 740
sleep .5
# Return to the owned session. Explicit !! command executes only harmless local printf.
xdotool mousemove --window "$window" 100 105 click 1
sleep .3
printf '%s' "!!printf 'Small copy icons — original output\\n'" | /usr/bin/xclip -selection clipboard
word Ask 230 590 760 70
xdotool key ctrl+v Return
for _ in $(seq 1 60); do
  shot shell; ocr shell
  grep -qi 'Small copy icons' artifacts/projects-shell-shell.txt && grep -qiE 'Exit *[0o]' artifacts/projects-shell-shell.txt && break
  sleep .2
 done
grep -qiE 'Exit *[0o]' artifacts/projects-shell-shell.txt
if grep -qiE 'Copy command|Copy output' artifacts/projects-shell-shell.txt; then echo 'Unexpected boxed copy labels'; exit 1; fi
# Directory completion uses the existing project snapshot; preview/selection sends no prompt.
printf '%s' '@.pi/' | /usr/bin/xclip -selection clipboard
word Ask 230 590 760 70
xdotool key ctrl+v; sleep 1
shot directory-menu; ocr directory-menu
grep -qi 'Directories' artifacts/projects-shell-directory-menu.txt
xdotool key Return; sleep .3
shot directory-chip
xdotool key ctrl+a BackSpace
# Actual owned process popup: Escape and click-away, never an inspection subprocess.
xdotool mousemove --window "$window" 145 727 click 1; sleep .4
shot processes; ocr processes
grep -qi 'Pi processes' artifacts/projects-shell-processes.txt
xdotool key Escape
xdotool mousemove --window "$window" 145 727 click 1; sleep .2
xdotool mousemove --window "$window" 100 105 click 1; sleep .3
# Standard SDK extension dialogs: FIFO, exact select answer, backend timeout cancellation.
submit_fixture() {
  printf '%s' "$1" | /usr/bin/xclip -selection clipboard
  word Ask 230 590 760 70
  xdotool key ctrl+v Return; sleep .2
  xdotool key Return; sleep .4
}
submit_fixture /backend-dialog-queue
shot decision-confirm; ocr decision-confirm
grep -qi 'Queued fixture confirmation' artifacts/projects-shell-decision-confirm.txt
xdotool key Escape; sleep .4
shot decision-select; ocr decision-select
grep -qi 'Queued fixture selection' artifacts/projects-shell-decision-select.txt
word Block 420 100 510 600
submit_fixture /backend-dialog-timeout
shot decision-timeout; ocr decision-timeout
grep -qi 'Timed fixture confirmation' artifacts/projects-shell-decision-timeout.txt
sleep 3.2
shot decision-cancelled; ocr decision-cancelled
! grep -qi 'Timed fixture confirmation' artifacts/projects-shell-decision-cancelled.txt
grep -qi 'Timed request finished' artifacts/projects-shell-decision-cancelled.txt
python3 - "$tmp" <<'PY'
import json,sys
from pathlib import Path
r=Path(sys.argv[1]); commands=[json.loads(line) for line in (r/'commands.jsonl').read_text().splitlines()]
allowed={'get_state','get_messages','get_entries','get_session_stats','get_settings','get_available_models','get_available_thinking_levels','get_commands','get_auth_providers','get_project_trust','list_packages','list_sessions','bash','prompt','extension_ui_response'}
assert {c['type'] for c in commands}<=allowed,commands
assert [c['excludeFromContext'] for c in commands if c['type']=='bash']==[True],commands
events=[json.loads(line) for line in (r/'events.jsonl').read_text().splitlines()]
assert not any(e['type'].startswith(('agent_','tool_execution_')) for e in events)
requests=[e for e in events if e['type']=='extension_ui_request' and e['method'] in {'confirm','select'}]
assert [e['method'] for e in requests]==['confirm','select','confirm'],requests
answers=[c for c in commands if c['type']=='extension_ui_response']
assert [c['id'] for c in answers]==[e['id'] for e in requests],answers
assert answers[0]['cancelled'] and answers[1]['value']=='Block' and answers[2]['cancelled'],answers
assert any(e['type']=='extension_ui_cancel' and e['id']==requests[2]['id'] for e in events),events
metadata=json.loads((r/'resources.jsonl').read_text().splitlines()[-1])
assert metadata['trusted'] and len(metadata['contextFiles'])==1,metadata
assert any(e['status']=='load-error' for e in metadata['loadedExtensions']),metadata
assert any('release-notes' in e['path'] and e['status']=='loaded' for e in metadata['loadedExtensions']),metadata
Path('artifacts/projects-shell-native-protocol.json').write_text(json.dumps({'commands':commands,'projectResources':metadata,'modelOrToolEvents':False},indent=2)+'\n')
PY
xdotool key ctrl+q
for _ in $(seq 1 75); do
  if ! kill -0 "$app" 2>/dev/null; then
    wait "$app"; app=""
    echo 'PASS: native chooser light/dark, project Resources wide/narrow, reported load error/context paths, small shell copy icons, owned process popup, queued confirm/select and backend timeout cancellation, explicit excluded local printf; no model/tool execution.'
    exit 0
  fi
  sleep .1
done
exit 1
