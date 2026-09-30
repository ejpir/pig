#!/usr/bin/env python3
"""pi desktop design study 02: every screen beyond the thread. Writes SVG to stdout.

Session views (New session, Tree, Changes, Context, Commands, Extensions) are tabs or overlays on a session.
App views (All Sessions, Models, Resources, Settings) sit at the bottom of the sidebar.
"""
import sys

from pi_study_common import (
    C, DARK, MONO, SERIF, SESSION_TABS, THINK, set_theme, THINK_COLOR, add, blend, button, composer, defs, diff_lines,
    checkbox, divider, dot, dump, popover, edit_card, icon, insp_title, kv, label, line, note, page, pill, quote, rect, reset, ring, section,
    seg, text, thinking_row, tile, toggle, tool_row, tparts, tw, user_msg, utabs, window_begin, window_end,
)

W, H = 2848, 4600
IDLE = {"Qwen signatures": "now"}


def card_list(x, y, w, h, title, count, rows, footer=None):
    rect(x, y, w, h, C["side"], 8, C["card"])
    label(x + 14, y + 22, title, C["muted"])
    text(x + w - 14, y + 22, count, C["faint"], size=11, family=MONO, anchor="end")
    for i, (main, meta) in enumerate(rows):
        yy = y + 46 + i * 20
        text(x + 14, yy, main, C["text2"], size=11.5, family=MONO)
        if meta:
            text(x + w - 14, yy, meta, C["faint"], size=11, anchor="end")
    if footer:
        text(x + 14, y + 46 + len(rows) * 20, footer, C["faint"], size=11)


def chip(x, y, s, on=True, h=22, size=11):
    return pill(x, y, s, C["text"] if on else C["faint"], C["select"] if on else C["hover"], size=size, h=h, pad=8,
                border=C["focus"] if on else C["line"])




# ---------------------------------------------------------------- 01 New session

def v_new():
    window_begin("New session", session_sel=0, new_row=True, tabs=["THREAD", "CHANGES", "TREE", "CONTEXT"],
                 status_right="ready · claude-opus-5-5 · medium")
    from pi_study_common import pimark
    pimark(636, 248, 48)
    text(660, 340, "Start with a prompt, a template, or a skill.", C["text"], size=24, family=SERIF, italic=True, anchor="middle")
    text(660, 366, "Loaded for ~/repos/pi. Context files and skill descriptions go to the model with your first prompt.",
         C["muted"], size=12.5, anchor="middle")
    label(288, 404, "LOADED FOR THIS PROJECT")
    card_list(288, 414, 368, 102, "CONTEXT FILES", "2",
              [("AGENTS.md", "project"), ("~/.pi/agent/AGENTS.md", "user")], "Loaded even before project trust.")
    card_list(664, 414, 368, 102, "SKILLS", "3",
              [("release", "Prepare a release"), ("interactive-testing", "Drive pi in tmux"), ("pdf", "Read and fill PDFs")])
    card_list(288, 524, 368, 102, "PROMPT TEMPLATES", "4",
              [("/wr", "Write a reply"), ("/review", "Review a pull request"), ("/fix-tests", "Fix failing tests")])
    card_list(664, 524, 368, 102, "EXTENSIONS", "2",
              [("git-guard", "npm · user"), ("pi-mcp-adapter", "git · user")], "Adds 2 commands and 1 tool.")
    label(288, 658, "START FROM")
    x = 288
    for s in ("/fix-tests", "/review", "/skill:release", "!git status"):
        x += pill(x, 668, s, C["text2"], C["chip"], size=11, h=24, pad=10, rx=5, border=C["chipLine"]) + 8
    composer(730, 104, "Ask pi. Type / for commands, ! to run a shell command.", placeholder=True, thinking="medium",
             buttons=(("Send  ⏎", "primary"),), stop=False)

    insp_title("New session", "Not started", "~/repos/pi")
    section(290, "MODEL")
    kv(316, "Model", "claude-opus-5-5", mono=True)
    kv(340, "Provider", "anthropic · OAuth")
    kv(364, "Context window", "200k", mono=True)
    kv(388, "Thinking", "medium", mono=True)
    divider(406)
    section(432, "TOOLS", "defaultTools")
    x = 1084
    for t in ("read", "bash", "edit", "write"):
        x += chip(x, 444, t) + 6
    x = 1084
    for t in ("grep", "find", "ls"):
        x += chip(x, 472, t, on=False) + 6
    divider(506)
    section(532, "PROJECT")
    kv(558, "Path", "~/repos/pi", mono=True)
    text(1084, 582, "Trust", C["muted"], size=12)
    text(1372, 582, "Trusted", C["text2"], size=12, anchor="end")
    dot(1372 - tw("Trusted", 12) - 10, 578, 3.5, C["green"])
    kv(606, "Git", "main · clean", mono=True)
    divider(624)
    section(650, "SESSION FILE")
    text(1084, 674, "Created on your first prompt in", C["faint"], size=11)
    text(1084, 692, "~/.pi/agent/sessions/--repos-pi--/", C["text2"], size=11, family=MONO)
    divider(712)
    section(738, "KEYS")
    kv(762, "Send", "⏎", mono=True)
    kv(784, "Shell, output to pi", "!cmd", mono=True)
    kv(806, "Shell, kept out", "!!cmd", mono=True)
    text(1084, 832, "Inspecting a new session in ~/repos/pi", C["faint"], size=10)


# ---------------------------------------------------------------- 02 Tree

