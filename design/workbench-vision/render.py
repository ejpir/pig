#!/usr/bin/env python3
"""Draw an independent, task-first Pi Desktop concept, not application captures.

Run from any directory. Uses repository fonts/icons, Pillow and a local Chromium.
The master SVG embeds its UI/code fonts and vector artwork; no network is used.
"""
from base64 import b64encode
from functools import lru_cache
from html import escape
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from xml.etree import ElementTree as ET

from PIL import Image, ImageFont

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUT = HERE / "screens"
MASTER = ROOT / "design/desktop-workbench-vision.svg"
SANS = "VisionPlex,sans-serif"
MONO = "VisionMono,monospace"
SERIF = "Georgia,DejaVu Serif,serif"
SIDEBAR = 216
GUTTER = 24
LIGHT = {
    "canvas": "#faf9f7", "sidebar": "#f0ede8", "bar": "#ebe7e2",
    "surface": "#ffffff", "line": "#ddd7d0", "selected": "#e3dfd9",
    "text": "#252f3d", "secondary": "#3e4753", "muted": "#665f59",
    "accent": "#4b607c", "on_accent": "#ffffff", "green": "#2e7950",
    "amber": "#8b631f", "queue": "#f8f3e9", "code": "#f3f1ed",
    "add": "#e4eee5", "remove": "#f2e4df", "red": "#a44835",
}
DARK = {
    "canvas": "#161d27", "sidebar": "#1a212b", "bar": "#1f2630",
    "surface": "#202731", "line": "#363d46", "selected": "#2d3239",
    "text": "#ebe7e4", "secondary": "#d5d8db", "muted": "#9ca2aa",
    "accent": "#8caecb", "on_accent": "#111820", "green": "#80bf95",
    "amber": "#e5bd73", "queue": "#292b2d", "code": "#1b232e",
    "add": "#243b30", "remove": "#422e30", "red": "#efa08c",
}
FONT_FILES = [
    ("VisionPlex", "IBMPlexSans-Regular.ttf", 400),
    ("VisionPlex", "IBMPlexSans-SemiBold.ttf", 600),
    ("VisionMono", "CommitMono-Regular.otf", 400),
]


def font_css(embedded=False):
    rules = []
    for family, filename, weight in FONT_FILES:
        if embedded:
            data = b64encode((ROOT / "assets/fonts" / filename).read_bytes()).decode()
            mime = "font/otf" if filename.endswith(".otf") else "font/ttf"
            url = f"data:{mime};base64,{data}"
        else:
            url = f"../../../assets/fonts/{filename}"
        rules.append(
            f"@font-face{{font-family:{family};font-weight:{weight};src:url('{url}')}}"
        )
    return "<style>" + "\n".join(rules) + "</style>"


@lru_cache(maxsize=None)
def font(size, family=SANS, weight=400):
    if family == MONO:
        path = ROOT / "assets/fonts/CommitMono-Regular.otf"
    elif family == SERIF:
        path = Path("/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf")
        if not path.exists():
            path = ROOT / "assets/fonts/IBMPlexSans-Italic.ttf"
    else:
        name = "SemiBold" if weight == 600 else "Regular"
        path = ROOT / f"assets/fonts/IBMPlexSans-{name}.ttf"
    return ImageFont.truetype(str(path), size)


def fit(value, width, size=13, family=SANS, weight=400):
    measure = font(size, family, weight)
    if measure.getlength(value) <= width:
        return value
    while value and measure.getlength(value + "…") > width:
        value = value[:-1]
    return value.rstrip() + "…"


