#!/usr/bin/env python3
"""Render focused Thread-polish proposals as static SVG and PNG artwork.

These are illustrative mocks, not native application captures or execution evidence.
They reuse the workbench study's vector drawing helpers, local fonts, and icon set.
"""
from html import escape
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from xml.etree import ElementTree as ET

from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUT = HERE / "screens"
MASTER = ROOT / "design/thread-polish-mocks.svg"
VISION_PATH = ROOT / "design/workbench-vision/render.py"

spec = importlib.util.spec_from_file_location("workbench_vision", VISION_PATH)
vision = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(vision)
Canvas = vision.Canvas
SANS = vision.SANS
MONO = vision.MONO
SERIF = vision.SERIF

WIDTH, HEIGHT = 1500, 820
SIDEBAR, GUTTER = 216, 24
WORK_LEFT, WORK_RIGHT = SIDEBAR + GUTTER, WIDTH - GUTTER

SCENES = [
    (
        "01-balanced-rail",
        "Balanced operational rail",
        "Recommended: full-width hit targets, a 1040px metadata rail, honest Modified labels, an actionable issue notice, and a compact idle composer.",
    ),
    (
        "02-grouped-result",
        "Grouped result card",
        "A more contained alternative: the turn result becomes one bordered object while the prose remains editorial and quiet.",
    ),
    (
        "03-fluid-ledger",
        "Fluid minimal ledger",
        "Keeps the work surface fully fluid, but moves metadata close to filenames and upgrades the shell block and failure affordance.",
    ),
]


def document(scene):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{scene.width}" '
        f'height="{scene.height}" viewBox="0 0 {scene.width} {scene.height}">\n'
        f'<title>Pi Desktop thread polish — {escape(scene.name)}</title>\n'
        '<desc>Static illustrative design proposal. Not application output or evidence of executed actions.</desc>\n'
        + vision.font_css()
        + "\n"
        + "\n".join(scene.parts)
        + "\n</svg>\n"
    )


def shell(s):
    """Current compact workbench chrome, included only to judge the content in context."""
    s.rect(0, 0, WIDTH, HEIGHT, "canvas")
    s.rect(0, 0, WIDTH, 36, "bar")
    s.rect(0, 36, SIDEBAR, HEIGHT - 60, "sidebar")
    s.line(0, 35.5, WIDTH)
    s.line(SIDEBAR - 0.5, 36, SIDEBAR - 0.5, HEIGHT - 24)
    s.icon_button("threads_sidebar_left_open", 8, 4, label="Toggle sidebar")
    s.icon_button("magnifying_glass", 40, 4, label="Search and commands")
    s.icon_button("plus", WIDTH - 72, 4, label="New session")
    s.icon_button("threads_sidebar_right_closed", WIDTH - 36, 4, label="Toggle details")

    s.icon("chevron_down", 16, 50, size=10)
    s.text(33, 62, "Open", 12, "muted")
    s.text(71, 62, "1", 10, "muted", family=MONO)
    s.rect(8, 72, 200, 34, "selected", 5)
    s.icon("thread", 21, 82, "secondary", 14)
    s.text(44, 94, "Qwen signatures", 13, weight=600)
    s.text(174, 94, "pi", 10, "muted", family=MONO, anchor="end")

    s.icon("chevron_down", 16, 122, size=10)
    s.text(33, 134, "Projects", 12, "muted")
    s.icon_button("plus", 176, 110, label="Open folder")
    s.icon("chevron_down", 16, 157, size=10)
    s.icon("folder", 35, 155, size=14)
    s.text(58, 168, "pi", 13, weight=600)
    for y, label in [(199, "Streaming retry"), (231, "LSP shutdown")]:
        s.icon("thread", 21, y - 12, size=14)
        s.text(44, y, label, 13, "secondary")
    s.text(44, 265, "12 saved sessions…", 12, "muted")
    for y, label in [(317, "zed"), (353, "minivm")]:
        s.icon("chevron_right", 17, y - 12, size=10)
        s.icon("folder", 35, y - 14, size=14)
        s.text(58, y, label, 13)
    s.icon("folder_add", 20, 387, size=15)
    s.text(44, 400, "Open folder…", 13, "muted")
    s.line(16, HEIGHT - 78, 200)
    s.icon("settings", 20, HEIGHT - 62, size=15)
    s.text(44, HEIGHT - 49, "Settings & tools", 13)

    s.text(WORK_LEFT, 61, "Thread", 13, weight=600)
    s.line(WORK_LEFT, 75, WORK_LEFT + 43, color="secondary", width=2)
    s.text(WORK_LEFT + 76, 61, "Changes  3", 13, "muted")
    s.rect(WIDTH - 344, 42, 128, 28, "selected", 5)
    s.text(WIDTH - 330, 60, "pi / main", 12, "muted", family=MONO)
    s.icon("chevron_down", WIDTH - 236, 50, size=10)
    s.icon_button("folder", WIDTH - 202, 40, label="Browse project files")
    s.icon_button("terminal", WIDTH - 172, 40, label="Terminal")
    s.line(SIDEBAR, 75.5, WIDTH)

    s.rect(0, HEIGHT - 24, WIDTH, 24, "bar")
    s.line(0, HEIGHT - 24, WIDTH)
    s.dot(17, HEIGHT - 12, "green", 2.5)
    s.text(28, HEIGHT - 8, "Local process", 10, "muted", family=MONO)
    s.text(WIDTH - 16, HEIGHT - 8, "3 changed files  ·  $0.31  ·  18% context", 10, "muted", family=MONO, anchor="end")


