"""Additional vector screens for the multi-screen streamline study.

All content is illustrative design material, not a rendered application or a
claim that a backend operation succeeded. Called by render.py.
"""
from pathlib import Path


def tabs(s, selected='Thread', inspector=True):
    end = s.width - 320 if inspector else s.width
    s.line(216, 75.5, end, 75.5)
    for x, label in [(240, 'Thread'), (317, 'Changes'), (416, 'Tree'), (482, 'Context')]:
        s.text(x, 60, label, 13, 'text' if label == selected else 'muted', 600 if label == selected else 400)
        if label == selected:
            s.line(x, 74, x + len(label) * 7, 74, 'secondary', 2)
    s.text(end - 94, 60, '~/repos/pi', 11, 'muted', family='Commit,monospace', anchor='end')
    s.icon('folder', end - 74, 48)
    s.icon('terminal', end - 36, 48)


def field(s, x, y, width, text):
    s.rect(x, y, width, 30, 'surface', 6, 'line')
    s.text(x + 12, y + 20, text, 12, 'muted')


def search_field(s, x, y, width, text='Filter…'):
    field(s, x, y, width, '')
    s.icon('magnifying_glass', x + 9, y + 8, size=14)
    s.text(x + 32, y + 20, text, 12, 'muted')


def scope(s, project=False, end=1024):
    s.rect(240, 44, 154, 28, 'bar', 6)
    s.rect(295 if project else 243, 47, 96 if project else 48, 22, 'surface', 4)
    s.text(267, 63, 'User', 12, anchor='middle')
    s.text(342, 63, 'Project · pi', 12, anchor='middle')
    s.line(216, 79.5, end, 79.5)


def inspector_title(s, title, subtitle):
    s.title(1044, 72, title)
    s.text(1044, 98, subtitle, 12, 'muted')
    s.line(1044, 120, 1324, 120)


def pair(s, y, title, value):
    s.text(1044, y, title, 13, 'secondary')
    s.text(1324, y, value, 13, anchor='end')


def switch(s, x, y, on=False):
    s.rect(x, y, 36, 20, 'accent' if on else 'line', 10)
    s.dot(x + (25 if on else 11), y + 10, 'surface', 7)


def idle(SVG):
    s = SVG()
    s.shell(inspector=False)
    tabs(s, inspector=False)
    left, right = 240, s.width - 24
    center = (216 + s.width) / 2
    s.text(center, 220, 'Ready in pi', 12, 'muted', anchor='middle')
    s.text(center, 270, 'What are we working on?', 30, family='Georgia,DejaVu Serif,serif', italic=True, anchor='middle')
    s.text(center, 303, 'Write a prompt, or choose a starting point below.', 14, 'muted', anchor='middle')
    s.rect(left, 337, right - left, 140, 'surface', 8, 'line', extra='data-layout="composer"')
    s.text(left + 16, 371, 'Ask Pi…', 15, 'muted')
    s.text(left + 16, 396, 'Type / for commands or @ to include context.', 12, 'muted')
    s.icon('attach', left + 15, 439)
    s.text(left + 49, 452, '/', 16, 'muted')
    s.text(left + 80, 451, 'atlas-large  ⌄', 12, family='Commit,monospace')
    s.dot(left + 237, 446, 'amber')
    s.text(left + 248, 451, 'high  ⌄', 12, 'muted')
    s.button(right - 95, 431, 78, 'Send ↵', True)
    s.button(left, 499, 164, '/skill:review')
    s.button(left + 176, 499, 164, '/release-notes')
    s.disclosure(left, 574, 'Available in this project', '1 skill · 1 template · 2 commands', end=right)
    s.text(left, 614, 'File history is off. Existing edits cannot be restored.', 12, 'muted')
    return s.save('idle')


