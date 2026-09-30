#!/usr/bin/env python3
"""pi desktop design study 05: more from jj.

Proposed additions on top of study 04's per-turn jj changes: turn links that survive restarts, restoring one file,
snapshots before commands, a workspace per parallel session, forking with files, best-of-N attempts, line-to-turn
annotation, telling pi about your edits, landing turns, undo with conflicts, the operation log, and read-only jj
tools for pi. Each screen is drawn in Evening (dark) and Moonstone (light). Writes SVG to stdout.
"""
import sys

import gen_desktop_views_study as views
import gen_desktop_workspace_study as ws
from pi_study_common import (
    C, DARK, MONO, MULTI_ACTIVE, MULTI_PROJECTS, PAGE, SANS, SERIF, THEME, add, blend, button, checkbox, composer, defs,
    dim_window, divider, dot, dump, icon, input_box, insp_title, kv, label, line, note, page, pill, popover, quote, radio, rect,
    reset, ring, section, seg, set_theme, text, thinking_row, toggle, tool_row, tparts, tw, utabs, window_end,
    window_begin,
)

ROW_PITCH = 910
W, H = 2848, 200 + 12 * ROW_PITCH + 30
MULTI = dict(active=MULTI_ACTIVE, projects=MULTI_PROJECTS, selected="Qwen signatures")
TABS = ["THREAD", "CHANGES  2", "TREE", "CONTEXT"]
READY = dict(placeholder=True, thinking="high", buttons=(("Send  ⏎", "primary"),), stop=False)


def idle_composer(y=700, h=134):
    composer(y, h, "Ask pi to work on this project…", **READY)
    ws.expand_icon(y)


def footer_caption(s):
    text(1084, 832, s, C["faint"], size=10)


def first_turn(y):
    """The first turn of the Qwen signatures thread; returns the y of its jj line."""
    y += ws.user_card(y, ["qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response.",
                          "Accept empty signatures there, but keep the check strict for Anthropic."]) + 20
    thinking_row(y)
    ws.reply(y + 24, [("The check lives in ", C["text2"]), ("openai-completions.ts", C["code"], MONO),
                      (". I'll allow empty strings for OpenCode models only.", C["text2"])])
    tool_row(y + 36, "edit", "Edit", "packages/ai/src/providers/openai-completions.ts", "+3 −1")
    tool_row(y + 60, "terminal", "Bash", "npm run check", "passed")
    return y + 88


def hover_row(y, ic, verb, target):
    rect(276, y, 768, 24, C["hover"], 5)
    icon(ic, 288, y + 1, 14, C["faint"])
    text(310, y + 12, verb, C["muted"], size=12)
    text(350, y + 12, target, C["text2"], size=11.5, family=MONO)


def you_node(cx, cy):
    dot(cx, cy, 3.5, C["sage"])


# ---------------------------------------------------------------- 01 Turn links survive restarts

def r_relink():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=TABS, tab_right="~/repos/pi · @ zvtmrpyq",
                 status_right="jj · 2 turns linked · $0.41 · 31% context")
    line(280, 246.5, 540, 246.5, C["line"])
    line(784, 246.5, 1044, 246.5, C["line"])
    text(662, 250, "Reopened 10:14 · 2 turns linked from the session", C["faint"], size=11, anchor="middle")
    ws.thread_two_turns(hover_first=True, y0=266)
    idle_composer()

    insp_title("kqxlmwsv", "Turn 1 · linked from the session", "change", kind="accent", serif=False)
    section(290, "IN THE SESSION FILE")
    pl, key, st = C["pl"], C["faint"], C["str"]
    rows = [[('"type": ', key), ('"custom"', st), (",", pl)],
            [('"customType": ', key), ('"pi-desktop-jj"', st), (",", pl)],
            [('"data": {', key)],
            [('  "entry": ', key), ('"7f3c21d8"', st), (",", pl)],
            [('  "change": ', key), ('"kqxlmwsvpnzq…"', st), (" }", pl)]]
    rect(1084, 302, 288, 18 + 17 * len(rows), C["deep"], 6, C["line"])
    for i, parts in enumerate(rows):
        tparts(1096, 324 + i * 17, parts, size=10.5, family=MONO, pre=True)
    section(436, "LINK")
    kv(460, "Entry", "your prompt · 09:41")
    for i, s in enumerate(("The change is still visible", "The entry is on this branch")):
        icon("check", 1084, 473 + i * 22, 13, C["green"])
        text(1104, 484 + i * 22, s, C["text2"], size=12)
    divider(512)
    section(538, "GIT")
    kv(562, "Commit", "3f9e2a1 on main", mono=True)
    text(1084, 586, "Commit messages are left as they are.", C["faint"], size=11)
    note(1084, 790, 288, ["Written by the desktop backend; pi keeps", "custom entries out of the model's context."])
    footer_caption("Inspecting turn kqxlmwsv in Qwen signatures")


# ---------------------------------------------------------------- 02 Restore one file

def r_restore():
    ws.r_changes()
    px, py, pw, ph = 690, 276, 354, 136
    popover(px, py, pw, ph)
    icon("undo", px + 14, py + 13, 14, C["accent"])
    text(px + 36, py + 25, "Restore this file?", C["text"], size=13, weight=600)
    text(px + 14, py + 50, "openai-completions.ts goes back to how it was", C["text2"], size=12)
    text(px + 14, py + 68, "before turn kqxlmwsv. No later turn edits it.", C["text2"], size=12)
    line(px, py + 84.5, px + pw, py + 84.5, C["line"])
    text(px + 14, py + 115, "Undoable from History", C["faint"], size=11)
    w = button(px + pw - 14, py + 98, "Restore file", "primary", anchor="end")
    button(px + pw - 22 - w, py + 98, "Cancel", "ghost", anchor="end")


# ---------------------------------------------------------------- 03 A snapshot before every command