class Canvas:
    """Small vector drawing surface with layout assertions and no raster content."""

    def __init__(self, name, width=1440, height=900, dark=False):
        self.name, self.width, self.height = name, width, height
        self.colors = DARK if dark else LIGHT
        self.parts = []
        self.work_end = width
        self.composer_bounds = None
        self.text_bounds = []

    @property
    def left(self):
        return SIDEBAR + GUTTER

    @property
    def right(self):
        return self.work_end - GUTTER

    def color(self, name):
        return self.colors.get(name, name)

    def rect(self, x, y, width, height, fill, radius=0, stroke=None, extra=""):
        assert width >= 0 and height >= 0
        border = f' stroke="{self.color(stroke)}"' if stroke else ""
        self.parts.append(
            f'<rect x="{x}" y="{y}" width="{width}" height="{height}" '
            f'rx="{radius}" fill="{self.color(fill)}"{border} {extra}/>'
        )

    def line(self, x, y, end, bottom=None, color="line", width=1):
        bottom = y if bottom is None else bottom
        self.parts.append(
            f'<path d="M{x} {y} L{end} {bottom}" fill="none" '
            f'stroke="{self.color(color)}" stroke-width="{width}"/>'
        )

    def text(self, x, y, value, size=13, color="text", weight=400,
             family=SANS, anchor="start", italic=False, max_width=None):
        measured = font(size, family, weight).getlength(value)
        if max_width is not None:
            assert measured <= max_width + 1, (self.name, value, measured, max_width)
        left = x - measured if anchor == "end" else x - measured / 2 if anchor == "middle" else x
        self.text_bounds.append((left, y, measured, value))
        style = f"font-family:{family};white-space:pre;"
        if italic:
            style += "font-style:italic;"
        self.parts.append(
            f'<text x="{x}" y="{y}" fill="{self.color(color)}" '
            f'font-size="{size}" font-weight="{weight}" text-anchor="{anchor}" '
            f'style="{style}">{escape(value)}</text>'
        )

    def title(self, x, y, value, size=27):
        self.text(x, y, value, size, family=SERIF, italic=True)

    def dot(self, x, y, color="muted", radius=3):
        self.parts.append(
            f'<circle cx="{x}" cy="{y}" r="{radius}" fill="{self.color(color)}"/>'
        )

    def icon(self, name, x, y, color="muted", size=16):
        source = (ROOT / "assets/icons" / f"{name}.svg").read_text()
        source = re.sub(
            r'(fill|stroke)="(?!none)[^"]+"',
            lambda match: f'{match[1]}="{self.color(color)}"', source,
        )
        opening, body = source.split(">", 1)
        opening = re.sub(r'\s(?:width|height)="[^"]+"', "", opening)
        opening = opening.replace(
            "<svg ", f'<svg x="{x}" y="{y}" width="{size}" height="{size}" ', 1,
        )
        self.parts.append(opening + ">" + body)

    def icon_button(self, name, x, y, selected=False, label=""):
        self.parts.append(f'<g><title>{escape(label or name)}</title>')
        if selected:
            self.rect(x, y, 28, 28, "selected", 5)
        self.icon(name, x + 6, y + 6)
        self.parts.append("</g>")

    def button(self, x, y, width, label, primary=False, disabled=False, height=30):
        fill = "selected" if disabled else "accent" if primary else "surface"
        ink = "muted" if disabled else "on_accent" if primary else "text"
        self.rect(x, y, width, height, fill, 5, None if primary or disabled else "line")
        self.text(x + width / 2, y + height / 2 + 4, label, 13, ink,
                  600 if primary else 400, anchor="middle", max_width=width - 14)

    def disclosure(self, x, y, label, value="", end=None):
        self.icon("chevron_right", x, y - 12, size=14)
        self.text(x + 22, y, label)
        if value:
            self.text(end or self.right, y, value, 12, "muted", anchor="end")

    def spinner(self, x, y):
        self.parts.append(
            f'<circle cx="{x}" cy="{y}" r="5" fill="none" '
            f'stroke="{self.color("line")}" stroke-width="1.5"/>'
        )
        self.parts.append(
            f'<path d="M{x} {y - 5} A5 5 0 0 1 {x + 5} {y}" fill="none" '
            f'stroke="{self.color("accent")}" stroke-width="1.7" stroke-linecap="round"/>'
        )

    def document(self):
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.width}" '
            f'height="{self.height}" viewBox="0 0 {self.width} {self.height}">\n'
            '<title>Pi Desktop — workbench vision, static design proposal</title>\n'
            '<desc>Illustrative content, not application output or evidence of executed actions.</desc>\n'
            + font_css() + "\n" + "\n".join(self.parts) + "\n</svg>\n"
        )

    def save(self):
        for left, y, width, text in self.text_bounds:
            assert left >= -1 and left + width <= self.width + 1, (self.name, text)
            assert 0 <= y <= self.height, (self.name, text)
        path = OUT / f"{self.name}.svg"
        path.write_text(self.document())
        ET.parse(path)
        return path