def changes(SVG):
    s = SVG()
    s.shell(inspector=False)
    tabs(s, 'Changes', inspector=False)
    s.line(416, 76, 416, 716)
    s.text(236, 109, 'Files', 12, 'muted', 600)
    s.text(397, 109, '1', 12, 'muted', anchor='end')
    s.rect(224, 127, 184, 60, 'selected', 6)
    s.icon('file', 235, 141, size=14)
    s.text(257, 153, 'openai-completions.ts', 12)
    s.text(257, 174, '+3 −1', 12, 'green')
    s.text(440, 109, 'openai-completions.ts', 16, family='Commit,monospace')
    s.text(440, 134, 'packages/ai/src/providers', 12, 'muted', family='Commit,monospace')
    s.button(969, 90, 128, 'Open in editor')
    s.button(1109, 90, 96, 'Copy diff')
    s.button(1217, 90, 103, 'Wrap · on')
    s.rect(440, 156, 880, 58, 'queue', 6)
    s.icon('info', 454, 169, 'amber', 16)
    s.text(480, 179, 'Observed tool edit · no snapshot', 13, weight=600)
    s.text(480, 200, 'Shell or external edits may be missing. Restore and undo are unavailable for this edit.', 12, 'muted')
    s.text(454, 249, 'edit · edit-1', 12, 'muted', family='Commit,monospace')
    code = [('211', ' ', 'if (block.type === "thinking") {', None),
            ('212', '−', '  if (!block.signature) throw new MissingSignature(model.id);', '#f1e4df'),
            ('212', '+', '  if (!block.signature && isAnthropic(model)) {', '#e0ebe3'),
            ('213', '+', '    throw new MissingSignature(model.id);', '#e0ebe3'),
            ('214', '+', '  }', '#e0ebe3'),
            ('215', ' ', '}', None)]
    s.rect(440, 266, 880, 172, 'surface', 6, 'line')
    for i, (number, sign, text, fill) in enumerate(code):
        y = 278 + i * 25
        if fill:
            s.rect(441, y - 1, 878, 25, fill)
        s.text(454, y + 17, number, 12, 'muted', family='Commit,monospace')
        s.text(492, y + 17, sign, 13, 'muted', family='Commit,monospace')
        s.text(518, y + 17, text, 13, 'secondary', family='Commit,monospace')
    s.text(440, 472, '1 tool-reported edit · shown in call order', 12, 'muted')
    s.text(1320, 690, 'Details available in the inspector', 12, 'muted', anchor='end')
    return s.save('changes')


