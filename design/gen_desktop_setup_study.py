#!/usr/bin/env python3
"""pi desktop design study 07: setup on first launch.

Nothing is bundled. Every launch checks for Node.js and pi; when either is missing or does not fit, the app opens
setup instead of starting sessions. Setup runs the install commands itself, in a terminal inside the window, so
password prompts work and every step is visible. Each screen is drawn in Evening (dark) and Moonstone (light).
Writes SVG to stdout.
"""
import sys

from pi_study_common import (
    C, DARK, MONO, PAGE, SANS, SERIF, THEME, add, blend, button, defs, dot, dump, icon, label, line, page, pill, pimark,
    radio, rect, reset, ring, set_theme, spinner, text, tparts, tw, window_end,
)

ROW_PITCH = 910
W, H = 2848, 200 + 5 * ROW_PITCH + 30
X0, X1 = 376, 1356
STEPS = [("Node.js", "22.19 or newer"), ("pi", "0.87.1 for pi-desktop"), ("Open a project", "a folder to work in")]
# Sign-in was dropped from setup; only the faded row 04 still shows it.
STEPS_WITH_SIGN_IN = STEPS[:2] + [("Sign in", "a model provider")] + STEPS[2:]


def setup_window(states, details, current, status_right, status_left="setup", steps=STEPS):
    """A window with the setup steps on the left; states: todo, run, done, fail."""
    add(f'<rect x="58" y="150" width="1324" height="732" rx="13" fill="#1b1612" filter="url(#shadow)" '
        f'opacity="{0.28 if THEME["name"] == "dark" else 0.16}"/>')
    add('<g clip-path="url(#windowClip)">')
    rect(48, 130, 1344, 740, C["canvas"])
    rect(48, 130, 1344, 52, C["bar"])
    rect(48, 182, 288, 664, C["side"])
    rect(48, 846, 1344, 24, C["status"])
    edge = C["deep"] if THEME["name"] == "dark" else C["lineStrong"]
    line(48, 181.5, 1392, 181.5, edge)
    line(336.5, 182, 336.5, 846, edge)
    line(48, 846.5, 1392, 846.5, edge)
    for cx, col in ((70, "#ff6058"), (90, "#febc2e"), (110, "#29c841")):
        dot(cx, 156, 6, col)
    pimark(141, 147, 18)
    text(168, 161, "pi", C["text"], size=14, weight=600)
    icon("package", 359, 147, 18, C["muted"])
    text(386, 161, "Set up pi-desktop", C["text"], size=14, weight=600)

    text(72, 232, "Welcome to pi", C["text"], size=24, family=SERIF, italic=True)
    text(72, 256, "A few things before your first session.", C["faint"], size=11.5)
    y = 292
    for i, ((name, sub), state, detail) in enumerate(zip(steps, states, details)):
        if i == current:
            rect(60, y - 10, 264, 54, C["select"], 6)
            rect(60, y - 4, 2, 42, C["accent"], 1)
        if i < len(steps) - 1:
            line(84.5, y + 23, 84.5, y + 56, C["line"], 1.4)
        cy = y + 11
        if state == "done":
            dot(84, cy, 11, C["green"])
            icon("check", 77, cy - 7, 14, C["onAccent"] if THEME["name"] == "dark" else "#ffffff")
        elif state == "fail":
            dot(84, cy, 11, C["coral"])
            icon("x", 78, cy - 6, 12, C["onAccent"] if THEME["name"] == "dark" else "#ffffff")
        elif state == "run":
            spinner(84, cy, 9, C["accent"])
        else:
            ring(84, cy, 10, C["accent"] if i == current else C["lineStrong"], 1.4, C["side"])
            text(84, cy + 4, str(i + 1), C["accent"] if i == current else C["faint"], size=10.5, weight=600,
                 family=MONO, anchor="middle")
        text(108, y + 11, name, C["text"] if i == current else C["text2"], size=13, weight=600 if i == current else 400)
        text(108, y + 29, detail or sub, C["coral"] if state == "fail" else C["faint"], size=11)
        y += 66
    icon("info", 72, 787, 13, C["faint"])
    text(92, 798, "Checked on every launch. Setup only", C["faint"], size=11)
    text(92, 814, "opens when something is missing.", C["faint"], size=11)

    dot(65, 858, 2.5, C["amber"] if "fail" in states or "todo" in states[:2] else C["green"])
    text(75, 862, status_left, C["muted"], size=10, family=MONO)
    text(1374, 862, status_right, C["muted"], size=10, family=MONO, anchor="end")