def v_tree():
    window_begin("Qwen signatures", session_sel=0, states=IDLE, tabs=SESSION_TABS, tab_active=2,
                 status_right="41 entries · 2 branches · $0.52")
    seg(280, 234, ["Default", "No tools", "User only", "Labeled", "All"], 0)
    text(1044, 250, "41 entries · 2 branches", C["faint"], size=11, anchor="end")
    MAIN, BR = 300, 326
    rows = [
        ("compact", MAIN, [("Compacted  ", C["amber"]), ("148.2k → 31.0k tokens", C["muted"])], "09:30", None),
        ("user", MAIN, [("qwen3.8-flash on OpenCode returns empty thinking signatures and we reject…", C["text"])], "09:41", None),
        ("pi", MAIN, [("The check lives in openai-completions.ts. I'll allow empty strings for…", C["text2"])], "09:42", None),
        ("tool", MAIN, [("edit  ", C["muted"], MONO), ("openai-completions.ts  ", C["text2"], MONO), ("+3 ", C["green"], MONO),
                        ("−1", C["coral"], MONO)], "", None),
        ("tool", MAIN, [("bash  ", C["muted"], MONO), ("npm run check", C["text2"], MONO)], "✓", ("checks-green", "label")),
        ("user", BR, [("Also add a regression test against the live OpenCode endpoint", C["muted"])], "09:52", None),
        ("pi", BR, [("Added openai-completions.test.ts calling OpenCode with a real key…", C["muted"])], "09:54", None),
        ("tool", BR, [("bash  ", C["faint"], MONO), ("vitest --run test/openai-completions.test.ts", C["muted"], MONO)], "✕",
         ("summarized · 1.4k", "summary")),
        ("user", MAIN, [("Add a regression test with the faux provider, no network", C["text"])], "09:58", None),
        ("pi", MAIN, [("Added a faux-provider case to openai-completions.test.ts.", C["text2"])], "09:59", None),
        ("tool", MAIN, [("edit  ", C["muted"], MONO), ("openai-completions.test.ts  ", C["text2"], MONO), ("+24", C["green"], MONO)], "",
         None),
        ("tool", MAIN, [("bash  ", C["muted"], MONO), ("vitest --run test/openai-completions.test.ts", C["text2"], MONO)], "✓",
         ("current leaf", "leaf")),
    ]
    y0, pitch = 276, 36
    cy = [y0 + i * pitch + 16 for i in range(len(rows))]
    sel = 6
    rect(266, y0 + sel * pitch + 1, 788, pitch - 2, C["select"], 5)
    rect(266, y0 + sel * pitch + 7, 2, pitch - 14, C["accent"], 1)
    add(f'<path d="M{MAIN} {cy[0]}V{cy[-1]}" stroke="{C["accent"]}" stroke-width="1.6" opacity="0.8"/>')
    add(f'<path d="M{MAIN} {cy[4]}C{MAIN} {cy[4] + 18} {BR} {cy[5] - 18} {BR} {cy[5]}V{cy[7]}" fill="none" '
        f'stroke="{C["lineStrong"]}" stroke-width="1.6"/>')
    for i, (kind, rail, parts, meta, chip_) in enumerate(rows):
        c = cy[i]
        active = rail == MAIN
        if kind == "user":
            dot(rail, c, 4.5, C["steel"] if active else C["faint"])
        elif kind == "pi":
            ring(rail, c, 4, C["text2"] if active else C["faint"], 1.5, C["canvas"])
        elif kind == "tool":
            rect(rail - 3.5, c - 3.5, 7, 7, C["muted"] if active else C["lineStrong"], 1.5)
        elif kind == "compact":
            add(f'<path d="M{rail} {c - 5}L{rail + 5} {c}L{rail} {c + 5}L{rail - 5} {c}Z" fill="{C["amber"]}"/>')
        tparts(348, c + 4.5, parts, size=12.5 if kind in ("user", "pi", "compact") else 11.5)
        if meta == "✓":
            icon("check", 1030, c - 6, 12, C["green"])
        elif meta == "✕":
            icon("x", 1030, c - 6, 12, C["coral"])
        elif meta:
            text(1042, c + 4, meta, C["faint"], size=10.5, family=MONO, anchor="end")
        if chip_:
            s, k = chip_
            px = 348 + sum(tw(p[0], 11.5, True) for p in parts) + 14
            fg, bg, bd = {"label": (C["amber"], blend(C["amber"], C["canvas"], 0.1), blend(C["amber"], C["canvas"], 0.35)),
                          "summary": (C["muted"], C["hover"], C["line"]),
                          "leaf": (C["accent"], C["select"], C["focus"])}[k]
            if k == "label":
                icon("tag", px, c - 6, 12, C["amber"])
                px += 16
            pill(px, c - 8, s, fg, bg, size=9.5, h=16, border=bd)
    y = 808
    for ic_kind, s, x in (("user", "you", 288), ("pi", "pi", 344), ("tool", "tool call", 392), ("compact", "compaction", 482),
                          ("rail", "current path", 590)):
        if ic_kind == "user":
            dot(x, y - 4, 4, C["steel"])
        elif ic_kind == "pi":
            ring(x, y - 4, 3.5, C["text2"], 1.5)
        elif ic_kind == "tool":
            rect(x - 3.5, y - 7.5, 7, 7, C["muted"], 1.5)
        elif ic_kind == "compact":
            add(f'<path d="M{x} {y - 9}L{x + 5} {y - 4}L{x} {y + 1}L{x - 5} {y - 4}Z" fill="{C["amber"]}"/>')
        else:
            line(x - 6, y - 4, x + 6, y - 4, C["accent"], 1.6)
        text(x + 10, y, s, C["faint"], size=11)
    text(288, 832, "Select a message to continue from it. Branches you leave stay in the file.", C["faint"], size=11)

    insp_title("Live endpoint test", "Not on the current path", "3 entries", kind="faint")
    section(290, "ENTRY")
    kv(316, "Role", "pi")
    kv(340, "Time", "09:54", mono=True)
    kv(364, "Output", "3.1k tokens", mono=True)
    kv(388, "Tool calls", "2", mono=True)
    divider(406)
    section(432, "MESSAGE")
    quote(1084, 444, 288, ["Added openai-completions.test.ts calling", "OpenCode with a real key and asserting the",
                           "empty signature is accepted."], size=11.5)
    section(540, "BRANCH SUMMARY", "VIEW", right_kind="bracket")
    text(1084, 564, "Written when you left this branch at 09:57.", C["faint"], size=11)
    text(1084, 582, "1.4k tokens, attached to the current path.", C["faint"], size=11)
    divider(600)
    section(626, "ACTIONS")
    button(1084, 638, "Continue from here", "primary", w=288)
    button(1084, 670, "Fork to new session", "ghost", w=184, ic="branch")
    button(1276, 670, "Label…", "ghost", w=96, ic="tag")
    checkbox(1084, 712)
    text(1106, 724, "Summarize the branch I leave", C["text2"], size=12)
    note(1084, 768, 288, ["The tree moves inside this session file.", "Fork copies the path up to here into a new one."])
    text(1084, 832, "Inspecting an entry on an inactive branch", C["faint"], size=10)


# ---------------------------------------------------------------- 03 Changes

