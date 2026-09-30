#!/usr/bin/env python3
"""pi desktop design study 06: preferences for the app itself.

pi-desktop's own settings (jj, language servers, editor, terminal, general, appearance) sit in the Settings view
next to pi's settings.json: the same rows, scopes and inspector, stored in the app's own settings file. Project
scope holds per-project overrides, saved in the project's .pi/pi-desktop.json. Each screen is drawn in Evening (dark) and Moonstone (light).
Writes SVG to stdout.
"""
import sys

from pi_study_common import (
    C, DARK, MONO, MULTI_ACTIVE, MULTI_PROJECTS, PAGE, SERIF, add, button, defs, divider, dot, dump, icon, insp_title,
    kv, label, line, note, page, pill, rect, reset, section, seg, set_theme, text, toggle, tparts, tw, window_begin,
    window_end,
)

ROW_PITCH = 910
W, H = 2848, 200 + 3 * ROW_PITCH + 30
MULTI = dict(active=MULTI_ACTIVE, projects=MULTI_PROJECTS, selected=None)
APP_FILE = "~/Library/Application Support/pi-desktop/settings.json"
PI_CATS = ["Model & thinking", "Interaction", "Tools", "Sessions & context", "Compaction", "Branch summaries",
           "Network & retries", "Shell", "Resources", "Updates & telemetry"]
DESKTOP_CATS = ["General", "Appearance", "jj", "Language servers", "Editor", "Terminal"]
PROJECT_CATS = {"jj", "Language servers"}


def frame(scope, selected, changed, right, status):
    """The Settings view with pi's categories and the app's own below them."""
    window_begin("Settings", title_icon="settings", actions=[("btn", "Open JSON", "file")], search="Search settings",
                 nav_sel="settings", title_chevron=False, multi=MULTI, status_right=status)
    seg(276, 193, ["User", "Project  ~/repos/pi"], scope, h=26)
    text(1044, 211, right, C["faint"], size=10.5, family=MONO, anchor="end")
    line(256, 228.5, 1064, 228.5, C["hover"])
    line(436.5, 229, 436.5, 846, C["hover"])
    y = 250
    for group, cats in (("PI", PI_CATS), ("PI DESKTOP", DESKTOP_CATS)):
        label(272, y, group, size=9.5)
        y += 8
        for cat in cats:
            usable = scope == 0 or group == "PI" or cat in PROJECT_CATS
            if cat == selected:
                rect(264, y, 164, 24, C["select"], 5)
                rect(264, y + 5, 2, 14, C["accent"], 1)
            if cat in changed:
                dot(418, y + 12, 3, C["accent"])
            col = C["text"] if cat == selected else (C["text2"] if usable else C["faint"])
            text(278, y + 16, cat, col, size=12, weight=500 if cat == selected else 400)
            y += 25
        y += 22


def row(y, title, key, selected=False, badge=None):
    """One setting: title, key, and an optional badge: changed, this project or inherits."""
    if selected:
        rect(448, y, 604, 50, C["hover"], 6)
    text(456, y + 21, title, C["text"], size=13)
    text(456, y + 38, key, C["faint"], size=10.5, family=MONO)
    if badge:
        fg, bg, bd = (C["faint"], C["hover"], C["line"]) if badge == "inherits" else (C["accent"], C["select"], C["focus"])
        pill(456 + tw(title, 13) * 1.12 + 10, y + 9, badge, fg, bg, size=9, h=15, border=bd)
    line(456, y + 50.5, 1044, y + 50.5, C["hover"])


def seg_right(y, items, active, faded=False):
    w = sum(tw(n, 11) + 24 for n in items) + 4
    if faded:
        add('<g opacity="0.5">')
    seg(round(1044 - w, 1), y, items, active)
    if faded:
        add("</g>")


def toggle_right(y, on, faded=False):
    if faded:
        add('<g opacity="0.5">')
    toggle(1016, y, on)
    if faded:
        add("</g>")


def chip_right(y, s, w=None):
    w = w or tw(s, 11, True) + 44
    rect(1044 - w, y, w, 26, C["chip"], 5, C["chipLine"])
    text(1044 - w + 12, y + 17, s, C["text2"], size=11, family=MONO)
    icon("chevron", 1024, y + 7, 11, C["faint"])