def r_snapshots():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  3", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · @ wlrtspqn", status_right="jj · snapshot before each command · $0.44")
    y = 236
    y += ws.user_card(y, ["npm run check fails on stale generated files. Clean them out and rebuild."]) + 20
    thinking_row(y, "Thought for 3s")
    ws.reply(y + 24, [("I'll reset the generated files, then rebuild.", C["text2"])])
    tool_row(y + 36, "edit", "Edit", "packages/ai/src/providers/openai-completions.ts", "+3 −1")
    ry = y + 60
    hover_row(ry, "terminal", "Bash", "git checkout -- .")
    w1 = button(1038, ry + 2, "Restore to before", "ghost", h=20, size=10.5, ic="undo", anchor="end")
    w2 = button(1038 - w1 - 6, ry + 2, "Diff", "ghost", h=20, size=10.5, anchor="end")
    pill(1038 - w1 - w2 - 16, ry + 4, "changed 5 files", C["amber"], blend(C["amber"], C["hover"], 0.12), size=10,
         anchor="end", border=blend(C["amber"], C["hover"], 0.4))
    tool_row(ry + 24, "terminal", "Bash", "npm run build", "passed")
    ws.turn_footer(ry + 56, "wlrtspqn", 1, 3, 1, current=True)

    px, py, pw = 548, ry + 54, 470
    files = [("packages/ai/src/providers/openai-completions.ts", [("+1 ", C["green"]), ("−3", C["coral"])], True),
             ("packages/ai/dist/index.js", [("−412", C["coral"])], False),
             ("packages/ai/dist/index.d.ts", [("−38", C["coral"])], False),
             ("packages/ai/dist/providers.js", [("−190", C["coral"])], False)]
    ph = 52 + 22 * len(files) + 22 + 52
    popover(px, py, pw, ph)
    text(px + 14, py + 24, "git checkout -- .", C["text"], size=11.5, family=MONO)
    text(px + 14 + tw("git checkout -- .", 11.5, True) + 8, py + 24, "changed 5 files", C["muted"], size=12)
    text(px + pw - 14, py + 24, "snapshot 09:47", C["faint"], size=10.5, family=MONO, anchor="end")
    line(px, py + 36.5, px + pw, py + 36.5, C["line"])
    yy = py + 58
    for path, delta, own in files:
        icon("file", px + 14, yy - 11, 13, C["amber"] if own else C["faint"])
        text(px + 34, yy, path, C["text2"], size=11, family=MONO)
        tparts(px + pw - 14, yy, delta, size=10.5, family=MONO, anchor="end")
        yy += 22
        if own:
            text(px + 34, yy - 4, "undid pi's edit from earlier in this turn", C["amber"], size=10.5)
            yy += 14
    text(px + 34, yy, "+1 more", C["faint"], size=11)
    line(px, yy + 14.5, px + pw, yy + 14.5, C["line"])
    text(px + 14, yy + 40, "Brings all 5 back as they were before it.", C["faint"], size=11)
    button(px + pw - 14, yy + 26, "Restore to before", "primary", ic="undo", anchor="end")
    idle_composer()

    insp_title("Qwen signatures", "Ready", "3 turns")
    ws.inspector_tabs(0)
    section(322, "HISTORY", "jj")
    line(1090.5, 342, 1090.5, 432, C["line"])
    ws.jj_node(1090, 342, current=True)
    text(1102, 346, "wlrtspqn", C["accent"], size=10.5, family=MONO)
    text(1176, 346, "Clean and rebuild", C["text2"], size=12)
    tparts(1372, 346, [("+3 ", C["green"]), ("−1", C["coral"])], size=10.5, family=MONO, anchor="end")
    for i, (cmd, t) in enumerate((("git checkout -- .", "09:47"), ("npm run build", "09:48"))):
        yy = 368 + i * 20
        rect(1101, yy - 7.5, 6, 6, C["lineStrong"], 1)
        tparts(1114, yy, [("before  ", C["faint"], SANS), (cmd, C["muted"], MONO)], size=10.5)
        text(1372, yy, t, C["faint"], size=10.5, family=MONO, anchor="end")
    for i, (change, desc, delta) in enumerate((("kqxlmwsv", "Empty signatures", [("+3 ", C["green"]), ("−1", C["coral"])]),
                                               ("main", "trunk", None))):
        yy = 414 + i * 22
        ws.jj_node(1090, yy - 4)
        text(1102, yy, change, C["faint"], size=10.5, family=MONO)
        text(1176, yy, desc, C["text2"], size=12)
        if delta:
            tparts(1372, yy, delta, size=10.5, family=MONO, anchor="end")
    text(1084, 474, "Snapshots stay inside the turn's change;", C["faint"], size=11)
    text(1084, 490, "only the turn's line shows in the thread.", C["faint"], size=11)
    divider(508)
    section(534, "SNAPSHOTS", "Settings")
    toggle(1084, 548, True)
    text(1122, 560, "Before each bash command", C["text2"], size=12)
    text(1084, 590, "On by default. Each one costs a jj", C["faint"], size=11)
    text(1084, 606, "snapshot, which is slower in large repos.", C["faint"], size=11)
    note(1084, 790, 288, ["A command that changes nothing", "leaves nothing: jj records differences."])
    footer_caption("Inspecting Qwen signatures in ~/repos/pi")


# ---------------------------------------------------------------- 04 Parallel sessions, separate workspaces

MULTI_WS = dict(
    active=[("Qwen signatures", "pi", "run"), ("Faux tests", "pi", "wait"), ("Sandbox perf", "minivm", "run")],
    projects=[("pi", "26", "run", [("Qwen signatures", "run"), ("Faux tests", "wait"), ("Mistral thinking", "2h"),
                                   ("Kimi K3 default", "5h")]),
              ("zed", "8", "wait", None), ("minivm", "12", "run", None)],
    selected="Faux tests")