def headline(title, lines_):
    text(X0, 240, title, C["text"], size=26, family=SERIF, italic=True)
    for i, s in enumerate(lines_):
        text(X0, 270 + i * 20, s, C["muted"], size=13)


def status_icon(cx, cy, kind):
    col = {"ok": C["green"], "bad": C["coral"], "warn": C["amber"]}[kind]
    ring(cx, cy, 10, blend(col, C["canvas"], 0.5), 1.4, blend(col, C["canvas"], 0.12))
    icon({"ok": "check", "bad": "x", "warn": "warning"}[kind], cx - 6, cy - 6, 12, col)


def check_card(y, kind, title, detail, badge, h=64):
    rect(X0, y, X1 - X0, h, C["side"], 8, C["card"])
    status_icon(X0 + 26, y + h / 2, kind)
    text(X0 + 50, y + 27, title, C["text"], size=13.5, weight=600)
    text(X0 + 50, y + 47, detail, C["muted"], size=12)
    col = {"ok": C["green"], "bad": C["amber"], "warn": C["amber"]}[kind]
    pill(X1 - 18, y + h / 2 - 9, badge, col, blend(col, C["side"], 0.12), size=10, h=18, anchor="end",
         border=blend(col, C["side"], 0.4))


def option_card(y, on, title, detail, command=None, h=58):
    rect(X0, y, X1 - X0, h, C["select"] if on else "none", 7, C["focus"] if on else C["line"])
    radio(X0 + 22, y + 20, on)
    text(X0 + 42, y + 24, title, C["text"], size=13, weight=600 if on else 400)
    text(X0 + 42, y + 43, detail, C["muted"], size=11.5)
    if command:
        w = tw(command, 11, True) + 24
        rect(X1 - 16 - w, y + 16, w, 26, C["deep"], 5, C["line"])
        text(X1 - 16 - w + 12, y + 33, command, C["pl"], size=11, family=MONO)


def terminal(y, h, command, lines_, state, meta):
    """The setup terminal: the exact command, its live output, and how it ended."""
    rect(X0, y, X1 - X0, h, C["deep"], 8, C["line"])
    add(f'<path d="M{X0} {y + 8}a8 8 0 0 1 8-8H{X1 - 8}a8 8 0 0 1 8 8V{y + 32}H{X0}Z" fill="{C["side"]}"/>')
    line(X0, y + 32.5, X1, y + 32.5, C["line"])
    icon("terminal", X0 + 14, y + 9, 14, C["muted"])
    label(X0 + 36, y + 21, "TERMINAL", C["muted"], size=9.5)
    text(X0 + 112, y + 21, command, C["text2"], size=11.5, family=MONO)
    if state == "run":
        wb = button(X1 - 14, y + 5, "Stop", "danger", h=22, size=10.5, anchor="end")
        mx = X1 - 14 - wb - 12
        text(mx, y + 20, meta, C["muted"], size=10.5, family=MONO, anchor="end")
        spinner(mx - tw(meta, 10.5, True) - 10, y + 16, 5, C["accent"])
    else:
        col = C["green"] if state == "ok" else C["coral"]
        icon("check" if state == "ok" else "x", X1 - 16 - tw(meta, 10.5, True) - 18, y + 9, 13, col)
        text(X1 - 14, y + 20, meta, col, size=10.5, family=MONO, anchor="end")
    for i, parts in enumerate(lines_):
        tparts(X0 + 16, y + 56 + i * 19, parts, size=11.5, family=MONO, pre=True)
    if state == "run":
        rect(X0 + 16, y + 56 + len(lines_) * 19 - 11, 7, 14, C["accent"], 1)


def cmd(s):
    return [("$ ", C["muted"]), (s, C["pl"])]


def brew(s, rest=None):
    parts = [("==> ", C["accent"]), (s, C["pl"])]
    if rest:
        parts.append((rest, C["faint"]))
    return parts


# ---------------------------------------------------------------- 01 The launch check

