#!/usr/bin/env python3
"""pi desktop design study 04: jj history, files with an editor, and a terminal.

The planned direction from docs/architecture.md: jj first (one change per agent turn), a file tree and editor
tabs over Zed's Project and Editor with language servers, and a terminal drawer on Zed's terminal crate.
Everything stays out of the way until asked for. Each screen is drawn in Evening (dark) and Moonstone (light).
Writes SVG to stdout.
"""
import sys

import pi_study_common as common
from pi_study_common import (
    C, DARK, MONO, MULTI_ACTIVE, MULTI_PROJECTS, SERIF, SESSION_TABS, THEME, THINK_COLOR, add, blend, button, composer,
    defs, divider, dot, dump, icon, insp_title, kv, label, line, note, page, popover, rect, reset, ring, section,
    set_theme, text, thinking_row, tool_row, tparts, tw, utabs, window_begin, window_end, diff_lines,
)

W, H = 2848, 7500
MULTI = dict(active=MULTI_ACTIVE, projects=MULTI_PROJECTS, selected="Qwen signatures")

common.ICONS.update({
    "undo": '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>',
    "redo": '<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13"/>',
    "maximize": '<path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7"/>',
    "filePlus": '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6M12 12v6M9 15h6"/>',
    "folderPlus": '<path d="M2 7V5a2 2 0 0 1 2-2h5l3 4h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2Z"/><path d="M12 10v6M9 13h6"/>',
    "collapse": '<path d="m7 20 5-5 5 5M7 4l5 5 5-5"/>',
})


# ---------------------------------------------------------------- pieces drawn like the current app

def user_card(y, lines):
    """The app's user message: a bordered card; its time shows on hover."""
    h = 18 + 20 * len(lines)
    rect(280, y, 764, h, C["status"] if THEME["name"] == "dark" else "#ffffff", 8, C["chipLine"])
    for i, s in enumerate(lines):
        text(296, y + 22 + i * 20, s, C["text"], size=13)
    return h


def reply(y, parts):
    tparts(288, y, parts, size=13)


def jj_node(cx, cy, current=False, gone=False):
    if current:
        dot(cx, cy, 3.5, C["accent"])
    else:
        ring(cx, cy, 3.2, C["lineStrong"] if gone else C["muted"], 1.4, C["canvas"])


def turn_footer(y, change, files, added, removed=0, hover=False, current=False):
    """One quiet line under a turn: its jj change and what it touched; actions on hover."""
    if hover:
        rect(276, y, 768, 24, C["hover"], 5)
    jj_node(292, y + 12, current)
    x = 304
    text(x, y + 16, change, C["accent"] if current else C["faint"], size=10.5, family=MONO)
    x += tw(change, 10.5, True) + 10
    parts = [(f"{files} file{'s' if files != 1 else ''}  ", C["faint"]), (f"+{added}", C["green"])]
    if removed:
        parts += [(" ", C["faint"]), (f"−{removed}", C["coral"])]
    tparts(x, y + 16, parts, size=10.5, family=MONO)
    if hover:
        w = button(1038, y + 2, "Undo turn", "ghost", h=20, size=10.5, ic="undo", anchor="end")
        button(1038 - w - 6, y + 2, "Diff", "ghost", h=20, size=10.5, anchor="end")
    return 24


def expand_icon(y):
    icon("maximize", 1018, y + 12, 13, C["faint"])


def inspector_tabs(active):
    utabs(1084, 282, ["OVERVIEW", "TREE", "FILES"], active, gap=22)
    line(1084, 296.5, 1372, 296.5, C["line"])


def history(y, rows, undone=None):
    """The inspector's jj history: one row per turn, the working copy first."""
    section(y, "HISTORY", "jj")
    y += 24
    line(1090.5, y - 4, 1090.5, y + 24 * (len(rows) - 1) - 4, C["line"])
    for i, (change, desc, delta, state) in enumerate(rows):
        yy = y + i * 24
        gone = state == "undone"
        jj_node(1090, yy - 4, state == "current", gone)
        text(1102, yy, change, C["accent"] if state == "current" else (C["lineStrong"] if gone else C["faint"]),
             size=10.5, family=MONO)
        text(1176, yy, desc, C["faint"] if gone else C["text2"], size=12)
        if gone:
            line(1176, yy - 4, 1176 + tw(desc, 12), yy - 4, C["faint"])
            text(1372, yy, "[ REDO ]", C["accent"], size=10, family=MONO, anchor="end", ls=0.8)
        elif delta:
            tparts(1372, yy, delta, size=10.5, family=MONO, anchor="end")
    return y + 24 * len(rows)


def context_summary(y):
    section(y, "CONTEXT", "auto-compact on")
    text(1084, y + 30, "62.4k", C["text"], size=20, weight=600)
    text(1146, y + 30, "/ 200k tokens", C["muted"], size=12)
    rect(1084, y + 40, 288, 6, C["track"], 3)
    rect(1084, y + 40, 90, 6, C["accent"], 3)
    text(1084, y + 62, "31% of window", C["faint"], size=11)
    return y + 70


