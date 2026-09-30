"""Shared palette, drawing helpers and window chrome for the pi desktop design studies.

Every window is drawn in the same local coordinates (window at 48,130, 1344x740):
sidebar 48-256, middle 256-1064, inspector 1064-1392, content 182-846, status bar 846-870.
Multi-screen studies wrap each window in a translated group.

Colours come from the active theme. Call set_theme("dark" | "light") before drawing a window;
C, THINK and THINK_COLOR are updated in place so imported names stay valid.
"""
import html

SANS = "IBM Plex Sans, Zed Plex Sans, Adwaita Sans, Helvetica Neue, sans-serif"
MONO = "Commit Mono, Zed Plex Mono, SF Mono, Menlo, DejaVu Sans Mono, monospace"
SERIF = "Plantin MT Pro, Georgia, DejaVu Serif, serif"


def blend(fg, bg, a):
    f = [int(fg[i:i + 2], 16) for i in (1, 3, 5)]
    b = [int(bg[i:i + 2], 16) for i in (1, 3, 5)]
    return "#" + "".join(f"{round(a * x + (1 - a) * y):02x}" for x, y in zip(f, b))


# Page colours are shared by both themes: the studies sit on pi.dev's moonstone paper.
PAGE = {
    "parchment": "#dacbc2", "moonstone": "#ebe7e4", "warmWhite": "#f3f2f0", "coolWhite": "#f0f2f3",
    "evening": "#252f3d", "driftwood": "#5c5752", "warm30": "#8b847d", "lightLine": "#cbc5bf",
    "terracotta": "#b86b52", "pageLabel": "#7a736d",
}

# Evening: dark surfaces from pi.dev's dark theme, signals from the logo.
DARK = {
    "deep": "#0d1116", "canvas": "#161d27", "side": "#1a212b", "bar": "#1f2630",
    "raised": "#252f3d", "line": "#2f3640", "lineStrong": "#424954", "status": "#131922",
    "hover": "#222a35", "chip": "#29313c", "chipLine": "#3a434f", "card": "#2a323e",
    "segBg": "#161d27", "segOn": "#34404f", "toggleOff": "#2c3440", "knobOff": "#8a909a",
    "thead": "#1b2230", "rowAlt": "#18202a", "track": "#252d38", "userBg": "#1d2531",
    "winBorder": "#3a414c", "overlay": "#05080c", "overlayOpacity": "0.55",
    "text": "#ebe7e4", "text2": "#d5d8db", "muted": "#9fa4ab", "faint": "#737981",
    "accent": "#6a9fcc", "select": "#273748", "tidal": "#4b607c", "focus": "#3a5068",
    "onAccent": "#0d1116", "onLevel": "#0d1116",
    "steel": "#4d9abf", "coral": "#f09082", "amber": "#f1be58", "green": "#5db87a",
    "sage": "#a3a473", "red": "#e8704f",
    "addBg": "#182824", "delBg": "#2d2325",
    "code": "#9cc2e0", "hunk": "#8fa6c2", "kw": "#8fb6dc", "str": "#e1b06e", "pl": "#b9bec5",
}
DARK_THINK = [("off", "#5a616b"), ("min", "#4b607c"), ("low", "#4d9abf"), ("med", "#a3a473"),
              ("high", "#f1be58"), ("xhigh", "#f09082"), ("max", "#e8704f")]

# Moonstone: light surfaces from pi.dev's light theme. Signals are darkened to read on white.
_L_CANVAS, _L_DEEP = "#faf9f7", "#eef0f2"
LIGHT = {
    "deep": _L_DEEP, "canvas": _L_CANVAS, "side": "#f2efeb", "bar": "#ebe7e4",
    "raised": "#e4ded8", "line": "#e3ddd7", "lineStrong": "#cbc3bb", "status": "#e6e1dc",
    "hover": "#e8e3de", "chip": "#ffffff", "chipLine": "#d3ccc5", "card": "#e0d9d2",
    "segBg": "#ebe7e3", "segOn": "#ffffff", "toggleOff": "#d9d3cd", "knobOff": "#ffffff",
    "thead": "#f2efeb", "rowAlt": "#f5f3f0", "track": "#e3ddd7", "userBg": blend("#4b607c", _L_CANVAS, 0.07),
    "winBorder": "#c9c1ba", "overlay": "#3a332d", "overlayOpacity": "0.28",
    "text": "#252f3d", "text2": "#3a4453", "muted": "#5c5752", "faint": "#8b847d",
    "accent": "#4b607c", "select": blend("#4b607c", _L_CANVAS, 0.11), "tidal": "#4b607c",
    "focus": blend("#4b607c", _L_CANVAS, 0.4),
    "onAccent": "#ffffff", "onLevel": "#ffffff",
    "steel": "#2f7fa8", "coral": "#c05a45", "amber": "#b97a14", "green": "#2e8a55",
    "sage": "#6f7040", "red": "#c0442a",
    "addBg": blend("#2e8a55", _L_DEEP, 0.13), "delBg": blend("#c05a45", _L_DEEP, 0.13),
    "code": "#2f6f9e", "hunk": "#5a6f8c", "kw": "#2f5f9e", "str": "#9a5b17", "pl": "#3f4854",
}
LIGHT_THINK = [("off", "#9aa0a8"), ("min", "#4b607c"), ("low", "#2f7fa8"), ("med", "#7d7e45"),
               ("high", "#c28a1c"), ("xhigh", "#c96a58"), ("max", "#c0442a")]