def r_workspace():
    window_begin("Faux tests", title_prefix="pi", multi=MULTI_WS, tabs=["THREAD", "CHANGES", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · main", status_right="waiting for your answer · $0.00")
    ws.user_card(236, ["Add a regression test with the faux provider, no network."])
    spinner_row = 294
    ws.jj_node(292, spinner_row + 12)
    text(304, spinner_row + 16, "Not started: another session is working in this folder", C["faint"], size=11)
    idle_composer()
    insp_title("Faux tests", "Waiting", "new", kind="amber")
    ws.inspector_tabs(0)
    section(322, "HISTORY", "jj")
    text(1084, 348, "No turns yet.", C["faint"], size=12)
    footer_caption("Inspecting Faux tests in ~/repos/pi")

    # Asked once, before the first run, when another session is working in the same folder.
    dim_window()
    dx, dy, w, h = 400, 236, 560, 386
    popover(dx, dy, w, h)
    icon("layers", dx + 22, dy + 20, 16, C["accent"])
    text(dx + 48, dy + 34, "Qwen signatures is working in ~/repos/pi", C["text"], size=14, weight=600)
    text(dx + 22, dy + 62, "Running Faux tests in the same folder mixes both sessions' edits.", C["text2"], size=12)
    text(dx + 22, dy + 80, "Where should it work?", C["text2"], size=12)
    options = [
        (True, "Own workspace, automatic", "~/repos/pi-ws/faux-tests", None),
        (False, "Own workspace, in a folder you choose", "A jj workspace wherever you pick.", "Choose…"),
        (False, "Same folder", "Edits from both sessions can end up in one turn.", None),
    ]
    for i, (on, title, detail, action) in enumerate(options):
        y = dy + 100 + i * 64
        rect(dx + 16, y, w - 32, 56, C["select"] if on else "none", 6, C["focus"] if on else C["line"])
        radio(dx + 36, y + 20, on)
        text(dx + 54, y + 24, title, C["text"], size=12.5, weight=600 if on else 400)
        text(dx + 54, y + 42, detail, C["muted"], size=11, family=MONO if detail.startswith("~") else SANS)
        if action:
            button(dx + w - 30, y + 16, action, "ghost", h=24, anchor="end", ic="folder")
    y = dy + 100 + 3 * 64 + 6
    checkbox(dx + 22, y, False)
    text(dx + 44, y + 12, "Remember for this project", C["text2"], size=12)
    text(dx + 212, y + 12, "saved in .pi/pi-desktop.json", C["faint"], size=11, family=MONO)
    line(dx, dy + h - 56.5, dx + w, dy + h - 56.5, C["line"])
    text(dx + 22, dy + h - 24, "Bring its turns into the main folder later.", C["faint"], size=11)
    wf = button(dx + w - 18, dy + h - 40, "Start", "primary", anchor="end")
    button(dx + w - 26 - wf, dy + h - 40, "Cancel", "ghost", anchor="end")


# ---------------------------------------------------------------- 05 Fork with the files

def r_fork_files():
    begin = views.window_begin

    def multi_begin(*args, **kwargs):
        kwargs.pop("session_sel", None)
        kwargs.pop("states", None)
        return begin(*args, multi=MULTI, title_prefix="pi", **kwargs)

    views.window_begin = multi_begin
    try:
        views.v_tree()
    finally:
        views.window_begin = begin
    rect(1064, 182, 328, 664, C["side"])
    insp_title("Live endpoint test", "Not on the current path", "3 entries", kind="faint")
    section(290, "ENTRY")
    kv(316, "Role", "pi")
    kv(340, "Time", "09:54", mono=True)
    kv(364, "Files after it", "pmrzkwot · +31", mono=True)
    divider(382)
    section(408, "ACTIONS")
    button(1084, 420, "Continue from here", "primary", w=288)
    button(1084, 452, "Fork to new session", "ghost", w=184, ic="branch")
    rect(1084, 452, 184, 24, "none", 5, C["focus"])
    button(1276, 452, "Label…", "ghost", w=96, ic="tag")
    note(1084, 790, 288, ["Fork copied the conversation; now it", "can take the files at that point along."])
    footer_caption("Inspecting an entry on an inactive branch")

    # Fork asks only when the files differ from the ones at this entry.
    dim_window()
    dx, dy, w, h = 420, 300, 500, 276
    popover(dx, dy, w, h)
    icon("branch", dx + 22, dy + 20, 16, C["accent"])
    text(dx + 48, dy + 34, "Fork from “Live endpoint test”", C["text"], size=14, weight=600)
    text(dx + 22, dy + 62, "The files changed after this entry. Which files should the", C["text2"], size=12)
    text(dx + 22, dy + 80, "new session start with?", C["text2"], size=12)
    options = [(True, "As at this entry", "A new jj workspace at pmrzkwot. The main folder", "is not touched."),
               (False, "As they are now", "The new session works in ~/repos/pi with", "today's files.")]
    for i, (on, title, l1, l2) in enumerate(options):
        y = dy + 100 + i * 66
        rect(dx + 16, y, w - 32, 58, C["select"] if on else "none", 6, C["focus"] if on else C["line"])
        radio(dx + 36, y + 20, on)
        text(dx + 54, y + 24, title, C["text"], size=12.5, weight=600 if on else 400)
        text(dx + 54, y + 42, l1 + " " + l2, C["muted"], size=11)
    text(dx + 22, dy + h - 22, "Only asked when the files differ.", C["faint"], size=11)
    wf = button(dx + w - 18, dy + h - 38, "Fork", "primary", anchor="end")
    button(dx + w - 26 - wf, dy + h - 38, "Cancel", "ghost", anchor="end")


# ---------------------------------------------------------------- 06 Best of N

ATTEMPTS = [
    ("A", "claude-opus-5-5", "high", ("check", "Done · 2m 10s", "green"),
     [("tokenizer.ts", "+48", "−25"), ("lexer.ts", "+6", "−2")], ("214 passed", "green"), ("−41%", "green"),
     "$0.31 · 41.2k tokens",
     [("del", "for (const ch of input) {"), ("add", "for (let i = 0; i < input.length;"), ("add", "     i++) {"),
      ("add", "  const code = input.charCodeAt(i);")]),
    ("B", "gpt-5.5", "high", ("x", "Tests failed · 3m 02s", "coral"),
     [("tokenizer.ts", "+61", "−30"), ("cache.ts", "+18", None)], ("3 failed", "coral"), ("−52%", "green"),
     "$0.08 · 39.9k tokens",
     [("add", "const cache = new Map<string,"), ("add", "  Token[]>();"), ("del", "return tokens;"),
      ("add", "cache.set(input, tokens);")]),
    ("C", "kimi-k3", "med", ("check", "Done · 1m 44s", "green"),
     [("tokenizer.ts", "+22", "−9")], ("214 passed", "green"), ("−34%", "green"), "$0.03 · 22.7k tokens",
     [("del", "const parts = input.split(/\\s+/);"), ("add", "let start = 0;"), ("add", "while (start < input.length) {"),
      ("add", "  const end = nextBreak(start);")]),
]


def attempt_card(x, y, w, h, attempt, selected):
    key, model, level, (st_icon, st_text, st_col), files, (tests, tcol), (bench, bcol), cost, diff = attempt
    rect(x, y, w, h, C["side"], 8, C["accent"] if selected else C["card"])
    rect(x + 12, y + 12, 22, 22, C["select"] if selected else C["hover"], 5, C["focus"] if selected else C["line"])
    text(x + 23, y + 27, key, C["accent"] if selected else C["text2"], size=11.5, weight=600, family=MONO, anchor="middle")
    text(x + 44, y + 27, model, C["text"], size=11.5, family=MONO)
    dot(x + w - 14 - tw(level, 10.5, True) - 9, y + 23, 3.5, ws.THINK_COLOR[level])
    text(x + w - 14, y + 27, level, C["muted"], size=10.5, family=MONO, anchor="end")
    icon(st_icon, x + 12, y + 44, 13, C[st_col])
    text(x + 32, y + 55, st_text, C["text2"], size=12)
    line(x + 12, y + 70.5, x + w - 12, y + 70.5, C["line"])
    yy = y + 92
    for name, plus, minus in files:
        icon("file", x + 12, yy - 11, 13, C["faint"])
        text(x + 32, yy, name, C["text2"], size=11, family=MONO)
        tparts(x + w - 14, yy, [(plus, C["green"])] + ([(" " + minus, C["coral"])] if minus else []), size=10.5,
               family=MONO, anchor="end")
        yy += 20
    yy = y + 150
    for ic, name, value, col in (("terminal", "npm test", tests, tcol), ("gauge", "bench", bench, bcol)):
        icon(ic, x + 12, yy - 11, 13, C["faint"])
        text(x + 32, yy, name, C["muted"], size=11, family=MONO)
        text(x + w - 14, yy, value, C[col], size=11, family=MONO, anchor="end")
        yy += 20
    rect(x + 12, y + 186, w - 24, 84, C["deep"], 5)
    for i, (kind, code) in enumerate(diff):
        ly = y + 192 + i * 18
        rect(x + 12, ly, w - 24, 18, C["addBg"] if kind == "add" else C["delBg"])
        text(x + 20, ly + 13, "+" if kind == "add" else "−", C["green"] if kind == "add" else C["coral"], size=10,
             family=MONO)
        text(x + 32, ly + 13, code, C["pl"], size=10, family=MONO, pre=True)
    text(x + 12, y + 292, cost, C["faint"], size=10.5, family=MONO)
    wk = button(x + w - 12, y + h - 36, f"Keep {key}", "primary" if selected else "ghost", anchor="end")
    button(x + w - 20 - wk, y + h - 36, "Diff", "ghost", anchor="end")


def r_best_of():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=TABS, tab_right="~/repos/pi · 3 workspaces",
                 status_right="3 attempts · $0.42 · 31% context")
    ws.user_card(236, ["Make the tokenizer at least 30% faster without changing its output."])
    pill(1030, 247, "3 attempts", C["accent"], C["select"], size=10, anchor="end", border=C["focus"])
    for i, attempt in enumerate(ATTEMPTS):
        attempt_card(280 + i * 258, 290, 248, 350, attempt, selected=attempt[0] == "C")
    composer(700, 134, "Ask pi to work on this project…", **READY)
    ws.expand_icon(700)
    by = 796
    base = C["chip"] if THEME["name"] == "dark" else C["hover"]
    aw = tw("1 attempt", 11, True) + 50
    rect(590, by, round(aw, 1), 24, base, 5)
    icon("layers", 597, by + 5, 13, C["muted"])
    text(616, by + 16, "1 attempt", C["text2"], size=11, family=MONO)
    icon("chevron", 590 + aw - 18, by + 6, 11, C["faint"])

    insp_title("Attempt C", "kimi-k3 · tests pass", "3 attempts", kind="green", serif=False)
    section(290, "COMPARE")
    cols = [(1084, "start", ""), (1196, "end", "TESTS"), (1262, "end", "LINES"), (1316, "end", "BENCH"),
            (1372, "end", "COST")]
    for x, anchor, s in cols:
        label(x, 314, s, anchor=anchor, size=9)
    rows = [("A", "opus", ("✓", "green"), "+54 −27", "−41%", "$0.31"),
            ("B", "gpt-5.5", ("3 ✕", "coral"), "+79 −30", "−52%", "$0.08"),
            ("C", "kimi-k3", ("✓", "green"), "+22 −9", "−34%", "$0.03")]
    for i, (key, model, (t, tc), lines_, bench, cost) in enumerate(rows):
        yy = 338 + i * 24
        if key == "C":
            rect(1076, yy - 16, 304, 22, C["select"], 4)
        text(1084, yy, key, C["accent"] if key == "C" else C["text2"], size=11.5, weight=600, family=MONO)
        text(1100, yy, model, C["text2"], size=11.5, family=MONO)
        text(1196, yy, t, C[tc], size=11, family=MONO, anchor="end")
        text(1262, yy, lines_, C["muted"], size=10.5, family=MONO, anchor="end")
        text(1316, yy, bench, C["green"], size=10.5, family=MONO, anchor="end")
        text(1372, yy, cost, C["muted"], size=10.5, family=MONO, anchor="end")
    divider(408)
    section(434, "KEEP")
    text(1084, 458, "Keeping C brings its turn into the main", C["text2"], size=12)
    text(1084, 476, "folder. A and B are abandoned; History", C["text2"], size=12)
    text(1084, 494, "can bring them back.", C["text2"], size=12)
    button(1084, 510, "Keep attempt C", "primary", w=288)
    button(1084, 542, "Diff C against A", "ghost", w=288)
    note(1084, 790, 288, ["Each attempt runs in its own workspace,", "from the same starting change."])
    footer_caption("Comparing 3 attempts in Qwen signatures")


# ---------------------------------------------------------------- 07 Which turn wrote this line

def code_lines():
    kw, st, pl, fn, cm = C["kw"], C["str"], C["pl"], C["code"], C["faint"]
    return [
        (205, [("function", kw), (" ", pl), ("readThinking", fn), ("(message: AssistantMessage, model: Model) {", pl)]),
        (206, [("  ", pl), ("const", kw), (" blocks = message.content;", pl)]),
        (207, [("  ", pl), ("const", kw), (" signatures: ", pl), ("string", kw), ("[] = [];", pl)]),
        (208, [("  ", pl), ("for", kw), (" (", pl), ("const", kw), (" block ", pl), ("of", kw), (" blocks) {", pl)]),
        (209, [("    ", pl), ("if", kw), (" (block.type === ", pl), ('"text"', st), (") ", pl), ("continue", kw), (";", pl)]),
        (210, [("    ", pl), ("// OpenCode sends empty signatures; Anthropic must not.", cm)]),
        (211, [("    ", pl), ("if", kw), (" (block.type === ", pl), ('"thinking"', st), (") {", pl)]),
        (212, [("      ", pl), ("if", kw), (" (block.signature?.length === ", pl), ("0", C["amber"]),
               (" && isAnthropic(model)) {", pl)]),
        (213, [("        ", pl), ("throw", kw), (" ", pl), ("new", kw), (" ", pl), ("MissingSignature", fn),
               ("(model.id);", pl)]),
        (214, [("      }", pl)]),
        (215, [("      signatures.", pl), ("push", fn), ("(block.signature ?? ", pl), ('""', st), (");", pl)]),
        (216, [("    }", pl)]),
        (217, [("  }", pl)]),
        (218, [("  ", pl), ("return", kw), (" signatures;", pl)]),
        (219, [("}", pl)]),
    ]


def r_annotate():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=TABS, tab_active=-1, tab_right=None,
                 status_right="TypeScript · Ln 212 · ⌥ held: turns shown")
    line(716.5, 190, 716.5, 214, C["line"])
    icon("file", 734, 194, 13, C["accent"])
    text(754, 206, "openai-completions.ts", C["text"], size=11.5, family=MONO)
    icon("x", 918, 196, 11, C["faint"])
    rect(734, 219, 196, 2, C["accent"], 1)

    owner = {205: "other", 206: "other", 207: "other", 210: "turn", 212: "turn", 213: "turn", 214: "turn"}
    colors = {"turn": C["accent"], "other": C["tidal"] if THEME["name"] == "dark" else blend(C["tidal"], C["canvas"], 0.5)}
    y0 = 246
    for i, (n, parts) in enumerate(code_lines()):
        y = y0 + i * 20
        if n == 212:
            rect(256, y - 14, 808, 20, C["hover"])
        who = owner.get(n)
        if who:
            rect(309, y - 14, 4 if n == 212 else 3, 20, colors[who])
        text(300, y, str(n), C["muted"] if n == 212 else C["faint"], size=11, family=MONO, anchor="end")
        tparts(324, y, parts, size=11.5, family=MONO, pre=True)
        if n == 212:
            text(796, y, "kqxlmwsv · turn 1 · Qwen signatures", C["faint"], size=10.5, family=MONO)

    px, py, pw, ph = 652, y0 + 7 * 20 + 12, 400, 132
    popover(px, py, pw, ph)
    ws.jj_node(px + 18, py + 20)
    text(px + 30, py + 24, "kqxlmwsv", C["accent"], size=11, family=MONO)
    text(px + 96, py + 24, "turn 1 · Qwen signatures · 09:43", C["muted"], size=11.5)
    text(px + pw - 14, py + 24, "4 lines", C["faint"], size=10.5, family=MONO, anchor="end")
    quote(px + 14, py + 36, pw - 28, ["qwen3.8-flash on OpenCode returns empty thinking signatures",
                                      "and we reject the response. Accept empty signatures there…"], size=11.5)
    w = button(px + 14, py + ph - 34, "Show in thread", "ghost", h=22, size=11, ic="message")
    button(px + 22 + w, py + ph - 34, "Diff", "ghost", h=22, size=11)
    text(px + pw - 14, py + ph - 19, "⌥-click a line", C["faint"], size=10.5, family=MONO, anchor="end")

    ly = 578
    line(256, ly - 20.5, 1064, ly - 20.5, C["line"])
    x = 280 + keycap(280, ly - 13, "⌥") + 8
    text(x, ly, "held:", C["faint"], size=11)
    x += 40
    for key, s in (("turn", "this session's turns"), ("other", "other sessions' turns")):
        rect(x, ly - 11, 3, 14, colors[key])
        text(x + 10, ly, s, C["faint"], size=11)
        x += tw(s, 11) + 40
    text(x, ly, "no bar: written by you, or before jj", C["faint"], size=11)

    insp_title("openai-completions.ts", "Written by 2 turns", "jj", kind="accent", serif=False)
    section(290, "WHO WROTE THIS FILE", "419 lines")
    rows = [("turn", "kqxlmwsv", "turn 1 · this session", "4 lines"),
            ("other", "pmwqlrzk", "OpenCode Go defaults", "3 lines"),
            (None, "", "You, or before jj", "412 lines")]
    for i, (key, change, who, count) in enumerate(rows):
        yy = 316 + i * 24
        rect(1084, yy - 11, 3, 14, colors[key] if key else C["chipLine"])
        x = 1096
        if change:
            text(x, yy, change, C["faint"], size=10.5, family=MONO)
            x += 66
        text(x, yy, who, C["text2"], size=12)
        text(1372, yy, count, C["faint"], size=10.5, family=MONO, anchor="end")
    divider(378)
    section(404, "SHOW")
    w = keycap(1084, 416, "⌥")
    text(1084 + w + 10, 430, "Hold to show turn bars", C["text2"], size=12)
    w = keycap(1084, 446, "⌥ click")
    text(1084 + w + 10, 460, "A line's turn and prompt", C["text2"], size=12)
    text(1084, 492, "Nothing is shown otherwise, so the", C["faint"], size=11)
    text(1084, 508, "editor looks as it always does.", C["faint"], size=11)
    note(1084, 790, 288, ["From jj's line annotation, so bars follow", "lines through later edits and moves."])
    footer_caption("Inspecting openai-completions.ts")


