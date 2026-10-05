#!/usr/bin/env python3
"""Render proposal SVGs/PNGs and a measured, annotated copy of an actual capture.

These are design mockups, not app screenshots. Run from any directory:
  python3 design/visual-review-2026-10-01/render.py
Uses local fonts/icons, Chromium and Pillow; no network dependencies.
"""
from html import escape
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUT = HERE / 'concepts'
LIGHT = dict(canvas='#faf9f7', side='#f2efeb', bar='#ebe7e4', surface='#ffffff',
             line='#ded8d1', selected='#e7e3de', text='#252f3d', secondary='#3a4453',
             muted='#6b645d', accent='#4b607c', on_accent='#ffffff',
             green='#2e7950', amber='#8b631f', queue='#faf6ef')
DARK = dict(canvas='#161d27', side='#1a212b', bar='#1f2630', surface='#1f2630',
            line='#363d46', selected='#2a3038', text='#ebe7e4', secondary='#d5d8db',
            muted='#959ca5', accent='#8caecb', on_accent='#111820',
            green='#80bf95', amber='#e5bd73', queue='#292b2d')


class SVG:
    def __init__(self, width=1344, height=740, dark=False):
        self.width, self.height = width, height
        self.c = DARK if dark else LIGHT
        self.parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
                      '<title>Pi Desktop — proposed design, not an app screenshot</title>',
                      '''<style>
@font-face {font-family:Plex;src:url('../../../assets/fonts/IBMPlexSans-Regular.ttf')}
@font-face {font-family:Plex;src:url('../../../assets/fonts/IBMPlexSans-SemiBold.ttf');font-weight:600}
@font-face {font-family:Commit;src:url('../../../assets/fonts/CommitMono-Regular.otf')}
text {font-family:Plex,sans-serif}
</style>''']

    def color(self, value):
        return self.c.get(value, value)

    def rect(self, x, y, w, h, fill, radius=0, stroke=None, extra=''):
        border = f' stroke="{self.color(stroke)}"' if stroke else ''
        self.parts.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{radius}" fill="{self.color(fill)}"{border} {extra}/>')

    def line(self, x, y, x2, y2, color='line', width=1):
        self.parts.append(f'<path d="M{x} {y} L{x2} {y2}" stroke="{self.color(color)}" stroke-width="{width}"/>')

    def text(self, x, y, value, size=13, color='text', weight=400, family=None, anchor='start', italic=False):
        styles = (f'font-family:{family};' if family else '') + ('font-style:italic;' if italic else '')
        if family and 'monospace' in family:
            styles += 'white-space:pre;'
        self.parts.append(f'<text x="{x}" y="{y}" fill="{self.color(color)}" font-size="{size}" font-weight="{weight}" text-anchor="{anchor}" style="{styles}">{escape(value)}</text>')

    def title(self, x, y, value):
        self.text(x, y, value, 23, family='Georgia,DejaVu Serif,serif', italic=True)

    def dot(self, x, y, color='muted', r=3):
        self.parts.append(f'<circle cx="{x}" cy="{y}" r="{r}" fill="{self.color(color)}"/>')

    def icon(self, name, x, y, color='muted', size=16):
        source = (ROOT / 'assets/icons' / f'{name}.svg').read_text()
        source = re.sub(r'(fill|stroke)="(?!none)[^"]+"', lambda m: f'{m[1]}="{self.color(color)}"', source)
        opening, body = source.split('>', 1)
        opening = re.sub(r'\s(?:width|height)="[^"]+"', '', opening)
        source = opening + '>' + body
        source = source.replace('<svg ', f'<svg x="{x}" y="{y}" width="{size}" height="{size}" ', 1)
        self.parts.append(source)

    def button(self, x, y, w, label, primary=False, h=30):
        self.rect(x, y, w, h, 'accent' if primary else 'surface', 6, None if primary else 'line')
        self.text(x + w / 2, y + h / 2 + 4, label, 12, 'on_accent' if primary else 'text', 600 if primary else 400, anchor='middle')

    def disclosure(self, x, y, label, detail='', end=1320):
        self.icon('chevron_right', x, y - 12, size=14)
        self.text(x + 22, y, label)
        if detail:
            self.text(end, y, detail, 12, 'muted', anchor='end')

    def shell(self, active='Thread', inspector=True, sample=True):
        w, h = self.width, self.height
        self.rect(0, 0, w, h, 'canvas')
        self.rect(0, 36, 216, h - 60, 'side')
        self.rect(0, 0, w, 36, 'bar')
        self.line(0, 35.5, w, 35.5)
        self.line(215.5, 36, 215.5, h - 24)
        self.icon('threads_sidebar_left_open', 14, 10)
        self.icon('magnifying_glass', 46, 10)
        self.icon('plus', w - 60, 10)
        self.icon('threads_sidebar_right_open' if inspector else 'threads_sidebar_right_closed', w - 28, 10)
        self.icon('chevron_down', 16, 54, size=12)
        self.text(34, 64, 'Open', 12, 'muted', 600)
        self.text(196, 64, '1', 12, 'muted', anchor='end')
        if active == 'Thread':
            self.rect(8, 78, 200, 32, 'selected', 6)
        self.dot(23, 94, 'accent' if sample else 'muted')
        self.text(38, 98, 'Qwen signatures' if sample else 'Catalog fixture', 13, weight=600)
        self.text(196, 98, 'pi', 11, 'muted', anchor='end')
        self.icon('chevron_down', 16, 130, size=12)
        self.text(34, 140, 'Projects', 12, 'muted', 600)
        self.icon('plus', 181, 130, size=14)
        self.icon('chevron_down', 16, 162, size=12)
        self.icon('folder', 34, 160)
        self.text(60, 174, 'pi', weight=600)
        self.text(195, 174, '7', 12, 'muted', anchor='end')
        for i, (name, age) in enumerate([('Mistral thinking', '2h'), ('Kimi K3 default', '5h'), ('Discount jev', '6h')]):
            y = 206 + i * 32
            self.icon('chat', 37, y - 12, size=14)
            self.text(60, y, name)
            self.text(197, y, age, 11, 'muted', anchor='end')
        self.text(60, 302, '3 more saved sessions', 12, 'muted')
        for i, name in enumerate(['zed', 'minivm']):
            y = 337 + i * 32
            self.icon('chevron_right', 16, y - 12, size=12)
            self.icon('folder', 34, y - 14)
            self.text(60, y, name)
        self.icon('plus', 18, 391, size=14)
        self.text(44, 404, 'Open folder…', 12, 'muted')
        self.line(16, h - 170, 200, h - 170)
        for i, (name, glyph) in enumerate([('All Sessions', 'list_tree'), ('Models', 'sparkle'), ('Resources', 'box'), ('Settings', 'settings')]):
            y = h - 157 + i * 32
            if active == name:
                self.rect(8, y - 2, 200, 30, 'selected', 6)
            self.icon(glyph, 20, y + 5)
            self.text(46, y + 18, name)
        self.rect(0, h - 24, w, 24, 'bar')
        self.line(0, h - 24, w, h - 24)
        self.dot(18, h - 12, 'muted')
        self.text(29, h - 8, 'Offline demo' if sample else 'Pi connected', 12, 'muted')
        self.text(w - 16, h - 8, 'Run details' if sample else '1 process', 12, 'muted', anchor='end')
        if inspector:
            self.rect(w - 320, 36, 320, h - 60, 'side')
            self.line(w - 320, 36, w - 320, h - 24)

    def save(self, name):
        path = OUT / f'{name}.svg'
        path.write_text('\n'.join(self.parts + ['</svg>']) + '\n')
        return path