def r_check():
    setup_window(["todo", "todo", "todo"], ["20.11.1 is too old", "not installed", None], None,
                 "Node.js too old · pi missing · sessions not started")
    headline("pi-desktop needs Node.js and pi", ["Sessions run on this Mac with Node.js and the pi version this app "
                                                 "is built for. Two are missing,", "so setup installs them. Nothing is "
                                                 "changed until you start it."])
    check_card(318, "bad", "Node.js 22.19 or newer",
               "Found Node 20.11.1 at /usr/local/bin/node, which is too old for pi.", "install")
    check_card(394, "bad", "pi 0.87.1 for pi-desktop",
               "Not installed yet. Your own pi 0.86.0 on PATH is left as it is.", "install")
    check_card(470, "ok", "Your pi settings",
               "~/.pi/agent · signed in to Anthropic and OpenAI · 24 sessions", "found")
    w = button(X0, 566, "Set up", "primary", h=32, size=12.5, w=120)
    button(X0 + w + 10, 566, "Check again", "ghost", h=32, size=12.5, w=120, ic="refresh")
    text(X0, 632, "The check runs node --version and a handshake with pi (get_backend_info), in about a tenth of a",
         C["faint"], size=11.5)
    text(X0, 650, "second. Until it passes, no session starts.", C["faint"], size=11.5)


# ---------------------------------------------------------------- 02 Installing Node.js

def r_node():
    setup_window(["run", "todo", "todo"], ["installing with Homebrew…", "waits for Node.js", None], 0,
                 "installing Node.js · brew install node")
    headline("Install Node.js", ["pi runs on Node.js 22.19 or newer. Pick how to install it. The command runs below, "
                                 "where you can", "answer any prompt, such as your password."])
    option_card(318, True, "With Homebrew", "Homebrew 4.6.2 is installed; brew upgrade keeps Node current.",
                "brew install node")
    option_card(384, False, "Download from nodejs.org", "Node 22.19.0 for pi-desktop only, checked against "
                "nodejs.org's SHA-256 list. No password.", "nodejs.org")
    option_card(450, False, "I'll install it myself", "Any Node 22.19+ on PATH works. Press Check again when it's there.")
    terminal(526, 250, "brew install node", [
        cmd("brew install node"),
        brew("Fetching downloads for: ", "node"),
        brew("Installing dependencies for node: ", "brotli, c-ares, icu4c@77, libnghttp2, libuv, llhttp"),
        brew("Pouring ", "icu4c@77--77.1.arm64_sequoia.bottle.tar.gz"),
        brew("Pouring ", "libuv--1.51.0.arm64_sequoia.bottle.tar.gz"),
        brew("Pouring ", "node--24.9.0.arm64_sequoia.bottle.tar.gz"),
        [("   ", C["pl"]), ("linking /opt/homebrew/bin/node", C["faint"])],
    ], "run", "running · 38s")
    button(X1, 796, "Continue", "disabled", h=32, size=12.5, w=120, anchor="end")
    button(X0, 796, "Back", "ghost", h=32, size=12.5, w=90)


# ---------------------------------------------------------------- 03 Installing pi

def r_pi():
    setup_window(["done", "done", "todo"], ["24.9.0 · Homebrew", "0.87.1 · ready", None], 1,
                 "pi 0.87.1 · protocol 1 · node 24.9.0", status_left="setup · ready")
    headline("Install pi 0.87.1", ["pi-desktop is built for pi 0.87.1. npm installs it into the app's own folder, "
                                   "with the exact packages", "and checksums of the lock file the app ships with."])
    rect(X0, 318, X1 - X0, 100, C["side"], 8, C["card"])
    for i, (k, v, mono) in enumerate((("Command", "npm ci --omit=dev", True),
                                      ("Folder", "~/Library/Application Support/pi-desktop/backend", True),
                                      ("Your own pi", "0.86.0 at /opt/homebrew/bin/pi, left as it is", False))):
        yy = 344 + i * 26
        text(X0 + 18, yy, k, C["muted"], size=12)
        text(X0 + 130, yy, v, C["text2"], size=11.5 if mono else 12, family=MONO if mono else SANS)
    terminal(434, 176, "npm ci --omit=dev", [
        cmd("npm ci --omit=dev"),
        [("added 212 packages, and audited 213 packages in 14s", C["pl"])],
        [("found ", C["pl"]), ("0", C["green"]), (" vulnerabilities", C["pl"])],
    ], "ok", "exit 0 · 14s")
    rect(X0, 626, X1 - X0, 44, blend(C["green"], C["canvas"], 0.08), 8, blend(C["green"], C["canvas"], 0.35))
    icon("check", X0 + 16, 640, 15, C["green"])
    text(X0 + 40, 653, "pi is ready", C["text"], size=12.5, weight=600)
    text(X0 + 130, 653, "pi-desktop-backend 0.1.0 · pi 0.87.1 · protocol 1 · Node 24.9.0", C["muted"], size=11.5,
         family=MONO)
    button(X1, 796, "Continue", "primary", h=32, size=12.5, w=120, anchor="end")
    button(X0, 796, "Back", "ghost", h=32, size=12.5, w=90)