C = {}
THINK = []
THINK_COLOR = {}
THEME = {"name": "dark"}


def set_theme(name):
    C.clear()
    C.update(PAGE)
    C.update(DARK if name == "dark" else LIGHT)
    THINK[:] = DARK_THINK if name == "dark" else LIGHT_THINK
    THINK_COLOR.clear()
    THINK_COLOR.update(dict(THINK))
    THEME["name"] = name


set_theme("dark")

out = []


def reset():
    out.clear()


def dump():
    return "\n".join(out) + "\n"


def e(s):
    return html.escape(s, quote=False)


def add(s):
    out.append(s)


def luminance(hexv):
    r, g, b = (int(hexv[i:i + 2], 16) / 255 for i in (1, 3, 5))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def tw(s, size, mono=False, ls=0.0):
    """Rough text width, good enough for placing chips and carets."""
    return len(s) * (size * (0.6 if mono else 0.5) + ls)


def rect(x, y, w, h, fill, rx=0, stroke=None, extra=""):
    st = f' stroke="{stroke}"' if stroke else ""
    add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{fill}"{st}{extra}/>')


def line(x1, y1, x2, y2, color, width=1, extra=""):
    add(f'<path d="M{x1} {y1}L{x2} {y2}" stroke="{color}" stroke-width="{width}"{extra}/>')


def text(x, y, s, fill, size=13, weight=400, family=SANS, anchor="start", ls=None, italic=False, pre=False):
    attrs = (f'x="{x}" y="{y}" fill="{fill}" font-family="{family}" font-size="{size}" '
             f'font-weight="{weight}" text-anchor="{anchor}"')
    if ls is not None:
        attrs += f' letter-spacing="{ls}"'
    if italic:
        attrs += ' font-style="italic"'
    if pre:
        attrs += ' xml:space="preserve" style="white-space:pre"'
    add(f"<text {attrs}>{e(s)}</text>")


def spans(parts):
    """parts: (text, colour[, family[, weight]]) tuples -> tspans."""
    res = []
    for p in parts:
        t, col = p[0], p[1]
        fam = f' font-family="{p[2]}"' if len(p) > 2 and p[2] else ""
        wt = f' font-weight="{p[3]}"' if len(p) > 3 else ""
        res.append(f'<tspan fill="{col}"{fam}{wt}>{e(t)}</tspan>')
    return "".join(res)


def tparts(x, y, parts, size=13, family=SANS, anchor="start", pre=False):
    extra = ' xml:space="preserve" style="white-space:pre"' if pre else ""
    add(f'<text x="{x}" y="{y}" font-family="{family}" font-size="{size}" text-anchor="{anchor}"{extra}>{spans(parts)}</text>')


def label(x, y, s, fill=None, anchor="start", size=10):
    text(x, y, s, fill or C["faint"], size=size, weight=500, family=MONO, anchor=anchor, ls=1.2)


def icon(name, x, y, size, color):
    s = size / 24
    add(f'<g transform="translate({x} {y}) scale({s:.4f})" fill="none" stroke="{color}" stroke-width="{1.3 / s:.2f}" '
        f'stroke-linecap="round" stroke-linejoin="round"><use xlink:href="#{name}"/></g>')