def thread(dark=False, wide=False, expanded=False):
    dx, dy = (256, 160) if wide else (0, 0)
    s = SVG(1344 + dx, 740 + dy, dark=dark)
    # The work surface is fluid at every width; only reading text has a measure.
    left, right = 240, s.width - 320 - 24
    content_width = right - left
    s.shell()
    s.line(216, 75.5, 1024 + dx, 75.5)
    for x, label in [(240, 'Thread'), (317, 'Changes  1'), (426, 'Tree'), (487, 'Context')]:
        s.text(x, 60, label, 13, 'text' if label == 'Thread' else 'muted', 600 if label == 'Thread' else 400)
    s.line(240, 74, 280, 74, 'secondary', 2)
    s.text(936 + dx, 60, '~/repos/pi', 11, 'muted', family='Commit,monospace', anchor='end')
    s.icon('folder', 949 + dx, 48)
    s.icon('terminal', 987 + dx, 48)
    s.rect(left, 100, content_width, 76, 'surface', 6, 'line', extra='data-layout="user-turn"')
    s.text(256, 128, 'qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response.', 15)
    s.text(256, 152, 'Accept empty signatures there, but keep the check strict for Anthropic.', 15)
    s.disclosure(246, 204, 'Reasoning', end=982)
    s.text(248, 241, 'The check lives in openai-completions.ts. I’ll allow empty strings for OpenCode', 15, 'secondary')
    s.text(248, 265, 'and leave the Anthropic path untouched.', 15, 'secondary')
    s.icon('chevron_down', 246, 294, size=14)
    s.text(270, 306, '4 tool calls', 12, 'muted', 600)
    tools = [('file', 'Read', 'packages/ai/src/providers/openai-completions.ts', '✓'),
             ('magnifying_glass', 'Grep', '"signature" in packages/ai', '✓'),
             ('pencil', 'Edit', 'packages/ai/src/providers/openai-completions.ts', '+3 −1'),
             ('terminal', 'Bash', 'npm run check', 'Running')]
    for i, (glyph, name, detail, state) in enumerate(tools):
        y = 338 + i * 32 + (172 if expanded and i > 2 else 0)
        s.icon(glyph, 248, y - 12, size=14)
        s.text(272, y, name, 12, 'muted')
        s.text(316, y, detail, 12, 'secondary', family='Commit,monospace')
        s.text(right - 38, y, state, 12, 'muted' if state == 'Running' else 'green', anchor='end')
        opened = expanded and name == 'Edit'
        s.icon('chevron_down' if opened else 'chevron_right', right - 22, y - 12, size=12)
        if opened:
            # An explicitly expanded tool, not an automatic wide-window panel.
            code = [('211', ' ', 'if (block.type === "thinking") {', None),
                    ('212', '−', '  if (!block.signature) throw new MissingSignature(model.id);', 'remove'),
                    ('212', '+', '  if (!block.signature && isAnthropic(model)) {', 'add'),
                    ('213', '+', '    throw new MissingSignature(model.id);', 'add'),
                    ('214', '+', '  }', 'add'),
                    ('215', ' ', '}', None)]
            fills = {'add': '#243b30' if dark else '#e0ebe3',
                     'remove': '#422e30' if dark else '#f1e4df'}
            s.rect(left, y + 12, content_width, 156, 'surface', 6, 'line', extra='data-layout="tool-detail"')
            for row, (number, sign, text, fill) in enumerate(code):
                baseline = y + 38 + row * 22
                if fill:
                    s.rect(left + 1, baseline - 16, content_width - 2, 22, fills[fill])
                s.text(left + 14, baseline, number, 12, 'muted', family='Commit,monospace')
                s.text(left + 52, baseline, sign, 13, 'muted', family='Commit,monospace')
                s.text(left + 78, baseline, text, 13, 'secondary', family='Commit,monospace')
    s.parts.append(f'<g transform="translate(0 {dy})">')
    s.rect(left, 548, content_width, 152, 'surface', 8, 'line', extra='data-layout="composer"')
    s.rect(left + 1, 549, content_width - 2, 30, 'queue', 7)
    s.text(254, 569, 'Queued · 1', 12, 'amber', 600)
    s.text(331, 569, 'Also add a regression test for the OpenCode path', 12, 'secondary')
    s.icon('close', right - 25, 557, size=13)
    s.line(left + 1, 579, right - 1, 579)
    s.text(256, 609, 'Run the qwen provider tests once check passes', 15)
    s.icon('attach', 255, 661)
    s.text(289, 674, '/', 16, 'muted')
    s.text(320, 674, 'claude-opus-5-5', 12, family='Commit,monospace')
    s.icon('chevron_down', 440, 662, size=12)
    s.dot(477, 669, 'amber')
    s.text(488, 674, 'high', 12, 'muted')
    s.icon('chevron_down', 520, 662, size=12)
    s.button(right - 237, 654, 104, 'Follow-up  ⌥↵')
    s.button(right - 121, 654, 76, 'Steer ↵', True)
    s.rect(right - 37, 654, 28, 30, 'surface', 6, 'line')
    s.rect(right - 27, 665, 8, 8, 'none', 1, 'secondary')
    s.parts.append('</g>')
    s.parts.append(f'<g transform="translate({dx} 0)">')
    s.title(1044, 72, 'Qwen signatures')
    s.text(1044, 98, 'pi · ~/repos/pi', 12, 'muted')
    s.line(1044, 120, 1324, 120)
    s.text(1044, 148, 'Context', 12, 'muted', 600)
    s.text(1044, 181, '62.4k', 25, weight=600)
    s.text(1116, 180, '/ 200k tokens', 12, 'muted')
    s.rect(1044, 195, 280, 4, 'line', 2)
    s.rect(1044, 195, 87, 4, 'accent', 2)
    s.text(1044, 221, '31% used · auto-compaction on', 12, 'muted')
    s.text(1044, 258, 'Session cost', 13, 'secondary')
    s.text(1324, 258, '$0.41', 13, anchor='end', family='Commit,monospace')
    s.text(1044, 290, 'Observed edits', 13, 'secondary')
    s.text(1324, 290, '1', 13, anchor='end', family='Commit,monospace')
    s.line(1044, 314, 1324, 314)
    s.disclosure(1044, 345, 'Usage details', end=1324)
    s.disclosure(1044, 381, 'Tools', 'Not reported', end=1324)
    s.disclosure(1044, 417, 'File history', 'Not recording', end=1324)
    s.text(1066, 443, 'No snapshots; past edits cannot be restored.', 12, 'muted')
    s.line(1044, 467, 1324, 467)
    s.disclosure(1044, 499, 'Session file', end=1324)
    s.disclosure(1044, 535, 'Extensions', '1 status', end=1324)
    s.parts.append('</g>')
    return s.save('thread-wide' if wide else 'thread-dark' if dark else 'thread-light')


