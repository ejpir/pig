#!/usr/bin/env python3
"""Check native Moonstone geometry/colors against the actual supplied study SVG.

Compare the 1344×740 application frame, not the surrounding presentation sheet.
Text/data and OS window controls deliberately aren't treated as pixel-identical.
"""
import json
from pathlib import Path
import xml.etree.ElementTree as ET

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "artifacts"
SVG = ET.parse(ROOT / "design/desktop-thread-study-light.svg")
RECTS = SVG.findall(".//{http://www.w3.org/2000/svg}rect")
ORIGIN = (48, 130)


def study_rect(x, y, w, h, stroke=False):
    rect = next(r for r in RECTS if all(float(r.get(key, -1)) == value
                for key, value in zip(("x", "y", "width", "height"), (x, y, w, h))))
    color = tuple(bytes.fromhex(rect.get("stroke" if stroke else "fill").removeprefix("#")))
    return (x - ORIGIN[0], y - ORIGIN[1], w, h), color


def bounds_of_color(image, expected, color, margin=24):
    x, y, w, h = expected
    left, top = max(0, x - margin), max(0, y - margin)
    crop = image.crop((left, top, min(image.width, x + w + margin), min(image.height, y + h + margin)))
    mask = Image.new("L", crop.size)
    mask.putdata([255 if max(abs(a - b) for a, b in zip(pixel, color)) <= 1 else 0
                  for pixel in crop.getdata()])
    found = mask.getbbox()
    assert found is not None, ("Missing design color", color, expected)
    x0, y0, x1, y1 = found
    return (left + x0, top + y0, x1 - x0, y1 - y0)


def main():
    actual = Image.open(OUT / "thread-moonstone.png").convert("RGB")
    reference = Image.open(ROOT / "design/desktop-thread-study-light.png").convert("RGB").crop((48, 130, 1392, 870))
    assert actual.size == reference.size == (1344, 740)
    checks = {}
    # Exact semantic surfaces, including ones previously approximated by bar/raised.
    for name, rect, sample in [
        ("toolbar", (48, 130, 1344, 52), (700, 30)),
        ("sidebar", (48, 182, 208, 664), (100, 410)),
        ("status", (48, 846, 1344, 24), (700, 730)),
        ("composer", (276, 700, 768, 134), (700, 630)),
        ("queue", (277, 701, 766, 29), (750, 580)),
    ]:
        _, color = study_rect(*rect)
        assert actual.getpixel(sample) == color, (name, actual.getpixel(sample), color)
        checks[name] = {"color": color}
    # User messages are bordered cards rather than filled bubbles. Tool details now
    # start collapsed and use selectable code blocks when expanded, so neither is
    # asserted against the older study's static message/diff geometry.
    for name, rect in [
        ("composer bounds", (276, 700, 768, 134)),
        ("queue bounds", (277, 701, 766, 29)),
    ]:
        expected, color = study_rect(*rect, stroke=False)
        if name == "composer bounds":
            # Selection/focus outlines are intentionally neutral rather than blue.
            color = (203, 195, 187)
        measured = bounds_of_color(actual, expected, color, margin=3 if name == "composer bounds" else 24)
        error = max(abs(a - b) for a, b in zip(expected, measured))
        assert error <= 3, (name, expected, measured)
        checks[name] = {"expected": expected, "rendered": measured, "max_error_px": error}
    report = {"reference": "design/desktop-thread-study-light.svg", "checks": checks,
              "note": "Live data, native OS controls, neutral selection borders, bordered messages, and collapsed/selectable tool details differ intentionally. Checks cover surfaces and composer/queue geometry, not pixel-perfect text."}
    (OUT / "study-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    comparison = Image.new("RGB", (2688, 764), "#ebe7e4")
    comparison.paste(reference, (0, 24))
    comparison.paste(actual, (1344, 24))
    draw = ImageDraw.Draw(comparison)
    draw.text((12, 6), "REFERENCE · supplied light study (window crop)", fill="#252f3d")
    draw.text((1356, 6), "NATIVE GPUI · Moonstone", fill="#252f3d")
    comparison.save(OUT / "study-comparison.png")
    print("PASS: SVG surfaces and layout landmarks match within 3px")


if __name__ == "__main__":
    main()