def shell(s, active="Thread", inspector=False, new=False):
    """Only the project/session hierarchy is permanent navigation."""
    width, height = s.width, s.height
    s.work_end = width - 320 if inspector else width
    s.rect(0, 0, width, height, "canvas")
    s.rect(0, 36, SIDEBAR, height - 60, "sidebar")
    s.rect(0, 0, width, 36, "bar")
    s.line(0, 35.5, width)
    s.line(SIDEBAR - .5, 36, SIDEBAR - .5, height - 24)
    s.icon_button("threads_sidebar_left_open", 8, 4, label="Toggle sidebar")
    s.icon_button("magnifying_glass", 40, 4, label="Search and commands · Ctrl K")
    s.icon_button("plus", width - 72, 4, label="New session")
    s.icon_button("threads_sidebar_right_open" if inspector else "threads_sidebar_right_closed",
                  width - 36, 4, label="Toggle details")
    s.text(20, 65, "Projects", 12, "muted", 600)
    s.icon_button("plus", 176, 43, label="Open a project folder")
    s.icon("chevron_down", 17, 83, size=12)
    s.icon("folder", 35, 81)
    s.text(60, 96, "pi", 14, weight=600)
    s.rect(8, 112, 200, 34, "selected", 5)
    s.icon("thread", 21, 121, "secondary", 14)
    s.text(44, 134, "New session" if new else "Qwen signatures", 13, weight=600)
    for y, label in [(166, "Streaming retry"), (198, "LSP shutdown")]:
        s.icon("thread", 21, y - 13, size=14)
        s.text(44, y, label, 13, "secondary")
    s.text(44, 234, "12 saved sessions…", 12, "muted")
    for y, label in [(280, "zed"), (316, "minivm")]:
        s.icon("chevron_right", 17, y - 12, size=12)
        s.icon("folder", 35, y - 14)
        s.text(60, y, label, 14)
    s.icon("folder_add", 20, 354)
    s.text(44, 368, "Open folder…", 13, "muted")
    s.line(16, height - 78, 200)
    s.icon("settings", 20, height - 60)
    s.text(44, height - 47, "Settings & tools", 13)

    # No permanent catalog destinations and no metrics toolbar.
    for x, label, key, tab_width in [(240, "Thread", "Thread", 43),
                                     (316, "Changes  2", "Changes", 78)]:
        if new and key == "Changes":
            continue
        s.text(x, 61, label, 13, "text" if active == key else "muted",
               600 if active == key else 400)
        if active == key:
            s.line(x, 74, x + tab_width, color="secondary", width=2)
    if active == "File":
        s.icon("file", 424, 47, size=14)
        s.text(446, 61, "openai-completions.ts", 13, weight=600)
        s.icon_button("close", 589, 40, label="Close file")
        s.line(424, 74, 616, color="secondary", width=2)
    s.text(s.work_end - 62, 61, "pi / main", 12, "muted", family=MONO, anchor="end")
    s.icon_button("terminal", s.work_end - 48, 40, label="Open terminal")
    s.line(SIDEBAR, 75.5, s.work_end)
    if inspector:
        s.rect(width - 320, 36, 320, height - 60, "sidebar")
        s.line(width - 320, 36, width - 320, height - 24)
    s.rect(0, height - 24, width, 24, "bar")
    s.line(0, height - 24, width)
    s.dot(18, height - 12)
    s.text(29, height - 8, "Local process", 12, "muted")
    s.text(width - 16, height - 8, "Ctrl K  ·  Search & commands", 12, "muted", anchor="end")


def composer(s, state="working", draft="", queue=None, y=None, context=None, revision=False):
    """One stable, fluid input surface; important actions do not need details."""
    y = s.height - 190 if y is None else y
    left, right = s.left, s.right
    s.composer_bounds = [left, y, right - left, 142]
    s.parts.append('<g data-component="composer">')
    if state == "working":
        s.spinner(left + 6, y - 23)
        s.text(left + 22, y - 18, "Working", 13, weight=600)
        s.text(left + 89, y - 18, "Checking the workspace", 13, "muted")
        s.button(right - 72, y - 40, 72, "Stop", height=30)
    elif state == "finished":
        s.icon("check", left, y - 35, "green", 16)
        s.text(left + 24, y - 21, "Finished", 13, weight=600)
        s.text(left + 94, y - 21, "Workspace checks passed", 13, "muted")
    else:
        s.dot(left + 6, y - 24)
        s.text(left + 22, y - 19, "Ready", 13, weight=600)
        s.text(left + 76, y - 19, "pi · project folder", 13, "muted")
    s.rect(left, y, right - left, 142, "surface", 7, "line",
           extra='data-layout="fluid-composer"')
    if queue:
        s.rect(left + 1, y + 1, right - left - 2, 29, "queue", 6)
        s.text(left + 14, y + 20, "Next", 12, "amber", 600)
        s.text(left + 57, y + 20, queue, 12, "secondary")
        s.icon_button("close", right - 32, y + 1, label="Remove queued follow-up")
        s.line(left + 1, y + 30, right - 1)
    elif context:
        s.icon("file", left + 13, y + 11, size=14)
        s.text(left + 34, y + 23, context, 12, "muted")
    s.text(left + 16, y + 61 if queue or context else y + 35,
           draft or "Describe a change, ask a question, or drop a file…",
           15, "text" if draft else "muted", max_width=right - left - 32)
    s.icon_button("attach", left + 8, y + 101, label="Attach files")
    s.text(left + 52, y + 121, "Claude Opus", 13)
    s.icon("chevron_down", left + 132, y + 109, size=12)
    s.text(left + 164, y + 121, "High", 13, "muted")
    s.icon("chevron_down", left + 196, y + 109, size=12)
    if state == "working":
        s.button(right - 292, y + 98, 114, "Steer now")
        s.button(right - 166, y + 98, 150, "Queue follow-up ↵", primary=True)
    else:
        label = "Send revision ↵" if revision else "Send ↵"
        width = 148 if revision else 98
        s.button(right - width - 16, y + 98, width, label, primary=True,
                 disabled=not bool(draft))
    s.parts.append("</g>")


