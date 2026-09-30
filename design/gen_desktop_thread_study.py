#!/usr/bin/env python3
"""pi desktop design study 01: the thread, plus the colour profile. Writes SVG to stdout."""
import sys

from pi_study_common import (
    C, DARK, DARK_THINK, LIGHT, LIGHT_THINK, MONO, SERIF, add, ctx_fill, set_theme, bash_card, composer, defs, divider, dot, dump, edit_card, icon, insp_title, kv,
    label, line, luminance, page, rect, reset, section, text, thinking_row, tool_row, tparts, user_msg, utabs,
    window_begin, window_end,
)

W, H = 1440, 1430


def thread_content():
    user_msg(236, ["qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response.",
                   "Accept empty signatures there, but keep the check strict for Anthropic."], "09:41")
    thinking_row(312)
    tparts(288, 340, [("The check lives in ", C["text2"]), ("openai-completions.ts", C["code"], MONO),
                      (". I'll allow empty strings for OpenCode models and", C["text2"])])
    text(288, 359, "leave the Anthropic path untouched.", C["text2"], size=13)
    tool_row(373, "file", "Read", "packages/ai/src/providers/openai-completions.ts", "412 lines")
    tool_row(397, "search", "Grep", '"signature" in packages/ai', "7 matches")
    edit_card(422)
    bash_card(558, "npm run check", [
        [("$ ", C["faint"]), ("biome check --write --error-on-warnings .", C["muted"])],
        [("Checked 1,284 files in 1.9s. No fixes applied.", C["text2"])],
        [("$ ", C["faint"]), ("tsgo --noEmit", C["muted"])],
    ])
    composer(700, 134, "Run the qwen provider tests too once check passes",
             queue="Also add a regression test for the OpenCode path")


def thread_inspector():
    insp_title("Qwen signatures", "Running · bash", "09:41 · 14 min", kind="run")
    line(1084, 289.5, 1372, 289.5, C["line"])
    utabs(1084, 276, ["OVERVIEW", "TREE", "FILES"], 0, size=10, gap=22)

    section(318, "CONTEXT", "auto-compact on")
    tparts(1084, 348, [("62.4k", C["text"], None, 600)], size=20)
    text(1148, 348, "/ 200k tokens", C["muted"], size=12)
    rect(1084, 358, 288, 6, C["track"], 3)
    rect(1084, 358, 89, 6, ctx_fill(), 3)
    text(1084, 382, "31% of window", C["faint"], size=11)
    divider(400)

    section(426, "USAGE", "this session")
    for i, (k, v) in enumerate((("Input", "48.1k"), ("Output", "6.2k"), ("Cache read", "212k"))):
        kv(452 + i * 24, k, v, mono=True)
    kv(524, "Cost", "$0.41", C["text"], mono=True, bold=True)
    divider(544)

    section(570, "CHANGED FILES", "REVIEW", right_kind="bracket")
    for i, (f, a, r) in enumerate((("openai-completions.ts", "+3", "−1"), ("openai-completions.test.ts", "+24", None),
                                   ("CHANGELOG.md", "+1", None))):
        y = 596 + i * 24
        icon("file", 1084, y - 11, 13, C["faint"])
        text(1104, y, f, C["text2"], size=11.5, family=MONO)
        parts = [(a, C["green"])] + ([("  ", C["faint"]), (r, C["coral"])] if r else [])
        tparts(1372, y, parts, size=11, family=MONO, anchor="end")
    divider(664)

    section(690, "SESSION FILE")
    text(1084, 716, "…/--repos-pi--/2026-09-28T09-41.jsonl", C["text2"], size=11, family=MONO)
    icon("copy", 1356, 704, 14, C["muted"])
    text(1084, 738, "38 entries · 1 branch · not forked", C["faint"], size=11)
    divider(756)
    section(782, "EXTENSION STATUS")
    dot(1088, 802, 3, C["green"])
    text(1098, 806, "git-guard", C["text2"], size=11.5, family=MONO)
    text(1372, 806, "tree clean", C["faint"], size=11, anchor="end")
    text(1084, 832, "Inspecting Qwen signatures in ~/repos/pi", C["faint"], size=10)


def swatch_group(x, y, caption, items, w=60, gap=8):
    label(x, y - 8, caption, C["pageLabel"], size=9)
    for i, (name, hexv) in enumerate(items):
        sx = x + i * (w + gap)
        rect(sx, y, w, 34, hexv, 6, "#c4bcb5" if luminance(hexv) > 0.6 else "none")
        text(sx, y + 48, name, C["evening"], size=10)
        text(sx, y + 61, hexv, C["pageLabel"], size=9.5, family=MONO)
    return x + len(items) * (w + gap)


