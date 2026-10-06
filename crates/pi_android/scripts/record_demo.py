#!/usr/bin/env python3
"""Record a walkthrough of the --ui-test APK on a connected phone.

The sample sessions run in real time while the story is tapped through as a
person would, with no fixture reloads in between. Only sample sessions are
shown, never real ones. Saves demo.mp4 and a smaller, sped-up demo-share.mp4.
"""
import argparse
from pathlib import Path
import re
import subprocess
import time

from test_interactions import ACTIVITY, Phone

REMOTE = "/sdcard/pi-demo.mp4"
# The shared cut plays this much faster than it was recorded.
SPEED = 1.35


class Demo:
    def __init__(self, phone):
        self.phone = phone

    def link(self, url):
        self.phone.run("shell", "am", "start", "-n", ACTIVITY, "-a", "android.intent.action.VIEW", "-d", url)

    def find(self, key, wait=4):
        """A control's bounds as soon as it is drawn, without waiting for the
        screen to settle."""
        deadline = time.monotonic() + wait
        while True:
            state = self.phone.state()
            if key in state["bounds"] or time.monotonic() > deadline:
                return state["scale"], state["bounds"][key]
            time.sleep(0.1)

    def press_if_shown(self, key):
        try:
            scale, (left, top, width, height) = self.find(key, wait=0.6)
        except KeyError:
            return
        self.touch(scale, left + width / 2, top + height / 2)

    def press(self, key, x=0.5, y=0.5):
        scale, (left, top, width, height) = self.find(key)
        self.touch(scale, left + width * x, top + height * y)

    def press_at(self, key, dx, dy):
        """Taps a point inside a probed area, for rows that are not probed."""
        scale, (left, top, _, _) = self.find(key)
        self.touch(scale, left + dx, top + dy)

    def touch(self, scale, x, y):
        self.phone.run("shell", "input", "tap", round(x * scale), round(y * scale))

    def drag(self, points, duration):
        scale = self.phone.state()["scale"]
        self.phone.run("shell", "input", "swipe", *(round(value * scale) for value in points), duration)

    def type(self, text):
        self.phone.run("shell", "input", "text", text.replace(" ", "%s"))

    def back(self):
        self.phone.key("KEYCODE_BACK")


def launcher_icon(phone, label="Pi UI tests"):
    """Where the app's icon is on the phone's home screen, in pixels."""
    phone.key("KEYCODE_HOME")
    time.sleep(1.2)
    phone.run("shell", "uiautomator", "dump", "/sdcard/demo-ui.xml")
    dump = phone.run("exec-out", "cat", "/sdcard/demo-ui.xml").decode()
    phone.run("shell", "rm", "/sdcard/demo-ui.xml")
    # The first is the personal profile's; a work profile's copy follows.
    found = re.search(rf'content-desc="{label}"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', dump)
    if not found:
        raise SystemExit(f"Put the {label} icon on the home screen first")
    left, top, right, bottom = map(int, found.groups())
    return (left + right) // 2, top + (bottom - top) * 2 // 5


