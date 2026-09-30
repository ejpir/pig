#!/usr/bin/env bash
# Native problem card for an LSP diagnostic, using a local deterministic server (no npm/model calls).
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
mkdir -p artifacts
binary="$(realpath "${PI_DESKTOP_BINARY:-${CARGO_TARGET_DIR:-target/linux}/debug/pi-desktop}")"
tmp="$(mktemp -d /tmp/pi-diagnostic.XXXXXX)"
app=""
cleanup() { if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi; rm -rf "$tmp"; }
trap cleanup EXIT
export HOME="$tmp/home" XDG_DATA_HOME="$tmp/data" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$tmp/project/.zed"
chmod 700 "$XDG_RUNTIME_DIR"
printf 'broken: [\n' > "$tmp/project/broken.yaml"
export PI_LSP_TEST_LOG="$tmp/lsp.jsonl" npm_config_offline=true
python3 - "$tmp/project/.zed/settings.json" "$repo/fixtures/diagnostic-lsp.py" "$PI_LSP_TEST_LOG" <<'PY'
import json,sys
json.dump({"prettier":{"allowed":False},"languages":{"YAML":{"prettier":{"allowed":False},"formatter":"language_server"}},"lsp":{"yaml-language-server":{"binary":{"path":sys.argv[2],"arguments":[],"env":{"PI_LSP_TEST_LOG":sys.argv[3]}}}}},open(sys.argv[1],"w"))
PY
export LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan
for driver in /usr/share/vulkan/icd.d/lvp*.json; do
  if [[ -f "$driver" ]]; then export VK_DRIVER_FILES="$driver" VK_ICD_FILENAMES="$driver"; break; fi
done
unset WAYLAND_DISPLAY PI_DESKTOP_RPC_ENTRY
export PI_DESKTOP_PI="$repo/fixtures/files-rpc.py" PI_FILES_COMMAND_LOG="$tmp/commands.log" PI_FILES_PROMPT_LOG="$tmp/prompts.jsonl"
"$binary" --project "$tmp/project" --light > artifacts/diagnostic-app.log 2>&1 &
app=$!
window=""
for _ in $(seq 1 100); do
  kill -0 "$app" 2>/dev/null || { echo 'App exited; see diagnostic-app.log' >&2; exit 1; }
  window=$(xdotool search --onlyvisible --name '^pi desktop$' 2>/dev/null | head -1 || true)
  [[ -n "$window" ]] && break
  sleep 0.1
done
[[ -n "$window" ]]
xdotool windowfocus --sync "$window"
sleep 2
click() { xdotool mousemove --window "$window" "$1" "$2"; sleep 0.15; xdotool click 1; sleep 0.3; }
shot() { import -window "$window" "artifacts/diagnostic-$1.png"; }
click 944 72
sleep 2
click 1100 145
xdotool type --clearmodifiers broken.yaml
sleep 0.3
click 1130 184
sleep 1
shot before-trust
click 1160 684
sleep 0.4
shot trust
click 250 210
for _ in $(seq 1 150); do
  grep -q textDocument/didOpen "$PI_LSP_TEST_LOG" 2>/dev/null && break
  sleep 0.1
done
shot server
grep -q textDocument/didOpen "$PI_LSP_TEST_LOG"
# The error spans the first six characters on the first line.
xdotool mousemove --window "$window" 304 151
sleep 2
shot hover
# One card (design study 05) instead of Zed's popovers: message and code, the
# server's hover text, its quick fix and Ask pi to fix. Words are found by OCR.
ocr() {
  python3 - "artifacts/diagnostic-$1.png" "$tmp/$1-2x.png" <<'PY'
import sys
from PIL import Image
image = Image.open(sys.argv[1])
image.resize((image.width * 2, image.height * 2)).save(sys.argv[2])
PY
  tesseract "$tmp/$1-2x.png" "artifacts/diagnostic-$1" --psm 11 tsv 2>/dev/null
  tesseract "$tmp/$1-2x.png" "artifacts/diagnostic-$1" --psm 11 2>/dev/null
}
# Prints the window position of the first OCR word starting with $2.
word() {
  python3 - "artifacts/diagnostic-$1.tsv" "$2" <<'PY'
import csv, sys
for row in csv.DictReader(open(sys.argv[1]), delimiter="\t", quoting=csv.QUOTE_NONE):
    if (row.get("text") or "").strip().startswith(sys.argv[2]):
        print((int(row["left"]) + int(row["width"]) // 2) // 2, (int(row["top"]) + int(row["height"]) // 2) // 2)
        break
else:
    sys.exit(f"{sys.argv[2]!r} not found by OCR")
PY
}
ocr hover
# OCR sometimes joins "pi to".
for text in 'Diagnostic hover regression probe' 'local-probe\(invalid-yaml\)' 'broken: sequence' 'Quick fix: Close the sequence' 'Ask pi ?to fix'; do
  grep -qiE "$text" artifacts/diagnostic-hover.txt || { echo "Card lacks: $text" >&2; exit 1; }
done
read -r x y < <(word hover Quick)
xdotool mousemove --window "$window" "$x" "$y"
sleep 0.3
xdotool click 1
for _ in $(seq 1 50); do
  grep -q '"text": "broken: \[\]' "$PI_LSP_TEST_LOG" && break
  sleep 0.1
done
grep -q '"text": "broken: \[\]' "$PI_LSP_TEST_LOG" || { echo 'The quick fix did not edit the buffer' >&2; exit 1; }
shot fixed
# The server still reports the error (it always does), so the card comes back.
xdotool mousemove --window "$window" 600 600
sleep 0.5
xdotool mousemove --window "$window" 304 151
sleep 2
shot ask
ocr ask
read -r x y < <(word ask Ask)
xdotool mousemove --window "$window" "$x" "$y"
sleep 0.3
xdotool click 1
for _ in $(seq 1 50); do
  [[ -s "$PI_FILES_PROMPT_LOG" ]] && break
  sleep 0.1
done
sleep 0.5
shot asked
python3 - "$PI_FILES_PROMPT_LOG" <<'PY'
import json, sys
prompts = [json.loads(line) for line in open(sys.argv[1])]
assert len(prompts) == 1, prompts
assert prompts[0] == "Fix this language server error in `broken.yaml:1:1`:\n\nDiagnostic hover regression probe local-probe(invalid-yaml)", prompts
PY
if grep -Eq 'no rendered diagnostic|panicked at|Failed to start language server|Unknown option: --printenv' artifacts/diagnostic-app.log; then echo 'Diagnostic renderer or startup failed' >&2; exit 1; fi
python3 - "$PI_FILES_COMMAND_LOG" <<'PY'
import sys
commands=set(open(sys.argv[1]).read().splitlines())
assert commands <= {'get_state','get_messages','get_session_stats','list_sessions','get_entries','get_settings','get_commands','prompt'},commands
assert 'prompt' in commands, commands
PY
xdotool key ctrl+q
for _ in $(seq 1 50); do
  if ! kill -0 "$app" 2>/dev/null; then wait "$app"; app=""; echo 'PASS: local LSP diagnostic, hover text and quick fix in one card; the fix edits the buffer; Ask pi to fix sends one prompt; no downloads or model calls; clean exit.'; exit 0; fi
  sleep 0.1
done
echo 'App did not quit' >&2
exit 1
