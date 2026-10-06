#!/usr/bin/env python3
"""Render the project browser mock to PNG and compose overview.png.

Usage: python3 design/android/projects/render.py [screen-name ...]
Set CHROME to pick a browser. Needs Pillow for the overview. Uses only local
files; makes no network calls.
"""
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCREENS = HERE / "screens"
FONTS = HERE.parent.parent.parent / "assets" / "fonts"
WIDTH, HEIGHT, SCALE = 412, 915, 2


def browser() -> str:
    shells = sorted(Path.home().glob(".cache/ms-playwright/chromium_headless_shell-*/*/headless_shell"))
    found = os.environ.get("CHROME") or (str(shells[-1]) if shells else None) or shutil.which("chromium")
    if not found:
        raise SystemExit("Set CHROME to a Chromium/headless_shell binary.")
    return found


def render(chrome: str, page: Path) -> Path:
    out = page.with_suffix(".png")
    subprocess.run(
        [chrome, "--headless", "--disable-gpu", "--hide-scrollbars", "--allow-file-access-from-files",
         f"--force-device-scale-factor={SCALE}", f"--window-size={WIDTH},{HEIGHT}",
         "--virtual-time-budget=1500", f"--screenshot={out}", page.as_uri()],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    return out


def overview(pngs: list[Path]) -> Path:
    from PIL import Image, ImageDraw, ImageFont

    columns, gap, caption = 5, 40, 44
    rows = (len(pngs) + columns - 1) // columns
    sheet = Image.new("RGB", (columns * WIDTH + (columns + 1) * gap,
                              rows * (HEIGHT + caption) + (rows + 1) * gap), "#ece7e1")
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.truetype(str(FONTS / "IBMPlexSans-SemiBold.ttf"), 20)
    mask = Image.new("L", (WIDTH, HEIGHT), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, WIDTH - 1, HEIGHT - 1), radius=36, fill=255)
    for index, png in enumerate(pngs):
        x = gap + (index % columns) * (WIDTH + gap)
        y = gap + (index // columns) * (HEIGHT + caption + gap)
        title = re.search(r"<title>(.*?)</title>", png.with_suffix(".html").read_text(encoding="utf-8")).group(1)
        draw.text((x + 4, y), title, font=font, fill="#252f3d")
        phone = Image.open(png).convert("RGB").resize((WIDTH, HEIGHT), Image.LANCZOS)
        sheet.paste(phone, (x, y + caption), mask)
        draw.rounded_rectangle((x - 1, y + caption - 1, x + WIDTH, y + caption + HEIGHT), radius=37, outline="#cbc3bb")
    out = HERE / "overview.png"
    sheet.save(out, optimize=True)
    return out


def main() -> None:
    wanted = set(sys.argv[1:])
    chrome = browser()
    pages = [page for page in sorted(SCREENS.glob("*.html")) if page.stem[:2].isdigit()]
    for page in pages:
        if not wanted or page.stem in wanted:
            print(render(chrome, page).relative_to(HERE))
    if not wanted:
        print(overview([page.with_suffix(".png") for page in pages]).relative_to(HERE))


if __name__ == "__main__":
    main()