def thread_two_turns(hover_first=True, undone_second=False, y0=236):
    """Two agent turns in ~/repos/pi, each ending in its jj change."""
    y = y0
    y += user_card(y, ["qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response.",
                       "Accept empty signatures there, but keep the check strict for Anthropic."]) + 20
    thinking_row(y)
    reply(y + 24, [("The check lives in ", C["text2"]), ("openai-completions.ts", C["code"], MONO),
                   (". I'll allow empty strings for OpenCode models only.", C["text2"])])
    tool_row(y + 36, "file", "Read", "packages/ai/src/providers/openai-completions.ts", "412 lines")
    tool_row(y + 60, "edit", "Edit", "packages/ai/src/providers/openai-completions.ts", "+3 −1")
    tool_row(y + 84, "terminal", "Bash", "npm run check", "passed")
    y += 112
    turn_footer(y, "kqxlmwsv", 1, 3, 1, hover=hover_first)
    y += 36
    if undone_second:
        add('<g opacity="0.45">')
    y += user_card(y, ["Also add a regression test with the faux provider, no network."]) + 22
    reply(y, [("Added a faux-provider case to ", C["text2"]), ("openai-completions.test.ts", C["code"], MONO),
              (".", C["text2"])])
    tool_row(y + 12, "edit", "Edit", "packages/ai/test/openai-completions.test.ts", "+24")
    tool_row(y + 36, "terminal", "Bash", "vitest --run test/openai-completions.test.ts", "1 passed")
    y += 64
    if undone_second:
        add("</g>")
        icon("undo", 286, y + 5, 13, C["muted"])
        tparts(306, y + 16, [("Turn undone", C["text2"]), ("  ·  1 file restored, change zvtmrpyq abandoned", C["faint"])],
               size=12)
        button(1038, y + 2, "Redo", "ghost", h=20, size=10.5, ic="redo", anchor="end")
    else:
        turn_footer(y, "zvtmrpyq", 1, 24, current=True)
    return y + 24


# ---------------------------------------------------------------- 01 Turns that edit files are jj changes

def r_turns():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  2", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · @ zvtmrpyq", status_right="jj · 2 turns · $0.41 · 31% context")
    thread_two_turns(hover_first=True)
    composer(700, 134, "Ask pi to work on this project…", placeholder=True, thinking="high",
             buttons=(("Send  ⏎", "primary"),), stop=False)
    expand_icon(700)

    insp_title("Qwen signatures", "Ready", "2 turns")
    inspector_tabs(0)
    y = history(322, [("zvtmrpyq", "Faux-provider test", [("+24", C["green"])], "current"),
                      ("kqxlmwsv", "Empty signatures", [("+3 ", C["green"]), ("−1", C["coral"])], "done"),
                      ("main", "trunk", None, "base")])
    button(1084, y + 4, "Undo last turn", "ghost", w=140, ic="undo")
    button(1232, y + 4, "Operation log", "ghost", w=140)
    divider(y + 46)
    context_summary(y + 72)
    note(1084, 790, 288, ["Each agent turn is a jj change.", "Undo abandons it; the op log can restore it."])
    text(1084, 832, "Inspecting Qwen signatures in ~/repos/pi", C["faint"], size=10)


# ---------------------------------------------------------------- 02 Changes, per turn

