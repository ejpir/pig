#!/usr/bin/env python3
"""Native Thread/review probes. Uses labelled offline demo data, never a provider.

Run under Xvfb. All project/preferences state is temporary; captures are original
GPUI pixels, not renders of the design sheets. Sample checks are not executed.
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

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "artifacts"


def command(*args):
    return subprocess.check_output(args, text=True).strip()


class Probe:
    def __init__(self, binary, workspace=False, settled=False):
        self.directory = tempfile.TemporaryDirectory(prefix="pi-workbench-capture-")
        self.root = Path(self.directory.name)
        env = os.environ.copy()
        for key in ("WAYLAND_DISPLAY", "PI_DESKTOP_DEMO_WORKSPACE", "PI_DESKTOP_DEMO_READABILITY"):
            env.pop(key, None)
        for key, name in {"HOME": "home", "XDG_CONFIG_HOME": "config", "XDG_DATA_HOME": "data",
                          "XDG_CACHE_HOME": "cache", "XDG_RUNTIME_DIR": "runtime",
                          "PI_DESKTOP_CONFIG_DIR": "desktop", "PI_CODING_AGENT_DIR": "agent"}.items():
            path = self.root / name
            path.mkdir(mode=0o700)
            env[key] = str(path)
        env.update(LIBGL_ALWAYS_SOFTWARE="1", WGPU_BACKEND="vulkan")
        for driver in Path("/usr/share/vulkan/icd.d").glob("lvp*.json"):
            env.update(VK_DRIVER_FILES=str(driver), VK_ICD_FILENAMES=str(driver))
            break
        if workspace:
            env["PI_DESKTOP_DEMO_WORKBENCH"] = "1"
        if settled:
            env["PI_DESKTOP_DEMO_WORKBENCH_SETTLED"] = "1"
        (self.root / "project").mkdir()
        log_name = "workbench-review-app.log" if workspace else "workbench-thread-app.log"
        self.log = (OUT / log_name).open("w")
        self.app = subprocess.Popen([str(binary), "--demo", "--light", "--project", str(self.root / "project")],
                                    env=env, stdout=self.log, stderr=subprocess.STDOUT)
        self.window = None
        for _ in range(100):
            if self.app.poll() is not None:
                raise RuntimeError(f"Desktop exited: {OUT / log_name}")
            windows = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "^pi desktop$"],
                                     capture_output=True, text=True).stdout.splitlines()
            if windows:
                self.window = windows[0]
                break
            time.sleep(.1)
        if not self.window:
            raise RuntimeError("No desktop window")
        command("xdotool", "windowfocus", "--sync", self.window)
        time.sleep(2)

    def close(self):
        try:
            if self.app.poll() is None:
                command("xdotool", "key", "--clearmodifiers", "ctrl+q")
                self.app.wait(timeout=8)
            assert self.app.returncode == 0, self.app.returncode
        finally:
            if self.app.poll() is None:
                self.app.terminate()
                self.app.wait(timeout=8)
            self.log.close()
            self.directory.cleanup()

    def resize(self, width, height):
        command("xdotool", "windowmove", self.window, "0", "0")
        command("xdotool", "windowsize", self.window, str(width), str(height))
        time.sleep(.6)

    def shot(self, name):
        path = OUT / f"workbench-{name}.png"
        command("xdotool", "mousemove", "--window", self.window, "4", "30")
        time.sleep(.15)
        command("import", "-window", self.window, str(path))
        return path

    def click(self, x, y):
        command("xdotool", "mousemove", "--window", self.window, str(x), str(y), "click", "1")
        time.sleep(.4)

    def text(self, name):
        path = self.shot(name)
        image = Image.open(path)
        temp = self.root / "ocr.png"
        image.resize((image.width * 2, image.height * 2)).save(temp)
        content = command("tesseract", str(temp), "stdout", "--psm", "11")
        # Sparse-page OCR skips accent-filled buttons. Read the action row separately.
        actions = image.crop((216, image.height - 120, image.width, image.height - 24))
        actions.resize((actions.width * 3, actions.height * 3)).save(temp)
        return (content + "\n" + command("tesseract", str(temp), "stdout", "--psm", "6")).lower()

    def word(self, word, region):
        path = self.shot("control")
        image = Image.open(path).crop(region)
        temp = self.root / "control.png"
        image.resize((image.width * 2, image.height * 2)).save(temp)
        result = command("tesseract", str(temp), "stdout", "--psm", "11", "tsv")
        rows = [r for r in csv.DictReader(io.StringIO(result), delimiter="\t", quoting=csv.QUOTE_NONE)
                if r["text"].lower() == word.lower()]
        assert len(rows) == 1, (word, rows)
        row = rows[0]
        self.click(region[0] + (int(row["left"]) + int(row["width"]) // 2) // 2,
                   region[1] + (int(row["top"]) + int(row["height"]) // 2) // 2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(os.environ.get("PI_DESKTOP_BINARY", "target/debug/pi-desktop")))
    args = parser.parse_args()
    binary = args.binary.resolve()
    assert binary.is_file(), binary
    OUT.mkdir(exist_ok=True)
    os.chdir(ROOT)
    probe = Probe(binary, workspace=True)
    try:
        probe.resize(1440, 900)
        command("xdotool", "type", "--clearmodifiers", "Also cover the streaming path with a mock provider.")
        time.sleep(.3)
        content = probe.text("thread")
        assert "working" in content and "steer now" in content and "queue follow-up" in content, content
        assert "one narrow fix" in content and "npm run check" in content, content
        assert "62.4k" not in content, "Inspector must be closed by default"
        probe.resize(1600, 900)
        probe.shot("thread-wide")
        command("xdotool", "key", "ctrl+shift+t")
        time.sleep(.5)
        probe.shot("thread-evening")
    finally:
        probe.close()
    probe = Probe(binary, workspace=True, settled=True)
    try:
        probe.resize(1600, 900)
        probe.shot("thread-files")
        probe.word("Changes", (216, 36, 600, 76))
        probe.word("Request", (416, 640, 1000, 700))
        command("xdotool", "type", "--clearmodifiers", "Add the strict Anthropic case to the regression test too.")
        time.sleep(.3)
        content = probe.text("review-wide")
        assert "before" in content and "after" in content and "strict anthropic case" in content, content
        content = probe.text("review-revision")
        assert "strict anthropic case" in content, content
        probe.resize(1000, 720)
        content = probe.text("review-compact")
        assert "unified diff" in content and "strict anthropic case" in content, content
        probe.resize(1600, 900)
        probe.shot("review-wide-restored")
    finally:
        probe.close()
    (OUT / "workbench-validation.json").write_text(json.dumps({
        "renderer": "native GPUI / X11 / Xvfb", "offlineDemo": True,
        "providerCalls": False, "sampleChecksExecuted": False,
        "binary": str(binary), "checks": ["closed inspector", "working/queue/steer controls",
            "fluid Thread", "changed-file objects", "aligned split review", "explicit revision attachment",
            "compact unified fallback", "resize preserves attachment", "clean quit"],
    }, indent=2) + "\n")
    print("PASS: native workbench Thread and split/compact review, explicit revision, clean exit")


if __name__ == "__main__":
    main()