def work(dark=False):
    s = Canvas("05-work-evening" if dark else "01-work")
    if dark:
        s.colors = DARK
    shell(s)
    left, right = s.left, s.right
    s.text(left, 111, "You", 12, "muted", 600)
    s.text(left + 36, 111, "10:24", 12, "muted")
    s.text(left, 143, "OpenCode returns empty thinking signatures. Accept them there,", 15)
    s.text(left, 168, "keep Anthropic strict, and add a regression test without a live API key.", 15)
    s.line(left, 196, right)
    s.title(left, 233, "One narrow fix. One regression test.")
    s.text(left, 264, "I’ve kept the signature check on the Anthropic path and added a mocked-provider test.", 15,
           "secondary", max_width=760)
    s.disclosure(left, 302, "Tool activity", "Read · search · 2 edits", end=right)
    s.text(left, 346, "Changed files", 13, weight=600)
    s.text(right, 346, "Review changes →", 13, "accent", anchor="end")
    s.line(left, 359, right)
    files = [("openai-completions.ts", "packages/ai/src/providers", "+1 −1"),
             ("openai-completions.test.ts", "packages/ai/test", "+24")]
    for index, (name, folder, delta) in enumerate(files):
        y = 387 + index * 65
        s.icon("file", left + 2, y - 13)
        s.text(left + 29, y, name, 14, weight=600)
        s.text(left + 29, y + 21, folder, 12, "muted", family=MONO)
        s.text(right - 30, y + 6, delta, 13, "green", family=MONO, anchor="end")
        s.icon("chevron_right", right - 14, y - 5, size=14)
        s.line(left, y + 34, right)
    s.icon("terminal", left + 1, 509, size=15)
    s.text(left + 26, 523, "npm run check", 13, family=MONO)
    s.icon("chevron_up", right - 15, 509, size=14)
    s.rect(left, 537, right - left, 106, "code", 5)
    for y, text, color in [
        (562, "✓ TypeScript", "green"),
        (586, "✓ Provider regression tests · 12 passed", "green"),
        (614, "  Checking the remaining workspace packages…", "secondary"),
    ]:
        s.text(left + 16, y, text, 13, color, family=MONO)
    composer(s, draft="Also cover the streaming path with a mock provider.",
             queue="Add a release note once checks finish")
    return s


BEFORE = [
    (208, "case 'thinking': {", None),
    (209, "  const signature = block.signature;", None),
    (210, "", None),
    (211, "  if (!signature) {", "remove"),
    (212, "    throw new MissingSignature(model.id);", None),
    (213, "  }", None),
    (214, "", None),
    (215, "  return {", None),
    (216, "    type: 'thinking',", None),
    (217, "    thinking: block.thinking,", None),
    (218, "    signature,", None),
    (219, "  };", None),
    (220, "}", None),
]
AFTER = [
    (208, "case 'thinking': {", None),
    (209, "  const signature = block.signature;", None),
    (210, "", None),
    (211, "  if (!signature && isAnthropic(model)) {", "add"),
    (212, "    throw new MissingSignature(model.id);", None),
    (213, "  }", None),
    (214, "", None),
    (215, "  return {", None),
    (216, "    type: 'thinking',", None),
    (217, "    thinking: block.thinking,", None),
    (218, "    signature,", None),
    (219, "  };", None),
    (220, "}", None),
]


def code_column(s, left, top, width, rows, label):
    s.rect(left, top, width, 30, "code", 4)
    s.text(left + 12, top + 20, label, 12, "muted")
    for index, (number, content, change) in enumerate(rows):
        y = top + 31 + index * 24
        if change:
            s.rect(left, y, width, 24, change)
        s.text(left + 35, y + 17, str(number), 12, "muted", family=MONO, anchor="end")
        s.text(left + 52, y + 17, content, 13, "secondary", family=MONO, max_width=width - 64)
    s.line(left, top + 31 + len(rows) * 24, left + width)