def settings():
    s = SVG()
    s.shell('Settings', inspector=False, sample=False)
    s.line(216, 79.5, 1344, 79.5)
    s.rect(240, 44, 155, 28, 'bar', 6)
    s.rect(243, 47, 48, 22, 'surface', 4)
    s.text(267, 63, 'User', 12, anchor='middle')
    s.text(341, 63, 'Project · pi', 12, 'muted', anchor='middle')
    s.rect(878, 44, 322, 28, 'surface', 6, 'line')
    s.icon('magnifying_glass', 888, 51, size=14)
    s.text(913, 63, 'Find a setting…', 12, 'muted')
    s.button(1212, 44, 108, 'Open JSON', h=28)
    s.line(391.5, 80, 391.5, 716)
    groups = [(110, 'Pi', ['Model & thinking', 'Interaction', 'Tools', 'Sessions & context', 'Compaction', 'Branch summaries', 'Terminal & display', 'Network & retries', 'Shell', 'Resources', 'Updates & telemetry']),
              (514, 'Pi Desktop', ['General', 'Appearance', 'jj', 'Language servers', 'Editor', 'Terminal'])]
    for start, title, items in groups:
        s.text(236, start, title, 12, 'muted', 600)
        for i, name in enumerate(items):
            y = start + 27 + i * (28 if title == 'Pi' else 26)
            if name == 'Model & thinking':
                s.rect(224, y - 20, 159, 28, 'selected', 6)
            s.text(236, y, name, 13)
    s.title(424, 122, 'Model & thinking')
    s.text(424, 148, 'Defaults for new sessions. Running sessions keep their current settings.', 13, 'muted')
    rows = [(195, 'Default provider', 'Use a configured provider automatically.', 'Automatic'),
            (269, 'Default model', 'Choose the model new sessions start with.', 'Automatic'),
            (343, 'Default thinking level', 'More thinking can improve answers and increase cost.', 'Medium'),
            (417, 'Enabled models', 'Models available when cycling in a session.', 'All available · Edit')]
    for y, name, description, value in rows:
        s.text(424, y, name, 14, weight=600)
        s.text(424, y + 22, description, 13, 'muted')
        s.button(1096, y - 15, 208, value)
        s.line(424, y + 40, 1304, y + 40)
    for y, name, description in [(491, 'Hide thinking blocks', 'Hide reasoning text in the conversation.'),
                                  (565, 'Show cache miss notices', 'Show a notice when a request misses the prompt cache.')]:
        s.text(424, y, name, 14, weight=600)
        s.text(424, y + 22, description, 13, 'muted')
        s.rect(1268, y - 8, 36, 20, 'line', 10)
        s.dot(1279, y + 2, 'surface', r=7)
        s.line(424, y + 40, 1304, y + 40)
    s.disclosure(424, 642, 'Advanced', 'Per-model thinking · budgets · cache warming', end=1304)
    s.text(424, 690, 'Pi settings · user scope', 12, 'muted')
    s.text(1304, 690, 'Show JSON keys', 12, 'muted', anchor='end')
    return s.save('settings')


