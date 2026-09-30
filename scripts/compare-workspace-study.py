#!/usr/bin/env python3
"""Compare actual GPUI/Xvfb windows with the supplied studies, not redrawings."""
from pathlib import Path
import json
from PIL import Image, ImageDraw

out = Path('artifacts')
views = Image.open('design/desktop-views-study-light.png').convert('RGB')
workspace = Image.open('design/desktop-workspace-study.png').convert('RGB')
references = {
    'landing': views.crop((48, 234, 1392, 974)),
    'thread': workspace.crop((1456, 268, 2800, 1008)),
    'changes': workspace.crop((1456, 1168, 2800, 1908)),
}
# Keep the current sidebar: compare only session content and the inspector.
roi = (208, 52, 1344, 716)
width, height = 1136, 664
sheet = Image.new('RGB', (width * 2, (height + 30) * 3), '#e6e1d7')
draw = ImageDraw.Draw(sheet)
natives = {}
for i, (name, reference) in enumerate(references.items()):
    native = Image.open(out / f'layout-{name}.png').convert('RGB')
    assert native.size == (1344, 740), native.size
    natives[name] = native
    reference.save(out / f'layout-{name}-reference.png')
    y = i * (height + 30)
    draw.text((10, y + 8), f'Supplied study: {name}', fill='black')
    draw.text((width + 10, y + 8), f'Native GPUI: {name}', fill='black')
    sheet.paste(reference.crop(roi), (0, y + 30))
    sheet.paste(native.crop(roi), (width, y + 30))
sheet.save(out / 'layout-study-comparison.png')

def mark(image):
    points = [(x, y) for y in range(100, 180) for x in range(500, 700)
              if image.getpixel((x, y)) == (240, 144, 130)]
    assert points, 'No native/study hero mark'
    return [min(x for x, _ in points), min(y for _, y in points),
            max(x for x, _ in points), max(y for _, y in points)]

def divider(image):
    return min(range(430, 461), key=lambda x: sum(image.getpixel((x, 510))))

brand_ref, brand_native = mark(references['landing']), mark(natives['landing'])
assert max(abs(a - b) for a, b in zip(brand_ref, brand_native)) <= 1
split_ref, split_native = divider(references['changes']), divider(natives['changes'])
assert abs(split_ref - split_native) <= 1
canvas = natives['thread'].getpixel((220, 500))
assert all(natives['thread'].getpixel((234, y)) == canvas for y in range(224, 290)), 'Tool rows regained a card border/background'
result = {'landing_mark': {'study': brand_ref, 'native': brand_native},
          'changes_divider_x': {'study': split_ref, 'native': split_native},
          'tool_rows_borderless_probe': True,
          'scope': 'Specific geometry checks, not full pixel equality; fixtures, fonts, honest unavailable fields and requested usage additions differ.'}
(out / 'layout-study-comparison.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
