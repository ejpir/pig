#!/usr/bin/env python3
"""Render the visual-workflow HTML mocks to PNG with a local headless Chromium.

Usage: python3 design/visual-workflow/render.py [screen-name ...]
Set CHROME to pick a browser. Uses only local files; makes no network calls.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCREENS = HERE / "screens"


def browser() -> str:
    shells = sorted(Path.home().glob(".cache/ms-playwright/chromium_headless_shell-*/*/headless_shell"))
    found = os.environ.get("CHROME") or (str(shells[-1]) if shells else None) or shutil.which("chromium")
    if not found:
        raise SystemExit("Set CHROME to a Chromium/headless_shell binary.")
    return found


def main() -> None:
    wanted = set(sys.argv[1:])
    chrome = browser()
    for page in sorted(SCREENS.glob("*.html")):
        if wanted and page.stem not in wanted:
            continue
        out = page.with_suffix(".png")
        subprocess.run(
            [chrome, "--headless", "--disable-gpu", "--hide-scrollbars", "--allow-file-access-from-files",
             "--force-device-scale-factor=1", "--window-size=1440,900", "--virtual-time-budget=1500",
             f"--screenshot={out}", page.as_uri()],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        print(out.relative_to(HERE))


if __name__ == "__main__":
    main()
