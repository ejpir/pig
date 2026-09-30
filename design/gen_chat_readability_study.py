#!/usr/bin/env python3
"""A visual proposal only. Uses the user's screenshot text, not live session data."""
from html import escape
from pathlib import Path
from PIL import ImageFont

fonts = Path(__file__).resolve().parent.parent / 'assets' / 'fonts'
regular = ImageFont.truetype(str(fonts / 'IBMPlexSans-Regular.ttf'), 16)
semibold = ImageFont.truetype(str(fonts / 'IBMPlexSans-SemiBold.ttf'), 16)

W, H = 1600, 1128
out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">',
       '<title>Pi chat readability — visual proposal, not implemented</title>']
C = dict(page='#ebe7e4', canvas='#faf9f7', panel='#f2efeb', line='#e3ddd7',
         text='#252f3d', muted='#77716a', accent='#4b607c', green='#2e8a55', code='#426581')

def rect(x,y,w,h,fill,stroke=None,r=0):
    out.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}"'+(f' stroke="{stroke}"' if stroke else '')+'/>')

def text(x,y,value,size=16,color=None,weight=400,mono=False):
    family='CommitMonoV143' if mono else 'IBM Plex Sans'
    out.append(f'<text x="{x}" y="{y}" font-family="{family}" font-size="{size}" font-weight="{weight}" fill="{color or C["text"]}">{escape(value)}</text>')

def rich(x,y,prefix,strong,suffix):
    text(x,y,prefix)
    x += regular.getlength(prefix)
    text(x,y,strong,weight=600)
    x += semibold.getlength(strong)
    x += regular.getlength(' ') * (len(suffix) - len(suffix.lstrip()))
    text(x,y,suffix.lstrip())

def line(x,y,w):
    rect(x,y,w,1,C['line'])

def small(x,y,value):
    text(x,y,value,12,C['muted'],mono=True)

def code(x,y,value,size=14):
    rect(x-4,y-size+1,len(value)*size*.6+8,size+7,'#eef0f2',r=3)
    text(x,y,value,size,C['code'],mono=True)

def activity(y,value):
    text(180,y,'›',18,C['muted'])
    text(203,y,value,13,C['muted'])
    text(866,y,'✓',14,C['green'])

def user(y,value):
    rect(176,y,720,52,'#ffffff','#d3ccc5',7)
    text(192,y+32,value,16)

rect(0,0,W,H,C['page'])
small(32,30,'VISUAL PROPOSAL / NOT IMPLEMENTED')
text(32,69,'Let the answers lead.',30,weight=600)
text(32,94,'Your screenshot’s content, with quieter activity and a comfortable reading width.',15,C['muted'])

# Reading view: a wide window, but a bounded text column.
rect(32,118,1008,976,C['canvas'],'#cbc3bb',10)
text(58,150,'THREAD',12,C['text'],mono=True)
text(151,150,'CHANGES',12,C['muted'],mono=True)
text(250,150,'TREE',12,C['muted'],mono=True)
text(315,150,'CONTEXT',12,C['muted'],mono=True)
small(814,150,'andromeda-sdk')
line(32,170,1008)
rect(58,168,57,2,C['accent'])

activity(204,'Tool activity · complete')
rich(176,240,'Added an ','API Server Storage Size',' time-series panel to the')
text(176,265,'Andromeda Grafana dashboard.',16)
text(188,303,'•  Queries',16)
code(274,303,'apiserver_storage_size_bytes')
text(188,332,'•  Displays values in bytes, grouped by API server instance',16)
rich(188,361,'•  Added an ','API Server',' dashboard section')
text(188,390,'•  Incremented dashboard version to 9',16)
text(188,419,'•  Updated snapshot',16)
text(176,450,'Validation: all 7 snapshot tests pass.',15,C['text'])