def review(compact=False):
    s = Canvas("07-review-compact" if compact else "02-review-wide",
               1000 if compact else 1600, 720 if compact else 900)
    shell(s, active="Changes")
    if compact:
        left, right = s.left, s.right
        s.button(left, 90, 292, "openai-completions.ts  ⌄")
        s.text(left + 311, 111, "1 of 2 files", 12, "muted")
        s.button(right - 107, 90, 107, "Open file")
        s.text(left, 151, "Tool-reported edit · not the full working-tree diff", 12, "muted")
        s.icon("info", left, 168, "amber", 14)
        s.text(left + 22, 181, "No snapshot for this edit. Restore is unavailable.", 12, "amber")
        rows = [(208, "case 'thinking': {", None),
                (209, "  const signature = block.signature;", None),
                (211, "− if (!signature) {", "remove"),
                (211, "+ if (!signature && isAnthropic(model)) {", "add"),
                (212, "    throw new MissingSignature(model.id);", None),
                (213, "  }", None),
                (214, "  …", None)]
        code_column(s, left, 207, right - left, rows, "Unified diff")
        s.text(left, 452, "Observed edits are already on disk.", 12, "muted")
        composer(s, state="finished", draft="Add the strict Anthropic case to the regression test too.",
                 context="openai-completions.ts", revision=True)
        return s

    # A file rail belongs to reviewing changes, not to every screen.
    s.line(416, 76, 416, 876)
    s.text(236, 110, "Observed files · 2", 12, "muted", 600)
    for index, (name, delta) in enumerate([
        ("openai-completions.ts", "+1 −1"), ("openai-completions.test.ts", "+24"),
    ]):
        y = 127 + index * 76
        if index == 0:
            s.rect(224, y, 184, 66, "selected", 5)
        s.text(236, y + 25, fit(name, 160, 12), 12, weight=600 if index == 0 else 400)
        s.text(236, y + 48, delta, 12, "green", family=MONO)
    s.text(236, 824, "+25 −1", 13, "green", family=MONO)
    s.text(236, 850, "Tool-reported changes", 12, "muted")
    left, right = 440, s.right
    s.text(left, 113, "openai-completions.ts", 19, weight=600)
    s.text(left, 138, "packages/ai/src/providers", 12, "muted", family=MONO)
    s.button(right - 218, 94, 101, "Split diff  ⌄")
    s.button(right - 105, 94, 105, "Open file")
    s.icon("info", left, 162, "amber", 14)
    s.text(left + 22, 175, "Observed edit, already on disk · not the full working tree · no snapshot to restore", 12, "amber")
    width = (right - left - 16) / 2
    code_column(s, left, 203, width, BEFORE, "Before")
    code_column(s, left + width + 16, 203, width, AFTER, "After")
    s.text(left, 586, "Only the provider guard changes. The test file covers the empty-signature case.", 13, "secondary")
    s.text(left, 617, "Select lines to include them in a revision request.", 12, "muted")
    # The input shares the diff's work area, not the file rail.
    s.parts.append('<g data-layout="review-composer" transform="translate(200 0)">')
    original_end = s.work_end
    s.work_end -= 200
    composer(s, state="finished", draft="Add the strict Anthropic case to the regression test too.",
             context="openai-completions.ts", revision=True)
    s.work_end = original_end
    s.composer_bounds[0] += 200
    s.parts.append("</g>")
    return s


def file_details():
    s = Canvas("03-file-details")
    shell(s, active="File", inspector=True)
    left, right = s.left, s.right
    s.text(left, 112, "packages / ai / src / providers / openai-completions.ts", 12, "muted", family=MONO)
    s.text(left, 144, "Current file", 13, weight=600)
    s.text(right, 144, "Read-only preview", 12, "muted", anchor="end")
    rows = [(206, "function readThinkingBlock(block, model) {", None),
            (207, "  switch (block.type) {", None)]
    rows += [(number, "    " + value, "selected" if 211 <= number <= 213 else None)
             for number, value, _ in AFTER]
    rows += [(221, "  }", None), (222, "}", None)]
    code_column(s, left, 166, right - left, rows, "TypeScript · UTF-8")
    s.text(left, 658, "Use Changes to compare this file with the observed edit.", 12, "muted")
    composer(s, state="finished", draft="Explain why the Anthropic check must stay strict.",
             context="openai-completions.ts · lines 211–213")

    x, end = s.width - 296, s.width - 24
    s.title(x, 78, "Selected edit", 23)
    s.icon_button("close", end - 24, 53, label="Close details and restore focus")
    s.text(x, 109, "openai-completions.ts", 13, weight=600)
    s.line(x, 132, end)
    s.text(x, 163, "From this session", 12, "muted", 600)
    s.text(x, 193, "Qwen signatures", 14)
    s.text(x, 220, "Recorded edit · 10:26", 12, "muted")
    s.text(x, 254, "Reveal in thread →", 13, "accent")
    s.line(x, 279, end)
    s.text(x, 311, "File history", 13, weight=600)
    s.text(x, 340, "No snapshot was recorded for this edit.", 12, "muted")
    s.text(x, 363, "Restore is unavailable.", 12, "amber")
    s.line(x, 390, end)
    s.disclosure(x, 424, "Full path", end=end)
    s.disclosure(x, 463, "Session context", "31% used", end=end)
    s.disclosure(x, 502, "Session tree", end=end)
    s.disclosure(x, 541, "Run details", end=end)
    s.text(x, 832, "Details are optional.", 12, "muted")
    s.text(x, 854, "File and revision actions stay in the work area.", 12, "muted", max_width=272)
    return s