def v_changes():
    window_begin("Qwen signatures", session_sel=0, states=IDLE, tabs=SESSION_TABS, tab_active=1,
                 status_right="3 files · +28 −1")
    line(496.5, 222, 496.5, 846, C["hover"])
    label(272, 246, "3 FILES")
    tparts(482, 246, [("+28 ", C["green"]), ("−1", C["coral"])], size=10.5, family=MONO, anchor="end")
    files = [("openai-completions.ts", "packages/ai/src/providers", "+3", "−1"),
             ("openai-com…test.ts", "packages/ai/test", "+24", None),
             ("CHANGELOG.md", "packages/ai", "+1", None)]
    for i, (name, d, a, r) in enumerate(files):
        y = 258 + i * 46
        if i == 0:
            rect(264, y, 224, 42, C["select"], 5)
            rect(264, y + 8, 2, 26, C["accent"], 1)
        icon("file", 274, y + 8, 13, C["accent"] if i == 0 else C["faint"])
        text(294, y + 19, name, C["text"] if i == 0 else C["text2"], size=11.5, family=MONO)
        text(294, y + 34, d, C["faint"], size=10, family=MONO)
        tparts(482, y + 34, [(a, C["green"])] + ([(" " + r, C["coral"])] if r else []), size=10.5, family=MONO, anchor="end")
    text(272, 418, "From edit and write calls", C["faint"], size=11)
    text(272, 434, "on the current path.", C["faint"], size=11)

    text(516, 246, "packages/ai/src/providers/openai-completions.ts", C["text2"], size=11.5, family=MONO)
    w = button(1044, 232, "COPY PATCH", "bracket", anchor="end")
    button(1044 - w - 14, 232, "OPEN", "bracket", anchor="end")
    text(516, 266, "2 edits · 09:43 and 10:02 · +3 −1", C["faint"], size=11)
    rect(508, 280, 536, 232, C["deep"], 6)
    rows = [
        ("", "", "", "hunk", [("@@ -205,9 +205,11 @@ function readThinking", C["hunk"])]),
        ("205", "205", " ", None, [("  ", C["pl"]), ("const", C["kw"]), (" blocks = message.content;", C["pl"])]),
        ("206", "206", " ", None, [("  ", C["pl"]), ("for", C["kw"]), (" (", C["pl"]), ("const", C["kw"]), (" block ", C["pl"]), ("of", C["kw"]), (" blocks) {", C["pl"])]),
        ("207", "207", " ", None, [("    ", C["pl"]), ("if", C["kw"]), (" (block.type === ", C["pl"]), ('"text"', C["str"]), (") ", C["pl"]), ("continue", C["kw"]), (";", C["pl"])]),
        ("208", "208", " ", None, [("", C["pl"])]),
        ("209", "209", " ", None, [("    ", C["pl"]), ("if", C["kw"]), (" (block.type === ", C["pl"]), ('"thinking"', C["str"]), (") {", C["pl"])]),
        ("210", "", "−", "del", [("      ", C["pl"]), ("if", C["kw"]), (" (!block.signature) ", C["pl"]), ("throw new", C["kw"]), (" MissingSignature(model.id);", C["pl"])]),
        ("", "210", "+", "add", [("      ", C["pl"]), ("if", C["kw"]), (" (block.signature === ", C["pl"]), ("undefined", C["kw"]), (" && isAnthropic(model)) {", C["pl"])]),
        ("", "211", "+", "add", [("        ", C["pl"]), ("throw new", C["kw"]), (" MissingSignature(model.id);", C["pl"])]),
        ("", "212", "+", "add", [("      }", C["pl"])]),
        ("211", "213", " ", None, [("      signatures.push(block.signature ?? ", C["pl"]), ('""', C["str"]), (");", C["pl"])]),
        ("212", "214", " ", None, [("    }", C["pl"])]),
        ("213", "215", " ", None, [("  }", C["pl"])]),
    ]
    diff_lines(540, 584, 598, 284, rows, width=536, x_bg=508, two_cols=True)
    for i, (path, a) in enumerate((("packages/ai/test/openai-completions.test.ts", "+24"), ("packages/ai/CHANGELOG.md", "+1"))):
        y = 526 + i * 40
        rect(508, y, 536, 32, C["side"], 6, C["card"])
        icon("chevronR", 518, y + 10, 12, C["muted"])
        text(538, y + 20, path, C["text2"], size=11.5, family=MONO)
        text(1030, y + 20, a, C["green"], size=11, family=MONO, anchor="end")
    note(516, 636, 520, ["Only edit and write calls are listed. Files that bash commands change",
                         "(formatters, code generators) don't appear here."])

    insp_title("openai-completions.ts", "Modified · +3 −1", "2 edits", kind="accent", serif=False)
    section(290, "FILE")
    text(1084, 314, "packages/ai/src/providers/", C["text2"], size=11, family=MONO)
    kv(338, "Lines", "412", mono=True)
    kv(362, "Language", "TypeScript")
    divider(380)
    section(406, "TOUCHED BY", "click to jump")
    line(1090.5, 428, 1090.5, 480, C["line"])
    for i, (ic, verb, when, meta) in enumerate((("file", "read", "09:42 · turn 2", ""), ("edit", "edit", "09:43 · turn 3", "+2 −1"),
                                                ("edit", "edit", "10:02 · turn 7", "+1"))):
        y = 432 + i * 26
        rect(1083, y - 12, 15, 15, C["side"])
        icon(ic, 1084, y - 11, 13, C["muted"])
        text(1106, y, verb, C["text2"], size=11.5, family=MONO)
        text(1146, y, when, C["faint"], size=11)
        if meta:
            parts = [(meta.split()[0], C["green"])] + ([(" " + meta.split()[1], C["coral"])] if " " in meta else [])
            tparts(1372, y, parts, size=10.5, family=MONO, anchor="end")
    divider(504)
    section(530, "ACTIONS")
    button(1084, 544, "Open in editor", "primary", w=140, ic="open")
    button(1232, 544, "Copy patch", "ghost", w=140, ic="copy")
    button(1084, 576, "Reveal in Finder", "ghost", w=288)
    divider(618)
    note(1084, 648, 288, ["pi keeps no file history of its own.", "Undo with git, or ask pi to revert it."])
    text(1084, 832, "Inspecting a changed file in Qwen signatures", C["faint"], size=10)


# ---------------------------------------------------------------- 04 Context

def v_context():
    window_begin("Qwen signatures", session_sel=0, states=IDLE, tabs=SESSION_TABS, tab_active=3)
    tile(280, 236, 182, 80, "CONTEXT", "62.4k", "of 200k · 31%")
    tile(472, 236, 182, 80, "COST", "$0.41", "this session")
    tile(664, 236, 182, 80, "CACHE READ", "81%", "of input tokens")
    tile(856, 236, 182, 80, "COMPACTIONS", "1", "148.2k → 31.0k")
    label(280, 348, "TOKENS PER RESPONSE")
    lx = 1044
    for name, col in (("output", C["amber"]), ("input", C["accent"]), ("cache read", C["tidal"])):
        text(lx, 348, name, C["faint"], size=10.5, anchor="end")
        lx -= tw(name, 10.5) + 8
        rect(lx - 8, 340, 8, 8, col, 2)
        lx -= 22
    ctx = [14, 19, 24, 30, 37, 44, 52, 31, 35, 38, 42, 46, 49, 53, 57, 62]
    outk = [1.2, 0.8, 2.1, 0.6, 1.5, 0.9, 2.4, 1.1, 0.7, 1.9, 0.5, 1.3, 0.8, 2.2, 0.6, 0.9]
    base, scale = 516, 2.2
    for gy, lab in ((516, "0"), (450, "30k"), (384, "60k")):
        line(280, gy + 0.5, 1004, gy + 0.5, C["hover"])
        text(1044, gy + 4, lab, C["faint"], size=9.5, family=MONO, anchor="end")
    for i, (c, o) in enumerate(zip(ctx, outk)):
        x = 286 + i * 45
        cache = 0.1 if i in (0, 7) else 0.84
        hc, hi, ho = c * cache * scale, c * (1 - cache) * scale, max(o * scale * 2, 2)
        rect(x, round(base - hc, 1), 30, round(hc, 1), C["tidal"])
        rect(x, round(base - hc - hi, 1), 30, round(hi, 1), C["accent"])
        rect(x, round(base - hc - hi - ho, 1), 30, round(ho, 1), C["amber"])
    mx = 286 + 7 * 45 - 7.5
    line(mx, 370, mx, 516, C["amber"], 1.2, ' stroke-dasharray="3 3"')
    text(mx + 6, 378, "compacted 09:30", C["amber"], size=9.5, family=MONO)
    text(286, 532, "response 1", C["faint"], size=9.5, family=MONO)
    text(1001, 532, "16", C["faint"], size=9.5, family=MONO, anchor="end")

    label(280, 566, "IN CONTEXT NOW")
    items = [("file", "System prompt", "pi default", ""),
             ("file", "Context files", "AGENTS.md, ~/.pi/agent/AGENTS.md", "project + user"),
             ("sparkle", "Skills", "release, interactive-testing, pdf", "descriptions only"),
             ("terminal", "Tools", "read, bash, edit, write", "4 active"),
             ("layers", "History", "current path after the 09:30 compaction", "21 entries")]
    for i, (ic, name, detail, right) in enumerate(items):
        y = 590 + i * 26
        icon(ic, 280, y - 11, 13, C["faint"])
        text(302, y, name, C["text2"], size=12.5)
        text(420, y, detail, C["muted"], size=11, family=MONO)
        text(1044, y, right, C["faint"], size=11, anchor="end")
        line(280, y + 8.5, 1044, y + 8.5, C["hover"])
    rect(280, 726, 764, 72, C["side"], 8, C["card"])
    add(f'<path d="M298 {745}L303 {750}L298 {755}L293 {750}Z" fill="{C["amber"]}"/>')
    tparts(314, 755, [("Compacted at 09:30", C["text"], None, 600), ("  ·  threshold", C["faint"])], size=13)
    text(314, 777, "148.2k → 31.0k tokens. The last 20k tokens stay as they were; a summary covers the rest.", C["muted"], size=12)
    button(1030, 742, "VIEW SUMMARY", "bracket", anchor="end")

    insp_title("Context", "31% of window used", "claude-opus-5-5", kind="accent")
    section(290, "AUTO-COMPACTION")
    text(1084, 316, "Enabled", C["muted"], size=12)
    toggle(1344, 304, True)
    kv(342, "Reserve for reply", "16,384", mono=True)
    kv(366, "Keep recent", "20,000", mono=True)
    text(1084, 388, "Runs when context passes window − reserve.", C["faint"], size=11)
    divider(404)
    section(430, "COMPACT NOW")
    rect(1084, 442, 288, 64, C["canvas"], 6, C["chipLine"])
    text(1096, 462, "Keep the provider decisions and", C["faint"], size=12)
    text(1096, 480, "the test plan.", C["faint"], size=12)
    button(1084, 516, "Compact", "primary", w=96)
    text(1192, 532, "Entries stay in the file.", C["faint"], size=11)
    divider(556)
    section(582, "RETRIES", "auto-retry on")
    dot(1089, 604, 3.5, C["amber"])
    text(1100, 608, "09:47 · 529 overloaded", C["text2"], size=12)
    text(1372, 608, "1 of 3", C["faint"], size=10.5, family=MONO, anchor="end")
    dot(1089, 628, 3.5, C["green"])
    text(1100, 632, "09:47 · recovered after 2.0s", C["text2"], size=12)
    divider(652)
    section(678, "PROMPT CACHE")
    kv(704, "Warming", "while streaming")
    kv(728, "Cache misses", "none this session")
    text(1084, 832, "Inspecting context for Qwen signatures", C["faint"], size=10)