def compact():
    s = SVG(1000, 680)
    s.shell('Models', inspector=False, sample=False)
    s.icon('threads_sidebar_right_open', 972, 10)
    s.text(240, 64, 'Configured models', 13, 'muted')
    s.button(782, 44, 86, 'Log in…', h=28)
    s.button(880, 44, 96, 'Refresh', h=28)
    s.line(216, 80, 1000, 80)
    s.text(240, 111, 'All providers  ⌄', 13)
    s.text(414, 111, 'All models  ⌄', 13)
    s.text(600, 111, '4 models', 12, 'muted', anchor='end')
    s.rect(216, 130, 784, 32, 'side')
    s.text(248, 151, 'Model / provider', 12, 'muted')
    s.text(560, 151, 'Context', 12, 'muted')
    for i, (model, size) in enumerate([('Atlas Large', '200k'), ('Atlas Small', '128k'), ('Local Code', '64k'), ('Unknown limits', '—')]):
        y = 164 + i * 56
        if i == 1:
            s.rect(224, y, 768, 52, 'selected', 6)
        s.text(248, y + 21, model, 14, weight=600 if i == 1 else 400)
        s.text(248, y + 41, 'fixture-ai' if i < 2 else 'fixture-local', 12, 'muted')
        s.text(561, y + 30, size, 12, family='Commit,monospace')
    # A requested inspector overlays content; it never shrinks the table to 400px.
    s.rect(0, 36, 640, 620, '#111820', extra='opacity="0.12"')
    s.rect(640, 36, 360, 620, 'side')
    s.line(640, 36, 640, 656)
    s.title(664, 74, 'Atlas Small')
    s.icon('close', 963, 56)
    s.text(664, 101, 'fixture-ai / atlas-small', 12, 'muted', family='Commit,monospace')
    s.text(664, 129, 'OAuth · stored', 12, 'muted')
    s.line(664, 148, 976, 148)
    for y, key, value in [(182, 'Context window', '128k'), (216, 'Max output', '16k'), (250, 'Input', 'Text'), (284, 'Reasoning', 'Not supported')]:
        s.text(664, y, key)
        s.text(976, y, value, 13, anchor='end')
    s.line(664, 307, 976, 307)
    s.text(664, 337, 'Price', 12, 'muted', 600)
    s.text(976, 337, 'USD / 1M tokens', 12, 'muted', anchor='end')
    s.text(664, 372, 'Input / output')
    s.text(976, 372, '$0.25 / $1.25', 13, family='Commit,monospace', anchor='end')
    s.disclosure(664, 414, 'More model details', end=976)
    s.text(664, 458, 'Include in model cycle', 13)
    s.rect(940, 444, 36, 20, 'line', 10)
    s.dot(951, 454, 'surface', r=7)
    s.text(664, 500, 'Selecting a model does not run a prompt.', 12, 'muted')
    s.line(640, 540, 1000, 540)
    s.button(664, 556, 312, 'Use in this session', True)
    s.button(664, 598, 312, 'Use and set as default…')
    return s.save('compact-models')