user(488,'bytes seems a little too big, maybe we should use gigs?')
activity(574,'4 tool calls · complete')
text(176,614,'Updated the panel to display GiB:',16)
text(188,652,'•  Query:',16)
code(264,652,'apiserver_storage_size_bytes / 1024^3')
text(188,681,'•  Grafana unit:',16)
code(308,681,'gbytes')
text(188,710,'•  Snapshot updated',16)
text(188,739,'•  All 7 snapshot tests pass.',16)

user(777,'create a PR for this')
text(180,861,'›',18,C['muted'])
text(203,861,'Read  and-pr/SKILL.md',13,C['muted'],mono=True)
text(176,891,'Thinking…',14,C['accent'])

rect(176,930,720,126,'#ffffff','#aebac9',9)
text(192,961,'Ask pi to work on this project…',15,C['muted'])
small(192,1033,'＋    /     gpt-5.6-sol  ▾     ● high  ▾')
rect(807,1014,72,27,C['accent'],r=5)
text(821,1033,'Send ↵',12,'#ffffff',600)
small(176,1080,'ONE LIVE STATUS WHILE WORKING · NO REPEATED THINKING ROWS')

# Expanded state: the raw activity stays available, in its original order.
small(1080,140,'ON DEMAND / EXPANDED ACTIVITY')
rect(1080,164,488,379,C['canvas'],'#cbc3bb',8)
text(1100,194,'⌄  4 tool calls · complete',14,C['muted'])
line(1100,211,448)
text(1100,240,'Edit',13,C['muted'])
text(1160,240,'…/andromeda-operator-dashboard.json',11,C['text'],mono=True)
text(1490,264,'+2 −2',11,C['green'],mono=True)
text(1100,298,'⌄ Bash',13,C['muted'])
text(1160,298,'npm run test:snapshot …',12,C['text'],mono=True)
text(1531,298,'✓',13,C['green'])
rect(1100,314,448,70,'#eef0f2',r=4)
text(1112,339,'npm run test:snapshot -- -u --runInBand',12,C['text'],mono=True)
text(1112,361,'&& npm run test:snapshot -- --runInBand',12,C['text'],mono=True)
text(1100,416,'Bash',13,C['muted'])
text(1160,416,'git diff --check && git status --short',11,C['text'],mono=True)
text(1531,416,'✓',13,C['green'])
text(1100,451,'Bash',13,C['muted'])
text(1160,451,'rg … dashboard.json tests/snapshot/…',11,C['text'],mono=True)
text(1531,451,'✓',13,C['green'])
line(1100,473,448)
text(1100,503,'Original calls and output remain selectable and copyable.',12,C['muted'])
text(1100,523,'Thinking disclosures live here, not between every call.',12,C['muted'])

text(1080,594,'Why this is easier to scan',21,weight=600)
for y,num,title,lines in [
    (639,'01','A bounded reading column',[
        'The window can be wide; the prose does not have to be.',
        'Keep text, tools and composer on the same 720px lane.']),
    (746,'02','Answers outweigh machinery',[
        'Completed activity becomes one quiet disclosure.',
        'Keep live progress visible; do not hide errors.']),
    (853,'03','Space creates a turn boundary',[
        'More breathing room before each new user message.',
        'No assistant cards, badges or extra visual containers.']),
]:
    small(1080,y,num)
    text(1120,y,title,16,weight=600)
    for i,value in enumerate(lines):
        text(1120,y+26+i*22,value,13,C['muted'])

line(1080,962,488)
text(1080,992,'No AI rewriting or inferred summaries.',15,weight=600)
text(1080,1018,'Preserve message text and chronology. Group only adjacent',13,C['muted'])
text(1080,1040,'tool/thinking events; intermediate explanations stay visible.',13,C['muted'])
small(32,1114,'LAYOUT STUDY ONLY · CONTENT FROM YOUR SCREENSHOT · ACTIVE STATE IS ILLUSTRATIVE')
out.append('</svg>')
Path(__file__).with_name('chat-readability-study.svg').write_text('\n'.join(out)+'\n')