def colour_profile():
    D, L = DARK, LIGHT
    line(48, 994.5, 1392, 994.5, "#d3cbc4")
    label(48, 1022, "COLOUR PROFILE", C["pageLabel"], size=11)
    text(48, 1052, "Evening & Moonstone", C["evening"], size=24, family=SERIF, italic=True)
    text(380, 1044, "From pi.dev. Slate or warm-paper surfaces, one blue for interaction. The logo's three colours become",
         C["driftwood"], size=12.5)
    text(380, 1062, "signals: steel for you, amber for waiting, coral for removal and stop. Light mode darkens them to read on paper.",
         C["driftwood"], size=12.5)
    r1 = 1106
    x = swatch_group(48, r1, "EVENING · SURFACES", [("well", D["deep"]), ("canvas", D["canvas"]), ("panel", D["side"]),
                                                    ("bar", D["bar"]), ("raised", D["raised"]), ("line", D["line"])])
    x = swatch_group(x + 22, r1, "TEXT", [("text", D["text"]), ("secondary", D["text2"]), ("muted", D["muted"]),
                                          ("faint", D["faint"])])
    x = swatch_group(x + 22, r1, "ACCENT", [("accent", D["accent"]), ("selection", D["select"]), ("tidal", D["tidal"])])
    swatch_group(x + 22, r1, "SIGNALS", [("you", D["steel"]), ("waiting", D["amber"]), ("removed", D["coral"]),
                                        ("done", D["green"])])
    r2 = 1206
    x = swatch_group(48, r2, "MOONSTONE · SURFACES", [("well", L["deep"]), ("canvas", L["canvas"]), ("panel", L["side"]),
                                                      ("bar", L["bar"]), ("raised", L["raised"]), ("line", L["line"])])
    x = swatch_group(x + 22, r2, "TEXT", [("text", L["text"]), ("secondary", L["text2"]), ("muted", L["muted"]),
                                          ("faint", L["faint"])])
    x = swatch_group(x + 22, r2, "ACCENT", [("accent", L["accent"]), ("selection", L["select"]), ("focus", L["focus"])])
    swatch_group(x + 22, r2, "SIGNALS", [("you", L["steel"]), ("waiting", L["amber"]), ("removed", L["coral"]),
                                        ("done", L["green"])])
    r3 = 1306
    tx = 48
    for caption, ramp in (("THINKING LEVEL · EVENING", DARK_THINK), ("THINKING LEVEL · MOONSTONE", LIGHT_THINK)):
        label(tx, r3 - 8, caption, C["pageLabel"], size=9)
        for i, (name, col) in enumerate(ramp):
            rect(tx + i * 40, r3, 38, 34, col, 4 if i in (0, len(ramp) - 1) else 0)
            text(tx + i * 40, r3 + 48, name, C["evening"], size=10)
        text(tx, r3 + 61, "cool → warm, same order as the TUI", C["pageLabel"], size=9.5)
        tx += len(ramp) * 40 + 30
    x = swatch_group(tx, r3, "DIFF · EVENING", [("added", D["addBg"]), ("removed", D["delBg"])])
    swatch_group(x + 22, r3, "DIFF · MOONSTONE", [("added", L["addBg"]), ("removed", L["delBg"])])


def main(theme="dark"):
    set_theme(theme)
    reset()
    defs(W, H, "<title>pi desktop — thread study, not implemented</title>"
               "<desc>Layout and colour study for a pi desktop app built from Zed components and driven by pi --mode rpc. "
               "Sidebar of projects, sessions and app views, a thread with tool calls, diffs and a live terminal, a composer "
               "that steers or queues, and an inspector limited to data pi reports. Colours from pi.dev. All names, numbers "
               "and paths are sample data.</desc>")
    page(W, H)
    label(48, 35, "PI  /  DESIGN STUDY  01" + ("  ·  LIGHT" if theme == "light" else ""), C["pageLabel"], size=11)
    add(f'<text x="48" y="80" fill="{C["evening"]}" font-family="{SERIF}" font-size="34" font-style="italic">'
        f'A quiet place to watch pi <tspan fill="{DARK["accent"]}">work</tspan>.</text>')
    text(48, 106, "One thread per session. Tool calls, diffs and terminals stay in place; the inspector only shows what "
                  "pi reports.", C["driftwood"], size=14)
    label(1392, 74, "VISUAL PROPOSAL · SAMPLE DATA", C["pageLabel"], anchor="end", size=11)

    window_begin("Qwen signatures", session_sel=0, tabs=["THREAD", "CHANGES  3", "TREE", "CONTEXT"], tab_active=0)
    thread_content()
    thread_inspector()
    window_end()

    for x, n, head, body in (
        (48, "01", "Work stays in the thread", "Reads stay one line. Edits open as diffs, bash as a live terminal."),
        (512, "02", "Steer or follow up", "While pi runs, ⏎ steers and ⌥⏎ queues. The queue sits on the composer."),
        (976, "03", "Only what pi reports", "Context, cost and files come from RPC events, nothing estimated."),
    ):
        dot(x + 13, 925, 13, C["parchment"])
        text(x + 13, 929, n, C["evening"], size=10, weight=600, family=MONO, anchor="middle")
        text(x + 36, 931, head, C["evening"], size=15, weight=600)
        text(x, 959, body, C["driftwood"], size=12)

    colour_profile()
    text(48, 1408, "LAYOUT STUDY ONLY — built from Zed's gpui, editor, markdown and terminal crates, driving pi --mode rpc. "
                   "Fonts: IBM Plex Sans for UI, Commit Mono for code and labels, Plantin for titles. Other screens: "
                   "desktop-views-study.svg.", C["warm30"], size=11)
    add("</svg>")
    return dump()


if __name__ == "__main__":
    sys.stdout.write(main("light" if "--theme=light" in sys.argv or "light" in sys.argv[1:] else "dark"))