def r_changes():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  2", "TREE", "CONTEXT"],
                 tab_active=1, tab_right="~/repos/pi · @ zvtmrpyq", status_right="jj diff · 2 turns · +27 −1")
    line(496.5, 222, 496.5, 846, C["hover"])
    label(272, 246, "2 TURNS")
    tparts(482, 246, [("+27 ", C["green"]), ("−1", C["coral"])], size=10.5, family=MONO, anchor="end")
    groups = [("zvtmrpyq", "Faux-provider test", True, [("openai-com…test.ts", "+24", None, False)]),
              ("kqxlmwsv", "Empty signatures", False, [("openai-com…ions.ts", "+3", "−1", True),
                                                        ("CHANGELOG.md", "+1", None, False)])]
    y = 258
    for change, desc, current, files in groups:
        jj_node(276, y + 13, current)
        text(288, y + 17, change, C["accent"] if current else C["faint"], size=10.5, family=MONO)
        text(356, y + 17, desc, C["muted"], size=11.5)
        y += 26
        for name, add_, rem, sel in files:
            if sel:
                rect(264, y, 224, 26, C["select"], 5)
                rect(264, y + 5, 2, 16, C["accent"], 1)
            icon("file", 288, y + 6, 13, C["accent"] if sel else C["faint"])
            text(308, y + 17, name, C["text"] if sel else C["text2"], size=11.5, family=MONO)
            tparts(482, y + 17, [(add_, C["green"])] + ([(" " + rem, C["coral"])] if rem else []), size=10.5,
                   family=MONO, anchor="end")
            y += 28
        y += 10
    text(272, y + 14, "From jj diff, so files that bash", C["faint"], size=11)
    text(272, y + 30, "commands change are included.", C["faint"], size=11)

    text(516, 246, "packages/ai/src/providers/openai-completions.ts", C["text2"], size=11.5, family=MONO)
    text(516, 266, "kqxlmwsv · turn 1 · 09:43 · +3 −1", C["faint"], size=11)
    w = button(1044, 252, "UNDO TURN", "bracket", anchor="end")
    button(1044 - w - 14, 252, "RESTORE FILE", "bracket", anchor="end")
    rect(508, 280, 536, 214, C["deep"], 6)
    kw, st, pl = C["kw"], C["str"], C["pl"]
    rows = [
        ("", "", "", "hunk", [("@@ -205,9 +205,11 @@ function readThinking", C["hunk"])]),
        ("205", "205", " ", None, [("  ", pl), ("const", kw), (" blocks = message.content;", pl)]),
        ("206", "206", " ", None, [("  ", pl), ("for", kw), (" (", pl), ("const", kw), (" block ", pl), ("of", kw),
                                   (" blocks) {", pl)]),
        ("209", "209", " ", None, [("    ", pl), ("if", kw), (" (block.type === ", pl), ('"thinking"', st), (") {", pl)]),
        ("210", "", "−", "del", [("      ", pl), ("if", kw), (" (!block.signature) ", pl), ("throw new", kw),
                                 (" MissingSignature(model.id);", pl)]),
        ("", "210", "+", "add", [("      ", pl), ("if", kw), (" (block.signature === ", pl), ("undefined", kw),
                                 (" && isAnthropic(model)) {", pl)]),
        ("", "211", "+", "add", [("        ", pl), ("throw new", kw), (" MissingSignature(model.id);", pl)]),
        ("", "212", "+", "add", [("      }", pl)]),
        ("211", "213", " ", None, [("      signatures.push(block.signature ?? ", pl), ('""', st), (");", pl)]),
        ("212", "214", " ", None, [("    }", pl)]),
        ("213", "215", " ", None, [("  }", pl)]),
    ]
    diff_lines(540, 584, 598, 284, rows, width=536, x_bg=508, two_cols=True)
    note(516, 534, 520, ["Restore puts this file back as it was before the turn. Undo turn abandons the whole change;",
                         "both are jj operations, so the History list can reverse them."])

    insp_title("openai-completions.ts", "Modified in 1 turn", "jj", kind="accent", serif=False)
    section(290, "FILE")
    text(1084, 314, "packages/ai/src/providers/", C["text2"], size=11, family=MONO)
    kv(338, "Lines", "414", mono=True)
    kv(362, "Language", "TypeScript")
    divider(380)
    section(406, "CHANGED BY")
    jj_node(1090, 426)
    tparts(1102, 430, [("kqxlmwsv", C["faint"], MONO), ("  turn 1 · 09:43", C["faint"])], size=11)
    tparts(1372, 430, [("+3 ", C["green"]), ("−1", C["coral"])], size=10.5, family=MONO, anchor="end")
    divider(450)
    section(476, "ACTIONS")
    button(1084, 490, "Open in editor", "primary", w=140, ic="open")
    button(1232, 490, "Restore file", "ghost", w=140, ic="undo")
    button(1084, 522, "Undo turn kqxlmwsv", "ghost", w=288)
    note(1084, 790, 288, ["Changes are what jj recorded,", "not only pi's edit and write calls."])
    text(1084, 832, "Inspecting a changed file in Qwen signatures", C["faint"], size=10)


# ---------------------------------------------------------------- 03 Opt in once

def r_opt_in():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  1", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · main", status_right="git · 1 file changed · $0.12 · 9% context")
    tone = C["accent"]
    rect(280, 232, 764, 58, blend(tone, C["canvas"], 0.08), 8, blend(tone, C["canvas"], 0.35))
    icon("undo", 296, 244, 16, tone)
    text(322, 254, "Undo agent turns with jj", C["text"], size=13, weight=600)
    text(322, 274, "This project uses git only. jj keeps one change per turn next to git; branches stay as they are.",
         C["text2"], size=12)
    w = button(1030, 248, "Turn on jj", "primary", anchor="end")
    button(1030 - w - 8, 248, "Not now", "ghost", anchor="end")
    y = 310
    y += user_card(y, ["qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response."]) + 22
    thinking_row(y)
    reply(y + 24, [("The check lives in ", C["text2"]), ("openai-completions.ts", C["code"], MONO),
                   (". I'll allow empty strings for OpenCode models only.", C["text2"])])
    tool_row(y + 36, "edit", "Edit", "packages/ai/src/providers/openai-completions.ts", "+3 −1")
    tool_row(y + 60, "terminal", "Bash", "npm run check", "passed")
    composer(700, 134, "Ask pi to work on this project…", placeholder=True, thinking="high",
             buttons=(("Send  ⏎", "primary"),), stop=False)
    expand_icon(700)

    insp_title("Qwen signatures", "Ready", "1 turn")
    inspector_tabs(0)
    section(322, "HISTORY", "git")
    text(1084, 348, "Turns are not recorded yet.", C["text2"], size=12)
    text(1084, 368, "With jj, each one becomes a change", C["faint"], size=11)
    text(1084, 384, "you can undo on its own.", C["faint"], size=11)
    button(1084, 400, "Turn on jj", "primary", w=288, ic="undo")
    rect(1084, 434, 288, 30, C["deep"], 6, C["line"])
    text(1096, 453, "jj git init --colocate", C["pl"], size=11, family=MONO)
    text(1084, 482, "Keeps .git and adds .jj beside it.", C["faint"], size=11)
    divider(500)
    context_summary(526)
    note(1084, 790, 288, ["Asked once per project.", "Settings can turn it on later."])
    text(1084, 832, "Inspecting Qwen signatures in ~/repos/pi", C["faint"], size=10)