def dot(cx, cy, r, fill):
    add(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{fill}"/>')


def ring(cx, cy, r, stroke, width=1.4, fill="none"):
    add(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{fill}" stroke="{stroke}" stroke-width="{width}"/>')


def spinner(cx, cy, r, color):
    ring(cx, cy, r, C["lineStrong"], 1.6)
    add(f'<path d="M{cx} {cy - r}A{r} {r} 0 0 1 {cx + r} {cy}" fill="none" stroke="{color}" stroke-width="1.8" stroke-linecap="round"/>')


def state_mark(cx, cy, state):
    """run = spinner, wait = amber dot, idle = hollow ring."""
    if state == "run":
        spinner(cx, cy, 5, C["accent"])
    elif state == "wait":
        dot(cx, cy, 3.5, C["amber"])
    elif state == "idle":
        ring(cx, cy, 3.5, C["faint"])


def pimark(x, y, size):
    s = size / 469.43
    add(f'<g transform="translate({x} {y}) scale({s:.5f})"><use xlink:href="#pimark"/></g>')


def popover(x, y, w, h):
    add(f'<rect x="{x + 4}" y="{y + 8}" width="{w - 8}" height="{h}" rx="10" fill="#000" '
        f'opacity="{0.45 if THEME["name"] == "dark" else 0.14}" filter="url(#popShadow)"/>')
    rect(x, y, w, h, C["bar"] if THEME["name"] == "dark" else C["chip"], 10, C["chipLine"])


def dim_window():
    rect(48, 130, 1344, 740, C["overlay"], 0, None, f' opacity="{C["overlayOpacity"]}"')


# ---------------------------------------------------------------- controls

def pill(x, y, s, fg, bg, size=10, mono=True, h=16, pad=6, rx=4, anchor="start", border=None):
    w = tw(s, size, mono) + 2 * pad
    x0 = x - w if anchor == "end" else x
    rect(x0, y, round(w, 1), h, bg, rx, border)
    text(x0 + pad, y + h / 2 + size * 0.36, s, fg, size=size, family=MONO if mono else SANS)
    return w


def button(x, y, s, kind="ghost", h=24, size=11, w=None, ic=None, anchor="start"):
    if kind == "bracket":
        s2 = f"[ {s} ]"
        text(x, y + h / 2 + 4, s2, C["accent"], size=10, family=MONO, ls=0.8, anchor=anchor)
        return tw(s2, 10, True, 0.8)
    pad = 12
    iw = 18 if ic else 0
    w = w or (tw(s, size) + 2 * pad + iw)
    x0 = x - w if anchor == "end" else x
    fg, bg, bd, wt = {
        "primary": (C["onAccent"], C["accent"], None, 600),
        "ghost": (C["text2"], C["chip"], C["chipLine"], 400),
        "danger": (C["coral"], blend(C["coral"], C["side"], 0.08), blend(C["coral"], C["side"], 0.35), 400),
        "amber": (C["onLevel"], C["amber"], None, 600),
        "disabled": (C["faint"], C["hover"], C["line"], 400),
    }[kind]
    rect(x0, y, round(w, 1), h, bg, 5, bd)
    if ic:
        icon(ic, x0 + 9, y + h / 2 - 7, 14, fg)
    text(x0 + w / 2 + iw / 2, y + h / 2 + size * 0.36, s, fg, size=size, weight=wt, anchor="middle")
    return w


def toggle(x, y, on):
    rect(x, y, 28, 16, C["accent"] if on else C["toggleOff"], 8, None if on else C["chipLine"])
    dot(x + (20 if on else 8), y + 8, 6, C["onAccent"] if on and THEME["name"] == "light" else
        (C["text"] if on else C["knobOff"]))


def checkbox(x, y, on=True):
    rect(x, y, 14, 14, C["accent"] if on else C["canvas"], 3, None if on else C["chipLine"])
    if on:
        icon("check", x + 1, y + 1, 12, C["onAccent"])


def radio(cx, cy, on):
    ring(cx, cy, 6.5, C["accent"] if on else C["lineStrong"], 1.4, C["canvas"])
    if on:
        dot(cx, cy, 3.5, C["accent"])


def seg(x, y, items, active, h=24, size=11, mono=False, pad=12, colors=None):
    """Segmented control. Returns total width."""
    widths = [tw(s, size, mono) + 2 * pad for s in items]
    total = sum(widths) + 4
    rect(x, y, round(total, 1), h, C["segBg"], 5, C["chipLine"])
    cx = x + 2
    for i, (s, w) in enumerate(zip(items, widths)):
        if i == active:
            rect(round(cx, 1), y + 2, round(w, 1), h - 4, C["segOn"], 4,
                 C["chipLine"] if THEME["name"] == "light" else None)
        col = C["text"] if i == active else C["muted"]
        if colors and colors[i]:
            col = colors[i] if i == active else blend(colors[i], C["segBg"], 0.55)
        text(round(cx + w / 2, 1), y + h / 2 + size * 0.36, s, col, size=size, weight=500 if i == active else 400,
             family=MONO if mono else SANS, anchor="middle")
        cx += w
    return total


def utabs(x, y, items, active, size=10.5, gap=26):
    """Underline tabs in uppercase mono. y is the text baseline."""
    cx = x
    for i, s in enumerate(items):
        w = tw(s, size, True, 1.2)
        text(cx, y, s, C["text"] if i == active else C["faint"], size=size, weight=500, family=MONO, ls=1.2)
        if i == active:
            rect(cx, y + 13, round(w - 1.2, 1), 2, C["accent"], 1)
        cx += w + gap
    return cx


def tile(x, y, w, h, title, value, sub, value_color=None):
    rect(x, y, w, h, C["side"], 8, C["card"])
    label(x + 14, y + 22, title, C["muted"])
    text(x + 14, y + 50, value, value_color or C["text"], size=20, weight=600)
    text(x + 14, y + 68, sub, C["faint"], size=11)


def input_box(x, y, w, value, mono=True, placeholder=False, h=30):
    rect(x, y, w, h, C["canvas"], 6, C["chipLine"])
    text(x + 12, y + h / 2 + 4.5, value, C["faint"] if placeholder else C["text"], size=12 if mono else 12.5,
         family=MONO if mono else SANS)


# ---------------------------------------------------------------- inspector helpers

IX0, IX1 = 1084, 1372


def insp_title(title, status, right=None, kind="idle", serif=True, size=22):
    if serif:
        text(IX0, 222, title, C["text"], size=size, family=SERIF, italic=True)
    else:
        text(IX0, 222, title, C["text"], size=size - 4, family=MONO, weight=500)
    if kind == "run":
        spinner(IX0 + 6, 243, 4.5, C["accent"])
    elif kind in ("green", "amber", "coral", "accent", "faint"):
        dot(IX0 + 6, 243, 3.5, C[kind])
    else:
        ring(IX0 + 6, 243, 3.5, C["faint"])
    text(IX0 + 18, 247, status, C["text2"], size=12)
    if right:
        text(IX1, 247, right, C["faint"], size=10.5, family=MONO, anchor="end")


def section(y, title, right=None, right_kind="text"):
    label(IX0, y, title)
    if right:
        if right_kind == "bracket":
            text(IX1, y, f"[ {right} ]", C["accent"], size=10, family=MONO, anchor="end", ls=0.8)
        else:
            text(IX1, y, right, C["faint"], size=10.5, anchor="end")


def divider(y, x0=IX0, x1=IX1):
    line(x0, y + 0.5, x1, y + 0.5, C["line"])


def kv(y, k, v, vcolor=None, mono=False, bold=False):
    text(IX0, y, k, C["muted"], size=12)
    text(IX1, y, v, vcolor or C["text2"], size=12 if not bold else 13, weight=600 if bold else 400,
         family=MONO if mono else SANS, anchor="end")


def quote(x, y, w, lines, size=12, color=None):
    h = 16 + len(lines) * (size + 6)
    rect(x, y, w, h, C["canvas"], 6, C["line"])
    rect(x, y + 6, 2, h - 12, C["tidal"], 1)
    for i, s in enumerate(lines):
        text(x + 12, y + 20 + i * (size + 6), s, color or C["text2"], size=size)
    return h


def note(x, y, w, lines, ic="info", color=None):
    color = color or C["faint"]
    icon(ic, x, y - 11, 13, color)
    for i, s in enumerate(lines):
        text(x + 20, y + i * 16, s, color, size=11)


def banner(x, y, w, h, title, body, ic="warning", tone="amber"):
    col = C[tone]
    rect(x, y, w, h, blend(col, C["canvas"], 0.08), 8, blend(col, C["canvas"], 0.4))
    icon(ic, x + 14, y + 10, 16, col)
    text(x + 40, y + 21, title, C["text"], size=13, weight=600)
    text(x + 40, y + 42, body, C["text2"], size=12)


# ---------------------------------------------------------------- defs

ICONS = {
    "message": '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2Z"/>',
    "folder": '<path d="M2 7V5a2 2 0 0 1 2-2h5l3 4h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2Z"/>',
    "plus": '<path d="M12 4v16M4 12h16"/>',
    "search": '<circle cx="10" cy="10" r="6"/><path d="m15 15 6 6"/>',
    "more": '<circle cx="4" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="20" cy="12" r="1"/>',
    "chevron": '<path d="m7 9 5 5 5-5"/>',
    "chevronR": '<path d="m9 6 6 6-6 6"/>',
    "chevronU": '<path d="m7 15 5-5 5 5"/>',
    "branch": '<path d="M6 3v12"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>',
    "compact": '<path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7"/>',
    "inspector": '<rect x="2" y="3" width="20" height="18" rx="2"/><path d="M15 3v18"/>',
    "file": '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6"/>',
    "edit": '<path d="m4 16-1 5 5-1L21 7l-4-4ZM14 6l4 4"/>',
    "terminal": '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="m6 10 3 2-3 2M12 15h6"/>',
    "check": '<path d="m5 12 4 4L19 6"/>',
    "sparkle": '<path d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9Z"/><path d="M19 17v4M17 19h4"/>',
    "clip": '<path d="m21 12-8.5 8.5a5 5 0 0 1-7-7L14 5a3.5 3.5 0 0 1 5 5l-8.5 8.5a2 2 0 0 1-3-3L15 8"/>',
    "slash": '<rect x="3" y="3" width="18" height="18" rx="4"/><path d="M15 7 9 17"/>',
    "queue": '<path d="M4 4v7a4 4 0 0 0 4 4h12"/><path d="m15 10 5 5-5 5"/>',
    "x": '<path d="M6 6l12 12M18 6 6 18"/>',
    "copy": '<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 8V3H3v13h5"/>',
    "key": '<circle cx="8" cy="8" r="5"/><path d="m12 12 9 9m-4-4 3-3m-7-1 3-3"/>',
    "arrowUp": '<path d="M12 19V5M5 12l7-7 7 7"/>',
    "stop": '<rect x="6" y="6" width="12" height="12" rx="2"/>',
    "settings": '<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1"/>',
    "list": '<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>',
    "package": '<path d="m12 2 9 5v10l-9 5-9-5V7Z"/><path d="m3 7 9 5 9-5M12 12v10M7.5 4.5l9 5"/>',
    "image": '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-5-5L5 21"/>',
    "type": '<path d="M4 7V4h16v3M9 20h6M12 4v16"/>',
    "shield": '<path d="M12 2 21 6v6c0 5-5 9-9 10-4-1-9-5-9-10V6Z"/>',
    "info": '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>',
    "warning": '<path d="m12 3 10 18H2Z M12 9v5M12 17h.1"/>',
    "trash": '<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/>',
    "download": '<path d="M12 3v12M7 10l5 5 5-5M5 21h14"/>',
    "share": '<path d="M4 12v8h16v-8M16 6l-4-4-4 4M12 2v13"/>',
    "tag": '<path d="M20 12 12 20l-9-9V3h8Z"/><circle cx="7.5" cy="7.5" r="1.5"/>',
    "star": '<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3 6.4 20.2l1.1-6.2L3 9.6l6.2-.9Z"/>',
    "refresh": '<path d="M20 10a8 8 0 1 0-1 7M20 3v7h-7"/>',
    "open": '<path d="M14 3h7v7M21 3l-9 9M19 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h5"/>',
    "bell": '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10 21h4"/>',
    "clock": '<circle cx="12" cy="12" r="9"/><path d="M12 6v6l4 2"/>',
    "gauge": '<path d="M12 14l4-4"/><path d="M3.3 19a10 10 0 1 1 17.4 0"/>',
    "layers": '<path d="m12 2 10 5-10 5L2 7ZM2 12l10 5 10-5M2 17l10 5 10-5"/>',
    "widget": '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/>',
    "tree": '<path d="M6 3v6M6 9h6a3 3 0 0 1 3 3v2M6 9v12"/><circle cx="15" cy="17" r="3"/>',
    "cpu": '<rect x="5" y="5" width="14" height="14" rx="2"/><path d="M9 1v4M15 1v4M9 19v4M15 19v4M1 9h4M1 15h4M19 9h4M19 15h4"/>',
}


def defs(width, height, extra=""):
    add(f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" '
        f'width="{width}" height="{height}" viewBox="0 0 {width} {height}">')
    add(extra)
    add("<defs>")
    add('<filter id="shadow" x="-15%" y="-20%" width="130%" height="150%"><feGaussianBlur stdDeviation="16"/></filter>')
    add('<filter id="popShadow" x="-20%" y="-20%" width="140%" height="160%"><feGaussianBlur stdDeviation="12"/></filter>')
    add('<clipPath id="windowClip"><rect x="48" y="130" width="1344" height="740" rx="12"/></clipPath>')
    add('<pattern id="grid" width="16" height="16" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="0.8" fill="#d8d0c9"/></pattern>')
    for name, pal in (("dark", DARK), ("light", LIGHT)):
        add(f'<linearGradient id="ctxFill-{name}" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{pal["tidal"]}"/>'
            f'<stop offset="1" stop-color="{pal["accent"] if name == "dark" else pal["steel"]}"/></linearGradient>')
    add('<g id="pimark">'
        '<path fill="#F09082" d="M0 0H352.07V234.71H234.71V117.36H0Z"/>'
        '<path fill="#4D9ABF" d="M0 117.36H117.36V234.71H234.71V352.07H117.36V469.43H0Z"/>'
        '<path fill="#F1BE58" d="M352.07 234.71H469.43V469.43H352.07Z"/></g>')
    for k, v in ICONS.items():
        add(f'<g id="{k}">{v}</g>')
    add("</defs>")


def ctx_fill():
    return f"url(#ctxFill-{THEME['name']})"


def page(width, height):
    rect(0, 0, width, height, PAGE["moonstone"])
    rect(0, 0, width, height, "url(#grid)")


# ---------------------------------------------------------------- window chrome

TODAY = [("Qwen signatures", "run"), ("Mistral thinking", "2h"), ("Kimi K3 default", "wait"), ("Discount jev", "6h")]
YESTERDAY = [("Attach latency", "1d"), ("RPC session list", "1d"), ("0.87.1 release", "2d")]
NAV = [("sessions", "list", "All Sessions"), ("models", "sparkle", "Models"),
       ("resources", "package", "Resources"), ("settings", "settings", "Settings")]
SESSION_TABS = ["THREAD", "CHANGES  3", "TREE", "CONTEXT"]
SESSION_ACTIONS = [("btn", "New", "plus"), ("icon", "branch"), ("icon", "compact"), ("icon", "more")]

# Multi-project sidebar: active sessions from every project, then projects with their sessions.
MULTI_ACTIVE = [("Qwen signatures", "pi", "run"), ("Agent panel fix", "zed", "wait"), ("Sandbox perf", "minivm", "run")]
MULTI_PROJECTS = [
    ("pi", "25", "run", [("Qwen signatures", "run"), ("Mistral thinking", "2h"), ("Kimi K3 default", "5h"),
                         ("Discount jev", "6h")]),
    ("zed", "8", "wait", None),
    ("minivm", "12", "run", None),
]


def session_row(title, state, selected, y, x_icon=68, x_text=94, size=13):
    if selected:
        rect(58, y, 188, 30 if size == 13 else 28, C["select"], 5)
        rect(58, y + 6, 2, 18 if size == 13 else 16, C["accent"], 1)
    icon("message", x_icon, y + 8, 14, C["accent"] if selected else C["faint"])
    text(x_text, y + 20, title, C["text"] if selected else C["text2"], size=size, weight=500 if selected else 400)
    if state in ("run", "wait", "idle"):
        state_mark(233, y + 15, state)
    else:
        text(238, y + 19, state, C["faint"], size=10.5, family=MONO, anchor="end")


def sidebar_single(session_sel, states, new_row):
    label(65, 211, "PROJECTS")
    rect(58, 221, 188, 30, C["hover"], 6)
    icon("folder", 68, 229, 15, C["text2"])
    text(94, 241, "pi", C["text"], size=13, weight=500)
    dot(214, 236, 3, C["accent"])
    text(236, 240, "4", C["muted"], size=11, family=MONO, anchor="end")
    for i, (name, n) in enumerate((("zed", "2"), ("minivm", "1"))):
        y = 263 + i * 30
        icon("folder", 68, y, 15, C["faint"])
        text(94, y + 12, name, C["text2"], size=13)
        text(236, y + 11, n, C["faint"], size=11, family=MONO, anchor="end")
    icon("plus", 69, 324, 13, C["faint"])
    text(94, 335, "Open Folder…", C["muted"], size=12)

    label(65, 377, "SESSIONS")
    icon("plus", 226, 367, 13, C["muted"])
    states = states or {}
    today = [(t, states.get(t, s)) for t, s in TODAY]
    yesterday = list(YESTERDAY)
    if new_row:
        today = [("New session", "now")] + today
        yesterday = yesterday[:2]
    y = 393
    for i, (t, s) in enumerate(today):
        session_row(t, s, session_sel == i, y)
        y += 30
    label(65, y + 20, "YESTERDAY", size=9)
    y += 28
    for t, s in yesterday:
        session_row(t, s, False, y)
        y += 30
    text(94, y + 19, "Show 18 older", C["faint"], size=11)


def sidebar_multi(active, projects, selected):
    """selected: session title highlighted in both the active list and its project."""
    label(65, 211, "ACTIVE")
    text(236, 211, str(len(active)), C["faint"], size=10.5, family=MONO, anchor="end")
    y = 221
    for title, proj, state in active:
        sel = title == selected
        if sel:
            rect(58, y, 188, 30, C["select"], 5)
            rect(58, y + 6, 2, 18, C["accent"], 1)
        state_mark(73, y + 15, state)
        text(90, y + 20, title, C["text"] if sel else C["text2"], size=12.5, weight=500 if sel else 400)
        text(238, y + 19, proj, C["faint"], size=10.5, family=MONO, anchor="end")
        y += 30
    y += 16
    label(65, y + 10, "PROJECTS")
    icon("plus", 226, y, 13, C["muted"])
    y += 20
    for name, count, state, sessions in projects:
        expanded = sessions is not None
        icon("chevron" if expanded else "chevronR", 60, y + 9, 11, C["faint"])
        icon("folder", 76, y + 7, 14, C["text2"] if expanded else C["faint"])
        text(98, y + 19, name, C["text"] if expanded else C["text2"], size=13, weight=500 if expanded else 400)
        if state == "run":
            dot(212, y + 14, 3, C["accent"])
        elif state == "wait":
            dot(212, y + 14, 3, C["amber"])
        text(238, y + 18, count, C["faint"], size=10.5, family=MONO, anchor="end")
        y += 28
        if expanded:
            for t, s in sessions:
                sel = t == selected
                if sel:
                    rect(58, y, 188, 28, C["select"], 5)
                    rect(58, y + 6, 2, 16, C["accent"], 1)
                icon("message", 94, y + 8, 13, C["accent"] if sel else C["faint"])
                text(114, y + 19, t, C["text"] if sel else C["text2"], size=12.5, weight=500 if sel else 400)
                if s in ("run", "wait", "idle"):
                    state_mark(233, y + 14, s)
                else:
                    text(238, y + 18, s, C["faint"], size=10.5, family=MONO, anchor="end")
                y += 28
            text(114, y + 16, f"Show {int(count) - len(sessions)} more", C["faint"], size=11)
            y += 26
    icon("plus", 62, y + 6, 12, C["faint"])
    text(98, y + 17, "Open Folder…", C["muted"], size=12)


def window_begin(title, title_icon="message", actions=SESSION_ACTIONS, search="Search sessions",
                 session_sel=None, states=None, new_row=False, nav_sel=None,
                 tabs=None, tab_active=0, tab_right="~/repos/pi · main",
                 status_right="3 files changed · $0.41 · 31% context", status_left_extra="anthropic",
                 title_chevron=True, title_prefix=None, multi=None, status_left_accent=False):
    add(f'<rect x="58" y="150" width="1324" height="732" rx="13" fill="#1b1612" filter="url(#shadow)" '
        f'opacity="{0.28 if THEME["name"] == "dark" else 0.16}"/>')
    add('<g clip-path="url(#windowClip)">')
    rect(48, 130, 1344, 740, C["canvas"])
    rect(48, 130, 1344, 52, C["bar"])
    rect(48, 182, 208, 664, C["side"])
    rect(1064, 182, 328, 664, C["side"])
    rect(48, 846, 1344, 24, C["status"])
    edge = C["deep"] if THEME["name"] == "dark" else C["lineStrong"]
    line(48, 181.5, 1392, 181.5, edge)
    line(256.5, 182, 256.5, 846, edge)
    line(1063.5, 182, 1063.5, 846, edge)
    line(48, 846.5, 1392, 846.5, edge)

    # title bar
    for cx, col in ((70, "#ff6058"), (90, "#febc2e"), (110, "#29c841")):
        dot(cx, 156, 6, col)
    pimark(141, 147, 18)
    text(168, 161, "pi", C["text"], size=14, weight=600)
    icon(title_icon, 279, 147, 18, C["muted"])
    x = 306
    if title_prefix:
        text(x, 161, title_prefix, C["muted"], size=14)
        x += tw(title_prefix, 14) + 8
        text(x, 161, "/", C["faint"], size=14)
        x += 14
    text(x, 161, title, C["text"], size=14, weight=600)
    x += tw(title, 14) + 8
    if title_chevron:
        icon("chevron", x, 150, 14, C["faint"])
        x += 16
    x = max(x + 14, 460)
    line(x, 145, x, 168, C["lineStrong"])
    x += 16
    for a in actions:
        if a[0] == "btn":
            w = tw(a[1], 12) + 44
            rect(x, 143, round(w, 1), 27, C["chip"], 6, C["chipLine"])
            icon(a[2], x + 9, 149, 15, C["text2"])
            text(x + 30, 161, a[1], C["text2"], size=12)
            x += w + 20
        else:
            icon(a[1], x, 147, 18, C["text2"])
            x += 36
    rect(780, 142, 268, 28, C["canvas"], 6, C["lineStrong"])
    icon("search", 790, 149, 15, C["faint"])
    text(813, 161, search, C["faint"], size=12)
    text(1036, 160, "⌘ K", C["faint"], size=11, family=MONO, anchor="end")
    label(1084, 160, "INSPECTOR", C["muted"], size=10.5)
    icon("inspector", 1355, 146, 19, C["text2"])

    if multi is not None:
        sidebar_multi(**multi)
    else:
        sidebar_single(session_sel, states, new_row)

    # sidebar: app views
    line(65, 707.5, 239, 707.5, C["line"])
    for i, (key, ic, name) in enumerate(NAV):
        y = 716 + i * 28
        sel = key == nav_sel
        if sel:
            rect(58, y, 188, 26, C["select"], 5)
            rect(58, y + 5, 2, 16, C["accent"], 1)
        icon(ic, 68, y + 6, 14, C["accent"] if sel else C["muted"])
        text(94, y + 17, name, C["text"] if sel else C["text2"], size=12.5, weight=500 if sel else 400)

    # middle tabs for session views
    if tabs is not None:
        line(256, 221.5, 1064, 221.5, C["hover"])
        utabs(280, 206, tabs, tab_active, gap=26)
        if tab_right:
            icon("branch", 1044 - tw(tab_right, 11, True) - 20, 195, 13, C["faint"])
            text(1044, 206, tab_right, C["muted"], size=11, family=MONO, anchor="end")

    # status bar
    dot(65, 858, 2.5, C["green"])
    text(75, 862, "pi 0.87.1 · rpc", C["muted"], size=10, family=MONO)
    if status_left_extra:
        text(186, 862, "·  " + status_left_extra, C["accent"] if status_left_accent else C["faint"], size=10, family=MONO)
    text(1374, 862, status_right, C["muted"], size=10, family=MONO, anchor="end")


def window_end():
    add("</g>")
    rect(48, 130, 1344, 740, "none", 12, C["winBorder"])


# ---------------------------------------------------------------- thread pieces

def user_msg(y, lines, time=None, h=None):
    h = h or 18 + 20 * len(lines)
    rect(280, y, 764, h, C["userBg"], 8)
    rect(280, y + 6, 2, h - 12, C["steel"], 1)
    for i, s in enumerate(lines):
        text(296, y + 22 + i * 20, s, C["text"], size=13)
    if time:
        text(1032, y + 21, time, C["faint"], size=10, family=MONO, anchor="end")
    return h


def thinking_row(y, s="Thought for 9s", level="high"):
    icon("sparkle", 288, y - 12, 14, THINK_COLOR[level])
    text(310, y, s, C["muted"], size=12, italic=True)
    icon("chevronR", 310 + tw(s, 12) + 4, y - 10, 12, C["faint"])


def tool_row(y, ic, verb, target, meta, status="ok", target_color=None):
    icon(ic, 288, y + 1, 14, C["faint"])
    text(310, y + 12, verb, C["muted"], size=12)
    text(350, y + 12, target, target_color or C["text2"], size=11.5, family=MONO)
    if status == "ok":
        text(1010, y + 12, meta, C["faint"], size=10.5, family=MONO, anchor="end")
        icon("check", 1018, y + 1, 13, C["green"])
    elif status == "fail":
        text(1010, y + 12, meta, C["coral"], size=10.5, family=MONO, anchor="end")
        icon("x", 1018, y + 1, 13, C["coral"])
    elif status == "wait":
        text(1016, y + 12, meta, C["amber"], size=10.5, family=MONO, anchor="end")
        dot(1027, y + 8, 3.5, C["amber"])


def diff_main():
    KW, STR, PL = C["kw"], C["str"], C["pl"]
    return [
        ("211", " ", None, [("    ", PL), ("if", KW), (" (block.type === ", PL), ('"thinking"', STR), (") {", PL)]),
        ("212", "−", "del", [("      ", PL), ("if", KW), (" (!block.signature) ", PL), ("throw new", KW),
                             (" MissingSignature(model.id);", PL)]),
        ("212", "+", "add", [("      ", PL), ("if", KW), (" (block.signature === ", PL), ("undefined", KW),
                             (" && isAnthropic(model)) {", PL)]),
        ("213", "+", "add", [("        ", PL), ("throw new", KW), (" MissingSignature(model.id);", PL)]),
        ("214", "+", "add", [("      }", PL)]),
    ]


def card_header(y, ic, verb, target, right_parts=None, h=30):
    icon(ic, 292, y + 8, 14, C["muted"])
    text(314, y + 20, verb, C["muted"], size=12)
    text(314 + tw(verb, 12) + 12, y + 20, target, C["text2"], size=11.5, family=MONO)
    if right_parts:
        tparts(1030, y + 20, right_parts, size=11, family=MONO, anchor="end")
    line(280, y + h + 0.5, 1044, y + h + 0.5, C["card"])


def well(y0, y1, x0=281, x1=1043):
    add(f'<path d="M{x0} {y0}H{x1}V{y1 - 7}a7 7 0 0 1-7 7H{x0 + 7}a7 7 0 0 1-7-7Z" fill="{C["deep"]}"/>')


def diff_lines(x_num, x_sign, x_code, y, rows, width=762, x_bg=281, pitch=17, two_cols=False):
    for i, row in enumerate(rows):
        yy = y + i * pitch
        if two_cols:
            old, new, sign, kind, parts = row
        else:
            num, sign, kind, parts = row
        if kind == "del":
            rect(x_bg, yy, width, pitch, C["delBg"])
        elif kind == "add":
            rect(x_bg, yy, width, pitch, C["addBg"])
        elif kind == "hunk":
            rect(x_bg, yy, width, pitch, blend(C["tidal"], C["deep"], 0.18 if THEME["name"] == "dark" else 0.08))
        if two_cols:
            text(x_num, yy + 12.5, old, C["faint"], size=10.5, family=MONO, anchor="end")
            text(x_num + 32, yy + 12.5, new, C["faint"], size=10.5, family=MONO, anchor="end")
        else:
            text(x_num, yy + 12.5, num, C["faint"], size=10.5, family=MONO, anchor="end")
        sc = C["coral"] if kind == "del" else C["green"] if kind == "add" else C["faint"]
        text(x_sign, yy + 12.5, sign, sc, size=11, family=MONO)
        tparts(x_code, yy + 12.5, parts, size=11, family=MONO, pre=True)


def edit_card(y, path="packages/ai/src/providers/openai-completions.ts"):
    rows = diff_main()
    h = 36 + 17 * len(rows) + 3
    rect(280, y, 764, h, C["side"], 8, C["card"])
    card_header(y, "edit", "Edit", path, [("+3", C["green"]), ("  ", C["faint"]), ("−1", C["coral"])])
    well(y + 31, y + h - 1)
    diff_lines(318, 330, 344, y + 34, rows)
    return h


def bash_card(y, cmd, lines, running=True, meta="running · 14s", h=118):
    rect(280, y, 764, h, C["side"], 8, C["card"])
    icon("terminal", 292, y + 8, 14, C["muted"])
    text(314, y + 20, "Bash", C["muted"], size=12)
    text(350, y + 20, cmd, C["text"], size=11.5, family=MONO)
    if running:
        spinner(920, y + 15, 5, C["accent"])
        text(1030, y + 19, meta, C["muted"], size=10.5, family=MONO, anchor="end")
    line(280, y + 30.5, 1044, y + 30.5, C["card"])
    well(y + 31, y + h - 1)
    for i, parts in enumerate(lines):
        tparts(296, y + 52 + i * 18, parts, size=11, family=MONO, pre=True)
    if running:
        rect(296, y + 52 + len(lines) * 18 - 10, 7, 13, C["accent"], 1)
    return h


def composer(y, h, value, placeholder=False, queue=None, model="claude-opus-5-5", thinking="high",
             buttons=(("Follow-up  ⌥⏎", "ghost"), ("Steer  ⏎", "primary")), stop=True, prefix=None, hint=None):
    rect(276, y, 768, h, C["bar"] if THEME["name"] == "dark" else C["chip"], 10, C["focus"])
    ty = y + 34
    base = C["bar"] if THEME["name"] == "dark" else C["chip"]
    if queue:
        qbg = blend(C["amber"], base, 0.07)
        rect(277, y + 1, 766, 29, qbg, 9)
        rect(277, y + 20, 766, 10, qbg)
        line(277, y + 30.5, 1043, y + 30.5, blend(C["amber"], base, 0.18))
        icon("queue", 290, y + 8, 14, C["amber"])
        text(312, y + 20, "FOLLOW-UP", C["amber"], size=9.5, weight=500, family=MONO, ls=1.2)
        text(388, y + 20, queue, C["text2"], size=12)
        text(996, y + 20, "1 queued", C["faint"], size=10.5, family=MONO, anchor="end")
        icon("x", 1012, y + 9, 12, C["muted"])
        ty = y + 60
    tx = 292
    if prefix:
        w = pill(292, ty - 12, prefix[0], C["onLevel"], prefix[1], size=9.5, h=16)
        tx = 292 + w + 8
    if placeholder:
        text(tx, ty, value, C["faint"], size=13)
    else:
        text(tx, ty, value, C["text"], size=13, family=MONO if prefix else SANS)
        rect(round(tx + tw(value, 13, bool(prefix)) + 2, 1), ty - 12, 1.5, 16, C["accent"])
    by = y + h - 38
    icon("clip", 292, by + 4, 15, C["muted"])
    icon("slash", 318, by + 4, 15, C["muted"])
    chipbg = C["chip"] if THEME["name"] == "dark" else C["hover"]
    mw = tw(model, 11, True) + 52
    rect(346, by, round(mw, 1), 24, chipbg, 5)
    icon("sparkle", 353, by + 5, 13, C["muted"])
    text(372, by + 16, model, C["text2"], size=11, family=MONO)
    icon("chevron", 346 + mw - 19, by + 6, 11, C["faint"])
    tx2 = 346 + mw + 8
    thw = tw(thinking, 11, True) + 50
    rect(round(tx2, 1), by, round(thw, 1), 24, chipbg, 5)
    dot(tx2 + 13, by + 12, 3.5, THINK_COLOR.get(thinking if thinking in THINK_COLOR else thinking[:3], C["muted"]))
    text(tx2 + 23, by + 16, thinking, C["text2"], size=11, family=MONO)
    icon("chevron", tx2 + thw - 18, by + 6, 11, C["faint"])
    if hint:
        text(tx2 + thw + 14, by + 16, hint, C["faint"], size=11)
    x = 1038
    if stop:
        rect(1014, by, 24, 24, blend(C["coral"], base, 0.1), 5, blend(C["coral"], base, 0.35))
        icon("stop", 1018, by + 4, 16, C["coral"])
        x = 1006
    for label_, kind in reversed(buttons):
        w = button(x, by, label_, kind, anchor="end")
        x -= w + 8