# ---------------------------------------------------------------- 05 Commands

def v_commands():
    window_begin("Qwen signatures", session_sel=0, states=IDLE, tabs=SESSION_TABS, tab_active=0,
                 status_right="$0.52 · 34% context")
    user_msg(236, ["qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response.",
                   "Accept empty signatures there, but keep the check strict for Anthropic."], "09:41")
    thinking_row(312)
    tparts(288, 340, [("The check lives in ", C["text2"]), ("openai-completions.ts", C["code"], MONO),
                      (". I'll allow empty strings for OpenCode models and", C["text2"])])
    text(288, 359, "leave the Anthropic path untouched.", C["text2"], size=13)
    tool_row(373, "file", "Read", "packages/ai/src/providers/openai-completions.ts", "412 lines")
    tool_row(397, "search", "Grep", '"signature" in packages/ai', "7 matches")
    edit_card(422)
    text(288, 580, "Checks and the new test pass. Anything else before the changelog entry?", C["text2"], size=13)
    composer(700, 134, "/co", thinking="high", buttons=(("Send  ⏎", "primary"),), stop=False)

    px0, px1, py0, py1 = 276, 852, 418, 692
    popover(px0, py0, px1 - px0, py1 - py0)
    tparts(292, 436, [("COMMANDS MATCHING  ", C["faint"]), ("/co", C["accent"])], size=10, family=MONO)
    text(836, 436, "4 sources", C["faint"], size=10.5, anchor="end")
    line(px0, 444.5, px1, 444.5, C["line"])
    groups = [
        ("BUILT-IN", [("/compact [instructions]", "Compact the context", "app"), ("/copy", "Copy the last reply", "app"),
                      ("/clone", "Copy this session at this point", "app")]),
        ("PROMPT TEMPLATES", [("/commit-msg", "Write a commit message for staged work", "project")]),
        ("SKILLS", [("/skill:code-review", "Review the current diff", "user")]),
        ("EXTENSIONS", [("/commit", "Commit and push with checks", "git-guard")]),
    ]
    y = 446
    for g, rows in groups:
        label(292, y + 11, g, size=9)
        y += 15
        for cmd, desc, src in rows:
            sel = cmd == "/commit"
            if sel:
                rect(280, y, 568, 26, C["select"], 5)
                rect(280, y + 5, 2, 16, C["accent"], 1)
            hit = cmd.find("co")
            parts = [(cmd[:hit], C["text"]), ("co", C["accent"]), (cmd[hit + 2:], C["text"])] if hit >= 0 else [(cmd, C["text"])]
            tparts(296, y + 17, parts, size=12, family=MONO)
            text(478, y + 17, desc, C["muted"] if not sel else C["text2"], size=12)
            pill(836, y + 5, src, C["muted"], C["hover"], size=9.5, h=16, anchor="end", border=C["line"])
            y += 26
        y += 1
    line(px0, 664.5, px1, 664.5, C["line"])
    text(292, 682, "↑↓ move   ⏎ run   ⇥ complete", C["faint"], size=10, family=MONO)
    text(836, 682, "Built-ins run in the app. The rest go to pi.", C["faint"], size=11, anchor="end")

    insp_title("/commit", "Extension command", "git-guard", kind="accent", serif=False)
    section(290, "SOURCE")
    kv(316, "Package", "npm:@acme/git-guard", mono=True)
    kv(340, "Version", "1.4.0 · pinned", mono=True)
    kv(364, "Scope", "user")
    kv(388, "Origin", "package")
    text(1084, 410, "…/node_modules/@acme/git-guard/index.ts", C["faint"], size=10.5, family=MONO)
    divider(428)
    section(454, "WHEN IT RUNS")
    text(1084, 478, "Right away, even while pi is working.", C["text2"], size=12)
    text(1084, 498, "The extension makes its own model calls.", C["faint"], size=11)
    divider(518)
    section(544, "ARGUMENTS")
    text(1084, 568, "None. It asks with dialogs when it needs input.", C["text2"], size=12)
    divider(588)
    section(614, "ALSO FROM GIT-GUARD")
    text(1084, 640, "/push-check", C["text2"], size=11.5, family=MONO)
    text(1372, 640, "Check before pushing", C["faint"], size=11, anchor="end")
    button(1084, 668, "Run /commit", "primary", w=288)
    text(1084, 832, "Commands come from get_commands, plus the app's built-ins", C["faint"], size=10)


# ---------------------------------------------------------------- 06 Extensions