def assistant_copy(s, polished_shell=False):
    x = WORK_LEFT
    s.text(x, 108, "Try it", 16, weight=600)
    if polished_shell:
        s.rect(x, 124, 790, 116, "code", 7, "line")
        s.rect(x + 1, 125, 788, 27, "surface", 6)
        s.text(x + 14, 144, "bash", 11, "muted", family=MONO, weight=600)
        s.icon("copy", x + 756, 132, "muted", 14)
        commands = [
            "./git-radar.sh --only-red --focus pi-desktop .",
            "./git-radar.sh --json .",
            "./git-radar.sh --top 10 --stale-days 14 .",
        ]
        for index, command in enumerate(commands):
            s.text(x + 16, 173 + index * 22, "$", 12, "accent", family=MONO, weight=600)
            s.text(x + 34, 173 + index * 22, command, 12, "amber", family=MONO)
        prose_y = 271
    else:
        s.rect(x, 124, 790, 94, "code", 7, "line")
        commands = [
            "bash /Users/eh04xk/repos/pi-desktop/git-radar.sh --only-red --focus pi-desktop .",
            "bash /Users/eh04xk/repos/pi-desktop/git-radar.sh --json .",
            "bash /Users/eh04xk/repos/pi-desktop/git-radar.sh --top 10 --stale-days 14 .",
        ]
        for index, command in enumerate(commands):
            # Keep raw copy visually honest in the first two alternatives.
            s.text(x + 14, 151 + index * 24, command, 12, "amber", family=MONO)
        prose_y = 249
    s.text(x, prose_y, "If you want, next I can add a", 15, "secondary")
    s.rect(x + 226, prose_y - 17, 43, 22, "selected", 4)
    s.text(x + 232, prose_y, "--fix", 13, "accent", family=MONO)
    s.text(x + 276, prose_y, "mode that auto-runs safe actions (fetch/prune + optional pull for behind", 15, "secondary")
    s.text(x, prose_y + 25, "repos).", 15, "secondary")
    return prose_y + 61