# ---------------------------------------------------------------- 04 Undo, then redo

def r_undo():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  1", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · @ kqxlmwsv", status_right="jj · turn undone · $0.41 · 31% context")
    thread_two_turns(hover_first=False, undone_second=True)
    composer(700, 134, "Ask pi to work on this project…", placeholder=True, thinking="high",
             buttons=(("Send  ⏎", "primary"),), stop=False)
    expand_icon(700)

    insp_title("Qwen signatures", "Ready", "1 turn")
    inspector_tabs(0)
    y = history(322, [("zvtmrpyq", "Faux-provider test", None, "undone"),
                      ("kqxlmwsv", "Empty signatures", [("+3 ", C["green"]), ("−1", C["coral"])], "current"),
                      ("main", "trunk", None, "base")])
    button(1084, y + 4, "Undo last turn", "ghost", w=140, ic="undo")
    button(1232, y + 4, "Operation log", "ghost", w=140)
    divider(y + 46)
    context_summary(y + 72)
    note(1084, 790, 288, ["The thread keeps the undone turn, faded,", "so pi and you both see what happened."])
    text(1084, 832, "Inspecting Qwen signatures in ~/repos/pi", C["faint"], size=10)


# ---------------------------------------------------------------- 05 Editor tab with a language server

TREE = [
    (0, "dir", "packages", True, None),
    (1, "dir", "ai", True, None),
    (2, "dir", "src", True, None),
    (3, "dir", "providers", True, None),
    (4, "file", "anthropic.ts", False, None),
    (4, "file", "openai-completions.ts", False, "M"),
    (4, "file", "opencode.ts", False, None),
    (2, "dir", "test", False, "•"),
    (2, "file", "package.json", False, None),
    (1, "dir", "coding-agent", False, None),
    (1, "dir", "tui", False, None),
    (0, "file", "AGENTS.md", False, None),
    (0, "file", "package.json", False, None),
]


def file_tree(y, selected="openai-completions.ts", new_file=None, menu_at=None):
    """The inspector's FILES tab: filter, create buttons, and the project tree with jj states."""
    rect(1084, y, 220, 26, C["canvas"], 6, C["chipLine"])
    icon("search", 1092, y + 6, 13, C["faint"])
    text(1112, y + 17, "Filter files", C["faint"], size=11.5)
    icon("filePlus", 1314, y + 5, 15, C["muted"])
    icon("folderPlus", 1336, y + 5, 15, C["muted"])
    icon("collapse", 1358, y + 5, 14, C["muted"])
    y += 36
    for depth, kind, name, open_, state in TREE:
        x = 1084 + depth * 14
        sel = name == selected
        if sel:
            rect(1076, y, 304, 22, C["select"], 4)
            rect(1076, y + 4, 2, 14, C["accent"], 1)
        if kind == "dir":
            icon("chevron" if open_ else "chevronR", x, y + 5, 11, C["faint"])
            icon("folder", x + 14, y + 4, 13, C["muted"] if open_ else C["faint"])
        else:
            icon("file", x + 14, y + 4, 13, C["accent"] if sel else C["faint"])
        text(x + 34, y + 15, name, C["text"] if sel else C["text2"], size=12)
        if state == "M":
            text(1368, y + 15, "M", C["amber"], size=10.5, family=MONO, anchor="end")
        elif state == "•":
            dot(1365, y + 11, 3, C["amber"])
        if menu_at == name:
            rect(1076, y, 304, 22, "none", 4, C["focus"])
        y += 22
        if new_file and name == new_file[0]:
            x = 1084 + (depth + 1) * 14
            icon("filePlus", x + 14, y + 4, 13, C["accent"])
            rect(x + 32, y + 1, 1372 - x - 32, 20, C["canvas"], 4, C["accent"])
            text(x + 40, y + 15, new_file[1], C["text"], size=12)
            rect(x + 40 + tw(new_file[1], 12) + 2, y + 5, 1.5, 12, C["accent"])
            y += 26
    return y