def keycap(x, y, s):
    w = tw(s, 11, True) + 14
    rect(x, y, round(w, 1), 18, C["chip"], 4, C["chipLine"])
    text(x + w / 2, y + 13, s, C["text2"], size=11, family=MONO, anchor="middle")
    return w


# ---------------------------------------------------------------- 08 Your edits, told to pi

def edit_chip(x, y, name, h=20, size=11):
    w = tw(name, size, True) + 46
    rect(x, y, round(w, 1), h, blend(C["sage"], C["canvas"], 0.14), 4, blend(C["sage"], C["canvas"], 0.45))
    icon("edit", x + 6, y + h / 2 - 6, 12, C["sage"])
    text(x + 22, y + h / 2 + size * 0.36, name, C["sage"], size=size, family=MONO)
    icon("x", x + w - 16, y + h / 2 - 5, 10, C["sage"])
    return w


def r_your_edits():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=TABS, tab_right="~/repos/pi · @ ytmnoqvs",
                 status_right="jj · your edits since turn 1 · $0.41")
    y = first_turn(236)
    ws.turn_footer(y, "kqxlmwsv", 1, 3, 1)
    y += 28
    rect(276, y, 768, 24, C["hover"], 5)
    you_node(292, y + 12)
    text(304, y + 16, "you", C["sage"], size=10.5, family=MONO)
    tparts(334, y + 16, [("edited 2 files  ", C["faint"]), ("+3", C["green"]), (" −1", C["coral"])], size=10.5,
           family=MONO)
    text(900, y + 16, "after this turn · 10:02", C["faint"], size=10.5, family=MONO, anchor="end")
    button(1038, y + 2, "Diff", "ghost", h=20, size=10.5, anchor="end")

    base = C["bar"] if THEME["name"] == "dark" else C["chip"]
    rect(276, 700, 768, 134, base, 10, C["focus"])
    w = edit_chip(292, 721, "Your edits · 2 files")
    s = "Now cover the Anthropic case in the test too"
    text(292 + w + 8, 735, s, C["text"], size=13)
    rect(292 + w + 8 + tw(s, 13) * 0.98 + 2, 722, 1.5, 16, C["accent"])
    ws.expand_icon(700)
    icon("clip", 292, 800, 15, C["muted"])
    icon("slash", 318, 800, 15, C["muted"])
    button(1038, 796, "Send  ⏎", "primary", anchor="end")

    px, py, pw, ph = 284, 566, 380, 126
    popover(px, py, pw, ph)
    text(px + 14, py + 24, "Since turn kqxlmwsv", C["text"], size=12.5, weight=600)
    text(px + pw - 14, py + 24, "you · 10:02", C["faint"], size=10.5, family=MONO, anchor="end")
    for i, (path, delta) in enumerate((("providers/openai-completions.ts", [("+2", C["green"]), (" −1", C["coral"])]),
                                       ("providers/opencode.ts", [("+1", C["green"])]))):
        yy = py + 50 + i * 22
        icon("file", px + 14, yy - 11, 13, C["faint"])
        text(px + 34, yy, path, C["text2"], size=11, family=MONO)
        tparts(px + pw - 14, yy, delta, size=10.5, family=MONO, anchor="end")
    line(px, py + 88.5, px + pw, py + 88.5, C["line"])
    text(px + 14, py + 110, "Sent as a diff with your message. × leaves it out.", C["faint"], size=11)

    insp_title("Qwen signatures", "Ready", "1 turn")
    ws.inspector_tabs(0)
    section(322, "IN THIS PROMPT")
    icon("edit", 1084, 337, 13, C["sage"])
    text(1104, 348, "Your edits since kqxlmwsv", C["text2"], size=11.5)
    text(1372, 348, "diff · 0.4k tokens", C["faint"], size=10.5, anchor="end")
    for i, (name, delta) in enumerate((("openai-completions.ts", [("+2", C["green"]), (" −1", C["coral"])]),
                                       ("opencode.ts", [("+1", C["green"])]))):
        yy = 370 + i * 20
        text(1104, yy, name, C["muted"], size=11, family=MONO)
        tparts(1372, yy, delta, size=10.5, family=MONO, anchor="end")
    text(1084, 428, "Added when files changed since pi's last", C["faint"], size=11)
    text(1084, 444, "turn, so pi does not work from stale reads.", C["faint"], size=11)
    divider(462)
    section(488, "HISTORY", "jj")
    rows = [("ytmnoqvs", "Your edits", [("+3 ", C["green"]), ("−1", C["coral"])], "you"),
            ("kqxlmwsv", "Empty signatures", [("+3 ", C["green"]), ("−1", C["coral"])], "turn"),
            ("main", "trunk", None, "base")]
    line(1090.5, 508, 1090.5, 508 + 24 * 2, C["line"])
    for i, (change, desc, delta, kind) in enumerate(rows):
        yy = 512 + i * 24
        if kind == "you":
            you_node(1090, yy - 4)
        else:
            ws.jj_node(1090, yy - 4)
        text(1102, yy, change, C["sage"] if kind == "you" else C["faint"], size=10.5, family=MONO)
        text(1176, yy, desc, C["text2"], size=12)
        if delta:
            tparts(1372, yy, delta, size=10.5, family=MONO, anchor="end")
    note(1084, 790, 288, ["Your edits and pi's turns have their own", "colours in the thread and in History."])
    footer_caption("Inspecting Qwen signatures in ~/repos/pi")