def contrast(fg, bg):
    def luminance(color):
        v = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
        v = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in v]
        return sum(c * w for c, w in zip(v, [0.2126, 0.7152, 0.0722]))
    low, high = sorted([luminance(fg), luminance(bg)])
    return round((high + 0.05) / (low + 0.05), 2)


def evidence():
    """Keep originals unchanged; annotate only a separate copy."""
    with Image.open(HERE / 'captures/01-thread-light.png') as source:
        shot = source.convert('RGB')
    assert shot.size == (1344, 740)
    assert shot.getpixel((100, 450)) == (242, 239, 235)
    assert shot.getpixel((500, 450)) == (250, 249, 247)
    assert shot.getpixel((1100, 450)) == (242, 239, 235)
    page = Image.new('RGB', (1424, 886), '#faf9f7')
    page.paste(shot, (40, 80))
    d = ImageDraw.Draw(page)
    font = ImageFont.truetype(str(ROOT / 'assets/fonts/IBMPlexSans-Regular.ttf'), 15)
    small = ImageFont.truetype(str(ROOT / 'assets/fonts/IBMPlexSans-Regular.ttf'), 13)
    d.text((40, 10), 'ACTUAL CAPTURE · 1344 × 740 · Linux / Xvfb / 1×', font=font, fill='#252f3d')
    for start, end, label in [(0, 208, '208 px sidebar'), (208, 1016, '808 px middle'), (1016, 1344, '328 px inspector')]:
        a, b = 40 + start, 40 + end
        d.line((a, 69, b, 69), fill='#6b645d')
        for x in (a, b):
            d.line((x, 65, x, 75), fill='#6b645d')
        d.text(((a + b - d.textlength(label, font=small)) / 2, 44), label, font=small, fill='#6b645d')
    for x, y, n in [(180, 190, '1'), (1304, 267, '2'), (941, 648, '3'), (604, 77, '4')]:
        x, y = x + 40, y + 80
        d.ellipse((x - 11, y - 11, x + 11, y + 11), fill='#252f3d')
        d.text((x - 4, y - 10), n, font=font, fill='white')
    d.text((40, 837), '1  Duplicate session selection     2  10px / low-contrast metadata     3  24px action buttons     4  36px header + 40px tabs', font=small, fill='#252f3d')
    d.text((40, 858), 'Source screenshot is unmodified in captures/. Numbers and dimension lines are review annotations only.', font=small, fill='#6b645d')
    page.save(HERE / 'measurements.png', optimize=True)
    pairs = [('current-light-caption-panel', '#8b847d', '#f2efeb'),
             ('current-dark-caption-panel', '#737981', '#1a212b'),
             ('current-light-warning-white', '#b97a14', '#ffffff'),
             ('proposed-light-caption-panel', LIGHT['muted'], LIGHT['side']),
             ('proposed-light-caption-bar', LIGHT['muted'], LIGHT['bar']),
             ('proposed-dark-caption-panel', DARK['muted'], DARK['side']),
             ('proposed-light-warning-white', LIGHT['amber'], LIGHT['surface'])]
    (HERE / 'measurements.json').write_text(json.dumps({
        'units': 'logical pixels; capture at 1x',
        'current': {'window': [1344, 740], 'header': 36, 'sidebar': 208, 'inspector': 328,
                    'middle': 808, 'session_toolbar': 40, 'status': 24,
                    'settings_category_rail': 180, 'settings_middle_remaining': 628,
                    'transcript_gutters': [24, 20], 'composer_gutters': [20, 20],
                    'primary_button_height': 24, 'new_session_header_target': 20,
                    'wide_window': [1600, 900], 'wide_middle': 1064},
        'contrast': [dict(name=n, foreground=f, background=b, ratio=contrast(f, b)) for n, f, b in pairs],
        'note': 'Geometry checked against source and screenshot surface boundaries. Contrast uses flat theme colors, not antialiasing; not a complete accessibility audit.'
    }, indent=2) + '\n')