def r_editor():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  2", "TREE", "CONTEXT"],
                 tab_active=-1, tab_right=None,
                 status_right="TypeScript · tsserver ● · Ln 212, Col 19 · 1 problem")
    # The file tab sits after the session tabs, closable, and takes the middle column while active.
    line(716.5, 190, 716.5, 214, C["line"])
    icon("file", 734, 194, 13, C["accent"])
    text(754, 206, "openai-completions.ts", C["text"], size=11.5, family=MONO)
    icon("x", 918, 196, 11, C["faint"])
    rect(734, 219, 196, 2, C["accent"], 1)

    kw, st, pl, fn = C["kw"], C["str"], C["pl"], C["code"]
    cm = C["faint"]
    lines = [
        (205, [("function", kw), (" ", pl), ("readThinking", fn), ("(message: AssistantMessage, model: Model) {", pl)]),
        (206, [("  ", pl), ("const", kw), (" blocks = message.content;", pl)]),
        (207, [("  ", pl), ("const", kw), (" signatures: ", pl), ("string", kw), ("[] = [];", pl)]),
        (208, [("  ", pl), ("for", kw), (" (", pl), ("const", kw), (" block ", pl), ("of", kw), (" blocks) {", pl)]),
        (209, [("    ", pl), ("if", kw), (" (block.type === ", pl), ('"text"', st), (") ", pl), ("continue", kw), (";", pl)]),
        (210, [("    ", pl), ("// OpenCode sends empty signatures; Anthropic must not.", cm)]),
        (211, [("    ", pl), ("if", kw), (" (block.type === ", pl), ('"thinking"', st), (") {", pl)]),
        (212, [("      ", pl), ("if", kw), (" (block.signature.length === ", pl), ("0", C["amber"]),
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
    y0 = 246
    for i, (n, parts) in enumerate(lines):
        y = y0 + i * 20
        if n == 212:
            rect(256, y - 14, 808, 20, C["hover"])
            dot(270, y - 4, 3, C["coral"])
        text(304, y, str(n), C["faint"] if n != 212 else C["muted"], size=11, family=MONO, anchor="end")
        tparts(320, y, parts, size=11.5, family=MONO, pre=True)
    # Diagnostic under `block.signature`, from the language server.
    sx = 320 + tw("      if (", 11.5, True)
    ex = sx + tw("block.signature", 11.5, True)
    yy = y0 + 7 * 20 + 3
    d = f"M{sx} {yy}" + "".join(f" q 2 {'-2.4' if k % 2 == 0 else '2.4'} 4 0" for k in range(int((ex - sx) / 4)))
    add(f'<path d="{d}" fill="none" stroke="{C["coral"]}" stroke-width="1.1"/>')
    rect(sx + 1, yy - 15, 1.5, 16, C["accent"])
    # Hover card with the diagnostic and two ways to act on it.
    px0, py0, pw, ph = sx - 20, yy + 10, 404, 112
    popover(px0, py0, pw, ph)
    icon("warning", px0 + 14, py0 + 12, 14, C["coral"])
    text(px0 + 36, py0 + 24, "'block.signature' is possibly 'undefined'.", C["text"], size=12)
    text(px0 + pw - 14, py0 + 24, "ts(18048)", C["faint"], size=10.5, family=MONO, anchor="end")
    tparts(px0 + 36, py0 + 44, [("(property) ", C["faint"]), ("signature?: string", C["code"])], size=11, family=MONO)
    line(px0, py0 + 58.5, px0 + pw, py0 + 58.5, C["line"])
    text(px0 + 14, py0 + 80, "Quick fix: use optional chaining", C["accent"], size=12)
    button(px0 + pw - 12, py0 + 68, "Ask pi to fix", "ghost", h=22, size=11, ic="sparkle", anchor="end")
    text(px0 + 14, py0 + 100, "⌘.  quick fixes    ⌥⏎  send to pi", C["faint"], size=10, family=MONO)

    # The composer stays one line while a file is open, mentioning the file.
    base = C["bar"] if THEME["name"] == "dark" else C["chip"]
    rect(276, 796, 768, 38, base, 10, C["focus"])
    icon("sparkle", 290, 808, 14, C["muted"])
    w = tw("openai-completions.ts:212", 10.5, True) + 16
    rect(312, 806, round(w, 1), 18, C["select"], 4, C["focus"])
    text(320, 819, "openai-completions.ts:212", C["accent"], size=10.5, family=MONO)
    text(312 + w + 10, 820, "Ask pi about this file…", C["faint"], size=12.5)
    button(1036, 803, "Send  ⏎", "primary", h=24, anchor="end")

    insp_title("Files", "~/repos/pi", "jj · 2 changed", kind="accent", serif=False)
    inspector_tabs(2)
    file_tree(310)
    note(1084, 790, 288, ["M and • mark what jj sees as changed.", "Open files reload when pi edits them."])
    text(1084, 832, "Browsing ~/repos/pi", C["faint"], size=10)


# ---------------------------------------------------------------- 06 File actions

def r_files():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  2", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · @ zvtmrpyq", status_right="jj · 2 turns · $0.41 · 31% context")
    thread_two_turns(hover_first=False)
    composer(700, 134, "Ask pi to work on this project…", placeholder=True, thinking="high",
             buttons=(("Send  ⏎", "primary"),), stop=False)
    expand_icon(700)

    insp_title("Files", "~/repos/pi", "jj · 2 changed", kind="accent", serif=False)
    inspector_tabs(2)
    file_tree(310, selected=None, new_file=("test", "faux-provider.test.ts"), menu_at="providers")
    # Right-click menu on a folder, flipped left so the tree and the new name stay visible.
    mx, my, mw = 898, 424, 184
    items = [("New File", "", False), ("New Folder", "", False), ("Rename", "F2", False), ("Copy Path", "", False),
             ("Reveal in Finder", "", False), None, ("Delete", "⌫", True)]
    mh = 12 + sum(8 if i is None else 26 for i in items)
    popover(mx, my, mw, mh)
    yy = my + 6
    for item in items:
        if item is None:
            line(mx, yy + 4.5, mx + mw, yy + 4.5, C["line"])
            yy += 8
            continue
        name, key, danger = item
        if name == "New File":
            rect(mx + 4, yy, mw - 8, 24, C["select"], 4)
        text(mx + 14, yy + 16, name, C["coral"] if danger else C["text2"], size=12)
        if key:
            text(mx + mw - 14, yy + 16, key, C["faint"], size=10.5, family=MONO, anchor="end")
        yy += 26
    note(1084, 790, 288, ["Creating, renaming and deleting files", "are jj changes too, so they undo."])
    text(1084, 832, "Browsing ~/repos/pi", C["faint"], size=10)


# ---------------------------------------------------------------- 07 Terminal drawer

def r_terminal():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  2", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · @ zvtmrpyq", status_right="2 terminals · ⌃` toggles · $0.41")
    y = 236
    y += user_card(y, ["Also add a regression test with the faux provider, no network."]) + 22
    reply(y, [("Added a faux-provider case to ", C["text2"]), ("openai-completions.test.ts", C["code"], MONO),
              (".", C["text2"])])
    tool_row(y + 12, "edit", "Edit", "packages/ai/test/openai-completions.test.ts", "+24")
    # A finished bash row offers to open its command in a real terminal on hover.
    rect(276, y + 34, 768, 24, C["hover"], 5)
    icon("terminal", 288, y + 39, 14, C["faint"])
    text(310, y + 50, "Bash", C["muted"], size=12)
    text(350, y + 50, "vitest --run test/openai-completions.test.ts", C["text2"], size=11.5, family=MONO)
    button(1038, y + 36, "Open in terminal", "ghost", h=20, size=10.5, ic="terminal", anchor="end")
    turn_footer(y + 64, "zvtmrpyq", 1, 24, current=True)

    composer(460, 110, "Ask pi to work on this project…", placeholder=True, thinking="high",
             buttons=(("Send  ⏎", "primary"),), stop=False)
    expand_icon(460)

    # Drawer: resizable from its top edge, below the composer, inside the thread column.
    top = 586
    line(256, top + 0.5, 1064, top + 0.5, C["lineStrong"])
    rect(640, top - 2, 40, 4, C["lineStrong"], 2)
    rect(256, top + 1, 808, 30, C["side"])
    label(276, top + 21, "TERMINAL", C["muted"])
    x = 360
    for name, active, busy in (("zsh", True, False), ("npm test", False, True)):
        w = tw(name, 11, True) + (40 if busy else 24)
        rect(x, top + 6, round(w, 1), 20, C["select"] if active else "none", 4, C["focus"] if active else None)
        text(x + 10, top + 20, name, C["text"] if active else C["muted"], size=11, family=MONO)
        if busy:
            dot(x + w - 12, top + 16, 3, C["accent"])
        x += w + 6
    icon("plus", x + 4, top + 9, 13, C["muted"])
    icon("maximize", 1014, top + 9, 13, C["muted"])
    icon("x", 1038, top + 9, 12, C["muted"])
    rect(256, top + 31, 808, 846 - top - 31, C["deep"])
    rows = [
        [("~/repos/pi", C["steel"]), (" @zvtmrpyq ", C["faint"]), ("$ ", C["muted"]),
         ("npm test -- test/openai-completions.test.ts", C["pl"])],
        [(" ✓ ", C["green"]), ("openai-completions > accepts empty signatures for opencode ", C["pl"]), ("4ms", C["faint"])],
        [(" ✓ ", C["green"]), ("openai-completions > keeps anthropic strict ", C["pl"]), ("2ms", C["faint"])],
        [(" Test Files  ", C["faint"]), ("1 passed", C["green"]), (" (1)", C["faint"])],
        [("      Tests  ", C["faint"]), ("2 passed", C["green"]), (" (2)", C["faint"])],
        [("~/repos/pi", C["steel"]), (" @zvtmrpyq ", C["faint"]), ("$ ", C["muted"])],
    ]
    for i, parts in enumerate(rows):
        tparts(276, top + 54 + i * 19, parts, size=11.5, family=MONO, pre=True)
    cx = 276 + tw("~/repos/pi @zvtmrpyq $ ", 11.5, True)
    rect(cx, top + 54 + 5 * 19 - 11, 7, 14, C["accent"], 1)

    insp_title("Qwen signatures", "Ready", "1 turn")
    inspector_tabs(0)
    y = history(322, [("zvtmrpyq", "Faux-provider test", [("+24", C["green"])], "current"),
                      ("main", "trunk", None, "base")])
    divider(y + 8)
    context_summary(y + 34)
    note(1084, 790, 288, ["Terminals start in the session's folder", "and stay out of pi's context."])
    text(1084, 832, "Inspecting Qwen signatures in ~/repos/pi", C["faint"], size=10)


def mention_chip(x, y, ic, name, h=20, size=11):
    w = tw(name, size, True) + 30
    rect(x, y, round(w, 1), h, C["select"], 4, C["focus"])
    icon(ic, x + 6, y + h / 2 - 6, 12, C["accent"])
    text(x + 22, y + h / 2 + size * 0.36, name, C["accent"], size=size, family=MONO)
    return w


def r_mentions():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=["THREAD", "CHANGES  2", "TREE", "CONTEXT"],
                 tab_right="~/repos/pi · @ zvtmrpyq", status_right="jj · 2 turns · $0.41 · 31% context")
    # A sent message keeps its mentions as chips; pi receives paths and text it can read.
    y = 236
    rect(280, y, 764, 38, C["status"] if THEME["name"] == "dark" else "#ffffff", 8, C["chipLine"])
    text(296, y + 24, "Why does", C["text"], size=13)
    x = 296 + tw("Why does", 13) * 1.2 + 6
    x += mention_chip(x, y + 9, "tree", "readThinking") + 6
    text(x, y + 24, "reject what", C["text"], size=13)
    x += tw("reject what", 13) * 1.2 + 6
    x += mention_chip(x, y + 9, "terminal", "zsh · 12 lines") + 6
    text(x, y + 24, "shows?", C["text"], size=13)
    thinking_row(y + 62)
    reply(y + 86, [("It throws before the OpenCode check. Line 212 reads ", C["text2"]),
                   ("block.signature.length", C["code"], MONO), (" when the field is missing.", C["text2"])])
    turn_footer(y + 100, "zvtmrpyq", 0, 0, current=True)

    # The composer: text with a mention chip already inserted, and the @ menu for the next one.
    base = C["bar"] if THEME["name"] == "dark" else C["chip"]
    rect(276, 700, 768, 134, base, 10, C["focus"])
    text(292, 734, "Compare", C["text"], size=13)
    x = 292 + tw("Compare", 13) * 1.2 + 6
    x += mention_chip(x, 721, "file", "openai-completions.ts") + 6
    text(x, 734, "with @op", C["text"], size=13)
    rect(x + tw("with @op", 13) * 1.2 + 2, 722, 1.5, 16, C["accent"])
    icon("maximize", 1018, 712, 13, C["faint"])
    by = 796
    icon("clip", 292, by + 4, 15, C["muted"])
    icon("slash", 318, by + 4, 15, C["muted"])
    button(1038, by, "Send  ⏎", "primary", anchor="end")

    mx, my, mw = 284, 330, 360
    groups = [
        ("Files", [("file", "opencode.ts", "packages/ai/src/providers", True),
                   ("file", "openai-completions.test.ts", "packages/ai/test", False)]),
        ("Symbols", [("tree", "openCodeModels", "opencode.ts:14 · const", False)]),
        ("Turns", [("undo", "kqxlmwsv", "Empty signatures · jj", False)]),
        ("Sessions", [("message", "OpenCode Go defaults", "pi · 2 days ago", False)]),
        ("Terminal", [("terminal", "npm test", "last output · 7 lines", False)]),
        ("Problems", [("warning", "1 in openai-completions.ts", "from tsserver", False)]),
    ]
    mh = 12 + sum(24 + 26 * len(items) for _, items in groups) + 26
    popover(mx, my, mw, mh)
    yy = my + 6
    for g, (title, items) in enumerate(groups):
        if g:
            line(mx, yy + 0.5, mx + mw, yy + 0.5, C["line"])
        text(mx + 14, yy + 17, title, C["muted"], size=12)
        yy += 24
        for ic, name, detail, sel in items:
            if sel:
                rect(mx + 6, yy, mw - 12, 24, C["select"], 5)
            icon(ic, mx + 16, yy + 5, 14, C["accent"] if sel else C["muted"])
            hit = name.lower().find("op")
            parts = ([(name[:hit], C["text"]), (name[hit:hit + 2], C["accent"]), (name[hit + 2:], C["text"])]
                     if hit >= 0 else [(name, C["text"])])
            tparts(mx + 38, yy + 16, parts, size=12, family=MONO)
            text(mx + mw - 14, yy + 16, detail, C["faint"], size=10.5, anchor="end")
            yy += 26
    line(mx, yy + 6.5, mx + mw, yy + 6.5, C["line"])
    text(mx + 14, yy + 23, "↑↓ select · ⏎ insert · ⇥ complete · esc dismiss", C["faint"], size=10, family=MONO)
    # Preview beside the highlighted file.
    ax, ay, aw = mx + mw + 8, my + 30, 300
    popover(ax, ay, aw, 128)
    text(ax + 12, ay + 22, "packages/ai/src/providers/opencode.ts", C["text2"], size=11, family=MONO)
    text(ax + 12, ay + 40, "88 lines · TypeScript · unchanged", C["faint"], size=10.5)
    rect(ax + 12, ay + 50, aw - 24, 66, C["deep"], 5)
    kw, pl, fn = C["kw"], C["pl"], C["code"]
    for i, parts in enumerate([[("export", kw), (" ", pl), ("const", kw), (" openCodeModels = [", pl)],
                                [("  ", pl), ('"kimi-k3"', C["str"]), (", ", pl), ('"qwen3.8-flash"', C["str"])],
                                [("];", pl)]]):
        tparts(ax + 22, ay + 68 + i * 16, parts, size=10.5, family=MONO, pre=True)

    insp_title("Qwen signatures", "Ready", "2 turns")
    inspector_tabs(0)
    section(322, "IN THIS PROMPT")
    for i, (ic, name, detail) in enumerate([("file", "openai-completions.ts", "path · pi reads it"),
                                            ("file", "opencode.ts", "path · pi reads it")]):
        yy = 348 + i * 24
        icon(ic, 1084, yy - 11, 13, C["muted"])
        text(1104, yy, name, C["text2"], size=11.5, family=MONO)
        text(1372, yy, detail, C["faint"], size=10.5, anchor="end")
    text(1084, 406, "Files and symbols go as paths and", C["faint"], size=11)
    text(1084, 422, "locations; terminal output, problems and", C["faint"], size=11)
    text(1084, 438, "turns go as text. Nothing is hidden.", C["faint"], size=11)
    divider(456)
    context_summary(482)
    note(1084, 790, 288, ["@ works in the composer, the expanded", "editor, and the editor tab's prompt line."])
    text(1084, 832, "Inspecting Qwen signatures in ~/repos/pi", C["faint"], size=10)


