#!/usr/bin/env python3
"""pi desktop design study 03: several projects at once, each screen in Evening (dark) and Moonstone (light).

The app runs one `pi --mode rpc` process per open session, started in that session's project folder.
Writes SVG to stdout.
"""
import sys

from gen_desktop_thread_study import thread_content, thread_inspector
from pi_study_common import (
    C, DARK, MONO, THEME, MULTI_ACTIVE, MULTI_PROJECTS, SERIF, SESSION_TABS, THINK_COLOR, add, banner, blend, button, checkbox,
    composer, ctx_fill, defs, dim_window, divider, dot, dump, edit_card, icon, input_box, insp_title, kv, label, line,
    note, page, popover, radio, rect, reset, ring, section, seg, set_theme, spinner, state_mark, text, thinking_row,
    tool_row, tparts, tw, user_msg, utabs, window_begin, window_end,
)

W, H = 2848, 3820
MULTI = dict(active=MULTI_ACTIVE, projects=MULTI_PROJECTS, selected="Qwen signatures")
PROCESSES = "3 processes · 2 running · 1 waiting"


# ---------------------------------------------------------------- 01 Active across projects

def r_active():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=SESSION_TABS, status_left_extra=PROCESSES)
    thread_content()
    thread_inspector()
    x0, y0, w, h = 700, 232, 344, 88
    popover(x0, y0, w, h)
    rect(x0, y0 + 10, 3, h - 20, C["amber"], 1.5)
    icon("bell", x0 + 16, y0 + 13, 15, C["amber"])
    tparts(x0 + 40, y0 + 25, [("zed", C["muted"], MONO), ("  ·  ", C["faint"]), ("Agent panel fix", C["text"], None, 600)], size=12.5)
    text(x0 + w - 14, y0 + 25, "now", C["faint"], size=10.5, family=MONO, anchor="end")
    text(x0 + 40, y0 + 45, "git-guard asks to allow a force push.", C["text2"], size=12)
    button(x0 + 40, y0 + 56, "Open", "primary", h=22)
    button(x0 + 104, y0 + 56, "Later", "ghost", h=22)


# ---------------------------------------------------------------- 02 New session in any project

def r_new():
    window_begin("Qwen signatures", title_prefix="pi", multi=MULTI, tabs=SESSION_TABS, status_left_extra=PROCESSES)
    thread_content()
    thread_inspector()
    dim_window()
    x0, y0, w, h = 470, 190, 500, 620
    popover(x0, y0, w, h)
    text(494, 230, "New session", C["text"], size=22, family=SERIF, italic=True)
    icon("x", 940, 216, 14, C["faint"])
    label(494, 266, "PROJECT")
    projects = [("pi", "~/repos/pi", "main", "run", "1 running"),
                ("zed", "~/repos/zed", "main", "wait", "1 waiting"),
                ("minivm", "~/repos/minivm", "feat/list-perf", "run", "1 running")]
    for i, (name, path, branch, state, st) in enumerate(projects):
        y = 276 + i * 42
        sel = i == 0
        if sel:
            rect(494, y, 452, 38, C["select"], 6, C["focus"])
        radio(512, y + 19, sel)
        icon("folder", 528, y + 12, 14, C["text2"] if sel else C["faint"])
        text(550, y + 24, name, C["text"], size=13, weight=600)
        text(550 + tw(name, 13) + 12, y + 24, path, C["faint"], size=11, family=MONO)
        text(820, y + 24, st, C["muted"], size=11, anchor="end")
        state_mark(820 - tw(st, 11) - 12, y + 20, state)
        text(930, y + 24, branch, C["faint"], size=10.5, family=MONO, anchor="end")
    icon("plus", 511, 404, 12, C["faint"])
    text(530, 414, "Open Folder…", C["muted"], size=12)
    label(494, 446, "RUN IN")
    seg(494, 456, ["Project folder", "New git worktree"], 1, h=26)
    label(494, 502, "BRANCH", size=9)
    input_box(494, 510, 452, "pi/qwen-regression-test")
    label(494, 558, "PATH", size=9)
    input_box(494, 566, 452, "~/repos/.worktrees/pi/qwen-regression-test")
    note(494, 624, 452, ["A session is already running in ~/repos/pi. A worktree", "keeps this one from editing the same files."],
         ic="warning", color=C["amber"])
    label(494, 674, "MODEL", size=9)
    chipbg = C["chip"] if THEME["name"] == "dark" else C["hover"]
    rect(494, 684, 166, 24, chipbg, 5)
    icon("sparkle", 501, 689, 13, C["muted"])
    text(520, 700, "claude-opus-5-5", C["text2"], size=11, family=MONO)
    icon("chevron", 642, 690, 11, C["faint"])
    rect(668, 684, 90, 24, chipbg, 5)
    dot(681, 696, 3.5, THINK_COLOR["med"])
    text(691, 700, "medium", C["text2"], size=11, family=MONO)
    icon("chevron", 740, 690, 11, C["faint"])
    checkbox(494, 726)
    text(516, 738, "Remove the worktree when this session is deleted", C["text2"], size=12)
    line(x0, 760.5, x0 + w, 760.5, C["line"])
    wc = button(946, 772, "Create session", "primary", anchor="end")
    button(946 - wc - 8, 772, "Cancel", "ghost", anchor="end")