def main():
    OUT.mkdir(exist_ok=True)
    from screens import all_screens
    from board import build

    paths = [thread(), thread(True), thread(wide=True, expanded=True), settings(), compact()]
    paths.extend(all_screens(SVG, OUT))
    shells = sorted(Path.home().glob('.cache/ms-playwright/chromium_headless_shell-*/*/headless_shell'))
    browser = os.environ.get('CHROME') or (str(shells[-1]) if shells else None) or shutil.which('chromium') or shutil.which('chromium-browser')
    if not browser:
        raise SystemExit('SVGs written; Chromium is required for PNG previews.')
    with tempfile.TemporaryDirectory(prefix='pi-design-browser-') as profile:
        for path in paths:
            match = re.search(r'width="(\d+)" height="(\d+)"', path.read_text())
            width, height = map(int, match.groups())
            subprocess.run([browser, '--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage',
                            '--allow-file-access-from-files', '--hide-scrollbars', '--no-first-run',
                            '--disable-background-networking', f'--user-data-dir={profile}',
                            f'--window-size={width},{height}', '--force-device-scale-factor=1',
                            '--virtual-time-budget=1500', f'--screenshot={path.with_suffix(".png")}', path.as_uri()],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
            with Image.open(path.with_suffix('.png')) as image:
                assert image.size == (width, height)
            print(path.name)
    evidence()
    build(HERE, ROOT)


if __name__ == '__main__':
    main()