# ---------------------------------------------------------------- 04 Sign in

def r_sign_in():
    setup_window(["done", "done", "done", "todo"], ["24.9.0 · Homebrew", "0.87.1 · ready", "2 providers", None], 2,
                 "pi 0.87.1 · 2 providers signed in", status_left="setup · ready", steps=STEPS_WITH_SIGN_IN)
    headline("Sign in to a model provider", ["Found your pi settings in ~/.pi/agent. pi-desktop uses the same sign-ins "
                                             "as pi in your terminal."])
    providers = [("Anthropic", "Claude subscription", True), ("OpenAI", "API key", True),
                 ("Google", "Gemini", False), ("OpenRouter", "API key", False), ("GitHub Copilot", "subscription", False)]
    rect(X0, 312, X1 - X0, 44 * len(providers), C["side"], 8, C["card"])
    for i, (name, method, signed) in enumerate(providers):
        y = 312 + i * 44
        if i:
            line(X0 + 16, y + 0.5, X1 - 16, y + 0.5, C["line"])
        icon("key", X0 + 18, y + 14, 15, C["green"] if signed else C["faint"])
        text(X0 + 46, y + 27, name, C["text"], size=13, weight=500)
        text(X0 + 190, y + 27, method, C["muted"], size=12)
        if signed:
            icon("check", X1 - 18 - tw("signed in", 11.5) * 1.1 - 20, y + 15, 13, C["green"])
            text(X1 - 18, y + 27, "signed in", C["green"], size=11.5, anchor="end")
        else:
            button(X1 - 14, y + 10, "Sign in", "ghost", h=24, size=11, anchor="end")
    text(X0, 312 + 44 * len(providers) + 30, "More providers…", C["accent"], size=12)
    icon("info", X0, 312 + 44 * len(providers) + 52, 13, C["faint"])
    text(X0 + 20, 312 + 44 * len(providers) + 63, "Sign in runs pi's /login in the setup terminal, since the backend "
         "cannot sign in on its own yet.", C["faint"], size=11.5)
    button(X1, 796, "Continue", "primary", h=32, size=12.5, w=120, anchor="end")
    button(X1 - 130, 796, "Skip for now", "ghost", h=32, size=12.5, w=120, anchor="end")
    button(X0, 796, "Back", "ghost", h=32, size=12.5, w=90)


# ---------------------------------------------------------------- 05 When a command fails

def r_fail():
    setup_window(["done", "fail", "todo"], ["24.9.0 · Homebrew", "npm could not download", None], 1,
                 "pi missing · sessions not started")
    headline("pi could not be installed", ["npm stopped before anything changed. Fix what it names, then try again."])
    tone = C["coral"]
    rect(X0, 300, X1 - X0, 58, blend(tone, C["canvas"], 0.08), 8, blend(tone, C["canvas"], 0.4))
    icon("warning", X0 + 16, 313, 16, tone)
    text(X0 + 42, 324, "registry.npmjs.org refused the download (403 Forbidden)", C["text"], size=13, weight=600)
    text(X0 + 42, 344, "A proxy or firewall is blocking npm. Nothing was installed, so nothing is left half done.",
         C["text2"], size=12)
    terminal(374, 196, "npm ci --omit=dev", [
        cmd("npm ci --omit=dev"),
        [("npm error ", C["coral"]), ("code E403", C["pl"])],
        [("npm error ", C["coral"]), ("403 Forbidden - GET https://registry.npmjs.org/@earendil-works%2fpi-coding-agent", C["pl"])],
        [("npm error ", C["coral"]), ("A complete log of this run can be found in:", C["pl"])],
        [("npm error ", C["coral"]), ("    ~/.npm/_logs/2026-09-30T10_14_02_411Z-debug-0.log", C["faint"])],
    ], "fail", "exit 1 · 3s")
    x = X0
    for s_, kind, ic, w in (("Try again", "primary", "refresh", 116), ("Copy command", "ghost", "copy", 146),
                            ("Open in Terminal", "ghost", "terminal", 164), ("Show log", "ghost", "file", 108)):
        x += button(x, 586, s_, kind, h=32, size=12.5, ic=ic, w=w) + 10
    icon("info", X0, 648, 13, C["faint"])
    text(X0 + 20, 659, "If Node.js or pi stop fitting later, for example after Node is removed, pi-desktop opens here at "
         "that step", C["faint"], size=11.5)
    text(X0 + 20, 677, "instead of starting sessions.", C["faint"], size=11.5)