def json_box(y, lines_, h=None):
    h = h or 16 + 18 * len(lines_)
    rect(1084, y, 288, h, C["deep"], 6, C["line"])
    for i, parts in enumerate(lines_):
        tparts(1098, y + 20 + i * 18, parts, size=11, family=MONO, pre=True)
    return y + h


def jkey(s):
    return (f'"{s}"', C["text2"])


P = lambda s: (s, C["pl"])  # noqa: E731


# ---------------------------------------------------------------- 01 The app's settings next to pi's

def r_user():
    frame(0, "jj", {"Model & thinking", "jj"}, APP_FILE, "pi-desktop settings · 1 changed from default")
    label(456, 256, "JJ")
    row(266, "Offer jj in git projects", "jj.offer")
    seg_right(279, ["Ask", "Never"], 0)
    row(316, "Snapshot before each command", "jj.snapshotBeforeCommands", selected=True)
    toggle_right(333, True)
    row(366, "Let pi read jj history", "jj.tools")
    text(1002, 395, "jj_log · jj_diff · jj_show", C["faint"], size=10.5, family=MONO, anchor="end")
    toggle_right(383, False)
    row(416, "Turn bars in the editor", "jj.turnBars", badge="changed")
    seg_right(429, ["While ⌥ is held", "Never"], 1)

    label(456, 498, "LANGUAGE SERVERS")
    row(508, "Tell pi about errors after edits", "languageServers.afterEdits")
    toggle_right(525, True)
    row(558, "Check files before a run ends", "languageServers.beforeRunEnds")
    toggle_right(575, True)

    label(456, 640, "EDITOR")
    row(650, "Font size", "editor.fontSize")
    chip_right(662, "13", w=70)
    row(700, "Show the problem card on hover", "editor.problemCard")
    toggle_right(717, True)

    insp_title("jj.snapshotBeforeCommands", "Default", "pi-desktop", kind="faint", serif=False)
    section(290, "DESCRIPTION")
    text(1084, 314, "Takes a jj snapshot before each bash call,", C["text2"], size=12)
    text(1084, 332, "so the files a command changes can be", C["text2"], size=12)
    text(1084, 350, "restored to just before it.", C["text2"], size=12)
    divider(368)
    section(394, "VALUE")
    kv(420, "Current", "on", mono=True)
    kv(444, "Default", "on", mono=True)
    kv(468, "~/repos/pi", "inherits", mono=True)
    divider(486)
    section(512, "COST IN ~/REPOS/PI")
    kv(538, "Median snapshot", "38 ms", mono=True)
    kv(562, "Slowest", "210 ms", mono=True)
    text(1084, 584, "Last 20 commands. Time is spent before", C["faint"], size=11)
    text(1084, 600, "the command starts.", C["faint"], size=11)
    divider(616)
    section(642, "IN SETTINGS.JSON")
    json_box(654, [[P("{ "), jkey("jj"), P(": { "), jkey("snapshotBeforeCommands"), P(":")],
                   [P("    "), ("true", C["kw"]), P(" } }")]])
    button(1084, 716, "Reset to default", "disabled", w=150)
    button(1250, 716, "DOCS", "bracket")
    text(1084, 832, "Editing pi-desktop's settings", C["faint"], size=10)


# ---------------------------------------------------------------- 02 Per-project overrides