def file_row(s, y, name, path, status, rail_right, line_right=WORK_RIGHT, compact=False):
    height = 52 if compact else 56
    s.line(WORK_LEFT, y, line_right)
    s.icon("file", WORK_LEFT + 4, y + (18 if compact else 20), "muted", 14)
    if path:
        s.text(WORK_LEFT + 30, y + 23, name, 13, weight=600)
        s.text(WORK_LEFT + 30, y + 43, path, 11, "muted", family=MONO)
    else:
        s.text(WORK_LEFT + 30, y + 33, name, 13, weight=600)
    color = "green" if status.startswith("+") else "muted"
    s.text(rail_right - 25, y + height / 2 + 5, status, 12, color, family=MONO, anchor="end")
    s.icon("chevron_right", rail_right - 14, y + height / 2 - 5, "muted", 10)
    return y + height


def issue_notice(s, x, y, width, integrated=False):
    fill = "#faf2e7"
    s.rect(x, y, width, 44, fill, 6, "line")
    s.icon("warning", x + 13, y + 14, "red", 15)
    s.text(x + 38, y + 27, "Completed with 1 issue", 13, weight=600)
    s.text(x + 190, y + 27, "One tool call failed", 12, "muted")
    if integrated:
        s.text(x + width - 18, y + 27, "Review failure  →", 12, "accent", weight=600, anchor="end")
    else:
        s.button(x + width - 146, y + 7, 132, "Review failed call", height=30)


def compact_composer(s, y, height=112):
    x, right = WORK_LEFT, WORK_RIGHT
    s.rect(x, y, right - x, height, "surface", 7, "line")
    s.text(x + 16, y + 32, "Describe a change, ask a question, or drop a file…", 14, "muted")
    s.icon_button("maximize", right - 36, y + 7, label="Expand composer")
    bottom = y + height - 15
    s.icon("attach", x + 8, bottom - 15, "muted", 15)
    s.text(x + 48, bottom - 2, "GPT-5.3 Codex", 12, "secondary")
    s.icon("chevron_down", x + 145, bottom - 13, "muted", 10)
    s.text(x + 176, bottom - 2, "Medium", 12, "secondary")
    s.icon("chevron_down", x + 233, bottom - 13, "muted", 10)
    s.button(right - 105, y + height - 43, 89, "Send ↵", primary=True, disabled=True, height=30)


def balanced_rail():
    s = Canvas("01-balanced-rail", WIDTH, HEIGHT)
    shell(s)
    files_y = assistant_copy(s)
    rail_right = min(WORK_RIGHT, WORK_LEFT + 1040)
    s.text(WORK_LEFT, files_y, "Changed 3 files", 13, weight=600)
    s.text(rail_right, files_y, "Review changes  →", 12, "accent", anchor="end")
    y = files_y + 16
    y = file_row(s, y, "turn_bars.rs", "crates/pi_desktop/src/desktop/files", "+37", rail_right)
    y = file_row(s, y, "files.rs", "crates/pi_desktop/src/desktop", "Modified", rail_right)
    y = file_row(s, y, "git-radar.sh", "./", "+34", rail_right)
    s.line(WORK_LEFT, y, WORK_RIGHT)
    issue_notice(s, WORK_LEFT, y + 20, rail_right - WORK_LEFT)
    compact_composer(s, HEIGHT - 160)
    return s


def grouped_result():
    s = Canvas("02-grouped-result", WIDTH, HEIGHT)
    shell(s)
    files_y = assistant_copy(s)
    card_right = WORK_LEFT + 980
    card_y = files_y - 22
    s.rect(WORK_LEFT, card_y, card_right - WORK_LEFT, 204, "surface", 8, "line")
    s.text(WORK_LEFT + 16, card_y + 29, "3 changed files", 13, weight=600)
    s.text(card_right - 16, card_y + 29, "Open review  →", 12, "accent", anchor="end")
    y = card_y + 43
    # Card rows use inset rules and keep all metadata inside one result object.
    old_left = WORK_LEFT
    for name, path, status in [
        ("turn_bars.rs", "desktop/files", "+37"),
        ("files.rs", "desktop", "Modified"),
        ("git-radar.sh", "Project root", "+34"),
    ]:
        s.line(old_left + 16, y, card_right - 16)
        s.icon("file", old_left + 18, y + 17, "muted", 14)
        s.text(old_left + 44, y + 23, name, 13, weight=600)
        s.text(old_left + 260, y + 23, path, 11, "muted", family=MONO)
        s.text(card_right - 45, y + 23, status, 12, "green" if status.startswith("+") else "muted", family=MONO, anchor="end")
        s.icon("chevron_right", card_right - 30, y + 13, "muted", 10)
        y += 48
    issue_notice(s, WORK_LEFT, card_y + 222, card_right - WORK_LEFT, integrated=True)
    compact_composer(s, HEIGHT - 160)
    return s