def story(demo, icon):
    phone = demo.phone
    # From the home screen into Pi: sessions working, one waiting for an answer.
    time.sleep(1)
    phone.run("shell", "input", "tap", *icon)
    time.sleep(1.6)

    # Find a session, and see the computers Pi runs on.
    demo.press('Name("search")')
    time.sleep(0.5)
    demo.type("aurora")
    time.sleep(1)
    demo.back()
    demo.back()
    time.sleep(0.5)
    demo.press("computers")
    time.sleep(1.2)
    demo.back()
    time.sleep(0.5)

    # A session writing a file, streaming live, and that step up close.
    demo.press("session-row-2")
    time.sleep(1.3)
    demo.press("turn-0-stage-1")
    time.sleep(1.8)
    demo.back()
    time.sleep(0.4)
    demo.back()
    time.sleep(0.5)

    # Pi asks before running the tests; allow it and watch them pass.
    demo.press('NamedInteger("answer", 1)')
    time.sleep(1.2)
    demo.press_at("question-card", 107, 165)  # "Allow once"
    time.sleep(0.3)
    demo.press('Name("answer")')
    time.sleep(1.8)
    demo.link("pi://test/advance/8")
    time.sleep(1.6)

    # Review the change and ask about two of its lines.
    demo.press('Name("review")')
    time.sleep(1)
    for below_top in (222, 266):  # lines 211 and 212
        demo.press_at('scroll:NamedInteger("review", 1)', 192, below_top)
        time.sleep(0.25)
    demo.press("draft")
    time.sleep(0.3)
    demo.type("Also accept it for DeepSeek")
    time.sleep(0.4)
    demo.press("send")
    time.sleep(0.8)
    demo.press_if_shown('Name("latest")')
    time.sleep(1.6)
    demo.back()
    time.sleep(0.5)

    # An answer with code and a diagram, and a conversation that carries on.
    demo.press("session-row-5")
    time.sleep(1.2)
    demo.drag((192, 600, 192, 330), 500)
    time.sleep(1.2)
    for prompt in ("How do I switch it to K2?", "What changes with K2?"):
        demo.press("draft")
        time.sleep(0.3)
        demo.type(prompt)
        time.sleep(0.3)
        demo.press("send")
        time.sleep(0.6)
        demo.press_if_shown('Name("latest")')
        time.sleep(5.6)
    demo.back()
    time.sleep(0.5)

    # A screenshot Pi took, zoomed, and the page it made, live.
    demo.press("session-row-7")
    time.sleep(1)
    demo.press("tool-image-0-0")
    time.sleep(1.2)
    scale, (left, top, width, height) = demo.find("bottom-sheet")
    middle = f"input tap {round((left + width / 2) * scale)} {round((top + height / 2) * scale)}"
    phone.run("shell", f"{middle}; {middle}")
    time.sleep(1)
    demo.drag((left + width * 0.3, top + height / 2, left + width * 0.7, top + height / 2), 500)
    time.sleep(0.6)
    demo.back()
    time.sleep(0.5)
    demo.press('Name("page-0-0")')
    time.sleep(1.4)
    demo.drag((60, 330, 320, 520), 600)
    demo.drag((320, 600, 80, 380), 600)
    time.sleep(0.6)
    demo.back()
    time.sleep(0.6)
    demo.back()
    time.sleep(0.5)

    # What a run cost, and the file history Pi keeps.
    demo.press("session-row-4")
    time.sleep(0.9)
    demo.press('Name("details")')
    time.sleep(1.4)
    demo.press("open-jj-history")
    time.sleep(1.4)
    demo.back()
    time.sleep(0.4)
    demo.back()
    time.sleep(0.4)
    demo.press('Name("settings")')
    time.sleep(1.3)
    demo.back()
    time.sleep(0.5)

    # Something new: pick a project, peek at a file, and pick a model.
    demo.press("new-session")
    time.sleep(0.8)
    demo.press('Name("project")')
    time.sleep(1)
    scale, (left, top, width, height) = demo.find('scroll:Name("project-tree")')
    demo.drag((left + width / 2, top + height * 0.8, left + width / 2, top + height * 0.3), 400)
    time.sleep(0.8)
    demo.press('Name("node:~/repos/pi/README.md")')
    time.sleep(1.4)
    demo.back()
    time.sleep(0.5)
    demo.press('Name("use-folder")')
    time.sleep(0.6)
    demo.press("composer-model")
    time.sleep(1.8)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    phone = Phone(args.serial, args.output)
    demo = Demo(phone)
    phone.fixture("sessions")
    demo.link("pi://test/play")
    icon = launcher_icon(phone)

    # screenrecord stops by itself after three minutes.
    recorder = subprocess.Popen(phone.adb + ["shell", "screenrecord", "--bit-rate", "12000000", REMOTE])
    time.sleep(0.8)
    try:
        story(demo, icon)
    finally:
        phone.run("shell", "pkill", "-INT", "screenrecord")
        recorder.wait(timeout=20)
        time.sleep(1)
    video = args.output / "demo.mp4"
    phone.run("pull", REMOTE, video)
    phone.run("shell", "rm", REMOTE)
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", video,
                    "-vf", f"setpts=PTS/{SPEED},fps=60,scale=720:-2",
                    "-c:v", "libx264", "-crf", "26", "-preset", "slow", "-pix_fmt", "yuv420p",
                    "-movflags", "+faststart", "-an", args.output / "demo-share.mp4"], check=True)
    print(f"Saved {video} and {args.output / 'demo-share.mp4'}")


if __name__ == "__main__":
    main()
