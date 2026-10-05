#!/usr/bin/env python3
"""Compare native Thread landmarks with the approved workbench SVG.

The proposal is 1440x900; the first-screen capture is 1344x740. Width/height
changes move only the right/bottom gutters. Content, controls, composer states
and text are deliberately not a pixel-perfect comparison.
"""
import json
from pathlib import Path
import xml.etree.ElementTree as ET

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "artifacts"
REFERENCE = ROOT / "design/workbench-vision/screens/01-work.svg"
RECTS = ET.parse(REFERENCE).findall(".//{http://www.w3.org/2000/svg}rect")


def reference_rect(x, y, width, height):
    rect = next(r for r in RECTS if all(float(r.get(key, -1)) == value
                for key, value in zip(("x", "y", "width", "height"), (x, y, width, height))))
    return tuple(bytes.fromhex(rect.get("fill").removeprefix("#")))


def bounds_of_color(image, expected, color, margin=4):
    x, y, width, height = expected
    left, top = max(0, x - margin), max(0, y - margin)
    crop = image.crop((left, top, min(image.width, x + width + margin), min(image.height, y + height + margin)))
    mask = Image.new("L", crop.size)
    mask.putdata([255 if max(abs(a - b) for a, b in zip(pixel, color)) <= 1 else 0
                  for pixel in crop.get_flattened_data()])
    found = mask.getbbox()
    assert found is not None, ("Missing neutral composer border", color, expected)
    x0, y0, x1, y1 = found
    return (left + x0, top + y0, x1 - x0, y1 - y0)


def main():
    actual = Image.open(OUT / "thread-moonstone.png").convert("RGB")
    reference = Image.open(REFERENCE.with_suffix(".png")).convert("RGB")
    assert actual.size == (1344, 740) and reference.size == (1440, 900)
    checks = {}
    for name, rect, sample in [
        ("canvas", (0, 0, 1440, 900), (1100, 250)),
        ("sidebar", (0, 36, 216, 840), (100, 450)),
        ("title bar", (0, 0, 1440, 36), (700, 30)),
    ]:
        color = reference_rect(*rect)
        measured = actual.getpixel(sample)
        error = max(abs(a - b) for a, b in zip(measured, color))
        assert error <= 2, (name, measured, color)
        checks[name] = {"reference": color, "native": measured, "max_channel_error": error}
    composer = next(r for r in RECTS if r.get("data-layout") == "fluid-composer")
    x, y, width, height = (int(float(composer.get(k))) for k in ("x", "y", "width", "height"))
    expected = (x, y + actual.height - reference.height,
                width + actual.width - reference.width, height)
    # Preserve the app's neutral keyboard/focus border; do not substitute the
    # illustrative sheet's lighter decorative stroke into production.
    measured = bounds_of_color(actual, expected, (203, 195, 187))
    error = max(abs(a - b) for a, b in zip(expected, measured))
    assert error <= 3, ("fluid composer", expected, measured)
    checks["fluid composer"] = {"reference_at_capture_size": expected, "native": measured, "max_error_px": error}
    assert measured[0] == 216 + 24 and actual.width - measured[0] - measured[2] == 24
    report = {"reference": str(REFERENCE.relative_to(ROOT)), "checks": checks,
              "note": "Two different viewport sizes and sample transcripts. Only warm surfaces and fluid composer gutters/bounds are compared. Native status-bar color, neutral borders and content remain intentional differences."}
    (OUT / "study-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    comparison = Image.new("RGB", (reference.width + actual.width, reference.height + 24), "#ebe7e4")
    comparison.paste(reference, (0, 24))
    comparison.paste(actual, (reference.width, 24))
    draw = ImageDraw.Draw(comparison)
    draw.text((12, 6), "PROPOSAL · workbench SVG · 1440x900", fill="#252f3d")
    draw.text((reference.width + 12, 6), "NATIVE GPUI · Moonstone · 1344x740", fill="#252f3d")
    comparison.save(OUT / "study-comparison.png")
    print("PASS: approved workbench surfaces and fluid gutters; composer landmarks within 3px")


if __name__ == "__main__":
    main()