def fluid_ledger():
    s = Canvas("03-fluid-ledger", WIDTH, HEIGHT)
    shell(s)
    files_y = assistant_copy(s, polished_shell=True)
    s.text(WORK_LEFT, files_y, "Changed 3 files", 13, weight=600)
    s.text(WORK_RIGHT, files_y, "Review changes  →", 12, "accent", anchor="end")
    y = files_y + 16
    rows = [
        ("turn_bars.rs", "crates/pi_desktop/src/desktop/files", "+37"),
        ("files.rs", "crates/pi_desktop/src/desktop", "Modified"),
        ("git-radar.sh", "Project root", "+34"),
    ]
    for name, path, status in rows:
        s.line(WORK_LEFT, y, WORK_RIGHT)
        s.icon("file", WORK_LEFT + 4, y + 18, "muted", 14)
        s.text(WORK_LEFT + 30, y + 24, name, 13, weight=600)
        title_width = vision.font(13, SANS, 600).getlength(name)
        s.text(WORK_LEFT + 44 + title_width, y + 24, path, 11, "muted", family=MONO)
        color = "green" if status.startswith("+") else "muted"
        s.text(WORK_LEFT + 650, y + 24, status, 12, color, family=MONO, anchor="end")
        s.icon("chevron_right", WORK_LEFT + 666, y + 14, "muted", 10)
        y += 48
    s.line(WORK_LEFT, y, WORK_RIGHT)
    issue_notice(s, WORK_LEFT, y + 18, WORK_RIGHT - WORK_LEFT, integrated=True)
    compact_composer(s, HEIGHT - 160)
    return s


def save_scene(scene):
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / f"{scene.name}.svg"
    path.write_text(document(scene))
    ET.parse(path)
    return path


def master(scenes):
    board_width, board_height = 1600, 2960
    board = Canvas("thread-polish-board", board_width, board_height)
    board.rect(0, 0, board_width, board_height, "#e9e4de")
    board.text(50, 51, "PI DESKTOP  /  THREAD POLISH  /  STATIC DESIGN OPTIONS", 13, "muted", weight=600)
    board.title(50, 112, "Make the result easier to scan—and easier to trust.", 38)
    board.text(50, 153, "Three focused alternatives based on the real-session screenshot. Same information; different containment and rhythm.", 17, "secondary")
    for index, (scene, (_, title, caption)) in enumerate(zip(scenes, SCENES)):
        top = 210 + index * 900
        board.text(50, top, f"0{index + 1}  {title}", 20, weight=600)
        board.text(1550, top, "1500 × 820", 12, "muted", family=MONO, anchor="end")
        scale = 0.96
        board.parts.append(
            f'<svg x="80" y="{top + 28}" width="{WIDTH * scale}" height="{HEIGHT * scale}" viewBox="0 0 {WIDTH} {HEIGHT}">'
        )
        board.parts.extend(scene.parts)
        board.parts.append("</svg>")
        board.rect(80, top + 28, WIDTH * scale, HEIGHT * scale, "none", stroke="#cfc8c0")
        board.text(80, top + 838, caption, 14, "muted")
    board.text(50, board_height - 28, "Illustrative proposal only · sample commands and statuses are not execution evidence", 12, "muted")
    content = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{board_width}" height="{board_height}" viewBox="0 0 {board_width} {board_height}">\n'
        '<title>Pi Desktop — thread polish design options</title>\n'
        '<desc>Three static vector proposals based on a real-session layout.</desc>\n'
        + vision.font_css(embedded=True)
        + "\n"
        + "\n".join(board.parts)
        + "\n</svg>\n"
    )
    MASTER.write_text(content)
    ET.parse(MASTER)


