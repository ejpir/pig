"""Assemble every proposal into a genuine multi-screen vector SVG and gallery."""
from html import escape
import math
from pathlib import Path
import re

from PIL import Image, ImageDraw, ImageFont

SCREENS = [
    ('thread-light', 'Thread · Moonstone', 'Readable text, one open-session entry, a quieter contextual inspector.'),
    ('thread-dark', 'Thread · Evening', 'Neutral selection and stronger secondary text; keep the ink-blue surfaces.'),
    ('thread-wide', 'Thread · wide window', '1600 × 900. Fluid tools/composer, readable prose. Edit explicitly expanded, not opened by resizing.'),
    ('idle', 'Start a conversation', 'The same fluid composer and 24px gutters. Lead with the prompt, not project metadata.'),
    ('search', 'Global search · results', 'A distinct global query, keyboard selection and clear scope.'),
    ('search-empty', 'Global search · no matches', 'An explicit empty state; background navigation is not filtered away.'),
    ('new-session', 'New Session · project folder', 'Keep the compact 500px form; clarify inherited settings and active-folder overlap.'),
    ('new-worktree', 'New Session · worktree', 'Conditional branch/path fields, no unusable option, one fixed action footer.'),
    ('sessions', 'All Sessions', 'One primary action. Put infrequent export/share/delete operations under More.'),
    ('models', 'Models · selected model', 'Compact provider filters, legible rows, explicit current/selected/cycle states.'),
    ('compact-models', 'Models · compact details drawer', '1000 × 680. Requested details overlay; actions do not disappear or squeeze the table.'),
    ('resources', 'Resources · user packages', 'Consistent scope, tabs and resource title; one place for intentional package actions.'),
    ('resources-trust', 'Resources · project trust', 'One trust entry point with consequences visible; trust is not a sandbox.'),
    ('settings', 'Settings · model & thinking', 'Inline explanations. JSON keys and advanced settings remain available on demand.'),
    ('appearance-dark', 'Settings · appearance', 'Theme previews convey a real choice; do not add a dashboard of new settings.'),
    ('changes', 'Changes · observed diff', 'A 200px file rail, wider diff and one action row. Missing snapshots remain explicit.'),
    ('tree', 'Tree · selected branch entry', 'Keep the distinctive branch geometry; separate inspection from navigation.'),
    ('context', 'Context · usage & compaction', 'Sparse bars have a sensible width. A real switch replaces an action-looking toggle.'),
]


