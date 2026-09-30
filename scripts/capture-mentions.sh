#!/usr/bin/env bash
# The composer's `@` menu in the real window (design study 08): files with a preview,
# symbols and problems from a local deterministic language server, a saved session and
# terminal output; a choice becomes a chip, and pi receives the file's path. A second,
# offline demo run shows a sent message with its chip. No npm or model calls.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
mkdir -p artifacts
binary="$(realpath "${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target/linux}/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-mentions.XXXXXX)"
app=""
cleanup() { if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"; }
trap cleanup EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
project="$tmp/project"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$project/.zed" "$project/packages/ai/src/providers" "$tmp/other"
chmod 700 "$XDG_RUNTIME_DIR"
printf 'export const openCodeModels = [\n  "kimi-k3", "qwen3.8-flash"\n];\n' > "$project/packages/ai/src/providers/opencode.ts"
printf 'export function readThinking() {}\n' > "$project/packages/ai/src/providers/openai-completions.ts"
printf 'broken: [\n' > "$project/broken.yaml"
export PI_LSP_TEST_LOG="$tmp/lsp.jsonl" npm_config_offline=true
python3 - "$project/.zed/settings.json" "$repo/fixtures/diagnostic-lsp.py" "$PI_LSP_TEST_LOG" <<'PY'
import json,sys
json.dump({"prettier":{"allowed":False},"languages":{"YAML":{"prettier":{"allowed":False},"formatter":"language_server"}},"lsp":{"yaml-language-server":{"binary":{"path":sys.argv[2],"arguments":[],"env":{"PI_LSP_TEST_LOG":sys.argv[3]}}}}},open(sys.argv[1],"w"))
PY
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY
export PI_DESKTOP_PI="$repo/fixtures/files-rpc.py" PI_FILES_COMMAND_LOG="$tmp/commands.log"
export PI_FILES_PROMPT_LOG="$tmp/prompts.jsonl" PI_FILES_OTHER_PROJECT="$tmp/other"

start() {
  "$binary" "$@" > artifacts/mentions-app.log 2>&1 &
  app=$!
  window=""
  for _ in $(seq 1 100); do
    kill -0 "$app" 2>/dev/null || { echo 'App exited; see mentions-app.log' >&2; exit 1; }
    window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
    [[ -n "$window" ]] && break
    sleep 0.1
  done
  [[ -n "$window" ]]
  xdotool windowfocus --sync "$window"
  sleep 2
}
quit() {
  xdotool key ctrl+q
  for _ in $(seq 1 50); do
    if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; return; fi
    sleep 0.1
  done
  echo 'App did not quit' >&2
  exit 1
}
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep 0.15; xdotool click 1; sleep 0.3; }
shot() { import -window "$window" "artifacts/mentions-$1.png"; }
# OCR at twice the size: artifacts/mentions-$1.txt, and word boxes in .tsv.
ocr() {
  python3 - "artifacts/mentions-$1.png" "$tmp/$1-2x.png" <<'PY'
import sys
from PIL import Image
image = Image.open(sys.argv[1])
image.resize((image.width * 2, image.height * 2)).save(sys.argv[2])
PY
  tesseract "$tmp/$1-2x.png" "artifacts/mentions-$1" --psm 11 tsv 2>/dev/null
  tesseract "$tmp/$1-2x.png" "artifacts/mentions-$1" --psm 11 2>/dev/null
}
word() {
  python3 - "artifacts/mentions-$1.tsv" "$2" <<'PY'
import csv, sys
for row in csv.DictReader(open(sys.argv[1]), delimiter="\t", quoting=csv.QUOTE_NONE):
    if (row.get("text") or "").strip() == sys.argv[2]:
        print((int(row["left"]) + int(row["width"]) // 2) // 2, (int(row["top"]) + int(row["height"]) // 2) // 2)
        break
else:
    sys.exit(f"{sys.argv[2]!r} not found by OCR")
PY
}
expect() {
  local name=$1; shift
  for text in "$@"; do
    grep -qiE "$text" "artifacts/mentions-$name.txt" || { echo "mentions-$name lacks: $text" >&2; exit 1; }
  done
}

start --project "$project" --light
# Open broken.yaml and trust the project, as capture-diagnostic-hover.sh does, so the
# language server runs and reports a problem.
click 944 72
sleep 2
click 1100 145
xdotool type --clearmodifiers broken.yaml
sleep 0.3
click 1130 184
sleep 1
click 1160 684
sleep 0.4
click 250 210
for _ in $(seq 1 150); do
  grep -q textDocument/didOpen "$PI_LSP_TEST_LOG" 2>/dev/null && break
  sleep 0.1
done
grep -q textDocument/didOpen "$PI_LSP_TEST_LOG"
# A shell with some output, then back to the thread.
xdotool key ctrl+grave
sleep 2
xdotool type --clearmodifiers 'echo hello-from-the-terminal'
xdotool key Return
sleep 1
xdotool key ctrl+grave
sleep 0.5
shot file
ocr file
read -r x y < <(word file THREAD)
click "$x" "$y"
sleep 1

xdotool type --clearmodifiers 'Compare @'
sleep 1.5
shot menu
ocr menu
expect menu Files Sessions Terminal Problems 'opencode\.ts' 'Saved elsewhere' 'hello|last output' 'in broken\.yaml'
xdotool type --clearmodifiers 'br'
sleep 1.5
shot symbols
ocr symbols
expect symbols Symbols 'broken\.yaml'
xdotool key BackSpace BackSpace
xdotool type --clearmodifiers 'op'
sleep 1.5
shot files
ocr files
# The highlighted file's preview: its path, size and first lines.
expect files 'line.{0,4}TypeScript' 'packages/ai/src/providers'
xdotool key Return
sleep 0.5
xdotool type --clearmodifiers 'with the tests'
sleep 0.5
shot chip
ocr chip
expect chip 'IN THIS PROMPT' 'pi reads it'
xdotool key Return
for _ in $(seq 1 50); do
  [[ -s "$PI_FILES_PROMPT_LOG" ]] && break
  sleep 0.1
done
python3 - "$PI_FILES_PROMPT_LOG" <<'PY'
import json, re, sys
prompts = [json.loads(line) for line in open(sys.argv[1])]
assert len(prompts) == 1, prompts
assert re.fullmatch(r"Compare @packages/ai/src/providers/(opencode|openai-completions)\.ts with the tests", prompts[0]), prompts
PY
if grep -Eq 'panicked at' artifacts/mentions-app.log; then echo 'The app panicked' >&2; exit 1; fi
quit

# Offline demo: a sent message keeps its chip. The demo's first session is busy, so
# the message goes to a new one.
start --demo --light
shot demo
ocr demo
read -r x y < <(word demo New)
click "$x" "$y"
sleep 1.5

xdotool type --clearmodifiers 'Compare @op'
sleep 1.5
xdotool key Return
sleep 0.3
xdotool type --clearmodifiers 'with its tests'
xdotool key Return
sleep 1.5
shot sent
ocr sent
expect sent 'Compare' 'openai-comp' 'with its tests'
quit
echo 'PASS: @ menu lists files with a preview, symbols, problems, a saved session and terminal output; a choice becomes a chip; pi receives the path; the sent message keeps its chip; no downloads or model calls.'
