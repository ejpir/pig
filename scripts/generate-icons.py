#!/usr/bin/env python3
"""Render the app icon from the pi mark in design/pi_study_common.py (`pimark`).

Writes assets/app-icon/*.png, packaging/macos/AppIcon.icns, and packaging/windows/app-icon.ico.
Requires Pillow. Run from anywhere: python3 scripts/generate-icons.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
MARK = 469.43  # pimark's own coordinate size
# The mark's three polygons, in pimark coordinates, with the study's colours.
SHAPES = [
    ("#F09082", [(0, 0), (352.07, 0), (352.07, 234.71), (234.71, 234.71), (234.71, 117.36), (0, 117.36)]),
    ("#4D9ABF", [(0, 117.36), (117.36, 117.36), (117.36, 234.71), (234.71, 234.71), (234.71, 352.07),
                 (117.36, 352.07), (117.36, 469.43), (0, 469.43)]),
    ("#F1BE58", [(352.07, 234.71), (469.43, 234.71), (469.43, 469.43), (352.07, 469.43)]),
]
EVENING = "#161d27"  # Evening canvas
EDGE = "#2f3640"  # Evening line
SUPERSAMPLE = 4


def icon(size):
    """macOS icon grid: an 824/1024 rounded square with the mark at about half its width."""
    canvas = size * SUPERSAMPLE
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    tile = canvas * 824 / 1024
    inset = (canvas - tile) / 2
    radius = tile * 0.225
    draw.rounded_rectangle((inset, inset, inset + tile, inset + tile), radius, fill=EVENING,
                           outline=EDGE, width=max(1, round(canvas / 256)))
    mark = tile * 0.5
    origin = (canvas - mark) / 2
    scale = mark / MARK
    for color, points in SHAPES:
        draw.polygon([(origin + x * scale, origin + y * scale) for x, y in points], fill=color)
    return image.resize((size, size), Image.Resampling.LANCZOS)


def main():
    assets = ROOT / "assets/app-icon"
    assets.mkdir(parents=True, exist_ok=True)
    for size in (1024, 512, 128):
        icon(size).save(assets / f"app-icon-{size}.png", optimize=True)
    source = icon(1024)
    (ROOT / "packaging/macos").mkdir(parents=True, exist_ok=True)
    source.save(ROOT / "packaging/macos/AppIcon.icns",
                sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
    (ROOT / "packaging/windows").mkdir(parents=True, exist_ok=True)
    icon(256).save(ROOT / "packaging/windows/app-icon.ico",
                   sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print("Wrote assets/app-icon/*.png, packaging/macos/AppIcon.icns, packaging/windows/app-icon.ico")


if __name__ == "__main__":
    main()
