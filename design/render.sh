#!/usr/bin/env bash
# Regenerate the study SVGs and render PNG previews with headless Chromium.
# Usage: ./render.sh                 (every study, dark and light where available)
#        ./render.sh thread views    (named studies)
set -euo pipefail
cd "$(dirname "$0")"

browser="${CHROME:-}"
if [ -z "$browser" ]; then
	for candidate in "$HOME"/.cache/ms-playwright/chromium_headless_shell-*/*/headless_shell chromium chromium-browser google-chrome \
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"; do
		if command -v "$candidate" >/dev/null 2>&1 || [ -x "$candidate" ]; then
			browser="$candidate"
			break
		fi
	done
fi

render_one() {
	local gen="$1" out="$2" theme="$3"
	python3 "$gen" "$theme" >"$out.svg"
	local size
	size=$(sed -n 's/.*<svg[^>]* width="\([0-9]*\)" height="\([0-9]*\)".*/\1,\2/p' "$out.svg" | head -1)
	if [ -n "$browser" ]; then
		"$browser" --headless --no-sandbox --disable-gpu --disable-dev-shm-usage --hide-scrollbars --window-size="$size" \
			--screenshot="$PWD/$out.png" "file://$PWD/$out.svg" >/dev/null 2>&1
		echo "$out.svg + .png ($size)"
	else
		echo "$out.svg (no Chromium found for PNG)"
	fi
}

render() {
	local name="$1" gen="gen_desktop_${1}_study.py"
	[ -f "$gen" ] || return 0
	render_one "$gen" "desktop-${name}-study" dark
	# Studies 01 and 02 also have a light rendering; studies 03 to 07 show both themes side by side.
	if [ "$name" != "projects" ] && [ "$name" != "workspace" ] && [ "$name" != "jj" ] && [ "$name" != "prefs" ] && [ "$name" != "setup" ]; then
		render_one "$gen" "desktop-${name}-study-light" light
	fi
}

if [ $# -gt 0 ]; then
	for name in "$@"; do render "$name"; done
else
	for name in thread views projects workspace jj prefs setup; do render "$name"; done
fi