# ---------------------------------------------------------------- 03 Two sessions, one folder

def r_overlap():
    active = [("Qwen signatures", "pi", "run"), ("Mistral thinking", "pi", "run"), ("Agent panel fix", "zed", "wait")]
    projects = [("pi", "25", "run", [("Qwen signatures", "run"), ("Mistral thinking", "run"), ("Kimi K3 default", "5h"),
                                     ("Discount jev", "6h")]),
                ("zed", "8", "wait", None), ("minivm", "12", None, None)]
    window_begin("Qwen signatures", title_prefix="pi", multi=dict(active=active, projects=projects, selected="Qwen signatures"),
                 tabs=SESSION_TABS, status_left_extra="4 processes · 2 running · 1 waiting", status_left_accent=True)
    banner(280, 234, 764, 58, "Mistral thinking also changed openai-completions.ts",
           "Both sessions in ~/repos/pi edited it. Its edit at 10:12 came after yours at 09:43.")
    wc = button(1030, 251, "Compare", "ghost", anchor="end")
    button(1030 - wc - 8, 251, "Open session", "ghost", anchor="end")
    user_msg(306, ["qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response.",
                   "Accept empty signatures there, but keep the check strict for Anthropic."], "09:41")
    thinking_row(386)
    tparts(288, 414, [("The check lives in ", C["text2"]), ("openai-completions.ts", C["code"], MONO),
                      (". I'll allow empty strings for OpenCode models and", C["text2"])])
    text(288, 433, "leave the Anthropic path untouched.", C["text2"], size=13)
    tool_row(447, "file", "Read", "packages/ai/src/providers/openai-completions.ts", "412 lines")
    tool_row(471, "search", "Grep", '"signature" in packages/ai', "7 matches")
    edit_card(496)
    text(288, 648, "Checks pass. The provider tests are next.", C["text2"], size=13)
    composer(700, 134, "Run the qwen provider tests too once check passes",
             queue="Also add a regression test for the OpenCode path")

    px, py, pw, ph = 56, 612, 380, 226
    popover(px, py, pw, ph)
    base = C["bar"] if THEME["name"] == "dark" else C["chip"]
    add(f'<path d="M190 {py + ph - 1}L198 {py + ph + 7}L206 {py + ph - 1}Z" fill="{base}"/>')
    label(72, 636, "PI PROCESSES")
    text(420, 636, "4 · 457 MB", C["faint"], size=10.5, family=MONO, anchor="end")
    line(px, 646.5, px + pw, 646.5, C["line"])
    procs = [("Qwen signatures", "pi", "run", "118 MB"), ("Mistral thinking", "pi", "run", "109 MB"),
             ("Agent panel fix", "zed", "wait", "121 MB"), ("Sandbox detail", "minivm", "idle", "stops in 4m")]
    for i, (t, proj, state, right) in enumerate(procs):
        y = 650 + i * 34
        if i == 0:
            rect(px + 4, y + 2, pw - 8, 30, C["select"], 5)
        state_mark(80, y + 17, state)
        text(96, y + 22, t, C["text"] if i == 0 else C["text2"], size=12.5)
        text(232, y + 22, proj, C["faint"], size=10.5, family=MONO)
        text(420, y + 22, right, C["muted"] if state != "idle" else C["faint"], size=11, family=MONO, anchor="end")
    line(px, 790.5, px + pw, 790.5, C["line"])
    text(72, 810, "Idle sessions stop after 10 minutes and", C["faint"], size=11)
    text(72, 826, "reopen from their session file.", C["faint"], size=11)

    insp_title("Qwen signatures", "Running · bash", "09:41 · 31 min", kind="run")
    section(290, "SAME PROJECT", "~/repos/pi")
    for i, (t, right) in enumerate((("Qwen signatures", "this session"), ("Mistral thinking", "running · 6 min"))):
        y = 316 + i * 24
        spinner(1090, y - 4, 4.5, C["accent"])
        text(1102, y, t, C["text2"], size=12)
        text(1372, y, right, C["faint"], size=11, anchor="end")
    divider(358)
    section(384, "SHARED FILES", "1")
    icon("file", 1084, 399, 13, C["amber"])
    text(1104, 410, "openai-completions.ts", C["text2"], size=11.5, family=MONO)
    text(1084, 430, "you 09:43 (+3 −1) · Mistral thinking 10:12 (+5 −2)", C["faint"], size=11)
    button(1084, 444, "Compare edits", "ghost", w=140)
    button(1232, 444, "Open other session", "ghost", w=140)
    divider(486)
    note(1084, 516, 288, ["pi doesn't lock files. Start parallel work", "in a worktree to keep sessions apart."])
    divider(552)
    section(578, "CONTEXT", "auto-compact on")
    tparts(1084, 608, [("64.9k", C["text"], None, 600)], size=20)
    text(1148, 608, "/ 200k tokens", C["muted"], size=12)
    rect(1084, 618, 288, 6, C["track"], 3)
    rect(1084, 618, 93, 6, ctx_fill(), 3)
    text(1084, 642, "32% of window", C["faint"], size=11)
    divider(660)
    section(686, "USAGE", "this session")
    kv(712, "Input", "51.3k", mono=True)
    kv(736, "Output", "6.9k", mono=True)
    kv(760, "Cost", "$0.47", C["text"], mono=True, bold=True)
    text(1084, 832, "Inspecting Qwen signatures in ~/repos/pi", C["faint"], size=10)