def tree(SVG):
    s = SVG()
    s.shell()
    tabs(s, 'Tree')
    for x, w, label in [(240, 75, 'Default'), (323, 83, 'No tools'), (414, 84, 'User only'), (506, 78, 'Labeled'), (592, 51, 'All')]:
        if label == 'Default':
            s.rect(x, 90, w, 28, 'selected', 6)
        s.text(x + w / 2, 109, label, 12, anchor='middle')
    s.text(1000, 109, '14 entries · 2 branches', 12, 'muted', anchor='end')
    labels = ['Compacted · 148.2k tokens before',
              'Qwen3.8-flash returns empty thinking signatures on OpenCode…',
              'Accept empty signatures for OpenCode; keep Anthropic strict.',
              'edit  openai-completions.ts  +3 −1',
              'bash  npm run check — checks passed',
              'Add a regression test against the live OpenCode endpoint',
              'Added a test using a real key — the live endpoint branch',
              'bash  vitest — missing key',
              'Add a regression test with the faux provider, no network',
              'Added a faux-provider case to openai-completions.test.ts.',
              'edit  openai-completions.test.ts  +24',
              'The regression test passes without network access.']
    s.line(252, 151, 252, 590, 'line', 2)
    s.parts.append(f'<path d="M252 307 Q252 322 276 338 L276 435" fill="none" stroke="{s.c["line"]}" stroke-width="2"/>')
    for i, text in enumerate(labels):
        y = 158 + i * 38
        branch = i in (5, 6, 7)
        if i == 6:
            s.rect(232, y - 24, 768, 34, 'selected', 6)
        s.dot(276 if branch else 252, y - 5, 'amber' if i == 0 else 'muted' if branch else 'accent', 4)
        s.text(304 if branch else 280, y, text, 13, 'secondary')
        s.text(989, y, ['09:30', '09:41', '09:42', '✓', '✓', '09:52', '09:54', '×', '09:58', '09:59', '✓', '10:01'][i], 11, 'muted', anchor='end')
    s.dot(248, 664, 'accent')
    s.text(260, 669, 'Current path', 12, 'muted')
    s.dot(377, 664, 'muted')
    s.text(389, 669, 'Other branch', 12, 'muted')
    s.text(240, 698, 'Select an entry to inspect it. Continuing from it is a separate action.', 12, 'muted')
    inspector_title(s, 'Live endpoint test', 'Other branch · 09:54')
    pair(s, 156, 'Role', 'Pi')
    pair(s, 190, 'Output', '3.1k tokens')
    pair(s, 224, 'Tool calls', '0')
    s.line(1044, 248, 1324, 248)
    for i, text in enumerate(['Added openai-completions.test.ts calling', 'OpenCode with a real key and asserting', 'the empty signature is accepted.']):
        s.text(1044, 279 + i * 22, text, 13, 'secondary')
    s.disclosure(1044, 367, 'Branch summary', 'Not recorded', end=1324)
    s.line(1024, 558, 1344, 558)
    s.text(1044, 584, 'Wait for the current run before navigating.', 12, 'muted')
    s.rect(1044, 602, 280, 30, 'selected', 6)
    s.text(1184, 622, 'Continue from here', 12, 'muted', anchor='middle')
    s.button(1044, 644, 174, 'Fork into new session…')
    s.button(1230, 644, 94, 'Label…')
    return s.save('tree')


def context(SVG):
    s = SVG()
    s.shell()
    tabs(s, 'Context')
    for i, (title, value, detail) in enumerate([('Context', '62.4k', 'of 200k · 31%'), ('Session cost', '$0.41', 'whole session'), ('Cache read', '82%', 'of input tokens'), ('Compactions', '1', 'on this path')]):
        x = 240 + i * 196
        s.text(x, 112, title, 12, 'muted')
        s.text(x, 147, value, 26, weight=600)
        s.text(x, 172, detail, 12, 'muted')
        if i < 3:
            s.line(x + 173, 98, x + 173, 178)
    s.line(240, 202, 1000, 202)
    s.text(240, 233, 'Tokens per response', 14, weight=600)
    for x, color, name in [(648, 'accent', 'Cache read'), (767, 'secondary', 'Input'), (850, 'amber', 'Output')]:
        s.rect(x, 224, 8, 8, color, 2)
        s.text(x + 16, 233, name, 12, 'muted')
    for y, label in [(274, '60k'), (334, '30k'), (394, '0')]:
        s.text(267, y, label, 11, 'muted', anchor='end')
        s.line(282, y - 4, 1000, y - 4)
    for i, heights in enumerate([(57, 12, 3), (80, 17, 4), (107, 24, 5)]):
        x, bottom = 302 + i * 54, 390
        for height, color in zip(heights, ['accent', 'secondary', 'amber']):
            bottom -= height
            s.rect(x, bottom, 28, height, color)
        s.text(x + 14, 419, str(i + 1), 12, 'muted', anchor='middle')
    s.text(240, 453, '3 responses · current branch', 12, 'muted')
    s.line(240, 477, 1000, 477)
    s.disclosure(240, 510, 'Reported context sources', 'Skills, observed tools and history', end=1000)
    s.text(262, 539, 'The exact request payload is not exposed by Pi. These are reported sources, not a prompt manifest.', 12, 'muted')
    s.line(240, 564, 1000, 564)
    s.disclosure(240, 599, 'Compacted at 09:30', '148.2k tokens before', end=1000)
    s.text(262, 628, 'Original entries remain in the session file.', 12, 'muted')
    inspector_title(s, 'Context', '31% of the context window used')
    s.text(1044, 155, 'Auto-compaction', 14, weight=600)
    switch(s, 1288, 140, True)
    s.text(1044, 184, 'Saved for future runs too.', 12, 'muted')
    s.line(1044, 212, 1324, 212)
    s.text(1044, 247, 'Compact now', 14, weight=600)
    s.text(1044, 272, 'Optional instructions', 12, 'muted')
    s.rect(1044, 285, 280, 92, 'surface', 6, 'line')
    s.text(1056, 310, 'Keep provider decisions and the test plan…', 12, 'muted')
    s.button(1044, 393, 280, 'Compact…', True)
    s.text(1044, 451, 'Uses the selected model to summarize.', 12, 'muted')
    s.text(1044, 472, 'Wait for active runs to finish.', 12, 'muted')
    s.line(1044, 500, 1324, 500)
    s.disclosure(1044, 535, 'Retries', 'None observed', end=1324)
    s.disclosure(1044, 575, 'Prompt cache', end=1324)
    return s.save('context')