def waiting():
    s = Canvas("04-waiting")
    shell(s)
    left, right = s.left, s.right
    s.text(left, 115, "Question from the running extension", 12, "muted")
    s.title(left, 164, "How should the regression test run?", 29)
    s.text(left, 204, "A mocked provider keeps this test deterministic. A live endpoint would need", 15, "secondary")
    s.text(left, 230, "credentials and network access. Choose the approach before continuing.", 15, "secondary")
    choices = [
        (286, "Use a mock provider", "Local fixture; no provider request is needed for this test.", True),
        (390, "Use a live endpoint", "Requires configured credentials; a provider request may incur a charge.", False),
    ]
    for y, title, detail, selected in choices:
        if selected:
            s.rect(left, y, right - left, 86, "selected", 6)
        else:
            s.line(left, y + 86, right)
        s.parts.append(
            f'<circle cx="{left + 22}" cy="{y + 30}" r="7" fill="none" '
            f'stroke="{s.color("muted")}" stroke-width="1.2"/>'
        )
        if selected:
            s.dot(left + 22, y + 30, "text", 3.5)
        s.text(left + 46, y + 35, title, 15, weight=600 if selected else 400)
        s.text(left + 46, y + 61, detail, 13, "muted")
    s.text(left, 522, "Use ↑ ↓ to choose. Enter submits the selected answer. Escape cancels this question.", 12, "muted")
    s.disclosure(left, 568, "Question details", "provider-regression", end=right)
    s.line(left, 710, right)
    s.icon("info", left, 735, "amber", 17)
    s.text(left + 27, 749, "Waiting for you", 14, weight=600)
    s.text(left, 778, "The extension is waiting for a reply. Selecting an option does not submit it.", 13, "muted")
    s.button(right - 327, 817, 136, "Cancel question")
    s.button(right - 179, 817, 179, "Continue with mock", primary=True)
    return s


def tools():
    s = work()
    s.name = "06-tools"
    # The underlying project/session list is unchanged by command search.
    s.rect(0, 36, s.width, s.height - 60, "#111820", extra='opacity="0.18"')
    x, y, width = 406, 96, 628
    s.rect(x - 4, y + 6, width + 8, 550, "#111820", 12, extra='opacity="0.10"')
    s.rect(x, y, width, 544, "surface", 10, "line")
    s.icon("magnifying_glass", x + 20, y + 20, size=17)
    s.text(x + 50, y + 34, "Find a session, file, or command…", 16, "muted")
    s.text(x + width - 20, y + 33, "Esc", 12, "muted", anchor="end")
    s.line(x + 1, y + 56, x + width - 1)
    s.text(x + 20, y + 85, "This session", 12, "muted", 600)
    rows = [("sparkle", "Switch model…", "Claude Opus"),
            ("list_tree", "Session tree…", ""),
            ("compact", "Context & compaction…", "31% used")]
    for index, (icon, label, hint) in enumerate(rows):
        yy = y + 99 + index * 39
        if index == 0:
            s.rect(x + 10, yy, width - 20, 35, "selected", 5)
        s.icon(icon, x + 22, yy + 9)
        s.text(x + 51, yy + 24, label, 14)
        s.text(x + width - 24, yy + 24, hint, 12, "muted", anchor="end")
    s.line(x + 20, y + 231, x + width - 20)
    s.text(x + 20, y + 260, "Tools & settings", 12, "muted", 600)
    for index, (icon, label) in enumerate([
        ("box", "Resources…"), ("sparkle", "Models & providers…"),
        ("settings", "Settings…"), ("thread", "All sessions…"),
    ]):
        yy = y + 277 + index * 39
        s.icon(icon, x + 22, yy + 9)
        s.text(x + 51, yy + 24, label, 14)
    s.line(x + 1, y + 495, x + width - 1)
    s.text(x + 20, y + 525, "↑ ↓ Move    Enter Open    Esc Close", 12, "muted")
    s.text(x + width - 20, y + 525, "Nothing runs on selection", 12, "muted", anchor="end")
    return s


