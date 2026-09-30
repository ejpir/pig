#!/usr/bin/env bash
# Native footer/active-tools validation against published Pi SDK; synthetic data only.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
binary="$(realpath "${PI_DESKTOP_BINARY:-target/linux/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-status-tools.XXXXXX)"
app=""
trap 'if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
export PI_CODING_AGENT_DIR="$tmp/agent" PI_OFFLINE=1 PI_SKIP_VERSION_CHECK=1
export PI_STATUS_TEST_ROOT="$tmp" PI_STATUS_TEST_REPO="$repo"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project" artifacts
chmod 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY PI_DESKTOP_PI PI_DESKTOP_DEMO_WORKSPACE PI_DESKTOP_DEMO_READABILITY
export PI_DESKTOP_RPC_ENTRY="$tmp/entry.mjs"
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
python3 - <<'PY'
import os
from pathlib import Path
root = Path(os.environ['PI_STATUS_TEST_ROOT'])
(root/'entry.mjs').write_text('''import {spawn} from 'node:child_process';
import {appendFileSync} from 'node:fs';
const root=process.env.PI_STATUS_TEST_ROOT, repo=process.env.PI_STATUS_TEST_REPO;
const plumbing=/^(PATH|HOME|PI_CODING_AGENT_DIR|PI_OFFLINE|PI_SKIP_VERSION_CHECK|PI_DESKTOP_LSP(?:_TOKEN)?|SystemRoot|ComSpec|TEMP|TMP|TMPDIR|LANG|LC_.*|TZ|SSL_CERT_FILE|NODE_EXTRA_CA_CERTS|HTTPS?_PROXY|ALL_PROXY|NO_PROXY|https?_proxy|all_proxy|no_proxy)$/;
const env=Object.fromEntries(Object.entries(process.env).filter(([key])=>plumbing.test(key)));
const child=spawn(process.execPath,[repo+'/packages/pi-desktop-backend/src/cli.mjs',
  '--offline','--no-session','-e',repo+'/packages/pi-desktop-backend/test/fixtures/extension.ts',
  '-e',repo+'/packages/pi-desktop-backend/test/fixtures/telemetry.ts',...process.argv.slice(2)],
  {env,stdio:['pipe','pipe','inherit']});
let input='', output='';
process.stdin.setEncoding('utf8'); child.stdout.setEncoding('utf8');
process.stdin.on('data',chunk=>{
  input+=chunk; let index;
  while((index=input.indexOf('\\n'))>=0){
    const line=input.slice(0,index); input=input.slice(index+1);
    appendFileSync(root+'/commands.jsonl',JSON.stringify({type:JSON.parse(line).type})+'\\n');
    child.stdin.write(line+'\\n');
  }
});
child.stdout.on('data',chunk=>{
  process.stdout.write(chunk); output+=chunk; let index;
  while((index=output.indexOf('\\n'))>=0){
    const line=output.slice(0,index); output=output.slice(index+1); const record=JSON.parse(line);
    appendFileSync(root+'/events.jsonl',JSON.stringify({type:record.type,method:record.method})+'\\n');
    if(record.type==='response'&&record.command==='get_state'&&record.success)
      appendFileSync(root+'/tools.jsonl',JSON.stringify(record.data.activeTools)+'\\n');
  }
});
process.stdin.on('end',()=>child.stdin.end());
child.on('error',error=>{console.error(error.message);process.exit(1);});
child.on('exit',code=>process.exit(code??1));
''')
PY
"$binary" --project "$tmp/project" > artifacts/status-tools-native-app.log 2>&1 & app=$!
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || exit 1
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep .1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
shot() { xdotool mousemove --window "$window" 10 52; sleep .2; import -window "$window" "artifacts/status-tools-$1.png"; }
for _ in $(seq 1 50); do
  shot wide
  tesseract artifacts/status-tools-wide.png stdout --psm 11 2>/dev/null > artifacts/status-tools-wide.txt
  if grep -qi 'backend_probe' artifacts/status-tools-wide.txt && grep -qi '87.3' artifacts/status-tools-wide.txt; then break; fi
  sleep .4