ROWS = [
    (r_turns, "Edits become jj changes", "one quiet line, actions on hover",
     "A turn that changed files ends with its change and what it touched; read-only turns add nothing. Diff and Undo turn on hover."),
    (r_changes, "Changes, per turn", "what jj recorded, not only edit calls",
     "The Changes tab groups files by turn and shows jj's diff, including files that commands changed. Restore a file or undo the turn."),
    (r_opt_in, "Opt in once", "jj beside git, never silently",
     "A project on plain git gets one dismissible prompt. Turning jj on runs jj git init --colocate; git keeps working as before."),
    (r_undo, "Undo, then redo", "reversible and visible",
     "Undoing abandons the change. The thread keeps the turn, faded, with Redo; the history shows the abandoned change."),
    (r_editor, "A file in a tab", "Zed's editor with a language server",
     "Files open as a tab after the session tabs. Diagnostics come from the language server; the composer shrinks to one line about the file."),
    (r_files, "File actions", "the tree lives in the inspector",
     "The inspector's FILES tab holds the tree with jj states. Right-click for file actions; new files are named in place."),
    (r_terminal, "A terminal drawer", "below the composer, inside the thread",
     "⌃` opens a drawer in the session's folder with tabs for several shells. A bash row can reopen its command there."),
    (r_mentions, "@ mentions", "files, symbols, turns, sessions, output",
     "Typing @ opens one grouped menu. Choices become chips in the draft and in the sent message; pi gets paths or text."),
]