def sessions(SVG):
    s = SVG()
    s.shell('All Sessions', sample=False)
    s.button(240, 44, 77, 'All · 3', h=28)
    s.text(334, 63, 'This project · 3', 12, 'muted')
    search_field(s, 563, 44, 278, 'Find a session…')
    s.button(857, 44, 143, 'Recent  ⌄', h=28)
    s.line(216, 79.5, 1024, 79.5)
    for x, label in [(240, 'Session'), (684, 'Project'), (806, 'Messages'), (935, 'Modified')]:
        s.text(x, 112, label, 12, 'muted')
    rows = [('Fix this language server error…', '5', 'Tue'), ('A named session', '30', 'Tue'), ('Run a loop', '12', 'Tue')]
    for i, (name, count, age) in enumerate(rows):
        y = 130 + i * 64
        if i == 0:
            s.rect(224, y, 792, 60, 'selected', 6)
        s.icon('chat', 240, y + 15)
        s.text(268, y + 27, name, 14, weight=600 if i == 0 else 400)
        s.text(268, y + 47, 'Saved session', 12, 'muted')
        s.text(684, y + 33, 'pi', 13, 'muted')
        s.text(847, y + 33, count, 12, 'muted', anchor='end')
        s.text(980, y + 33, age, 12, 'muted', anchor='end')
        s.line(240, y + 63, 1000, y + 63)
    s.text(240, 691, '3 saved sessions · selecting a row does not resume it', 12, 'muted')
    s.title(1044, 72, 'Fix this language')
    s.title(1044, 103, 'server error…')
    s.text(1044, 133, 'Closed · pi', 12, 'muted')
    s.line(1044, 155, 1324, 155)
    pair(s, 190, 'Messages', '5')
    pair(s, 224, 'Modified', 'Sep 29 · 12:00')
    pair(s, 258, 'Forked from', '—')
    s.text(1044, 306, 'First message', 12, 'muted', 600)
    for i, line in enumerate(['Fix this language server error in', 'file.ts:21:34:', 'Cannot find name agentStartMs.', 'Keep this original message.']):
        s.text(1044, 335 + i * 23, line, 13, 'secondary')
    s.disclosure(1044, 454, 'Session file', end=1324)
    s.line(1024, 576, 1344, 576)
    s.button(1044, 596, 280, 'Resume session', True)
    s.button(1044, 638, 88, 'Rename…')
    s.button(1140, 638, 88, 'Fork…')
    s.button(1236, 638, 88, 'More  ⌄')
    s.text(1044, 696, 'Export, share and delete are under More.', 12, 'muted')
    return s.save('sessions')