# ---------------------------------------------------------------- 04 Project scope and trust

def r_trust():
    window_begin("Resources", title_icon="package", actions=[("btn", "Install…", "plus"), ("icon", "refresh")],
                 search="Search resources", nav_sel="resources", title_chevron=False,
                 multi=dict(active=MULTI_ACTIVE, projects=MULTI_PROJECTS, selected=None), status_left_extra=PROCESSES,
                 status_right="project pi · 1 package · 2 extensions")
    seg(276, 194, ["User", "Project"], 1, h=26)
    rect(400, 194, 240, 26, C["chip"], 6, C["focus"])
    icon("folder", 410, 200, 14, C["text2"])
    tparts(432, 211, [("pi", C["text"], None, 600), ("   ~/repos/pi", C["faint"], MONO)], size=12)
    icon("chevronU", 620, 200, 12, C["muted"])
    text(1044, 212, ".pi/settings.json", C["faint"], size=11, family=MONO, anchor="end")

    rect(276, 238, 768, 40, C["side"], 8, C["card"])
    icon("shield", 290, 250, 15, C["green"])
    tparts(314, 263, [("Trusted", C["text"], None, 600), ("   saved for this folder on Sep 12", C["faint"])], size=12.5)
    button(1030, 246, "REVOKE", "bracket", anchor="end")
    line(256, 320.5, 1064, 320.5, C["hover"])
    utabs(276, 306, ["PACKAGES  1", "EXTENSIONS  2", "SKILLS  2", "PROMPTS  4", "CONTEXT FILES  1"], 1)
    exts = [(".pi/extensions/rpc-demo.ts", "project · adds a demo dialog", "1 error at load", "coral"),
            (".pi/extensions/release-notes.ts", "project · adds /release-notes", "loaded", None)]
    for i, (path, sub, st, col) in enumerate(exts):
        y = 330 + i * 52
        icon("file", 282, y + 12, 15, C["muted"])
        text(308, y + 24, path, C["text2"], size=12, family=MONO)
        text(308, y + 41, sub, C["faint"], size=11)
        text(1040, y + 24, st, C[col] if col else C["faint"], size=11, anchor="end")
        if col:
            dot(1040 - tw(st, 11) - 9, y + 20, 3, C[col])
        line(276, y + 51.5, 1044, y + 51.5, C["hover"])
    label(276, 462, "PROJECT SETTINGS")
    rect(276, 472, 768, 104, C["deep"], 6, C["line"])
    K, S, P = C["text2"], C["str"], C["pl"]
    rows = [[("{", P)],
            [("  ", P), ('"packages"', K), (": [", P), ('"./tools/pi-release"', S), ("],", P)],
            [("  ", P), ('"extensions"', K), (": [", P), ('".pi/extensions/rpc-demo.ts"', S), (", ", P),
             ('".pi/extensions/release-notes.ts"', S), ("],", P)],
            [("  ", P), ('"defaultThinkingLevel"', K), (": ", P), ('"medium"', S)],
            [("}", P)]]
    for i, parts in enumerate(rows):
        tparts(292, 492 + i * 18, parts, size=11, family=MONO, pre=True)
    text(276, 598, "Loaded after trust. Project values override user settings for sessions in this folder.",
         C["faint"], size=11)
    note(276, 632, 700, ["Context files such as AGENTS.md load even without trust."])

    px, py, pw, ph = 400, 226, 400, 176
    popover(px, py, pw, ph)
    label(416, 248, "PROJECT SCOPE")
    line(px, 256.5, px + pw, 256.5, C["line"])
    rows = [("pi", "~/repos/pi", "Trusted", "green", "1 package · 2 extensions"),
            ("zed", "~/repos/zed", "Trusted", "green", "nothing declared"),
            ("minivm", "~/repos/minivm", "Not trusted", "amber", "2 extensions waiting")]
    for i, (name, path, trust, col, sub) in enumerate(rows):
        y = 260 + i * 40
        if i == 0:
            rect(px + 4, y + 1, pw - 8, 38, C["select"], 5)
            icon("check", 412, y + 12, 13, C["accent"])
        icon("folder", 432, y + 11, 14, C["text2"] if i == 0 else C["faint"])
        text(454, y + 18, name, C["text"], size=12.5, weight=600)
        text(454, y + 33, path, C["faint"], size=10.5, family=MONO)
        text(784, y + 18, trust, C[col] if col == "amber" else C["text2"], size=11.5, anchor="end")
        dot(784 - tw(trust, 11.5) - 9, y + 14, 3, C[col])
        text(784, y + 33, sub, C["faint"], size=10.5, anchor="end")
    text(416, 394, "Trust is saved per folder.", C["faint"], size=10.5)

    insp_title("pi", "Trusted", "~/repos/pi", kind="green")
    section(290, "PROJECT")
    kv(316, "Path", "~/repos/pi", mono=True)
    kv(340, "Branch", "main", mono=True)
    kv(364, "Sessions", "25", mono=True)
    kv(388, "Running now", "1", mono=True)
    divider(406)
    section(432, "PROJECT FILES")
    for i, (f, right) in enumerate(((".pi/settings.json", "3 keys"), ("AGENTS.md", "context"), (".pi/extensions/", "2"),
                                    (".pi/skills/", "2"), (".pi/prompts/", "4"))):
        y = 458 + i * 22
        text(1084, y, f, C["text2"], size=11.5, family=MONO)
        text(1372, y, right, C["faint"], size=11, anchor="end")
    divider(564)
    section(590, "TRUST")
    text(1084, 614, "Trusted on Sep 12, for this folder only.", C["text2"], size=12)
    button(1084, 628, "Revoke trust", "danger", w=288)
    note(1084, 690, 288, ["Revoking stops project packages and", "extensions from loading in new sessions."])
    text(1084, 832, "Inspecting project ~/repos/pi", C["faint"], size=10)


