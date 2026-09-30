#!/usr/bin/env python3
"""Compare actual Xvfb pixels with the supplied rendered SVG, not a redrawn mock."""
import json
from pathlib import Path
from PIL import Image, ImageDraw

artifacts=Path("artifacts")
reference=Image.open("design/desktop-views-study-light.png").convert("RGB").crop((1456,234,2800,974))
actual=Image.open(artifacts/"views-tree.png").convert("RGB")
assert actual.size==reference.size==(1344,740)
reference.save(artifacts/"tree-study-reference.png")
# The sidebar is deliberately not compared: the user asked to preserve the newer one.
roi=(208,52,1344,716)
comparison=Image.new("RGB",(2272,696),"white")
draw=ImageDraw.Draw(comparison)
draw.text((12,8),"SUPPLIED STUDY (Tree content)",fill="black")
draw.text((1148,8),"ACTUAL GPUI / XVFB (real RPC fields or explicitly offline data)",fill="black")
comparison.paste(reference.crop(roi),(0,32))
comparison.paste(actual.crop(roi),(1136,32))
comparison.save(artifacts/"tree-study-comparison.png")

def rail(image):
    # Between the nodes, the current path should be continuous through the branch.
    ys=[y for y in range(170,552) if abs((y-162+18)%36-18)>7]
    counts={x:sum(image.getpixel((x,y))[2]-image.getpixel((x,y))[0]>20 for y in ys) for x in range(240,264)}
    x=max(counts,key=counts.get)
    return {"x":x,"coverage":round(counts[x]/len(ys),3)}
results={"reference_current_path":rail(reference),"actual_current_path":rail(actual),"sidebar":"preserved, not compared to older study","limitations":"Data text, unavailable summary/token details, and platform fonts are not pixel-equality assertions."}
(artifacts/"tree-study-comparison.json").write_text(json.dumps(results,indent=2)+"\n")
print(json.dumps(results,indent=2))
assert abs(results["reference_current_path"]["x"]-results["actual_current_path"]["x"])<=2
assert results["actual_current_path"]["coverage"]>=0.95,"Current path is not continuously connected"