# ---------------------------------------------------------------- 09 Land

def r_land():
    ws.r_changes()
    dim_window()
    dx, dy, w, h = 400, 206, 640, 612
    popover(dx, dy, w, h)
    text(dx + 24, dy + 40, "Land 2 turns", C["text"], size=20, family=SERIF, italic=True)
    text(dx + 24, dy + 62, "Squash them into one commit on main and push it.", C["muted"], size=12)
    icon("x", dx + w - 30, dy + 20, 13, C["muted"])
    label(dx + 24, dy + 98, "TURNS")
    turns = [(True, "zvtmrpyq", "Faux-provider test", [("+24", C["green"])], False),
             (True, "kqxlmwsv", "Empty signatures", [("+3 ", C["green"]), ("−1", C["coral"])], False),
             (False, "ytmnoqvs", "Your edits (working copy)", [("+3 ", C["green"]), ("−1", C["coral"])], True)]
    for i, (on, change, desc, delta, you) in enumerate(turns):
        y = dy + 110 + i * 28
        checkbox(dx + 24, y + 5, on)
        if you:
            you_node(dx + 56, y + 12)
        else:
            ws.jj_node(dx + 56, y + 12)
        text(dx + 68, y + 16, change, C["sage"] if you else C["faint"], size=10.5, family=MONO)
        text(dx + 142, y + 16, desc, C["text2"] if on else C["faint"], size=12)
        tparts(dx + w - 24, y + 16, delta, size=10.5, family=MONO, anchor="end")
    y = dy + 202
    toggle(dx + 24, y, True)
    text(dx + 62, y + 12, "Squash into one commit", C["text2"], size=12)
    text(dx + w - 24, y + 12, "else each turn stays a commit", C["faint"], size=11, anchor="end")
    y = dy + 252
    label(dx + 24, y, "MESSAGE")
    icon("sparkle", dx + 96, y - 11, 12, C["muted"])
    text(dx + 114, y, "written by pi from the diff", C["faint"], size=11)
    text(dx + w - 24, y, "[ REGENERATE ]", C["accent"], size=10, family=MONO, anchor="end", ls=0.8)
    rect(dx + 24, y + 12, w - 48, 104, C["canvas"], 6, C["focus"])
    msg = [("fix(ai): accept empty thinking signatures from OpenCode", C["text"]), ("", None),
           ("OpenCode's qwen3.8-flash sends empty signatures. Keep the", C["text2"]),
           ("check strict for Anthropic and add a faux-provider test.", C["text2"])]
    for i, (s, col) in enumerate(msg):
        if s:
            text(dx + 38, y + 34 + i * 19, s, col, size=11.5, family=MONO)
    checkbox(dx + 24, y + 128, False)
    text(dx + 46, y + 140, "Keep pi's Pi-Session and Pi-Entry trailers in the commit", C["text2"], size=12)
    y = dy + 430
    label(dx + 24, y, "BOOKMARK")
    input_box(dx + 24, y + 10, 300, "qwen-empty-signatures")
    label(dx + 348, y, "PUSH TO")
    seg(dx + 348, y + 13, ["origin", "fork", "Don't push"], 0)
    line(dx, dy + 492.5, dx + w, dy + 492.5, C["line"])
    text(dx + 24, dy + 518, "jj squash · jj describe · jj bookmark set · jj git push", C["faint"], size=10.5, family=MONO)
    text(dx + 24, dy + 538, "Everything but the push can be undone from History.", C["faint"], size=11)
    wl = button(dx + w - 24, dy + h - 50, "Land and push", "primary", h=28, anchor="end")
    button(dx + w - 32 - wl, dy + h - 50, "Cancel", "ghost", h=28, anchor="end")


