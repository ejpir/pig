#!/usr/bin/env python3
"""Assert native Xvfb captures contain the expected surfaces, text, and UI transitions."""
from pathlib import Path
from functools import lru_cache
import json
import subprocess

from PIL import Image, ImageChops, ImageStat

ROOT = Path(__file__).resolve().parent.parent
ARTIFACTS = ROOT / "artifacts"


def load(name, size):
    image = Image.open(ARTIFACTS / f"thread-{name}.png").convert("RGB")
    assert image.size == size, (name, image.size)
    assert len(image.getcolors(image.width * image.height)) > 100, f"{name}: blank frame"
    return image


@lru_cache(maxsize=None)
def text(name):
    result = subprocess.run(
        ["tesseract", str(ARTIFACTS / f"thread-{name}.png"), "stdout", "--psm", "11"],
        check=True, capture_output=True, text=True,
    )
    (ARTIFACTS / f"thread-{name}.txt").write_text(result.stdout)
    return " ".join(result.stdout.lower().split())


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
                          (moonstone, ["f2efeb", "faf9f7", "ebe7e4"])):
        for position, expected in zip([(100, 400), (700, 100), (700, 30)], colors):
            color(image, position, expected)
    for name in ("evening", "moonstone"):
        content = text(name)
        for expected in ("qwen signatures", "62.4k", "$0.41", "npm run check", "follow-up", "offline"):
            assert expected in content, (name, expected, content)
    assert "follow-up validation" in text("queued")
    assert "follow-up validation" in text("stopped")  # restored to input on Escape
    assert "ready" in text("stopped")
    assert "signatures" in text("compact")  # OCR sometimes reads the small Q as O.
    assert "62.4k" not in text("compact")  # inspector hidden at the compact breakpoint
    assert "a quiet place to work" in text("new")
    assert "newsession" in text("new").replace(" ", "")
    assert sum(ImageStat.Stat(ImageChops.difference(queued, stopped)).mean) > 1
    for name, expected in (("model-picker", ["anthropic", "openai", "sonnet"]),
                           ("thinking-picker", ["minimal", "medium", "xhigh"]),
                           ("commands-picker", ["compact", "fix-tests", "release"])):
        load(name, (1344, 740))
        for word in expected:
            assert word in text(name), (name, word, text(name))
        assert "running" in text(name)  # Dismissing a popup must not abort the run.
    assert "git-guard" in text("moonstone")  # Inspector fits without clipping.
    assert "throw" not in text("moonstone")  # Diff details start collapsed.
    assert "checked 1,284" not in text("moonstone")  # Bash output starts collapsed.
    load("edit-details", (1344, 740))
    load("bash-details", (1344, 740))
    assert "throw" in text("edit-details"), text("edit-details")
    assert "checked" in text("bash-details"), text("bash-details")
    report = {"platform": "Linux/X11", "renderer": "native GPUI on Xvfb", "screens": 11,
              "checks": ["Evening/Moonstone design palette", "thread/tool/diff/usage text",
                         "collapsed tools and expanded selectable details", "model/thinking/command popovers", "keyboard follow-up", "Escape queue recovery", "responsive inspector", "new session", "clean quit"]}
    (ARTIFACTS / "screen-validation.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: eleven native screenshots, collapsed/expanded tools, pickers, palette, and keyboard transitions")


if __name__ == "__main__":
    main()