ROWS = [
    (r_active, "Active across projects", "nothing waits unseen",
     "Running and waiting sessions from every project sit at the top of the sidebar. Background sessions notify when they finish or need you."),
    (r_new, "New session", "any project, or its own worktree",
     "Pick the project, then run in its folder or in a fresh git worktree when another session is already working there."),
    (r_overlap, "Two sessions, one folder", "overlap made visible",
     "pi doesn't lock files, so the app warns when sessions touch the same file. The status bar lists every pi process."),
    (r_trust, "Project scope", "trust and resources per folder",
     "Each project has its own .pi/settings.json, packages and trust decision. Pick the project to see and change them."),
]


def main():
    reset()
    defs(W, H, "<title>pi desktop — several projects at once, visual proposal, not implemented</title>"
               "<desc>Follow-up to desktop-thread-study.svg and desktop-views-study.svg. One pi --mode rpc process per open "
               "session, started in that session's folder. Active sessions across projects, new sessions in any project or "
               "a git worktree, overlap warnings, the process list, and per-project trust. Each screen is shown in the "
               "Evening (dark) and Moonstone (light) themes. All names and numbers are sample data.</desc>")
    page(W, H)
    label(48, 42, "PI  /  DESIGN STUDY  03", C["pageLabel"], size=11)
    add(f'<text x="48" y="96" fill="{C["evening"]}" font-family="{SERIF}" font-size="40" font-style="italic">'
        f'Every project, <tspan fill="{DARK["accent"]}">at once</tspan>.</text>')
    text(48, 128, "One pi process per open session, started in that session's folder. The sidebar shows what is running or "
                  "waiting everywhere. Every screen is drawn in Evening (dark) and Moonstone (light).",
         C["driftwood"], size=14)
    label(2800, 92, "VISUAL PROPOSAL · SAMPLE DATA · FOLLOWS STUDIES 01–02", C["pageLabel"], anchor="end", size=11)

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

    text(48, 3780, "LAYOUT STUDY ONLY — follows desktop-thread-study.svg and desktop-views-study.svg. Process counts and memory "
                   "are sample values; one idle pi --mode rpc process measured about 110 MB. The app creates worktrees itself; "
                   "pi needs no change for them.", C["warm30"], size=11)
    add("</svg>")
    return dump()


if __name__ == "__main__":
    sys.stdout.write(main())