def v_extensions():
    window_begin("Kimi K3 default", session_sel=2, tabs=SESSION_TABS, tab_active=0,
                 status_right="git-guard: watching · mcp: 3 servers · $0.02")
    user_msg(236, ["Switch the OpenCode Go default to Kimi K3 and rebuild the ai package."], "10:14")
    text(288, 296, "Updated the default in model-defaults.ts. Cleaning the old build before rebuilding.", C["text2"], size=13)
    tool_row(310, "edit", "Edit", "packages/coding-agent/src/core/model-defaults.ts", "+1 −1")
    tool_row(334, "terminal", "Bash", "rm -rf packages/ai/dist", "waiting on git-guard", status="wait")
    cy0, ch = 364, 142
    rect(280, cy0, 764, ch, blend(C["amber"], C["side"], 0.05), 10, blend(C["amber"], C["side"], 0.45))
    icon("shield", 296, cy0 + 14, 15, C["amber"])
    text(318, cy0 + 26, "GIT-GUARD ASKS", C["amber"], size=10, weight=500, family=MONO, ls=1.2)
    icon("clock", 922, cy0 + 15, 13, C["faint"])
    text(1028, cy0 + 26, "blocks in 24s", C["faint"], size=10.5, family=MONO, anchor="end")
    text(296, cy0 + 56, "Allow a destructive command?", C["text"], size=15, weight=600)
    text(296, cy0 + 78, "rm -rf deletes 214 files in packages/ai/dist. None of them are tracked by git.", C["text2"], size=12.5)
    x = 296
    x += button(x, cy0 + 98, "Allow once", "primary") + 8
    x += button(x, cy0 + 98, "Allow for this session", "ghost") + 8
    button(x, cy0 + 98, "Block", "danger")
    text(1028, cy0 + 114, "select · 3 options", C["faint"], size=10.5, family=MONO, anchor="end")
    icon("check", 288, 525, 13, C["green"])
    tparts(310, 536, [("git-guard asked “Push to origin/main?”", C["muted"]), ("  ·  you chose Allow at 09:12", C["faint"])], size=12)
    icon("bell", 288, 549, 13, C["amber"])
    tparts(310, 560, [("notify  ", C["amber"], MONO), ("Force push is blocked by policy", C["muted"]), ("  ·  09:13", C["faint"])], size=12)

    rect(276, 652, 768, 38, C["side"], 8, C["card"])
    icon("widget", 288, 664, 13, C["faint"])
    text(308, 675, "GIT-GUARD", C["faint"], size=9.5, weight=500, family=MONO, ls=1.2)
    text(386, 675, "main is 2 commits ahead of origin · 1 file staged · last check 10:14", C["text2"], size=11, family=MONO)
    composer(700, 134, "git status", prefix=("! SHELL", C["green"]), thinking="high",
             buttons=(("Run  ⏎", "primary"),), stop=False, hint="Output goes to pi. !! keeps it out.")

    tx0, ty0 = 744, 236
    popover(tx0, ty0, 300, 46)
    rect(tx0, ty0, 300, 46, "none", 8, blend(C["amber"], C["bar"], 0.5))
    icon("warning", tx0 + 12, ty0 + 9, 15, C["amber"])
    text(tx0 + 36, ty0 + 20, "Force push blocked by policy", C["text"], size=12)
    text(tx0 + 36, ty0 + 36, "git-guard · notify · warning", C["faint"], size=10.5)
    icon("x", tx0 + 278, ty0 + 10, 11, C["faint"])

    insp_title("Extensions", "1 waiting for you", "3 loaded", kind="amber")
    section(290, "LOADED")
    exts = [("git-guard", "npm · user", "waiting", "amber", None),
            ("pi-mcp-adapter", "git · user", "3 servers", "green", None),
            ("rpc-demo", ".pi/extensions · project", "1 error", "coral", "tool_call handler threw")]
    for i, (name, src, st, col, err) in enumerate(exts):
        y = 316 + i * 44
        text(1084, y, name, C["text2"], size=12, family=MONO)
        text(1084, y + 17, src if not err else f"{src} · {err}", C["faint"] if not err else C["coral"], size=10.5)
        text(1372, y, st, C[col], size=11, anchor="end")
        dot(1372 - tw(st, 11) - 9, y - 4, 3, C[col])
    divider(440)
    section(466, "IN THIS APP", "ctx.ui")
    methods = ["select", "confirm", "input", "editor", "notify", "status", "widget", "title"]
    for i, m in enumerate(methods):
        x = 1084 + (i % 2) * 148
        y = 490 + (i // 2) * 22
        icon("check", x, y - 10, 12, C["green"])
        text(x + 18, y, m, C["text2"], size=11.5, family=MONO)
    icon("x", 1084, 570, 12, C["coral"])
    text(1102, 580, "custom()", C["text2"], size=11.5, family=MONO)
    text(1372, 580, "terminal only", C["faint"], size=11, anchor="end")
    divider(600)
    section(626, "PENDING")
    text(1084, 650, "git-guard · select · 3 options", C["text2"], size=12)
    text(1084, 670, "Answer in the thread or here.", C["faint"], size=11)
    button(1084, 684, "Allow once", "primary", w=140)
    button(1232, 684, "Block", "danger", w=140)
    text(1084, 832, "Dialogs pause the extension until answered", C["faint"], size=10)


# ---------------------------------------------------------------- 07 All Sessions

def v_sessions():
    window_begin("All Sessions", title_icon="list", actions=[("btn", "New", "plus")], search="Search 64 sessions",
                 nav_sel="sessions", title_chevron=False, status_right="64 sessions · 3 projects")
    rect(256, 182, 808, 44, C["side"])
    seg(276, 192, ["All  64", "This project  25", "Named  12"], 0, h=25)
    rect(566, 192, 100, 25, C["chip"], 5, C["chipLine"])
    text(578, 209, "Recent", C["text2"], size=11)
    icon("chevron", 646, 199, 11, C["faint"])
    text(1044, 209, "Sorted by last change", C["faint"], size=11, anchor="end")
    line(256, 225.5, 1064, 225.5, C["deep"])
    rect(256, 226, 808, 28, C["thead"])
    for x, s, a in ((304, "Name", "start"), (620, "Project", "start"), (820, "Messages", "end"), (900, "Modified", "start")):
        text(x, 244, s, C["muted"], size=11, weight=500, anchor=a)
    line(256, 253.5, 1064, 253.5, C["deep"])
    rows = [
        ("Qwen signatures", None, "pi", "22", "12 min ago", "run"),
        ("Mistral thinking", None, "pi", "18", "2 h ago", None),
        ("Kimi K3 default", None, "pi", "9", "5 h ago", "wait"),
        ("Discount jev", None, "pi", "31", "6 h ago", None),
        (None, "why does attach take 900ms on a cold start", "pi", "14", "yesterday", None),
        ("RPC session list", None, "pi", "40", "yesterday", "fork"),
        ("Zed ACP spike", None, "zed", "27", "yesterday", None),
        (None, "sketch a gpui list with sticky headers", "zed", "6", "Tue", None),
        ("Sandbox detail study", None, "minivm", "52", "Mon", None),
        ("0.87.1 release", None, "pi", "23", "Sep 26", None),
        (None, "fix the flaky virtiofs test", "minivm", "11", "Sep 25", None),
        ("Theme okhsl pass", None, "pi", "35", "Sep 24", None),
        ("Radius relay auth", None, "pi", "19", "Sep 23", None),
        (None, "why is the agent panel blank after reload", "zed", "8", "Sep 22", None),
        ("Desktop workspace study", None, "minivm", "44", "Sep 21", None),
        ("Queue modes docs", None, "pi", "12", "Sep 20", "fork"),
    ]
    sel = 8
    for i, (name, first, proj, n, mod, mark) in enumerate(rows):
        y = 254 + i * 34
        rect(256, y, 808, 34, C["canvas"] if i % 2 == 0 else C["rowAlt"])
        if i == sel:
            rect(256, y, 808, 34, C["select"])
            rect(256, y + 7, 2, 20, C["accent"], 1)
        icon("message", 280, y + 10, 14, C["accent"] if i == sel else C["faint"])
        if name:
            text(304, y + 22, name, C["text"] if i == sel else C["text2"], size=13, weight=500 if i == sel else 400)
            nx = 304 + tw(name, 13) + 16
        else:
            text(304, y + 22, f"“{first}”", C["muted"], size=12.5, italic=True)
            nx = 304 + tw(first, 12.5) + 20
        if mark == "run":
            from pi_study_common import spinner
            spinner(nx + 6, y + 17, 5, C["accent"])
        elif mark == "wait":
            dot(nx + 6, y + 17, 3.5, C["amber"])
        elif mark == "fork":
            icon("branch", nx, y + 11, 12, C["faint"])
        text(620, y + 22, proj, C["muted"], size=11.5, family=MONO)
        text(820, y + 22, n, C["text2"], size=11.5, family=MONO, anchor="end")
        text(900, y + 22, mod, C["muted"], size=12)
    line(256, 813.5, 1064, 813.5, C["line"])
    text(276, 834, "Showing 16 of 64. Unnamed sessions show their first message.", C["faint"], size=11)

    insp_title("Sandbox detail study", "Closed", "minivm")
    section(290, "DETAILS")
    kv(316, "Project", "~/repos/minivm", mono=True)
    kv(340, "Created", "Sep 21 · 20:14")
    kv(364, "Modified", "Mon · 17:53")
    kv(388, "Messages", "52", mono=True)
    kv(412, "Forked from", "—")
    divider(430)
    section(456, "FIRST MESSAGE")
    quote(1084, 468, 288, ["Make the middle pane show what the", "selected sandbox is doing."])
    section(546, "FILE")
    text(1084, 570, "…/--repos-minivm--/2026-09-21T20-14.jsonl", C["text2"], size=10.5, family=MONO)
    icon("copy", 1356, 558, 14, C["muted"])
    divider(588)
    section(614, "ACTIONS")
    button(1084, 626, "Resume", "primary", w=288)
    button(1084, 658, "Rename…", "ghost", w=92)
    button(1182, 658, "Fork…", "ghost", w=92)
    button(1280, 658, "Clone", "ghost", w=92)
    button(1084, 690, "Export HTML", "ghost", w=140, ic="download")
    button(1232, 690, "Export JSONL", "ghost", w=140, ic="download")
    button(1084, 722, "Share link…", "ghost", w=140, ic="share")
    button(1232, 722, "Delete", "danger", w=140, ic="trash")
    note(1084, 780, 288, ["Share uploads to a private gist, or to", "Radius when signed in. Review it first."])
    text(1084, 832, "Inspecting a session in ~/repos/minivm", C["faint"], size=10)


# ---------------------------------------------------------------- 08 Models

def v_models():
    window_begin("Models", title_icon="sparkle", actions=[("btn", "Log in…", "key"), ("icon", "refresh")],
                 search="Search 312 models", nav_sel="models", title_chevron=False,
                 status_right="312 models · 5 providers · 4 in the cycle")
    provs = [("Anthropic", "Signed in · OAuth", "green", "14"), ("OpenAI", "API key · env", "green", "38"),
             ("OpenCode", "API key · auth.json", "green", "22"), ("Google", "Not configured", "faint", "19"),
             ("llama.cpp", "Local router", "green", "3")]
    for i, (name, st, col, n) in enumerate(provs):
        x = 276 + i * 154
        rect(x, 198, 146, 62, C["side"], 8, C["card"])
        text(x + 12, 222, name, C["text"] if col != "faint" else C["muted"], size=12.5, weight=600)
        text(x + 134, 222, n, C["faint"], size=11, family=MONO, anchor="end")
        if col == "faint":
            ring(x + 16, 242, 3, C["faint"])
        else:
            dot(x + 16, 242, 3, C[col])
        text(x + 26, 246, st, C["muted"], size=10.5)
    seg(276, 276, ["All", "In cycle  4", "Reasoning", "Images"], 0)
    text(1044, 292, "312 models · price per 1M tokens", C["faint"], size=11, anchor="end")
    rect(256, 312, 808, 28, C["thead"])
    for x, s, a in ((306, "Model", "start"), (486, "Provider", "start"), (640, "Context", "end"), (706, "Max out", "end"),
                    (730, "Input", "start"), (800, "Thinking", "start"), (1044, "In / Out", "end")):
        text(x, 330, s, C["muted"], size=11, weight=500, anchor=a)
    icon("star", 280, 319, 13, C["muted"])
    models = [
        (1, "claude-opus-5-5", "anthropic", "200k", "32k", 1, 7, "$15 / $75", 0),
        (1, "claude-sonnet-5", "anthropic", "200k", "64k", 1, 5, "$3 / $15", 0),
        (0, "claude-haiku-4-5", "anthropic", "200k", "64k", 1, 5, "$1 / $5", 0),
        (1, "gpt-5.6", "openai", "400k", "128k", 1, 7, "$1.25 / $10", 0),
        (0, "gpt-5.6-mini", "openai", "400k", "128k", 1, 6, "$0.25 / $2", 0),
        (1, "kimi-k3", "opencode", "256k", "32k", 0, 5, "$0.60 / $2.50", 0),
        (0, "qwen3.8-flash", "opencode", "128k", "32k", 0, 4, "$0.05 / $0.40", 0),
        (0, "gemini-3-pro", "google", "1M", "64k", 1, 5, "$2 / $12", 1),
        (0, "qwen3-coder-30b", "llama.cpp", "128k", "32k", 0, 1, "local", 0),
    ]
    for i, (star, mid, prov, ctx, mx, img, levels, price, dim) in enumerate(models):
        y = 340 + i * 32
        rect(256, y, 808, 32, C["canvas"] if i % 2 == 0 else C["rowAlt"])
        if i == 0:
            rect(256, y, 808, 32, C["select"])
            rect(256, y + 7, 2, 18, C["accent"], 1)
        tc = C["faint"] if dim else C["text2"]
        if star:
            add(f'<g transform="translate(280 {y + 9}) scale(0.5417)" fill="{C["amber"]}" stroke="{C["amber"]}" '
                f'stroke-width="1.8" stroke-linejoin="round"><use xlink:href="#star"/></g>')
        else:
            icon("star", 280, y + 9, 13, C["lineStrong"])
        text(306, y + 21, mid, C["text"] if i == 0 else tc, size=12, family=MONO, weight=500 if i == 0 else 400)
        text(486, y + 21, prov, C["faint"] if dim else C["muted"], size=11.5, family=MONO)
        text(640, y + 21, ctx, tc, size=11.5, family=MONO, anchor="end")
        text(706, y + 21, mx, tc, size=11.5, family=MONO, anchor="end")
        icon("type", 730, y + 9, 13, C["faint"] if dim else C["muted"])
        if img:
            icon("image", 750, y + 9, 13, C["faint"] if dim else C["muted"])
        for k, (_, col) in enumerate(THINK):
            if k < levels:
                rect(800 + k * 11, y + 12, 8, 8, col if not dim else blend(col, C["canvas"], 0.4), 2)
            else:
                rect(800.5 + k * 11, y + 12.5, 7, 7, "none", 2, C["lineStrong"])
        text(1044, y + 21, price, tc, size=11.5, family=MONO, anchor="end")
    text(276, 660, "Amber stars mark the models in the cycle (enabledModels). Squares show thinking levels, off to max.",
         C["faint"], size=11)
    text(276, 678, "llama.cpp lists what its router has loaded. Google models stay dimmed until a key is set.", C["faint"], size=11)

    insp_title("Claude Opus 5.5", "Signed in via OAuth", "anthropic", kind="green")
    text(1084, 268, "anthropic/claude-opus-5-5", C["faint"], size=11, family=MONO)
    section(296, "LIMITS")
    kv(322, "Context window", "200k", mono=True)
    kv(346, "Max output", "32k", mono=True)
    kv(370, "Input", "text, image")
    divider(388)
    section(414, "THINKING", "default high")
    for k, (name, col) in enumerate(THINK):
        x = 1084 + k * 41.5
        active = name == "high"
        rect(x, 426, 38, 22, col if active else C["canvas"], 4, None if active else blend(col, C["canvas"], 0.55))
        text(x + 19, 441, name, C["onLevel"] if active else blend(col, C["text"], 0.6), size=9.5, family=MONO, anchor="middle",
             weight=600 if active else 400)
    text(1084, 470, "Set per model with modelThinkingLevels.", C["faint"], size=11)
    divider(488)
    section(514, "PRICE", "per 1M tokens")
    kv(540, "Input", "$15.00", mono=True)
    kv(564, "Output", "$75.00", mono=True)
    kv(588, "Cache read", "$1.50", mono=True)
    kv(612, "Cache write", "$18.75", mono=True)
    divider(630)
    text(1084, 660, "In the cycle", C["muted"], size=12)
    toggle(1344, 648, True)
    button(1084, 684, "Use in this session", "primary", w=288)
    button(1084, 716, "Set as default", "ghost", w=288)
    note(1084, 774, 288, ["Log in opens pi in a terminal for OAuth.", "API keys can come from env or auth.json."])
    text(1084, 832, "Inspecting a model from the catalog", C["faint"], size=10)


# ---------------------------------------------------------------- 09 Resources

def v_resources():
    window_begin("Resources", title_icon="package", actions=[("btn", "Install…", "plus"), ("icon", "refresh")],
                 search="Search resources", nav_sel="resources", title_chevron=False,
                 status_right="4 packages · 5 extensions · 3 skills · 6 prompts")
    rect(276, 198, 768, 58, blend(C["amber"], C["canvas"], 0.08), 8, blend(C["amber"], C["canvas"], 0.4))
    icon("warning", 290, 208, 16, C["amber"])
    text(316, 219, "This project wants to load 1 package and 2 extensions.", C["text"], size=13, weight=600)
    text(316, 240, ".pi/settings.json declares them. They run with your permissions.", C["text2"], size=12)
    w = button(1030, 215, "Trust", "amber", anchor="end")
    w2 = button(1030 - w - 8, 215, "Not now", "ghost", anchor="end")
    button(1030 - w - w2 - 22, 215, "REVIEW", "bracket", anchor="end")
    line(256, 300.5, 1064, 300.5, C["hover"])
    utabs(276, 286, ["PACKAGES  4", "EXTENSIONS  5", "SKILLS  3", "PROMPTS  6", "CONTEXT FILES  2"], 0)
    pk = [("npm:@acme/git-guard@1.4.0", "user", "1 extension · 2 commands · pinned version", "up to date", None),
          ("git:github.com/nicobailon/pi-mcp-adapter@v0.9", "user", "1 extension · 1 tool · git ref v0.9", "up to date", None),
          ("npm:@termdraw/pi@0.3.1", "user", "1 extension · 1 command", "0.3.2 available", "amber"),
          ("./tools/pi-release", "project", "1 skill · 2 prompts · local path", "waits for trust", "amber")]
    for i, (spec, scope, prov, st, col) in enumerate(pk):
        y = 312 + i * 58
        if i == 0:
            rect(264, y + 2, 792, 54, C["select"], 6)
            rect(264, y + 12, 2, 34, C["accent"], 1)
        icon("package", 282, y + 14, 16, C["accent"] if i == 0 else C["muted"])
        text(310, y + 25, spec, C["text"] if i == 0 else C["text2"], size=12, family=MONO)
        pc = C["amber"] if scope == "project" else C["muted"]
        pill(310 + tw(spec, 12, True) + 10, y + 13, scope, pc, C["hover"], size=9.5, h=16, border=C["line"])
        text(310, y + 44, prov, C["faint"], size=11)
        text(1040, y + 25, st, C[col] if col else C["faint"], size=11, anchor="end")
        if col:
            dot(1040 - tw(st, 11) - 9, y + 21, 3, C[col])
        if i < 3:
            line(276, y + 57.5, 1044, y + 57.5, C["hover"])
    label(276, 572, "INSTALL")
    rect(276, 584, 520, 30, C["canvas"], 6, C["chipLine"])
    text(288, 603, "npm:@scope/name@version, git:host/repo@ref, or ./path", C["faint"], size=11, family=MONO)
    seg(806, 587, ["User", "Project"], 0)
    button(1044, 587, "Install", "primary", anchor="end")
    note(276, 648, 700, ["User installs go to ~/.pi/agent/settings.json. Project installs go to .pi/settings.json",
                         "and load only after project trust."])

    insp_title("git-guard", "Loaded", "npm", kind="green")
    section(290, "SOURCE")
    kv(316, "Spec", "npm:@acme/git-guard@1.4.0", mono=True)
    kv(340, "Scope", "user")
    text(1084, 362, "~/.pi/agent/settings.json", C["faint"], size=10.5, family=MONO)
    text(1084, 382, "~/.pi/agent/npm/node_modules/@acme/git-guard", C["faint"], size=10.5, family=MONO)
    divider(400)
    section(426, "PROVIDES")
    kv(452, "Extensions", "1", mono=True)
    kv(476, "Commands", "/commit, /push-check", mono=True)
    kv(500, "Tools", "none")
    kv(524, "Skills", "none")
    kv(548, "Prompts and themes", "none")
    divider(566)
    section(592, "ACTIONS")
    button(1084, 604, "Update", "disabled", w=88)
    button(1180, 604, "Move to project", "ghost", w=192)
    button(1084, 636, "Remove", "danger", w=288, ic="trash")
    note(1084, 694, 288, ["Packages run code with your permissions.", "Read the source before you install one."])
    text(1084, 832, "Inspecting a package from user settings", C["faint"], size=10)


# ---------------------------------------------------------------- 10 Settings

def v_settings():
    window_begin("Settings", title_icon="settings", actions=[("btn", "Open JSON", "file")], search="Search settings",
                 nav_sel="settings", title_chevron=False, status_right="user settings · 3 changed from default")
    seg(276, 193, ["User  ~/.pi/agent/settings.json", "Project  .pi/settings.json"], 0, h=26)
    text(1044, 211, "3 changed from default", C["faint"], size=11, anchor="end")
    line(256, 228.5, 1064, 228.5, C["hover"])
    line(436.5, 229, 436.5, 846, C["hover"])
    cats = ["Model & thinking", "Interaction", "Tools", "Sessions & context", "Compaction", "Branch summaries",
            "Network & retries", "Shell", "Resources", "Updates & telemetry"]
    for i, c in enumerate(cats):
        y = 240 + i * 28
        if i == 0:
            rect(264, y, 164, 26, C["select"], 5)
            rect(264, y + 5, 2, 16, C["accent"], 1)
            dot(418, y + 13, 3, C["accent"])
        text(278, y + 17, c, C["text"] if i == 0 else C["text2"], size=12.5, weight=500 if i == 0 else 400)

    def row(y, title, key, changed=False, selected=False):
        if selected:
            rect(448, y, 604, 50, C["hover"], 6)
        text(456, y + 21, title, C["text"], size=13)
        text(456, y + 38, key, C["faint"], size=10.5, family=MONO)
        if changed:
            pill(456 + tw(title, 13) + 10, y + 9, "changed", C["accent"], C["select"], size=9, h=15, border=C["focus"])
        line(456, y + 50.5, 1044, y + 50.5, C["hover"])

    label(456, 256, "MODEL & THINKING")
    row(266, "Default model", "defaultModel", changed=True)
    rect(794, 278, 250, 26, C["chip"], 5, C["chipLine"])
    text(806, 295, "anthropic / claude-opus-5-5", C["text2"], size=11, family=MONO)
    icon("chevron", 1024, 285, 11, C["faint"])

    row(316, "Default thinking level", "defaultThinkingLevel", changed=True, selected=True)
    names = [n for n, _ in THINK]
    sw = sum(tw(n, 10, True) + 14 for n in names) + 4
    seg(round(1044 - sw, 1), 329, names, 4, h=24, size=10, mono=True, pad=7, colors=[c for _, c in THINK])

    row(366, "Models in the cycle", "enabledModels", changed=True)
    x = 1044
    for s in ("+", "kimi-k3", "gpt-5.6", "claude-*")[::1]:
        w = tw(s, 10.5, True) + 16
        x -= w
        pill(x, 379, s, C["text2"] if s != "+" else C["muted"], C["chip"], size=10.5, h=22, pad=8, border=C["chipLine"])
        x -= 6

    row(416, "Hide thinking blocks", "hideThinkingBlock")
    toggle(1016, 433, False)
    row(466, "Cache warming", "cacheWarming")
    s2 = sum(tw(n, 11) + 24 for n in ("off", "streaming", "idle")) + 4
    seg(round(1044 - s2, 1), 479, ["off", "streaming", "idle"], 1)
    row(516, "Cache-miss notices", "showCacheMissNotices")
    toggle(1016, 533, False)

    label(456, 598, "INTERACTION")
    row(608, "Steering delivery", "steeringMode")
    s3 = sum(tw(n, 11) + 24 for n in ("one at a time", "all")) + 4
    seg(round(1044 - s3, 1), 621, ["one at a time", "all"], 0)
    row(658, "Follow-up delivery", "followUpMode")
    seg(round(1044 - s3, 1), 671, ["one at a time", "all"], 0)
    row(708, "Project trust default", "defaultProjectTrust")
    s4 = sum(tw(n, 11) + 24 for n in ("ask", "always", "never")) + 4
    seg(round(1044 - s4, 1), 721, ["ask", "always", "never"], 0)
    text(round(1044 - s4 - 10, 1), 737, "user file only", C["faint"], size=10.5, anchor="end")

    insp_title("defaultThinkingLevel", "Changed in user settings", "was medium", kind="accent", serif=False)
    section(290, "DESCRIPTION")
    text(1084, 314, "Thinking level for new sessions. A level in", C["text2"], size=12)
    text(1084, 332, "modelThinkingLevels wins for that model.", C["text2"], size=12)
    text(1084, 350, "xhigh and max need a model that supports them.", C["faint"], size=11)
    divider(368)
    section(394, "VALUE")
    kv(420, "Current", "high", THINK_COLOR["high"], mono=True)
    kv(444, "Default", "medium", mono=True)
    kv(468, "Allowed", "off … max", mono=True)
    divider(486)
    section(512, "IN SETTINGS.JSON")
    rect(1084, 524, 288, 76, C["deep"], 6, C["line"])
    text(1098, 546, "{", C["pl"], size=11, family=MONO)
    tparts(1098, 564, [("  ", C["pl"]), ('"defaultThinkingLevel"', C["text2"]), (": ", C["pl"]), ('"high"', C["str"])], size=11, family=MONO, pre=True)
    text(1098, 582, "}", C["pl"], size=11, family=MONO)
    text(1084, 620, "~/.pi/agent/settings.json", C["faint"], size=10.5, family=MONO)
    button(1084, 636, "Reset to default", "ghost", w=150)
    button(1250, 636, "DOCS", "bracket")
    note(1084, 700, 288, ["New sessions start at this level. Running", "sessions keep the level they have."])
    text(1084, 832, "Editing ~/.pi/agent/settings.json", C["faint"], size=10)


VIEWS = [
    (v_new, "New session", "What's loaded, before you type",
     "Context files, skills, templates and extensions pi found for the project, and a composer that knows / and !."),
    (v_tree, "Tree", "History as a tree, not a scroll",
     "Branches stay in the session file. Continue from any entry, fork it to a new session, or label it."),
    (v_changes, "Changes", "Everything this session touched",
     "Files from edit and write calls on the current path, one diff per file, and the calls that made them."),
    (v_context, "Context", "What pi sends, and what it costs",
     "Usage per response, what is loaded right now, retries, and compaction you can run with instructions."),
    (v_commands, "Commands", "Slash commands from every source",
     "Built-ins, prompt templates, skills and extension commands in one list, each with where it came from."),
    (v_extensions, "Extensions", "Extensions ask, the app answers",
     "Dialogs, widgets, status and notices from ctx.ui, plus ! shell commands in the composer."),
    (v_sessions, "All Sessions", "Every session, every project",
     "Search, resume, rename, fork, export, share or delete. Unnamed sessions show their first message."),
    (v_models, "Models", "Providers, models, and what they cost",
     "Sign-in state per provider, the catalog with limits and prices, and which models are in the cycle."),
    (v_resources, "Resources", "Packages, extensions, skills and prompts",
     "What is installed, where it came from, what it adds, and a trust prompt before project code runs."),
    (v_settings, "Settings", "settings.json, with the defaults shown",
     "User and project scopes side by side. Every control names its key."),
]


def main(theme="dark"):
    set_theme(theme)
    reset()
    defs(W, H, "<title>pi desktop — every screen, visual proposal, not implemented</title>"
               "<desc>Follow-up to desktop-thread-study.svg. New session, Tree, Changes, Context, Commands and Extensions are "
               "session views; All Sessions, Models, Resources and Settings are app views. Each uses only data pi exposes "
               "through RPC, session files, settings.json and pi list. All names, numbers and prices are sample data.</desc>")
    page(W, H)
    label(48, 42, "PI  /  DESIGN STUDY  02" + ("  ·  LIGHT" if theme == "light" else ""), C["pageLabel"], size=11)
    add(f'<text x="48" y="96" fill="{C["evening"]}" font-family="{SERIF}" font-size="40" font-style="italic">'
        f'Every part of pi, in <tspan fill="{DARK["accent"]}">one window</tspan>.</text>')
    text(48, 128, "Session views are tabs or overlays on a session. App views sit at the bottom of the sidebar. Every screen "
                  "uses data pi already exposes: RPC commands and events, session files, settings.json and pi list.",
         C["driftwood"], size=14)
    label(2800, 92, "VISUAL PROPOSAL · SAMPLE DATA · FOLLOWS STUDY 01", C["pageLabel"], anchor="end", size=11)

    for k, (fn, name, tag, sub) in enumerate(VIEWS):
        dx = (k % 2) * 1408
        y = 190 + (k // 2) * 880
        dot(48 + dx + 13, y - 5, 13, C["parchment"])
        text(48 + dx + 13, y - 1, f"{k + 1:02d}", C["evening"], size=10, weight=600, family=MONO, anchor="middle")
        tparts(48 + dx + 36, y, [(name, C["evening"], None, 600), ("  —  " + tag, C["driftwood"])], size=16)
        text(48 + dx, y + 24, sub, C["driftwood"], size=12.5)
        add(f'<g transform="translate({dx} {y + 44 - 130})">')
        fn()
        window_end()
        add("</g>")

    text(48, 4560, "LAYOUT STUDY ONLY — follows desktop-thread-study.svg and its colour profile. Session views: New session, "
                   "Thread, Changes, Tree, Context, plus the command list and extension dialogs. App views: All Sessions, "
                   "Models, Resources, Settings. Names, numbers and prices are sample data.", C["warm30"], size=11)
    add("</svg>")
    return dump()


if __name__ == "__main__":
    sys.stdout.write(main("light" if "light" in sys.argv[1:] else "dark"))