def models(SVG):
    s = SVG()
    s.shell('Models', sample=False)
    s.text(240, 64, 'Configured models', 13, 'muted')
    s.button(818, 44, 83, 'Log in…', h=28)
    s.button(913, 44, 87, 'Refresh', h=28)
    s.line(216, 79.5, 1024, 79.5)
    s.button(240, 94, 165, 'All providers  ⌄')
    s.button(417, 94, 152, 'All capabilities  ⌄')
    search_field(s, 696, 94, 304, 'Find a model…')
    for x, label in [(240, 'Cycle'), (300, 'Model / provider'), (633, 'Context'), (733, 'Input'), (888, 'In / out · $/1M')]:
        s.text(x, 158, label, 12, 'muted')
    data = [('Atlas Large', 'atlas-large · fixture-ai', '200k', 'Text, image', '$3 / $15'),
            ('Atlas Small', 'atlas-small · fixture-ai', '128k', 'Text', '$0.25 / $1.25'),
            ('Local Code', 'local-code · fixture-local', '64k', 'Text', '$0 / $0'),
            ('Unknown limits', 'unknown-limits · fixture-local', '—', '—', '—')]
    for i, row in enumerate(data):
        y = 176 + i * 66
        if i == 1:
            s.rect(224, y, 792, 62, 'selected', 6)
        s.text(254, y + 34, '★' if i in (0, 2) else '☆', 15, 'amber' if i in (0, 2) else 'muted')
        s.text(300, y + 27, row[0], 14, weight=600 if i == 1 else 400)
        s.text(300, y + 48, row[1], 12, 'muted')
        if i == 0:
            s.text(479, y + 27, 'Current session', 11, 'muted')
        s.text(633, y + 35, row[2], 12)
        s.text(733, y + 35, row[3], 12)
        s.text(988, y + 35, row[4], 12, anchor='end')
        s.line(240, y + 65, 1000, y + 65)
    s.text(240, 690, '4 configured models · stars mark the saved cycle · unknown values stay unknown', 12, 'muted')
    inspector_title(s, 'Atlas Small', 'fixture-ai / atlas-small')
    s.text(1044, 153, 'OAuth · stored', 12, 'muted')
    pair(s, 199, 'Context window', '128k')
    pair(s, 233, 'Max output', '16k')
    pair(s, 267, 'Input', 'Text')
    pair(s, 301, 'Reasoning', 'Not supported')
    s.line(1044, 325, 1324, 325)
    pair(s, 360, 'Input / output', '$0.25 / $1.25')
    s.text(1044, 386, 'USD per million tokens', 12, 'muted')
    s.disclosure(1044, 428, 'More pricing details', end=1324)
    s.text(1044, 477, 'Include in model cycle', 13)
    switch(s, 1288, 463)
    s.line(1024, 574, 1344, 574)
    s.button(1044, 593, 280, 'Use in this session', True)
    s.button(1044, 635, 280, 'Use and set as default…')
    s.text(1044, 695, 'Inspecting a model never runs a prompt.', 12, 'muted')
    return s.save('models')


def resource_tabs(s, selected='Packages'):
    for x, label, count in [(240, 'Packages', '3'), (360, 'Extensions', '1'), (497, 'Skills', '1'), (586, 'Prompts', '1'), (697, 'Context files', '—')]:
        s.text(x, 111, f'{label}  {count}', 13, 'text' if label == selected else 'muted')
        if label == selected:
            s.line(x, 124, x + 90, 124, 'secondary', 2)
    s.line(216, 127, 1024, 127)