def build(here, root):
    """Called after all individual SVG/PNG proposals have been rendered."""
    out = here / 'concepts'
    width, stride, top = 2808, 884, 220
    height = top + math.ceil(len(SCREENS) / 2) * stride + 60
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
             '<title>Pi Desktop — 18-screen streamline design study</title>',
             '<desc>Proposed designs, not app screenshots. Eighteen vector screens covering the main views, dialogs, search states and compact layouts.</desc>',
             '''<style>
@font-face {font-family:Plex;src:url('../assets/fonts/IBMPlexSans-Regular.ttf')}
@font-face {font-family:Plex;src:url('../assets/fonts/IBMPlexSans-SemiBold.ttf');font-weight:600}
@font-face {font-family:Commit;src:url('../assets/fonts/CommitMono-Regular.otf')}
text {font-family:Plex,sans-serif}
</style>''',
             f'<rect width="{width}" height="{height}" fill="#ebe7e4"/>',
             '<text x="40" y="48" fill="#6b645d" font-size="17">PI DESKTOP · DESIGN STUDY · 1 OCTOBER 2026 · PROPOSALS, NOT IMPLEMENTED UI</text>',
             '<text x="40" y="109" fill="#252f3d" font-size="44" style="font-family:Georgia,DejaVu Serif,serif;font-style:italic">A quieter workbench — the whole app.</text>',
             '<text x="40" y="150" fill="#6b645d" font-size="20">18 vector screens · warm surfaces · legible text · one task per pane · no added title-bar clutter</text>',
             '<text x="40" y="184" fill="#6b645d" font-size="16">Zoom to inspect. Click a screen title for its individual SVG. Rationale and 26 actual Xvfb captures: visual-review-2026-10-01/README.md</text>']
    # The preview is composed from the same full-resolution renders, not an enormous browser viewport.
    image = Image.new('RGB', (width, height), '#ebe7e4')
    draw = ImageDraw.Draw(image)
    font_path = root / 'assets/fonts/IBMPlexSans-Regular.ttf'
    large = ImageFont.truetype(str(font_path), 42)
    title_font = ImageFont.truetype(str(font_path), 23)
    small = ImageFont.truetype(str(font_path), 17)
    draw.text((40, 22), 'PI DESKTOP · DESIGN STUDY · PROPOSED UI', font=small, fill='#6b645d')
    draw.text((40, 64), 'A quieter workbench — the whole app.', font=large, fill='#252f3d')
    draw.text((40, 132), '18 vector screens · open desktop-streamline-study.svg to zoom into the full study', font=title_font, fill='#6b645d')
    cards = []
    for index, (name, title, caption) in enumerate(SCREENS):
        x = 40 + (index % 2) * 1384
        y = top + (index // 2) * stride
        source = (out / f'{name}.svg').read_text()
        match = re.search(r'width="(\d+)" height="(\d+)"', source)
        w, h = map(int, match.groups())
        scale = min(1, 1344 / w, 760 / h)
        sx, sy = x + (1344 - w * scale) / 2, y + 42
        label = f'{index + 1:02d}  {title}'
        href = f'visual-review-2026-10-01/concepts/{name}.svg'
        parts += [f'<g id="screen-{name}">',
                  f'<a href="{href}"><text x="{x}" y="{y + 23}" fill="#252f3d" font-size="24">{escape(label)}</text></a>',
                  f'<text x="{x + 1344}" y="{y + 23}" fill="#6b645d" font-size="16" text-anchor="end">{w} × {h}</text>',
                  f'<g transform="translate({sx} {sy}) scale({scale})">',
                  source.replace('../../../assets/fonts/', '../assets/fonts/'),
                  '</g>',
                  f'<text x="{x}" y="{y + 830}" fill="#6b645d" font-size="17">{escape(caption)}</text>', '</g>']
        draw.text((x, y - 3), label, font=title_font, fill='#252f3d')
        with Image.open(out / f'{name}.png') as frame:
            frame = frame.convert('RGB').resize((round(w * scale), round(h * scale)), Image.Resampling.LANCZOS)
            image.paste(frame, (round(sx), round(sy)))
        draw.text((x, y + 810), caption, font=small, fill='#6b645d')
        cards.append(f'<figure><h2>{escape(label)}</h2><a href="concepts/{name}.svg"><img src="concepts/{name}.png" loading="lazy" alt="Proposed {escape(title)}"></a><figcaption>{escape(caption)} <a href="concepts/{name}.svg">SVG</a> · <a href="concepts/{name}.png">PNG</a></figcaption></figure>')
    parts.append('</svg>')
    (root / 'design/desktop-streamline-study.svg').write_text('\n'.join(parts) + '\n')
    image.resize((1680, round(height * 1680 / width)), Image.Resampling.LANCZOS).save(here / 'overview.png', optimize=True)
    (here / 'all-screens.html').write_text('''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Pi Desktop — all 18 proposed screens</title><style>
@font-face{font-family:Plex;src:url('../../assets/fonts/IBMPlexSans-Regular.ttf')}
*{box-sizing:border-box}body{margin:0;background:#faf9f7;color:#252f3d;font:15px/1.6 Plex,system-ui,sans-serif}
main{max-width:1800px;margin:auto;padding:32px}h1{font:italic 36px Georgia,serif}h2{font-size:17px;font-weight:500}
a{color:#4b607c;text-underline-offset:4px}a:focus-visible{outline:2px solid #81766b;outline-offset:4px}
.grid{display:grid;grid-template-columns:1fr 1fr;gap:32px}figure{margin:0;min-width:0}
img{display:block;width:100%;border:1px solid #ded8d1}figcaption{font-size:13px;color:#6b645d;margin-top:8px}
@media(max-width:1000px){.grid{grid-template-columns:1fr}}
</style></head><body><main><h1>A quieter workbench — all 18 screens.</h1>
<p>These are vector design proposals, not actual app captures. Click any screen to inspect its SVG at full size.</p>
<p><a href="../desktop-streamline-study.svg">Open the complete multi-screen SVG</a> · <a href="index.html">Before / after evidence</a> · <a href="README.md">Full review</a></p>
<div class="grid">''' + '\n'.join(cards) + '</div></main></body></html>\n')
    print('design/desktop-streamline-study.svg —', len(SCREENS), 'screens')