# Status: "open" still needs a decision, "decided" records the user's answer (2026-09-30).
ROWS = [
    (r_check, "The launch check", "nothing bundled, nothing assumed",
     "Every launch checks Node.js and pi before any session starts. When one is missing or does not fit, setup opens "
     "and says what it found; existing pi settings are reused.",
     "decided", "check on every launch; no bundle; missing or unfit Node.js or pi opens setup instead of sessions."),
    (r_node, "Installing Node.js", "the app runs the command, you see it",
     "Setup offers the package manager it finds (Homebrew here, winget on Windows, apt or dnf on Linux when their Node is "
     "22.19+) and a checked download from nodejs.org. Afterwards the app looks in the install folders, not the old PATH.",
     "decided", "both: the package manager and a nodejs.org download."),
    (r_pi, "Installing pi", "the version the app is built for",
     "npm installs pi 0.87.1 and the backend into the app's folder from the lock file the app ships with. A pi you "
     "installed yourself is not touched; the handshake confirms the result.",
     "decided", "for the app only; a global npm install -g can need root."),
    (r_sign_in, "Sign in", "reuse what pi already knows",
     "Sign-ins come from ~/.pi/agent, shared with pi in the terminal. A provider without one signs in through pi's "
     "/login in the setup terminal.",
     "dropped", "left to the first session; Resources already signs in with pi in a terminal."),
    (r_fail, "When a command fails", "say what broke, keep what worked",
     "The step turns red, the terminal keeps the output, and the banner names the likely cause. Try again, copy the "
     "command, or run it in the system Terminal.",
     "decided", "the setup terminal: the drawer's terminal, on macOS, Linux and Windows (ConPTY)."),
]

STATUS = {
    "open": ("DECIDE", PAGE["terracotta"]),
    "decided": ("DECIDED", "#2e8a55"),
    "dropped": ("DROPPED", PAGE["pageLabel"]),
}


def main():
    reset()
    defs(W, H, "<title>pi desktop — setup on first launch, visual proposal, not implemented</title>"
               "<desc>Nothing is bundled: every launch checks Node.js and pi, and setup installs what is missing by "
               "running the commands in a terminal inside the window. Each screen in Evening (dark) and Moonstone "
               "(light). Versions, paths and output are sample data.</desc>")
    page(W, H)
    label(48, 42, "PI  /  DESIGN STUDY  07", C["pageLabel"], size=11)
    add(f'<text x="48" y="96" fill="{C["evening"]}" font-family="{SERIF}" font-size="40" font-style="italic">'
        f'Set up once, <tspan fill="{DARK["accent"]}">checked every launch</tspan>.</text>')
    text(48, 128, "No bundled runtime: pi-desktop finds Node.js and pi, and installs what is missing with the commands "
                  "you would run yourself, in plain view.", C["driftwood"], size=14)
    label(2800, 92, "VISUAL PROPOSAL · SAMPLE DATA · FOLLOWS STUDY 06", C["pageLabel"], anchor="end", size=11)

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
            opacity = ' opacity="0.45"' if status == "dropped" else ""
            add(f'<g transform="translate({dx} {y + 80 - 130})"{opacity}>')
            fn()
            window_end()
            add("</g>")
        set_theme("dark")

    text(48, H - 20, "LAYOUT STUDY ONLY — follows studies 01–06 and their colour profile. Versions, paths, command "
                     "output and providers are sample data.", C["warm30"], size=11)
    add("</svg>")
    return dump()


if __name__ == "__main__":
    sys.stdout.write(main())