# ---------------------------------------------------------------- 10 Undo with conflicts

def r_conflict_undo():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=TABS, tab_right="~/repos/pi · @ zvtmrpyq",
                 status_right="jj · 1 file with conflicts · $0.41 · 31% context")
    add('<g opacity="0.45">')
    y = first_turn(236)
    add("</g>")
    icon("undo", 286, y + 5, 13, C["muted"])
    tparts(306, y + 16, [("Turn undone", C["text2"]), ("  ·  change kqxlmwsv abandoned, zvtmrpyq kept", C["faint"])],
           size=12)
    button(1038, y + 2, "Redo", "ghost", h=20, size=10.5, ic="redo", anchor="end")
    y += 36
    y += ws.user_card(y, ["Also treat a null signature like an empty one."]) + 22
    ws.reply(y, [("Handled ", C["text2"]), ("null", C["code"], MONO), (" next to the empty string.", C["text2"])])
    tool_row(y + 12, "edit", "Edit", "openai-completions.ts", "+2 −1")
    y += 44
    tone = C["amber"]
    rect(276, y, 768, 24, blend(tone, C["canvas"], 0.08), 5)
    icon("warning", 286, y + 5, 13, tone)
    tparts(306, y + 16, [("zvtmrpyq", C["faint"], MONO), ("  openai-completions.ts has conflict markers", C["text2"])],
           size=11.5)
    button(1038, y + 2, "Open file", "ghost", h=20, size=10.5, ic="file", anchor="end")

    # The fix is drafted, not sent: you read it and press Send.
    base = C["bar"] if THEME["name"] == "dark" else C["chip"]
    rect(276, 700, 768, 134, base, 10, C["focus"])
    x = 292
    text(x, 735, "Resolve the conflict markers in", C["text"], size=13)
    x += tw("Resolve the conflict markers in", 13) * 0.98 + 8
    x += ws.mention_chip(x, 721, "file", "openai-completions.ts") + 8
    text(x, 735, "left by undoing", C["text"], size=13)
    x += tw("left by undoing", 13) * 0.98 + 8
    x += ws.mention_chip(x, 721, "undo", "kqxlmwsv") + 4
    text(x, 735, ".", C["text"], size=13)
    text(292, 760, "Keep the null handling from zvtmrpyq.", C["text"], size=13)
    rect(292 + tw("Keep the null handling from zvtmrpyq.", 13) * 0.98 + 2, 747, 1.5, 16, C["accent"])
    ws.expand_icon(700)
    icon("clip", 292, 800, 15, C["muted"])
    icon("slash", 318, 800, 15, C["muted"])
    text(346, 812, "Drafted by the app · edit it or send it", C["faint"], size=11)
    button(1038, 796, "Send  ⏎", "primary", anchor="end")

    insp_title("Qwen signatures", "Ready", "2 turns")
    ws.inspector_tabs(0)
    section(322, "WHEN UNDO WOULD CONFLICT")
    text(1084, 346, "Undo asks first:", C["text2"], size=12)
    button(1084, 358, "Undo and draft a fix", "primary", w=288, ic="sparkle")
    button(1084, 390, "Undo both turns", "ghost", w=140)
    button(1232, 390, "Cancel", "ghost", w=140)
    divider(430)
    section(456, "THEN")
    steps = [["kqxlmwsv is abandoned."],
             ["zvtmrpyq is rebased onto its parent;", "openai-completions.ts gets conflict markers."],
             ["The composer gets a draft asking pi", "to resolve them. You send it."],
             ["pi's fix is a new turn you can undo."]]
    yy = 484
    for i, lines_ in enumerate(steps):
        ring(1092, yy - 4, 8, C["lineStrong"], 1.2, C["side"])
        text(1092, yy, str(i + 1), C["muted"], size=10, family=MONO, anchor="middle")
        for j, s_ in enumerate(lines_):
            text(1108, yy + j * 17, s_, C["text2"], size=12)
        yy += 17 * len(lines_) + 14
    note(1084, 790, 288, ["jj keeps the conflict inside the change,", "so nothing is lost if the fix is wrong."])
    footer_caption("Inspecting Qwen signatures in ~/repos/pi")


