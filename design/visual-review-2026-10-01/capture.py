#!/usr/bin/env python3
"""Capture the real GPUI app with isolated demo/catalog data; never call a model.

From the repository root:
  xvfb-run -a -s '-screen 0 1920x1080x24' python3 design/visual-review-2026-10-01/capture.py
Needs a built desktop, xdotool, ImageMagick, Tesseract and Pillow.
"""
import argparse
import csv
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


def command(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True).strip()


class Capture:
    def __init__(self, binary, output, records):
        self.binary, self.output, self.records = binary, output, records

    def start(self, root, scene):
        env = os.environ.copy()
        for key in list(env):
            if key.startswith(('PI_', 'ANTHROPIC_', 'OPENAI_')) or key == 'WAYLAND_DISPLAY':
                env.pop(key)
        env.update(
            HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
            XDG_CONFIG_HOME=str(root / 'config'), XDG_CACHE_HOME=str(root / 'cache'),
            XDG_RUNTIME_DIR=str(root / 'runtime'), PI_CODING_AGENT_DIR=str(root / 'agent'),
            PI_DESKTOP_CONFIG_DIR=str(root / 'desktop'), LIBGL_ALWAYS_SOFTWARE='1',
            WGPU_BACKEND='vulkan', PI_OFFLINE='1', PI_SKIP_VERSION_CHECK='1',
        )
        for folder in ('home', 'runtime', 'agent', 'desktop', 'project'):
            (root / folder).mkdir()
        (root / 'runtime').chmod(0o700)
        (root / 'desktop/settings.json').write_text('{"appearance":{"theme":"moonstone"}}\n')
        drivers = sorted(Path('/usr/share/vulkan/icd.d').glob('lvp*.json'))
        if drivers:
            env.update(VK_DRIVER_FILES=str(drivers[0]), VK_ICD_FILENAMES=str(drivers[0]))
        args = [str(self.binary), '--project', str(root / 'project')]
        if scene == 'demo':
            args.append('--demo')
        else:
            env.update(PI_DESKTOP_PI=str(ROOT / 'fixtures/catalog-rpc.py'),
                       PI_CATALOG_LOG=str(root / 'commands.jsonl'))
        self.scene = scene
        self.log = (root / 'app.log').open('w')
        self.app = subprocess.Popen(args, env=env, cwd=ROOT, stdout=self.log, stderr=self.log)
        for _ in range(100):
            if self.app.poll() is not None:
                raise RuntimeError((root / 'app.log').read_text())
            try:
                self.window = command('xdotool', 'search', '--onlyvisible', '--name', '^pi desktop$').splitlines()[0]
                break
            except subprocess.CalledProcessError:
                time.sleep(0.1)
        else:
            raise RuntimeError('No app window')
        command('xdotool', 'windowfocus', '--sync', self.window)
        command('xdotool', 'windowmove', self.window, 0, 0)
        self.resize(1344, 740)
        time.sleep(3)

    def key(self, *keys):
        command('xdotool', 'key', '--clearmodifiers', *keys)
        time.sleep(0.4)

    def click(self, x, y):
        command('xdotool', 'mousemove', '--window', self.window, round(x), round(y))
        command('xdotool', 'click', 1)
        time.sleep(0.5)

    def resize(self, width, height):
        command('xdotool', 'windowsize', self.window, width, height)
        time.sleep(0.7)

    def shot(self, name):
        command('xdotool', 'mousemove', '--window', self.window, 6, 20)
        time.sleep(0.6)  # Let hover chrome and scrolling settle.
        path = self.output / f'{name}.png'
        command('import', '-window', self.window, path)
        with Image.open(path) as image:
            width, height = image.size
        self.records.append(dict(file=path.name, scene=self.scene, width=width, height=height))
        print(path.name, width, height, flush=True)

    def word(self, needle, box):
        """Locate painted labels instead of guessing old chrome coordinates."""
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'ocr.png'
            command('import', '-window', self.window, path)
            with Image.open(path) as frame:
                image = frame.crop(box)
                image.resize((image.width * 3, image.height * 3)).save(path)
            data = command('tesseract', path, 'stdout', '--psm', 11, 'tsv')
            rows = list(csv.DictReader(io.StringIO(data), delimiter='\t', quoting=csv.QUOTE_NONE))
            row = next((r for r in rows if r['text'].strip().lower().rstrip('…:.') == needle.lower()), None)
            if row is None:
                raise RuntimeError((needle, [r['text'] for r in rows if r['text'].strip()]))
            self.click(box[0] + (int(row['left']) + int(row['width']) / 2) / 3,
                       box[1] + (int(row['top']) + int(row['height']) / 2) / 3)

    def stop(self):
        if getattr(self, 'app', None):
            self.app.terminate()
            try:
                self.app.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.app.kill()
                self.app.wait()
        if getattr(self, 'log', None):
            self.log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path(os.environ.get('PI_DESKTOP_BINARY', ROOT / 'target/check/debug/pi-desktop')))
    parser.add_argument('--out', type=Path, default=HERE / 'captures')
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    records = []
    capture = Capture(args.binary.resolve(), output, records)
    with tempfile.TemporaryDirectory(prefix='pi-visual-demo-') as temporary:
        try:
            capture.start(Path(temporary), 'demo')
            capture.shot('01-thread-light')
            capture.key('ctrl+shift+t')
            capture.shot('02-thread-dark')
            capture.key('ctrl+shift+t', 'ctrl+k')
            capture.shot('03-search')
            command('xdotool', 'type', '--clearmodifiers', 'no-such-session')
            capture.shot('04-search-empty')
            capture.key('Escape')
            capture.click(330, 55)
            capture.shot('05-changes')
            capture.click(425, 55)
            capture.shot('06-tree')
            capture.click(495, 55)
            capture.shot('07-context')
            capture.click(250, 55)
            capture.key('ctrl+n')
            capture.shot('08-new-session')
            capture.word('worktree', (422, 160, 922, 530))
            capture.shot('09-new-worktree')
            capture.key('Escape')
            capture.resize(1600, 900)
            capture.shot('10-thread-wide')
            capture.resize(1000, 680)
            capture.shot('11-thread-compact')
            capture.click(978, 18)
            capture.click(978, 18)
            capture.shot('12-thread-compact-after-toggle')
            capture.resize(960, 600)
            capture.key('ctrl+n')
            capture.shot('13-new-session-minimum')
            capture.click(410, 328)
            capture.shot('13b-new-worktree-minimum')
            capture.key('Escape')
        finally:
            capture.stop()
    with tempfile.TemporaryDirectory(prefix='pi-visual-catalog-') as temporary:
        root = Path(temporary)
        try:
            capture.start(root, 'catalog')
            capture.shot('14-idle-session')
            capture.word('All', (0, 560, 208, 716))
            capture.shot('15-sessions')
            capture.word('Models', (0, 560, 208, 716))
            capture.word('atlas-small', (220, 230, 1010, 425))
            capture.shot('16-models')
            capture.word('Resources', (0, 560, 208, 716))
            capture.shot('17-resources-project')
            capture.click(252, 61)
            capture.word('Skills', (220, 85, 1010, 125))
            capture.word('review', (220, 120, 1010, 230))
            capture.shot('18-resources-skill')
            capture.word('Settings', (0, 560, 208, 716))
            capture.shot('19-settings')
            capture.word('Appearance', (208, 300, 390, 650))
            capture.shot('20-settings-appearance')
            capture.click(879, 141)  # Evening; changes only this temporary config.
            capture.shot('21-settings-dark')
            capture.click(950, 141)  # Moonstone.
            capture.resize(1000, 680)
            capture.shot('22-settings-compact')
            capture.click(978, 18)
            capture.click(978, 18)
            capture.shot('23-settings-compact-after-toggle')
            capture.word('Models', (0, 500, 208, 655))
            capture.shot('24-models-compact')
            capture.word('Resources', (0, 500, 208, 655))
            capture.shot('25-resources-compact')
            commands = [json.loads(line)['type'] for line in (root / 'commands.jsonl').read_text().splitlines()]
            assert not any(kind in {'prompt', 'bash', 'install_package', 'remove_package'} for kind in commands), commands
        finally:
            capture.stop()
    manifest = dict(revision=command('git', '-C', ROOT, 'rev-parse', 'HEAD'),
                    binary=str(args.binary.resolve()),
                    environment='Linux aarch64, Xvfb, 1x, lavapipe; isolated offline demo and catalog fixture',
                    captures=records)
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