def resources(SVG):
    s = SVG()
    s.shell('Resources', sample=False)
    scope(s)
    search_field(s, 607, 44, 251, 'Filter resources…')
    s.button(870, 44, 130, '+ Install…', h=28)
    resource_tabs(s)
    for i, (name, origin) in enumerate([('git-guard', 'npm:@fixture/git-guard@1.4.0'), ('pi-review', 'git:example.invalid/pi-review@v1'), ('local-kit', './local-kit')]):
        y = 140 + i * 70
        if i == 0:
            s.rect(224, y, 792, 66, 'selected', 6)
        s.icon('box', 240, y + 20)
        s.text(271, y + 27, name, 14, weight=600 if i == 0 else 400)
        s.text(271, y + 50, origin, 12, 'muted', family='Commit,monospace')
        s.text(988, y + 36, 'Reported', 12, 'muted', anchor='end')
        s.line(240, y + 69, 1000, y + 69)
    s.text(240, 690, 'User resources · reported by the selected session', 12, 'muted')
    inspector_title(s, 'git-guard', 'Package · user scope')
    s.text(1044, 155, 'Source', 12, 'muted', 600)
    s.text(1044, 182, 'npm:@fixture/git-guard@1.4.0', 12, family='Commit,monospace')
    s.text(1044, 222, 'Installed path', 12, 'muted', 600)
    s.text(1044, 249, '/offline/packages/git-guard', 12, family='Commit,monospace')
    s.line(1044, 277, 1324, 277)
    s.text(1044, 308, 'Reported commands', 12, 'muted', 600)
    s.text(1044, 340, '/commit', 13, family='Commit,monospace')
    s.text(1044, 366, '/check', 13, family='Commit,monospace')
    s.disclosure(1044, 413, 'Package details', end=1324)
    s.text(1044, 465, 'Inspect source before loading code.', 12, 'muted')
    s.text(1044, 486, 'Project trust is not an OS sandbox.', 12, 'muted')
    s.line(1024, 574, 1344, 574)
    s.button(1044, 593, 280, 'Reload resources…')
    s.button(1044, 635, 280, 'Remove package…')
    s.text(1044, 695, 'Changes require an explicit action.', 12, 'muted')
    return s.save('resources')


def trust(SVG):
    s = SVG()
    s.shell('Resources', sample=False)
    scope(s, project=True)
    s.text(1000, 63, '~/repos/pi', 12, 'muted', family='Commit,monospace', anchor='end')
    s.rect(240, 100, 760, 96, 'queue', 8)
    s.icon('info', 256, 118, 'amber')
    s.text(284, 132, 'Project code is not loaded', 15, weight=600)
    s.text(284, 158, 'Trust this folder only if you are comfortable running its extensions and settings.', 12, 'muted')
    s.text(284, 179, 'Context files may still load without project trust.', 12, 'muted')
    s.text(240, 231, 'Saved trust and the running process can differ. Trust is not a security sandbox.', 12, 'muted')
    s.button(240, 250, 164, 'Review trust…', True)
    s.button(416, 250, 155, 'Inspect project files')
    s.line(240, 309, 1000, 309)
    s.text(240, 343, 'Project resources', 14, weight=600)
    s.text(240, 375, 'No project extensions, skills or prompts reported in this process.', 13, 'muted')
    s.disclosure(240, 427, 'Project settings', 'Not reported', end=1000)
    inspector_title(s, 'pi', 'Project · not trusted in this process')
    s.text(1044, 156, 'Folder', 12, 'muted', 600)
    s.text(1044, 184, '~/repos/pi', 12, family='Commit,monospace')
    pair(s, 232, 'Open sessions', '1')
    pair(s, 266, 'Working now', '0')
    pair(s, 300, 'Saved decision', 'None reported')
    s.line(1044, 328, 1324, 328)
    s.text(1044, 361, 'What trust means', 14, weight=600)
    for i, line in enumerate(['Project code can run with your permissions.', 'Revoking trust affects new processes;', 'it does not unload code already running.']):
        s.text(1044, 391 + i * 23, line, 12, 'muted')
    s.disclosure(1044, 498, 'Reported resources', end=1324)
    return s.save('resources-trust')