def start():
    s = Canvas("08-start")
    shell(s, new=True)
    left, right = s.left, s.right
    s.title(left, 202, "What should we change?", 32)
    s.text(left, 239, "Start with the task. Bring in files and details as you need them.", 15, "muted")
    composer(s, state="ready", y=312)
    s.text(left, 486, "Type / for commands or @ to include a file. Nothing is sent until you press Send.", 12, "muted")
    s.text(left, 550, "A starting point", 12, "muted", 600)
    for y, label in [(589, "Review the local changes"), (633, "Explain this project")]:
        s.icon("thread", left, y - 14, size=16)
        s.text(left + 28, y, label, 14)
        s.icon("chevron_right", right - 14, y - 14, size=14)
        s.line(left, y + 16, right)
    s.text(left, 708, "Project", 12, "muted", 600)
    s.text(left + 65, 708, "~/repos/pi", 12, "muted", family=MONO)
    s.text(left, 737, "File history is off. Past edits cannot be restored.", 12, "muted")
    return s


SCENES = [
    ("The work, not the machinery", "An optional inspector. Two work views. Changes are useful objects, not buried log lines."),
    ("Review where the space is", "1600 × 900. A real side-by-side diff, file navigation and a revision input. No empty outer rails."),
    ("Details, only when requested", "A file takes the main stage. Provenance and context live in an optional inspector."),
    ("A clear hand-off", "A pending extension question has one decision and one explicit submit. Selection alone does nothing."),
    ("The same workbench, after dark", "Ink-blue surfaces, neutral selection and readable contrast. Not a different layout."),
    ("Reachable, not permanently visible", "Models, resources, context and history remain one command away. Settings & tools also works by mouse."),
    ("Small window, same capabilities", "1000 × 720. A file picker replaces the rail; the diff is unified. Revision and file actions remain visible."),
    ("An invitation to work", "No metrics dashboard or onboarding cards. The same fluid composer, project context and a useful starting point."),
]


def assemble(scenes):
    width, height = 3000, 4500
    board = Canvas("board", width, height)
    board.rect(0, 0, width, height, "#e9e4de")
    board.text(48, 49, "PI DESKTOP  /  WORKBENCH VISION  /  DESIGN PROPOSAL", 14, "muted", 600)
    board.title(48, 115, "Less interface. More work.", 47)
    board.text(48, 159, "Projects → tasks → changes. Keep the work spacious, the chrome quiet, and control explicit.", 20)
    board.text(48, 195, "Eight vector screens · illustrative states, not implemented UI or actual app output · click a title for the full-size SVG", 15, "muted")
    for index, (scene, (title, caption)) in enumerate(zip(scenes, SCENES)):
        x = 48 + (index % 2) * 1464
        y = 243 + (index // 2) * 1040
        board.parts.append(f'<g id="screen-{scene.name}">')
        board.parts.append(f'<a href="workbench-vision/screens/{scene.name}.svg">')
        board.text(x, y + 23, f"{index + 1:02d}  {title}", 23, weight=600)
        board.parts.append("</a>")
        board.text(x + 1440, y + 23, f"{scene.width} × {scene.height}", 13, "muted", anchor="end")
        scale = min(1440 / scene.width, 900 / scene.height, 1)
        sx = x + (1440 - scene.width * scale) / 2
        sy = y + 44
        board.parts.append(f'<svg x="{sx}" y="{sy}" width="{scene.width * scale}" height="{scene.height * scale}" viewBox="0 0 {scene.width} {scene.height}">')
        board.parts.extend(scene.parts)
        board.parts.append("</svg>")
        board.rect(sx, sy, scene.width * scale, scene.height * scale, "none", stroke="#cfc8c0")
        board.text(x, y + 977, caption, 16, "muted", max_width=1440)
        board.parts.append("</g>")
    board.text(48, 4460, "Free-rein direction, separate from the conservative streamline study. No production code changed.", 15, "muted")
    board.parts.append('<a href="workbench-vision/README.md">')
    board.text(2952, 4460, "Read the interaction notes →", 15, "accent", anchor="end")
    board.parts.append("</a>")
    notices = "\n\n".join(
        (ROOT / "licenses" / filename).read_text()
        for filename in ["IBM-PLEX-OFL.txt", "COMMIT-MONO-OFL.txt", "ZED-ICONS-ISC.txt"]
    )
    content = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">\n'
        '<title>Pi Desktop — Less interface. More work. Eight-screen workbench vision</title>\n'
        '<desc>Static, illustrative design proposals. UI and code fonts are embedded. All screens are vector artwork, not raster screenshots.</desc>\n'
        f'<metadata>{escape(notices)}</metadata>\n'
        + font_css(embedded=True) + "\n" + "\n".join(board.parts) + "\n</svg>\n"
    )
    MASTER.write_text(content)
    ET.parse(MASTER)