def r_project():
    frame(1, "jj", {"jj"}, "shared with the team · .pi/pi-desktop.json", "~/repos/pi · 1 override")
    label(456, 256, "JJ IN ~/REPOS/PI")
    row(266, "Offer jj in git projects", "jj.offer", badge="inherits")
    text(1044, 295, "jj is on here", C["faint"], size=11, anchor="end")
    row(316, "Snapshot before each command", "jj.snapshotBeforeCommands", badge="inherits")
    toggle_right(333, True, faded=True)
    row(366, "Let pi read jj history", "jj.tools", selected=True, badge="this project")
    text(1002, 395, "jj_log · jj_diff · jj_show", C["faint"], size=10.5, family=MONO, anchor="end")
    toggle_right(383, True)
    row(416, "Turn bars in the editor", "jj.turnBars", badge="inherits")
    seg_right(429, ["While ⌥ is held", "Never"], 1, faded=True)
    label(456, 498, "LANGUAGE SERVERS IN ~/REPOS/PI")
    row(508, "Tell pi about errors after edits", "languageServers.afterEdits", badge="inherits")
    toggle_right(525, True, faded=True)
    row(558, "Check files before a run ends", "languageServers.beforeRunEnds", badge="inherits")
    toggle_right(575, True, faded=True)
    icon("info", 456, 623, 13, C["faint"])
    text(476, 634, "Only settings that can differ per project are listed. Changing one here overrides your default",
         C["faint"], size=11)
    text(476, 650, "for this project only; greyed categories have none.", C["faint"], size=11)

    insp_title("jj.tools", "On for this project", "override", kind="accent", serif=False)
    section(290, "DESCRIPTION")
    text(1084, 314, "Gives pi read-only jj_log, jj_diff and", C["text2"], size=12)
    text(1084, 332, "jj_show tool calls, answered by the app.", C["text2"], size=12)
    divider(350)
    section(376, "VALUE")
    kv(402, "This project", "on", mono=True)
    kv(426, "Your default", "off", mono=True)
    kv(450, "Default", "off", mono=True)
    divider(468)
    section(494, "IN .PI/PI-DESKTOP.JSON")
    json_box(506, [[P("{ "), jkey("jj"), P(": { "), jkey("tools"), P(": "), ("true", C["kw"]), P(" } }")]])
    text(1084, 560, "Its own file beside pi's .pi/settings.json,", C["faint"], size=11)
    text(1084, 576, "so the two never overwrite each other.", C["faint"], size=11)
    button(1084, 594, "Remove override", "ghost", w=150)
    note(1084, 790, 288, ["Everyone who opens the project gets it;", "commit the file to share it."])
    text(1084, 832, "Editing overrides for ~/repos/pi", C["faint"], size=10)


# ---------------------------------------------------------------- 03 General and appearance

def r_general():
    frame(0, "General", {"Model & thinking", "jj"}, APP_FILE, "backend 0.1.0 · pi 0.87.1 · node 22.19.0")
    label(456, 256, "GENERAL")
    row(266, "Backend", "general.backend", selected=True)
    chip_right(278, "Installed · pi 0.87.1")
    row(316, "Node.js", "general.node")
    chip_right(328, "Automatic · /opt/homebrew/bin/node")
    row(366, "Reopen sessions at launch", "general.reopenSessions")
    toggle_right(383, True)
    row(416, "Ask before closing a running session", "general.confirmClose")
    toggle_right(433, True)
    row(466, "Remember “Not now” answers", "general.rememberDismissed")
    toggle_right(483, True)

    label(456, 540, "APPEARANCE")
    row(550, "Theme", "appearance.theme")
    seg_right(563, ["System", "Evening", "Moonstone"], 0)
    row(600, "Text size", "appearance.textSize")
    seg_right(613, ["Small", "Default", "Large"], 1)

    label(456, 682, "TERMINAL")
    row(692, "Shell", "terminal.shell")
    chip_right(704, "Automatic · /bin/zsh")

    insp_title("general.backend", "Installed by setup", "pi-desktop", kind="faint", serif=False)
    section(290, "DESCRIPTION")
    text(1084, 314, "What each session runs: pi-desktop's", C["text2"], size=12)
    text(1084, 332, "backend with pi 0.87.1, which setup", C["text2"], size=12)
    text(1084, 350, "installed with npm into the app's folder.", C["text2"], size=12)
    divider(368)
    section(394, "FOUND")
    kv(420, "Backend", "pi-desktop-backend 0.1.0", mono=True)
    kv(444, "pi", "0.87.1", mono=True)
    kv(468, "Protocol", "1", mono=True)
    kv(492, "Node.js", "22.19.0", mono=True)
    icon("check", 1084, 505, 13, C["green"])
    text(1104, 516, "Knows every command this app sends", C["text2"], size=12)
    button(1084, 532, "Run setup again", "ghost", w=140, ic="refresh")
    button(1232, 532, "Custom backend…", "ghost", w=140, ic="folder")
    divider(574)
    section(600, "WHEN IT DOES NOT FIT")
    icon("x", 1084, 613, 13, C["coral"])
    text(1104, 624, "Sessions do not start; setup opens at", C["text2"], size=12)
    text(1104, 642, "the step that needs attention, such as", C["text2"], size=12)
    text(1104, 660, "Node 22.19+ or protocol 1.", C["text2"], size=12)
    note(1084, 790, 288, ["PI_DESKTOP_RPC_ENTRY and PI_DESKTOP_NODE", "still win, for development."])
    text(1084, 832, "Editing pi-desktop's settings", C["faint"], size=10)