# ---------------------------------------------------------------- 11 History of every operation

OPS = [
    ("10:14", "undo", "Undo turn zvtmrpyq", "you", None),
    ("10:02", "layers", "Record turn zvtmrpyq · Faux-provider test", "pi", [("+24", "green")]),
    ("10:01", "terminal", "Snapshot before  vitest --run test/openai-completions.test.ts", "pi", None),
    ("09:58", "edit", "Your edits · CHANGELOG.md", "you", [("+1", "green")]),
    ("09:52", "branch", "git commit made outside the app · 3f9e2a1", "git", None),
    ("09:43", "layers", "Record turn kqxlmwsv · Empty signatures", "pi", [("+3 ", "green"), ("−1", "coral")]),
    ("09:42", "terminal", "Snapshot before  npm run check", "pi", None),
    ("09:41", "clock", "Snapshot before the turn", "pi", None),
    ("09:30", "check", "Turn on jj", "you", None),
]


def r_oplog():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=TABS, tab_active=1,
                 tab_right="~/repos/pi · @ kqxlmwsv", status_right="jj op log · 38 operations")
    utabs(280, 250, ["TURNS  2", "OPERATIONS  38"], 1, size=10, gap=22)
    line(264, 264.5, 1056, 264.5, C["line"])
    text(1044, 250, "this project, all sessions", C["faint"], size=11, anchor="end")
    y0, pitch, sel = 276, 34, 5
    for i, (t, ic, desc, who, delta) in enumerate(OPS):
        y = y0 + i * pitch
        if i == sel:
            rect(266, y + 1, 788, pitch - 2, C["select"], 5)
            rect(266, y + 7, 2, pitch - 14, C["accent"], 1)
        elif i % 2:
            rect(266, y + 1, 788, pitch - 2, C["rowAlt"], 5)
        cy = y + pitch / 2
        text(288, cy + 4, t, C["faint"], size=10.5, family=MONO)
        icon(ic, 336, cy - 7, 14, C["muted"])
        if desc.startswith("Snapshot before  "):
            tparts(362, cy + 4, [("Snapshot before  ", C["muted"]), (desc[17:], C["text2"], MONO)], size=12)
        else:
            text(362, cy + 4, desc, C["text"] if i == sel else C["text2"], size=12.5)
        fg = {"pi": C["accent"], "you": C["sage"], "git": C["muted"]}[who]
        pill(880, cy - 8, who, fg, blend(fg, C["canvas"], 0.12), size=10, border=blend(fg, C["canvas"], 0.35))
        if i == sel:
            button(1040, cy - 11, "Restore to here", "ghost", h=22, size=11, ic="undo", anchor="end")
        elif delta:
            tparts(1040, cy + 4, [(s, C[c]) for s, c in delta], size=10.5, family=MONO, anchor="end")
    y = y0 + len(OPS) * pitch + 26
    text(288, y, "Everything that changed the files is here, including commands and edits made outside pi.", C["faint"],
         size=11)
    text(288, y + 18, "Show 29 older", C["accent"], size=11)

    insp_title("Record turn kqxlmwsv", "09:43 · by pi", "jj op", kind="accent", serif=False, size=20)
    section(290, "RESTORING TO HERE")
    text(1084, 314, "Puts the project's files back as they", C["text2"], size=12)
    text(1084, 332, "were at 09:43:", C["text2"], size=12)
    for i, (name, what, col) in enumerate((("openai-completions.test.ts", "removed", "coral"),
                                            ("CHANGELOG.md", "−1", "coral"),
                                            ("commit 3f9e2a1", "undone", "muted"))):
        yy = 358 + i * 22
        icon("file" if i < 2 else "branch", 1084, yy - 11, 13, C["faint"])
        text(1104, yy, name, C["text2"], size=11, family=MONO)
        text(1372, yy, what, C[col], size=10.5, family=MONO, anchor="end")
    divider(424)
    button(1084, 440, "Restore project to 09:43", "primary", w=288, ic="undo")
    text(1084, 492, "Restoring is an operation too, so you", C["faint"], size=11)
    text(1084, 508, "can restore back to 10:14 afterwards.", C["faint"], size=11)
    note(1084, 790, 288, ["Same as jj op restore, with the jj", "built into the app."])
    footer_caption("Inspecting an operation in ~/repos/pi")


# ---------------------------------------------------------------- 12 jj tools for pi

def jj_row(y, name, target, meta):
    icon("branch", 288, y + 1, 14, C["faint"])
    text(310, y + 12, name, C["muted"], size=11.5, family=MONO)
    text(310 + tw(name, 11.5, True) + 12, y + 12, target, C["text2"], size=11.5, family=MONO)
    text(1010, y + 12, meta, C["faint"], size=10.5, family=MONO, anchor="end")
    icon("check", 1018, y + 1, 13, C["green"])


def r_jj_tools():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  5", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · @ qpvuntsm", status_right="jj · 5 turns · $0.63 · 38% context")
    y = 236
    y += ws.user_card(y, ["The parser broke somewhere in the last few turns. Find where."]) + 20
    thinking_row(y, "Thought for 4s")
    jj_row(y + 12, "jj_log", "last 5 turns", "read-only")
    wy = y + 42
    rect(280, wy, 764, 76, C["deep"], 6)
    log = [("qpvuntsm", "turn 5", "add streaming", "3 files", "+80", "−12"),
           ("kkmpptxz", "turn 4", "fix the parser", "1 file", "+9", "−4"),
           ("wlrtspqn", "turn 3", "lexer cleanup", "2 files", "+14", "−20")]
    for i, (change, turn, desc, files, plus, minus) in enumerate(log):
        yy = wy + 22 + i * 19
        tparts(296, yy, [(change + "  ", C["accent"] if i == 1 else C["faint"]), (turn + "  ", C["faint"]),
                         (f"{desc:<18}", C["pl"]), (f"{files:<9}", C["faint"]), (plus, C["green"]), (" " + minus, C["coral"])],
               size=11, family=MONO, pre=True)
    y = wy + 88
    jj_row(y, "jj_diff", "kkmpptxz  src/parser.ts", "+9 −4")
    y += 40
    ws.reply(y, [("Turn 4 (", C["text2"]), ("kkmpptxz", C["code"], MONO), (") moved ", C["text2"]),
                 ("advance()", C["code"], MONO), (" ahead of the lookahead at line 142, so the", C["text2"])])
    ws.reply(y + 20, [("lookahead reads the next token instead of the current one.", C["text2"])])
    y += 40
    tone = C["accent"]
    rect(280, y, 764, 42, blend(tone, C["canvas"], 0.07), 8, blend(tone, C["canvas"], 0.32))
    icon("undo", 294, y + 13, 15, tone)
    tparts(318, y + 26, [("pi suggests undoing turn ", C["text"]), ("kkmpptxz", C["code"], MONO),
                         (" · only you can undo it", C["faint"])], size=12.5)
    w = button(1030, y + 9, "Undo turn kkmpptxz", "primary", ic="undo", anchor="end")
    button(1030 - w - 8, y + 9, "Dismiss", "ghost", anchor="end")
    idle_composer()

    insp_title("Qwen signatures", "Ready", "5 turns")
    ws.inspector_tabs(0)
    section(322, "JJ TOOLS FOR PI", "read-only")
    toggle(1084, 336, True)
    text(1122, 348, "Let pi read this project's jj history", C["text2"], size=12)
    text(1122, 366, "Off by default; turned on here.", C["faint"], size=11)
    for i, (name, desc) in enumerate((("jj_log", "turns and changes"), ("jj_diff", "a change's diff, per file"),
                                      ("jj_show", "one change's description, files"))):
        yy = 394 + i * 22
        text(1084, yy, name, C["text"], size=11.5, family=MONO)
        text(1146, yy, desc, C["faint"], size=11)
    text(1084, 470, "pi calls them like read or bash. The", C["faint"], size=11)
    text(1084, 486, "app answers with its built-in jj.", C["faint"], size=11)
    divider(504)
    section(530, "THIS RUN")
    kv(554, "jj calls", "2", mono=True)
    kv(578, "Result tokens", "1.1k", mono=True)
    divider(596)
    ws.context_summary(622)
    note(1084, 790, 288, ["pi can suggest an undo; the button", "stays yours."])
    footer_caption("Inspecting Qwen signatures in ~/repos/pi")