def main():
    reset()
    defs(W, H, "<title>pi desktop — jj history, files and a terminal, visual proposal, not implemented</title>"
               "<desc>Follow-up to studies 01–03 and the planned direction in docs/architecture.md: jj first, a file tree "
               "and editor tabs over Zed's Project and Editor with language servers, and a terminal drawer on Zed's "
               "terminal crate. Each screen in Evening (dark) and Moonstone (light). Names and numbers are sample data.</desc>")
    page(W, H)
    label(48, 42, "PI  /  DESIGN STUDY  04", C["pageLabel"], size=11)
    add(f'<text x="48" y="96" fill="{C["evening"]}" font-family="{SERIF}" font-size="40" font-style="italic">'
        f'Files, a terminal, and <tspan fill="{DARK["accent"]}">every turn undoable</tspan>.</text>')
    text(48, 128, "Nothing new is on screen until it is asked for: a quiet line per turn, a tab for a file, a drawer for a "
                  "terminal, a tree in the inspector. Every screen is drawn in Evening (dark) and Moonstone (light).",
         C["driftwood"], size=14)
    label(2800, 92, "VISUAL PROPOSAL · SAMPLE DATA · FOLLOWS STUDIES 01–03", C["pageLabel"], anchor="end", size=11)

    for r, (fn, name, tag, sub) in enumerate(ROWS):
        y = 200 + r * 900
        dot(61, y - 5, 13, C["parchment"])
        text(61, y - 1, f"{r + 1:02d}", C["evening"], size=10, weight=600, family=MONO, anchor="middle")
        tparts(84, y, [(name, C["evening"], None, 600), ("  —  " + tag, C["driftwood"])], size=16)
        text(48, y + 24, sub, C["driftwood"], size=12.5)
        for col, theme, cap in ((0, "dark", "EVENING  ·  DARK"), (1, "light", "MOONSTONE  ·  LIGHT")):
            dx = col * 1408
            set_theme(theme)
            label(48 + dx, y + 56, cap, C["pageLabel"], size=10)
            add(f'<g transform="translate({dx} {y + 68 - 130})">')
            fn()
            window_end()
            add("</g>")
        set_theme("dark")

    text(48, H - 40, "LAYOUT STUDY ONLY — follows desktop-thread-study.svg, desktop-views-study.svg and "
                     "desktop-projects-study.svg. jj change IDs, diagnostics and test output are sample data.",
         C["warm30"], size=11)
    add("</svg>")
    return dump()


if __name__ == "__main__":
    sys.stdout.write(main())