def appearance(SVG):
    s = SVG(dark=True)
    s.shell('Settings', inspector=False, sample=False)
    scope(s, end=1344)
    search_field(s, 878, 44, 322, 'Find a setting…')
    s.button(1212, 44, 108, 'Open JSON', h=28)
    s.line(392, 80, 392, 716)
    s.text(236, 111, 'Pi', 12, 'muted', 600)
    for i, name in enumerate(['Model & thinking', 'Interaction', 'Tools', 'Sessions & context', 'Compaction', 'Branch summaries', 'Terminal & display', 'Network & retries', 'Shell', 'Resources', 'Updates & telemetry']):
        s.text(236, 140 + i * 28, name, 13)
    s.text(236, 483, 'Pi Desktop', 12, 'muted', 600)
    for i, name in enumerate(['General', 'Appearance', 'jj', 'Language servers', 'Editor', 'Terminal']):
        y = 514 + i * 28
        if name == 'Appearance':
            s.rect(224, y - 20, 159, 28, 'selected', 6)
        s.text(236, y, name)
    s.title(424, 125, 'Appearance')
    s.text(424, 157, 'A warm page by day. A quiet workbench at night.', 14, 'muted')
    for i, (name, bg, sidebar, ink) in enumerate([('System', '#ebe7e4', '#f2efeb', '#706961'), ('Evening', '#161d27', '#1a212b', '#959ca5'), ('Moonstone', '#faf9f7', '#f2efeb', '#706961')]):
        x = 424 + i * 301
        s.rect(x, 193, 276, 202, 'selected' if name == 'Evening' else 'side', 8, 'line')
        s.rect(x + 12, 205, 252, 138, bg, 5)
        s.rect(x + 12, 205, 252, 15, sidebar, 5)
        s.rect(x + 12, 220, 48, 123, sidebar)
        for j in range(3):
            s.rect(x + 22, 235 + j * 14, 26, 3, ink, 1)
        for j, width in enumerate([126, 156, 109]):
            s.rect(x + 73, 238 + j * 14, width, 3, ink, 1)
        s.rect(x + 73, 304, 174, 25, sidebar, 4)
        s.text(x + 16, 373, name, 14, weight=600 if name == 'Evening' else 400)
        if name == 'Evening':
            s.icon('check', x + 242, 359)
    s.text(424, 432, 'Evening', 14, weight=600)
    s.text(507, 432, 'Changed in user settings', 12, 'muted')
    s.text(424, 468, 'System follows your operating system’s appearance.', 13, 'muted')
    s.text(424, 493, 'Ctrl+Shift+T temporarily switches themes until the next launch.', 13, 'muted')
    s.line(424, 530, 1304, 530)
    s.disclosure(424, 568, 'Setting details', 'appearance.theme', end=1304)
    s.button(424, 605, 140, 'Reset to System')
    s.text(424, 691, 'Pi Desktop preferences · never read by the agent', 12, 'muted')
    return s.save('appearance-dark')


def backdrop(SVG, output):
    s = SVG()
    # Reuse the vector thread—not a screenshot—under transient overlays.
    s.parts.append((Path(output) / 'thread-light.svg').read_text())
    return s


def search(SVG, output, empty=False):
    s = backdrop(SVG, output)
    s.rect(0, 36, 1344, 680, '#111820', extra='opacity="0.12"')
    s.rect(372, 72, 600, 440, 'surface', 10, 'line')
    s.icon('magnifying_glass', 392, 90)
    s.text(420, 105, 'no-such-session' if empty else 'Search sessions and actions…', 15, 'text' if empty else 'muted')
    s.text(951, 104, 'Esc', 12, 'muted', anchor='end')
    s.line(373, 122, 971, 122)
    if empty:
        s.title(672, 220, '')
        s.text(672, 230, 'No matches for “no-such-session”', 18, anchor='middle')
        s.text(672, 264, 'Try a session, project or action name.', 13, 'muted', anchor='middle')
        s.button(604, 298, 136, 'Clear search')
    else:
        s.text(392, 149, 'Actions & views', 12, 'muted', 600)
        for i, (glyph, label, hint) in enumerate([('plus', 'New session', 'Ctrl+N'), ('folder', 'Open folder…', 'Ctrl+O'), ('list_tree', 'All Sessions', ''), ('sparkle', 'Models', ''), ('box', 'Resources', ''), ('settings', 'Settings', '')]):
            y = 162 + i * 36
            if i == 0:
                s.rect(384, y, 576, 32, 'selected', 6)
            s.icon(glyph, 394, y + 8)
            s.text(424, y + 22, label, 14)
            s.text(946, y + 22, hint, 12, 'muted', anchor='end')
        s.line(392, 390, 952, 390)
        s.text(392, 414, 'Open sessions', 12, 'muted', 600)
        s.icon('chat', 394, 435)
        s.text(424, 449, 'Qwen signatures', 14)
        s.text(947, 449, 'pi', 12, 'muted', anchor='end')
    s.line(373, 480, 971, 480)
    s.text(392, 502, '↑↓ Move     Enter Open     Esc Close', 12, 'muted')
    s.text(952, 502, 'Global search', 12, 'muted', anchor='end')
    return s.save('search-empty' if empty else 'search')