def gallery():
    cards = []
    for name, title, caption in SCENES:
        cards.append(
            f'<figure><h2>{escape(title)}</h2><a href="screens/{name}.svg">'
            f'<img src="screens/{name}.png" alt="{escape(title)} static UI proposal"></a>'
            f'<figcaption>{escape(caption)} <a href="screens/{name}.svg">SVG</a> · '
            f'<a href="screens/{name}.png">PNG</a></figcaption></figure>'
        )
    (HERE / "index.html").write_text(
        """<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Pi Desktop — thread polish mocks</title><style>
@font-face{font-family:Plex;src:url('../../assets/fonts/IBMPlexSans-Regular.ttf')}
*{box-sizing:border-box}body{margin:0;background:#faf9f7;color:#252f3d;font:15px/1.6 Plex,system-ui,sans-serif}
main{max-width:1580px;margin:auto;padding:38px}h1{font:italic 40px Georgia,serif;margin:.2em 0}h2{font-size:19px;margin:0 0 12px}
p{max-width:980px;color:#575f69}a{color:#4b607c;text-underline-offset:4px}.grid{display:grid;gap:42px}
figure{margin:0}img{display:block;width:100%;border:1px solid #ddd7d0;background:white}figcaption{margin-top:9px;color:#665f59;font-size:13px}
@media(max-width:800px){main{padding:20px}}</style></head><body><main>
<h1>Thread result polish</h1>
<p>Three static comparisons for changed-file rhythm, honest unknown counts, actionable failure status, shell presentation, and idle-composer density. These are illustrative designs—not native captures or execution evidence.</p>
<p><a href="../thread-polish-mocks.svg">Open the combined vector sheet</a> · <a href="overview.png">Overview PNG</a> · <a href="README.md">Decision notes</a></p>
<div class="grid">"""
        + "\n".join(cards)
        + "</div></main></body></html>\n"
    )


def render(browser, url, target, width, height, profile):
    subprocess.run(
        [
            browser,
            "--headless",
            "--no-sandbox",
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--allow-file-access-from-files",
            "--hide-scrollbars",
            "--no-first-run",
            "--disable-background-networking",
            f"--user-data-dir={profile}",
            f"--window-size={width},{height}",
            "--force-device-scale-factor=1",
            "--virtual-time-budget=1200",
            f"--screenshot={target}",
            url,
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=60,
    )
    with Image.open(target) as image:
        assert image.size == (width, height), (target, image.size)


def main():
    scenes = [balanced_rail(), grouped_result(), fluid_ledger()]
    paths = [save_scene(scene) for scene in scenes]
    master(scenes)
    gallery()
    shells = sorted(Path.home().glob(".cache/ms-playwright/chromium_headless_shell-*/*/headless_shell"))
    browser = os.environ.get("CHROME") or (str(shells[-1]) if shells else None) or shutil.which("chromium")
    if not browser:
        raise SystemExit("SVGs written. Set CHROME to render PNG previews.")
    with tempfile.TemporaryDirectory(prefix="pi-thread-polish-") as profile:
        for scene, path in zip(scenes, paths):
            target = path.with_suffix(".png")
            render(browser, path.as_uri(), target, WIDTH, HEIGHT, profile)
            print(target.relative_to(ROOT))
        render(browser, MASTER.as_uri(), HERE / "overview.png", 1600, 2960, profile)
    (HERE / "manifest.json").write_text(
        json.dumps(
            {
                "kind": "static design proposal, not application captures",
                "source": "real-session layout from the user-provided screenshot",
                "master": "../thread-polish-mocks.svg",
                "screens": [
                    {"name": name, "size": [WIDTH, HEIGHT], "title": title}
                    for name, title, _ in SCENES
                ],
                "selected": "02-grouped-result",
            },
            indent=2,
        )
        + "\n"
    )
    print(MASTER.relative_to(ROOT))


if __name__ == "__main__":
    main()