done
grep -qi 'backend_probe' artifacts/status-tools-wide.txt
height=$(identify -format '%h' artifacts/status-tools-wide.png)
xdotool mousemove --window "$window" 300 "$((height-12))"
sleep 1.4
import -window "$window" artifacts/status-tools-tooltip.png
tesseract artifacts/status-tools-tooltip.png stdout --psm 11 2>/dev/null > artifacts/status-tools-tooltip.txt
grep -qi 'last reported run statistics' artifacts/status-tools-tooltip.txt
grep -qiE '26[.]9[s5]' artifacts/status-tools-tooltip.txt
# Descriptions/origins are hover-only, not additional inspector rows.
python3 - <<'PY' > "$tmp/tool-point"
from PIL import Image
import csv,io,subprocess
im=Image.open('artifacts/status-tools-wide.png');im.resize((im.width*3,im.height*3)).save('artifacts/status-tools-ocr.png')
rows=csv.DictReader(io.StringIO(subprocess.check_output(['tesseract','artifacts/status-tools-ocr.png','stdout','--psm','11','tsv'],text=True,stderr=subprocess.DEVNULL)),delimiter='\t')
r=next(r for r in rows if r['text']=='backend_probe' and int(r['left'])//3>1000)
print((int(r['left'])+int(r['width'])//2)//3,(int(r['top'])+int(r['height'])//2)//3)
PY
read -r tx ty < "$tmp/tool-point"
xdotool mousemove --window "$window" "$tx" "$ty"; sleep 1.4
import -window "$window" artifacts/status-tools-tool-tooltip.png
tesseract artifacts/status-tools-tool-tooltip.png stdout --psm 11 2>/dev/null > artifacts/status-tools-tool-tooltip.txt
grep -qi 'validation never' artifacts/status-tools-tool-tooltip.txt
grep -qi 'executes it' artifacts/status-tools-tool-tooltip.txt
xdotool mousemove --window "$window" 10 52
xdotool windowsize "$window" 1000 740
sleep .8
shot narrow
python3 - "$tmp" <<'PY'
from pathlib import Path
from PIL import Image
import json, subprocess, sys
root=Path(sys.argv[1])
def ocr(image, mode):
    image.resize((image.width*3,image.height*3)).save('artifacts/status-tools-ocr.png')
    return subprocess.check_output(['tesseract','artifacts/status-tools-ocr.png','stdout','--psm',str(mode)],text=True,stderr=subprocess.DEVNULL)
for name in ['wide','narrow']:
    image=Image.open(f'artifacts/status-tools-{name}.png')
    footer=ocr(image.crop((0,image.height-26,image.width,image.height)),7)
    body=ocr(image.crop((207,92,image.width,image.height-26)),11)
    assert '87.3' in footer and 'TPS' in footer,footer
    assert 'TPS' not in body,body
    assert 'validation never executes' not in body.lower(),body
    assert 'Active tool inventory is not exposed' not in body,body
    Path(f'artifacts/status-tools-{name}-footer.txt').write_text(footer)
tools=json.loads((root/'tools.jsonl').read_text().splitlines()[-1])
assert any(tool['name']=='read' and tool['sourceInfo']['source']=='builtin' for tool in tools),tools
assert any(tool['name']=='backend_probe' and tool['description'] for tool in tools),tools
commands={json.loads(line)['type'] for line in (root/'commands.jsonl').read_text().splitlines()}
allowed={'get_state','get_messages','get_session_stats','list_sessions','get_commands'}
assert commands<=allowed,commands
events=[json.loads(line)['type'] for line in (root/'events.jsonl').read_text().splitlines()]
assert not any(event.startswith(('agent_','tool_execution_')) for event in events),events
Path('artifacts/status-tools-native-protocol.json').write_text(json.dumps({'commands':sorted(commands),'activeTools':tools,'modelOrToolEvents':False},indent=2)+'\n')
PY
xdotool key ctrl+q
for _ in $(seq 1 75); do
  if ! kill -0 "$app" 2>/dev/null; then
    wait "$app"; app=""
    echo 'PASS: TPS stays in the small footer at wide/narrow widths, live SDK tools replace the placeholder, no model or tool execution, clean exit.'
    exit 0
  fi
  sleep .1
done
exit 1