def new_session(SVG, output, worktree=False):
    s = backdrop(SVG, output)
    s.rect(0, 0, 1344, 740, '#111820', extra='opacity="0.24"')
    height = 616 if worktree else 468
    y = (740 - height) / 2
    s.rect(422, y, 500, height, 'surface', 10, 'line')
    s.title(446, y + 43, 'New session')
    s.icon('plus', 852, y + 27, size=16)
    s.icon('close', 884, y + 27, size=16)
    s.text(446, y + 86, 'Project', 12, 'muted', 600)
    for i, name in enumerate(['pi', 'zed', 'minivm']):
        yy = y + 101 + i * 40
        if i == 0:
            s.rect(446, yy, 452, 36, 'selected', 6)
        s.parts.append(f'<circle cx="460" cy="{yy + 18}" r="6" fill="none" stroke="{s.c["muted"]}"/>')
        if i == 0:
            s.dot(460, yy + 18, 'text', 3)
        s.icon('folder', 477, yy + 10)
        s.text(503, yy + 23, name, 14, weight=600 if i == 0 else 400)
        s.text(570, yy + 23, '~/repos/' + name, 12, 'muted')
        if i == 0:
            s.text(883, yy + 23, '1 working', 12, 'amber', anchor='end')
    s.text(446, y + 246, 'Run in', 12, 'muted', 600)
    s.rect(446, y + 256, 308, 30, 'bar', 6)
    s.rect(599 if worktree else 449, y + 259, 152, 24, 'surface', 4)
    s.text(524, y + 276, 'Project folder', 12, anchor='middle')
    s.text(673, y + 276, 'New git worktree', 12, anchor='middle')
    if worktree:
        s.text(446, y + 315, 'Branch', 12, 'muted')
        field(s, 446, y + 325, 452, 'pi/my-task')
        s.text(446, y + 380, 'Path', 12, 'muted')
        field(s, 446, y + 390, 452, 'Absolute path for the new worktree')
        s.text(446, y + 448, 'This session edits a separate working folder.', 12, 'muted')
        model_y = y + 485
    else:
        s.icon('info', 446, y + 303, 'amber')
        s.text(471, y + 315, 'Another session is working in this folder.', 12, 'amber')
        s.text(471, y + 336, 'Use a worktree to keep file changes separate.', 12, 'muted')
        model_y = y + 359
    s.text(446, model_y, 'Initial model', 12, 'muted', 600)
    s.button(446, model_y + 12, 230, 'Pi configured default  ⌄')
    s.text(695, model_y + 32, 'Thinking · inherited', 12, 'muted')
    s.line(423, y + height - 56, 921, y + height - 56)
    s.button(698, y + height - 42, 76, 'Cancel')
    s.button(786, y + height - 42, 112, 'Create session', True)
    return s.save('new-worktree' if worktree else 'new-session')


def all_screens(SVG, output):
    return [idle(SVG), changes(SVG), tree(SVG), context(SVG), sessions(SVG),
            models(SVG), resources(SVG), trust(SVG), appearance(SVG),
            search(SVG, output), search(SVG, output, empty=True),
            new_session(SVG, output), new_session(SVG, output, worktree=True)]