def gallery(scenes):
    cards = []
    for index, (scene, (title, caption)) in enumerate(zip(scenes, SCENES)):
        cards.append(
            f'<figure><h2>{index + 1:02d} · {escape(title)}</h2>'
            f'<a href="screens/{scene.name}.svg"><img src="screens/{scene.name}.png" '
            f'alt="Proposed {escape(title)}" loading="lazy"></a>'
            f'<figcaption>{escape(caption)} <a href="screens/{scene.name}.svg">SVG</a>'
            f' · <a href="screens/{scene.name}.png">PNG</a></figcaption></figure>'
        )
    (HERE / "index.html").write_text('''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Pi Desktop — workbench vision</title><style>
@font-face{font-family:Plex;src:url('../../assets/fonts/IBMPlexSans-Regular.ttf')}
*{box-sizing:border-box}body{margin:0;background:#faf9f7;color:#252f3d;font:15px/1.65 Plex,system-ui,sans-serif}
main{max-width:1840px;margin:auto;padding:36px}h1{font:italic 42px Georgia,serif}h2{font-size:18px;font-weight:500}
a{color:#4b607c;text-underline-offset:4px}a:focus-visible{outline:2px solid #81766b;outline-offset:4px}
.grid{display:grid;grid-template-columns:1fr 1fr;gap:40px 28px}figure{margin:0;min-width:0}
img{display:block;width:100%;border:1px solid #ddd7d0}figcaption{margin-top:10px;font-size:13px;color:#6b645d}
p{max-width:1000px}@media(max-width:1000px){.grid{grid-template-columns:1fr}main{padding:24px}}
</style></head><body><main><h1>Less interface. More work.</h1>
<p>A separate, more opinionated direction: projects and sessions in the sidebar; thread, files and changes in the work area; details only when requested. These are static vector proposals, not app captures or implemented behavior.</p>
<p><a href="../desktop-workbench-vision.svg">Open the eight-screen SVG</a> · <a href="overview.png">Overview preview</a> · <a href="README.md">Interaction notes</a> · <a href="../desktop-streamline-study.svg">Earlier streamline study</a></p>
<div class="grid">''' + "\n".join(cards) + "</div></main></body></html>\n")


def render(browser, url, target, width, height, profile):
    subprocess.run([
        browser, "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
        "--allow-file-access-from-files", "--hide-scrollbars", "--no-first-run",
        "--disable-background-networking", f"--user-data-dir={profile}",
        f"--window-size={width},{height}", "--force-device-scale-factor=1",
        "--virtual-time-budget=1500", f"--screenshot={target}", url,
    ], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
    with Image.open(target) as image:
        assert image.size == (width, height)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    scenes = [work(), review(), file_details(), waiting(), work(dark=True), tools(),
              review(compact=True), start()]
    paths = [scene.save() for scene in scenes]
    assemble(scenes)
    gallery(scenes)
    shells = sorted(Path.home().glob(".cache/ms-playwright/chromium_headless_shell-*/*/headless_shell"))
    browser = os.environ.get("CHROME") or (str(shells[-1]) if shells else None) or shutil.which("chromium")
    if not browser:
        raise SystemExit("SVGs written. Set CHROME to render PNG previews.")
    with tempfile.TemporaryDirectory(prefix="pi-workbench-vision-") as temporary:
        for scene, path in zip(scenes, paths):
            render(browser, path.as_uri(), path.with_suffix(".png"), scene.width, scene.height, temporary)
            print(path.name)
        # Render the real master, rather than assembling unrelated PNG thumbnails.
        preview = Path(temporary) / "overview.svg"
        preview.write_text(MASTER.read_text().replace('width="3000" height="4500"', 'width="1800" height="2700"', 1))
        render(browser, preview.as_uri(), HERE / "overview.png", 1800, 2700, temporary)
    (HERE / "manifest.json").write_text(json.dumps({
        "kind": "static design proposal, not application captures",
        "master": "../desktop-workbench-vision.svg",
        "screens": [{"name": scene.name, "size": [scene.width, scene.height],
                     "composer": scene.composer_bounds} for scene in scenes],
        "rules": {"title_bar": 36, "sidebar": SIDEBAR, "work_gutter": GUTTER,
                  "footer": 24, "default_inspector": "closed"},
    }, indent=2) + "\n")
    print(MASTER.relative_to(ROOT), "— eight vector screens")


if __name__ == "__main__":
    main()