# Status: "open" still needs a decision, "decided" records the user's answer (2026-09-30), "parked" is not now.
ROWS = [
    (r_relink, "Turn links survive restarts", "stored in the session file",
     "When a turn is recorded, the desktop backend appends its jj change to the session file as a custom entry "
     "(SessionManager.appendCustomEntry), which pi keeps out of the model's context. Reopening reads the links back and checks them.",
     "decided", "a custom entry in the session file; commit messages stay as they are."),
    (r_restore, "Restore one file", "undo part of a turn",
     "Restore puts one file back as it was before the turn and rebases later changes. If a later turn edits the same file, "
     "it is refused and names that turn, like Undo.",
     "decided", "confirm first."),
    (r_snapshots, "A snapshot before every command", "nothing a command destroys is lost",
     "pi-desktop snapshots the files before each bash call. A command that changed files says how many; hover to see them "
     "or restore to just before it.",
     "decided", "a setting, on by default, before bash calls; measure the cost on a large repo before settling the default."),
    (r_workspace, "Parallel sessions, separate workspaces", "no more mixed edits",
     "When a session is about to start work in a folder another session is working in, a popup asks where it should work. "
     "Afterwards one button brings its turns back into the main folder.",
     "decided", "a popup: own workspace in the automatic location or a folder you choose, or the same folder."),
    (r_fork_files, "Fork with the files", "the conversation and the code together",
     "Forking from a Tree entry asks which files the new session starts with, when they changed since that entry. "
     "\u201cAs at this entry\u201d starts a jj workspace at that entry's change.",
     "decided", "a question in the fork flow, asked only when the files differ."),
    (r_best_of, "Best of N", "one prompt, several attempts",
     "Run a prompt as 2–4 attempts, each in its own workspace and optionally with another model. Compare tests, size and "
     "cost, then keep one.",
     "dropped", "not building it."),
    (r_annotate, "Which turn wrote this line", "code linked to the conversation",
     "While ⌥ is held, jj's line annotation marks lines written by turns in the editor gutter. ⌥-click a line for the "
     "turn and its prompt; Show in thread jumps to it.",
     "decided", "shown only while ⌥ is held; ⌥-click a line for its turn."),
    (r_your_edits, "Your edits, told to pi", "pi works from current files",
     "Files you changed since pi's last turn show as a line in the thread and as a removable chip in the next prompt, "
     "sent as a diff.",
     "parked", "not now; adding them by default, or offering them, gets in a developer's way."),
    (r_land, "Land", "squash, describe, push",
     "Pick turns, squash them into one commit, let pi write the message from the diff, set a bookmark and push. One "
     "sheet over Changes.",
     "parked", "not now."),
    (r_conflict_undo, "Undo with conflicts", "instead of a refusal",
     "When a later turn builds on the one you undo, you can still undo it: jj keeps the conflict, and the composer gets "
     "a drafted prompt asking pi to resolve it.",
     "decided", "the fix is drafted in the composer; you review and send it."),
    (r_oplog, "History of every operation", "one undo for everything",
     "Changes gets an Operations tab next to Turns: turns, snapshots, your edits and git commits made outside. Restore "
     "the project to any point; restoring can be undone too.",
     "decided", "Operations is its own tab inside Changes."),
    (r_jj_tools, "jj tools for pi", "read-only, answered by the app",
     "pi gets jj_log, jj_diff and jj_show as tool calls. It can find the turn that broke something and suggest an undo; "
     "only you can click it.",
     "decided", "off by default; turned on per project."),
]

STATUS = {
    "open": ("DECIDE", PAGE["terracotta"]),
    "decided": ("DECIDED", "#2e8a55"),
    "parked": ("PARKED", PAGE["pageLabel"]),
    "dropped": ("DROPPED", PAGE["pageLabel"]),
}


def main():
    reset()
    defs(W, H, "<title>pi desktop — more from jj, visual proposal, not implemented</title>"
               "<desc>Follow-up to study 04's per-turn jj changes: turn links in the session file, restoring a file, snapshots "
               "before commands, workspaces for parallel sessions, fork with files, best of N, line annotation, your "
               "edits in the prompt, landing, undo with conflicts, the operation log, and read-only jj tools for pi. "
               "Each screen in Evening (dark) and Moonstone (light). Names and numbers are sample data.</desc>")
    page(W, H)
    label(48, 42, "PI  /  DESIGN STUDY  05", C["pageLabel"], size=11)
    add(f'<text x="48" y="96" fill="{C["evening"]}" font-family="{SERIF}" font-size="40" font-style="italic">'
        f'More from jj: <tspan fill="{DARK["accent"]}">find, fork and compare</tspan> turns.</text>')
    text(48, 128, "Twelve proposals on top of study 04. DECIDED rows record your answers, DECIDE rows still need one, and "
                  "parked and dropped rows are faded. Every screen is drawn in Evening (dark) and Moonstone (light).",
         C["driftwood"], size=14)
    label(2800, 92, "VISUAL PROPOSAL · SAMPLE DATA · FOLLOWS STUDY 04", C["pageLabel"], anchor="end", size=11)

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
            opacity = ' opacity="0.45"' if status in ("parked", "dropped") else ""
            add(f'<g transform="translate({dx} {y + 80 - 130})"{opacity}>')
            fn()
            window_end()
            add("</g>")
        set_theme("dark")

    text(48, H - 20, "LAYOUT STUDY ONLY — follows desktop-workspace-study.svg (study 04) and desktop-views-study.svg. "
                     "jj change IDs, commands, models and numbers are sample data.", C["warm30"], size=11)
    add("</svg>")
    return dump()


if __name__ == "__main__":
    sys.stdout.write(main())
