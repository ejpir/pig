#!/usr/bin/env python3
"""Render the Android screens to PNG and compose the overview sheet.

Usage: python3 design/android/render.py [screen-name ...]
Set CHROME to pick a browser. Needs Pillow for the overview. Uses only local
files; makes no network calls.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCREENS = HERE / "screens"
FONTS = HERE.parent.parent / "assets" / "fonts"
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
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ModuleNotFoundError:
        return overview_with_magick(pngs)

    columns, phone_w, phone_h, gap, caption = 6, WIDTH, HEIGHT, 40, 44
    rows = (len(pngs) + columns - 1) // columns
    sheet = Image.new("RGB", (columns * phone_w + (columns + 1) * gap,
                              rows * (phone_h + caption) + (rows + 1) * gap), "#ece7e1")
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.truetype(str(FONTS / "IBMPlexSans-SemiBold.ttf"), 20)
    mask = Image.new("L", (phone_w, phone_h), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, phone_w - 1, phone_h - 1), radius=36, fill=255)
    for index, png in enumerate(pngs):
        x = gap + (index % columns) * (phone_w + gap)
        y = gap + (index // columns) * (phone_h + caption + gap)
        page = png.with_suffix(".html").read_text(encoding="utf-8")
        title = re.search(r"<title>(.*?)</title>", page).group(1)
        draw.text((x + 4, y), title, font=font, fill="#252f3d")
        phone = Image.open(png).convert("RGB").resize((phone_w, phone_h), Image.LANCZOS)
        sheet.paste(phone, (x, y + caption), mask)
        draw.rounded_rectangle((x - 1, y + caption - 1, x + phone_w, y + caption + phone_h), radius=37, outline="#cbc3bb")
    out = HERE / "overview.png"
    sheet.save(out, optimize=True)
    return out


def overview_with_magick(pngs: list[Path]) -> Path:
    """Keep the design renderer useful on machines without Pillow."""
    magick = shutil.which("magick")
    if not magick:
        raise SystemExit("Install Pillow or ImageMagick to compose overview.png.")
    out = HERE / "overview.png"
    with tempfile.TemporaryDirectory(prefix="pi-android-overview-") as directory:
        cards = []
        for index, png in enumerate(pngs):
            page = png.with_suffix(".html").read_text(encoding="utf-8")
            title = re.search(r"<title>(.*?)</title>", page).group(1)
            card = Path(directory) / f"{index:02}.png"
            subprocess.run(
                [magick, str(png), "-resize", f"{WIDTH}x{HEIGHT}!", "-gravity", "north",
                 "-background", "#ece7e1", "-splice", "0x44", "-gravity", "northwest",
                 "-font", str(FONTS / "IBMPlexSans-SemiBold.ttf"), "-pointsize", "20",
                 "-fill", "#252f3d", "-annotate", "+4+10", title, str(card)],
                check=True,
            )
            cards.append(str(card))
        subprocess.run(
            [magick, "montage", "-font", str(FONTS / "IBMPlexSans-Regular.ttf"),
             *cards, "-tile", "6x", "-geometry", "+20+20", "-background", "#ece7e1",
             str(out)],
            check=True,
        )
    return out


def main() -> None:
    wanted = set(sys.argv[1:])
    chrome = browser()
    for page in sorted(SCREENS.glob("*.html")):
        if not wanted or page.stem in wanted:
            print(render(chrome, page).relative_to(HERE))
    if not wanted:
        print(overview(sorted(SCREENS.glob("*.png"))).relative_to(HERE))


if __name__ == "__main__":
    main()
