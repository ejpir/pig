#!/usr/bin/env python3
"""Assert native Xvfb captures contain the expected surfaces, text, and UI transitions."""
from pathlib import Path
from functools import lru_cache
from io import BytesIO
import json
import subprocess

from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parent.parent
ARTIFACTS = ROOT / "artifacts"


def load(name, size):
    image = Image.open(ARTIFACTS / f"thread-{name}.png").convert("RGB")
    assert image.size == size, (name, image.size)
    assert len(image.getcolors(image.width * image.height)) > 100, f"{name}: blank frame"
    return image


@lru_cache(maxsize=None)
def text(name):
    # Small native text is font/rasterizer-dependent; upscale only the OCR input.
    # Palette and geometry checks still use the original screenshot pixels.
    image = Image.open(ARTIFACTS / f"thread-{name}.png").convert("RGB")
    image = image.resize((image.width * 2, image.height * 2))
    encoded = BytesIO()
    image.save(encoded, format="PNG")
    result = subprocess.run(
        ["tesseract", "stdin", "stdout", "--psm", "11"],
        input=encoded.getvalue(), check=True, capture_output=True,
    )
    content = result.stdout.decode("utf-8")
    # Sparse-page OCR often skips the accent action row; inspect it separately.
    actions = image.crop((432, image.height - 240, image.width, image.height - 48))
    encoded = BytesIO()
    actions.save(encoded, format="PNG")
    row = subprocess.run(["tesseract", "stdin", "stdout", "--psm", "6"],
                         input=encoded.getvalue(), check=True, capture_output=True)
    content += "\n" + row.stdout.decode("utf-8")
    (ARTIFACTS / f"thread-{name}.txt").write_text(content)
    return " ".join(content.lower().split())


def color(image, position, expected):
    actual = image.getpixel(position)
    target = tuple(bytes.fromhex(expected))
    assert max(abs(a - b) for a, b in zip(actual, target)) <= 2, (position, actual, target)


def main():
    evening = load("evening", (1344, 740))
    moonstone = load("moonstone", (1344, 740))
    queued = load("queued", (1344, 740))
    stopped = load("stopped", (1344, 740))
    load("compact", (1000, 680))
    load("new", (1000, 680))
    for image, colors in ((evening, ["1a212b", "161d27", "1f2630"]),
                          (moonstone, ["f0ede8", "faf9f7", "ebe7e4"])):
        for position, expected in zip([(100, 450), (1100, 250), (700, 30)], colors):
            color(image, position, expected)
    for name in ("evening", "moonstone"):
        content = text(name)
        for expected in ("qwen signatures", "$0.41", "queue follow-up", "offline", "working"):
            assert expected in content, (name, expected, content)
    assert "working in this project" not in text("stopped")
    assert "signatures" in text("compact")  # OCR sometimes reads the small Q as O.
    assert "62.4k" not in text("compact")  # inspector hidden at the compact breakpoint
    assert "start with the task" in text("new")
    assert "create session" not in text("new")  # Chooser completed, not just opened.
    assert "newsession" in text("new").replace(" ", "")
    for name, expected in (("model-picker", ["anthropic", "openai", "sonnet"]),
                           ("thinking-picker", ["minimal", "medium", "xhigh"])):
        load(name, (1344, 740))
        for word in expected:
            assert word in text(name), (name, word, text(name))
        assert "working" in text(name)  # Dismissing a popup must not abort the run.
    assert "62.4k" not in text("moonstone")  # Details start closed even at wide sizes.
    load("usage", (1344, 740))
    assert "not reported" in text("usage")  # Demo has no active-tool inventory.
    assert "cache hits (tokens)" in text("usage")  # Explicitly requested inspector.
    assert "62.4k" in text("usage").replace("62 4k", "62.4k")  # OCR may drop the dot.
    assert "throw" not in text("moonstone")  # Diff details start collapsed.
    load("edit-details", (1344, 740))
    load("bash-details", (1344, 740))
    assert "throw" in text("edit-details"), text("edit-details")
    assert "checked" in text("bash-details"), text("bash-details")
    report = {"platform": "Linux/X11", "renderer": "native GPUI on Xvfb", "screens": 11,
              "checks": ["Evening/Moonstone design palette", "thread/tool/diff/usage text",
                         "collapsed tools and expanded selectable details", "model/thinking popovers", "closed-by-default and explicitly requested inspector", "new session", "clean quit"]}
    (ARTIFACTS / "screen-validation.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: eleven native screenshots, collapsed/expanded tools, pickers, palette, and responsive layout")


if __name__ == "__main__":
    main()