# Status: "open" still needs a decision, "decided" records the user's answer (2026-09-30).
ROWS = [
    (r_user, "The app's settings, next to pi's", "one Settings view, two groups",
     "PI DESKTOP categories sit below pi's in the same list, with the same rows and inspector. They are saved in "
     "pi-desktop's own settings.json in the app's config folder; pi never reads it.",
     "decided", "one Settings view with two groups."),
    (r_project, "Per-project overrides", "in the project, shared with the team",
     "Project scope lists only the settings that can differ per project. Overrides go to the project's "
     ".pi/pi-desktop.json, beside pi's .pi/settings.json; the others show what they inherit.",
     "decided", "overrides live in .pi/pi-desktop.json, shared with everyone who opens the project."),
    (r_general, "General, appearance and terminal", "what setup installed, and what flags do today",
     "Which backend and Node run sessions, reopening sessions, theme and text size, and the terminal shell. These "
     "replace --light, PI_DESKTOP_RPC_ENTRY and PI_DESKTOP_NODE for everyday use.",
     "decided", "nothing bundled: setup (study 07) installs Node.js and pi; anything that does not fit opens setup "
     "instead of sessions."),
]

STATUS = {
    "open": ("DECIDE", PAGE["terracotta"]),
    "decided": ("DECIDED", "#2e8a55"),
}


def main():
    reset()
    defs(W, H, "<title>pi desktop — preferences for the app, visual proposal, not implemented</title>"
               "<desc>pi-desktop's own settings in the Settings view next to pi's settings.json: jj, language servers, "
               "editor, terminal, general and appearance, with per-project overrides in .pi/pi-desktop.json. Each screen in Evening "
               "(dark) and Moonstone (light). Paths and numbers are sample data.</desc>")
    page(W, H)
    label(48, 42, "PI  /  DESIGN STUDY  06", C["pageLabel"], size=11)
    add(f'<text x="48" y="96" fill="{C["evening"]}" font-family="{SERIF}" font-size="40" font-style="italic">'
        f'Settings for <tspan fill="{DARK["accent"]}">the app itself</tspan>.</text>')
    text(48, 128, "pi-desktop gets its own settings, in the Settings view beside pi's: the switches study 05 needs, "
                  "plus what today takes command-line flags and environment variables. All three rows are decided.",
         C["driftwood"], size=14)
    label(2800, 92, "VISUAL PROPOSAL · SAMPLE DATA · FOLLOWS STUDIES 02 AND 05", C["pageLabel"], anchor="end", size=11)

    for r, (fn, name, tag, sub, status, decide) in enumerate(ROWS):
        y = 200 + r * ROW_PITCH
        dot(61, y - 5, 13, C["parchment"])
        text(61, y - 1, f"{r + 1:02d}", C["evening"], size=10, weight=600, family=MONO, anchor="middle")
        tparts(84, y, [(name, C["evening"], None, 600), ("  —  " + tag, C["driftwood"])], size=16)
        text(48, y + 24, sub, C["driftwood"], size=12.5)
        word, color = STATUS[status]
        tparts(48, y + 44, [(word + "  ", color, MONO, 600), (decide, C["evening"])], size=12.5)
        for col, theme, cap in ((0, "dark", "EVENING  ·  DARK"), (1, "light", "MOONSTONE  ·  LIGHT")):
            dx = col * 1408
            set_theme(theme)
            label(48 + dx, y + 68, cap, C["pageLabel"], size=10)
            add(f'<g transform="translate({dx} {y + 80 - 130})">')
            fn()
            window_end()
            add("</g>")
        set_theme("dark")

    text(48, H - 20, "LAYOUT STUDY ONLY — follows the Settings view of desktop-views-study.svg (study 02) and "
                     "desktop-jj-study.svg (study 05). Paths, versions and timings are sample data.", C["warm30"], size=11)
    add("</svg>")
    return dump()


if __name__ == "__main__":
    sys.stdout.write(main())
