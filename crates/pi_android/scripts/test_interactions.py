#!/usr/bin/env python3
"""Repeatable touch/keyboard regression checks for the separate --ui-test APK.

Only deterministic sample fixtures are acted on. Reads app-private, fixture-only
telemetry using run-as; never uses an activity launch to inspect input state.
Screenshots, assertions and a JSON report are saved for every run.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import time
import struct
import uuid
import xml.etree.ElementTree as ET
import zlib

PACKAGE = "dev.pi.android.uitest"
ACTIVITY = f"{PACKAGE}/dev.pi.gpui.GpuiActivity"


class Phone:
    def __init__(self, serial, output):
        self.adb = ["adb", "-s", serial]
        self.output = output
        self.output.mkdir(parents=True, exist_ok=True)
        self.results = []
        self.fixture_name = None

    def run(self, *args):
        return subprocess.check_output(self.adb + list(map(str, args)), stderr=subprocess.STDOUT, timeout=20)

    def state(self):
        state = json.loads(self.run("exec-out", "run-as", PACKAGE, "cat", "files/ui-test-state.json"))
        if self.fixture_name and state["fixture"] == self.fixture_name:
            assert state["sample"], "Refusing to act on anything except sample sessions"
        return state

    def wait(self, predicate, description, timeout=12):
        deadline = time.monotonic() + timeout
        latest = None
        while time.monotonic() < deadline:
            try:
                latest = self.state()
                if predicate(latest):
                    self.results.append({"check": description, "passed": True})
                    print(f"PASS {description}", flush=True)
                    return latest
            except (subprocess.CalledProcessError, json.JSONDecodeError):
                pass
            time.sleep(0.2)
        self.capture("failure")
        self.output.joinpath("failure-state.json").write_text(json.dumps(latest, indent=2) + "\n")
        self.results.append({"check": description, "passed": False})
        summary = {key: value for key, value in (latest or {}).items() if key != "bounds"}
        raise AssertionError(f"{description}; last state: {summary}; bounds saved in failure-state.json")

    def fixture(self, name):
        try:
            previous = self.state()["time"]
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            previous = 0
        self.fixture_name = name
        self.run("shell", "am", "start", "-n", ACTIVITY, "-a", "android.intent.action.VIEW", "-d", f"pi://preview/{name}")
        self.wait(lambda state: state["fixture"] == name and state["time"] > previous, f"loaded {name}")
        return self.settled_state()

    def settled_state(self):
        # InputMethodManager reports hidden before the resize animation finishes.
        # Target two fresh, equal layouts, never a pre-IME telemetry snapshot.
        deadline = time.monotonic() + 8
        after = time.time()
        previous = None
        while time.monotonic() < deadline:
            state = self.state()
            if state["time"] >= after:
                if previous and state["bounds"] == previous["bounds"]:
                    return state
                previous = state
                after = state["time"] + 0.001
            time.sleep(0.15)
        raise AssertionError("The app layout did not settle")

    def bounds(self, name, index=None, state=None):
        state = state or self.state()
        candidates = [(key, bounds) for key, bounds in state["bounds"].items()
                      if (key == name or f'"{name}"' in key)
                      and (index is None or re.search(rf",\s*{index}\)$", key))]
        if len(candidates) != 1:
            raise AssertionError(f"Expected one control {name}/{index}, got {[key for key, _ in candidates]}")
        return candidates[0][1], state["scale"]

    def tap(self, name, index=None):
        (x, y, width, height), scale = self.bounds(name, index, self.settled_state())
        self.run("shell", "input", "tap", round((x + width / 2) * scale), round((y + height / 2) * scale))

    def swipe(self, start, end, duration=450):
        scale = self.state()["scale"]
        self.run("shell", "input", "swipe", *(round(value * scale) for value in (*start, *end)), duration)

    def key(self, key):
        self.run("shell", "input", "keyevent", key)

    def text(self, text):
        # The smoke text deliberately contains only shell-safe ASCII characters.
        if not re.fullmatch(r"[A-Za-z0-9_.-]+", text):
            raise ValueError("Use fixture data for non-ASCII or multi-line input")
        self.run("shell", "input", "text", text)

    def keyboard(self):
        dump = self.run("shell", "dumpsys", "input_method").decode()
        return bool(re.search(r"(?:mInputShown|isInputViewShown)=true", dump))

    def wait_keyboard(self, visible):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            if self.keyboard() == visible:
                self.results.append({"check": f"keyboard visible={visible}", "passed": True})
                return
            time.sleep(0.2)
        self.capture("keyboard-failure")
        raise AssertionError(f"Keyboard visibility did not become {visible}")

    def capture(self, name):
        self.output.joinpath(f"{name}.png").write_bytes(self.run("exec-out", "screencap", "-p"))

    def native_nodes(self):
        dump = self.run("exec-out", "uiautomator", "dump", "/dev/tty").decode()
        xml = dump[dump.index("<?xml"):]
        xml = xml[:xml.index("</hierarchy>") + len("</hierarchy>")]
        return list(ET.fromstring(xml).iter("node"))

    def native_tap(self, label, timeout=12):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            matches = [node for node in self.native_nodes()
                       if label in (node.get("text"), node.get("content-desc"))]
            if matches:
                x1, y1, x2, y2 = map(int, re.findall(r"\d+", matches[0].get("bounds")))
                self.run("shell", "input", "tap", (x1 + x2) // 2, (y1 + y2) // 2)
                return
            time.sleep(0.2)
        self.capture("native-failure")
        raise AssertionError(f"System picker control not found: {label}")

    def report(self):
        self.output.joinpath("report.json").write_text(json.dumps(self.results, indent=2) + "\n")


def input_and_selectors(phone):
    phone.fixture("start")
    phone.wait_keyboard(False)
    phone.tap("draft")
    phone.wait_keyboard(True)
    phone.text("Hello")
    phone.key("KEYCODE_ENTER")
    phone.text("world")
    phone.wait(lambda s: s["draft_chars"] == 11, "short multiline draft has exactly 11 characters")
    phone.capture("short-input")
    phone.key("KEYCODE_BACK")
    phone.wait_keyboard(False)
    phone.tap("composer-model")
    phone.wait(lambda s: s["sheet"] == "Model", "model picker opens independently")
    phone.wait_keyboard(False)
    phone.tap("model-search")
    phone.wait_keyboard(True)
    phone.text("Sonnet")
    phone.wait(lambda s: s["model_search_chars"] == 6, "model search accepts typing")
    phone.key("KEYCODE_BACK")
    phone.wait_keyboard(False)
    time.sleep(1.1)  # allow resized control bounds to be published
    phone.tap("model", 0)
    phone.wait(lambda s: s["sheet"] is None and s["model"] == "Sonnet 5.5" and s["draft_chars"] == 11, "model selection preserves the draft")
    phone.tap("composer-thinking")
    phone.wait(lambda s: s["sheet"] == "Thinking", "thinking has its own sheet")
    phone.wait_keyboard(False)
    phone.tap("thinking", 2)
    phone.wait(lambda s: s["sheet"] is None and s["thinking"] == "Low" and s["draft_chars"] == 11, "thinking selection preserves the draft")


def gboard_typing(phone):
    """English, portrait four-row Gboard (no number row or suggestion toolbar).

    Real touchscreen keys exercise InputConnection composition. Bounds come
    from keyboard occlusion, not this device's pixel dimensions. Other keyboard
    layouts can run the non-IME cases without changing their system settings.
    """
    method = phone.run("shell", "settings", "get", "secure", "default_input_method").decode()
    assert "com.google.android.inputmethod.latin" in method, "The ime case requires Gboard"
    before = phone.fixture("start")
    viewport, _ = phone.bounds('scroll:Name("start")', state=before)
    bottom = viewport[1] + viewport[3]
    phone.tap("draft")
    phone.wait_keyboard(True)
    shown = phone.settled_state()
    viewport, scale = phone.bounds('scroll:Name("start")', state=shown)
    top = viewport[1] + viewport[3]
    width = shown["viewport"][0]
    assert bottom - top > 100, "Keyboard must occlude the lower viewport"
    positions = {letter: ((i + 0.5) / 10, 0.125) for i, letter in enumerate("qwertyuiop")}
    positions.update({letter: ((i + 1) / 10, 0.375) for i, letter in enumerate("asdfghjkl")})
    positions.update({letter: ((i + 2) / 10, 0.625) for i, letter in enumerate("zxcvbnm")})
    positions.update({" ": (0.5, 0.875), "delete": (0.925, 0.625)})

    def touch(letter):
        x, y = positions[letter]
        phone.run("shell", "input", "tap", round(width * x * scale), round((top + (bottom - top) * y) * scale))

    for letter in "hello":
        touch(letter)
    phone.wait(lambda s: s["draft_chars"] == 5, "Gboard touch composition produces five characters once")
    touch(" ")
    phone.wait(lambda s: s["draft_chars"] == 6, "Gboard commits the composing word with space")
    touch("delete")
    touch("delete")
    phone.wait(lambda s: s["draft_chars"] == 4, "Gboard deletion re-enters the previous word without duplicating it")
    touch("o")
    phone.wait(lambda s: s["draft_chars"] == 5, "Gboard resumes editing the composing word")
    phone.capture("gboard-hello")
    phone.key("KEYCODE_BACK")
    phone.wait_keyboard(False)
    phone.tap("composer-model")
    phone.wait(lambda s: s["sheet"] == "Model", "model opens after a real IME composition")
    phone.tap("model-search")
    phone.wait_keyboard(True)
    phone.settled_state()
    for letter in "sonnet":
        touch(letter)
    phone.wait(lambda s: s["model_search_chars"] == 6 and s["draft_chars"] == 5,
               "new search input cannot inherit the previous field's composing word")
    phone.capture("gboard-search")
    phone.fixture("start")
    phone.wait_keyboard(False)
    phone.wait(lambda s: s["draft_chars"] == 0, "replaced draft cannot be repopulated by a retired IME connection")


def long_input_and_stop(phone):
    state = phone.fixture("follow-up-input")
    count = state["draft_chars"]
    assert count > 9000
    phone.tap("draft")
    phone.wait_keyboard(True)
    phone.text("XYZ")
    phone.wait(lambda s: s["draft_chars"] == count + 3, "long Unicode follow-up accepts an edit without lost or duplicated text")
    phone.capture("long-follow-up-edited")
    phone.key("KEYCODE_BACK")
    phone.wait_keyboard(False)

    phone.fixture("working")
    phone.tap("draft")
    phone.wait_keyboard(True)
    phone.text("KeepMyDraft")
    phone.wait(lambda s: s["draft_chars"] == 11, "draft is ready while the session runs")
    phone.key("KEYCODE_BACK")
    phone.wait_keyboard(False)
    time.sleep(1.1)
    phone.tap("stop")
    phone.wait(lambda s: s["state"] == "Stopped" and s["draft_chars"] == 11, "toolbar Stop stops the sample and keeps the unsent draft")
    phone.capture("stopped-with-draft")


def model_scroll(phone):
    phone.fixture("model-long-list")
    phone.wait_keyboard(False)
    before, _ = phone.bounds("model-search")
    viewport, _ = phone.bounds('scroll:Name("model-list")')
    x, y, width, height = viewport
    phone.swipe((x + width / 2, y + height * 0.8), (x + width / 2, y + height * 0.2))
    phone.wait(lambda s: s["sheet_scroll"][0] < -100, "long model list scrolls")
    after, _ = phone.bounds("model-search")
    assert abs(before[1] - after[1]) < 1, "Search must stay visible while scrolling models"
    phone.capture("model-list-scrolled")
    state = phone.settled_state()
    thumb, _ = phone.bounds('thumb:Name("model-list")', state=state)
    tx, ty, tw, th = thumb
    phone.swipe((tx + tw / 2, ty + th / 2), (tx + tw / 2, y + height * 0.85), duration=650)
    phone.wait(lambda s: s["sheet"] == "Model" and -s["sheet_scroll"][0] > s["sheet_scroll"][1] * 0.5,
               "scrollbar drags the long model list without dismissing the sheet")


def images(phone):
    phone.fixture("image-only")
    phone.wait(lambda s: s["draft_images"] == 1 and s["draft_chars"] == 0, "image-only draft retains image bytes")
    phone.tap("attachment-0")
    phone.wait(lambda s: (s["sheet"] or "").startswith("Image("), "thumbnail opens full image preview")
    phone.capture("image-preview")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda s: s["sheet"] is None and s["draft_images"] == 1, "closing image preview keeps the attachment")
    phone.tap("send")
    phone.wait(lambda s: s["route"].startswith("Thread(") and s["draft_images"] == 0, "image-only prompt submits through the shared composer")


def conversation(phone):
    phone.fixture("done")
    viewport, _ = phone.bounds('scroll:NamedInteger("thread", 1)')
    x, y, width, height = viewport
    phone.swipe((x + width / 2, y + height * 0.2), (x + width / 2, y + height * 0.8))
    phone.tap("turn-0-activity")
    phone.wait(lambda s: s["state"] == "Done" and s["expanded_activities"] == 1,
               "completed activity rail expands")
    phone.tap("turn-0-stage-0")
    phone.wait(lambda s: (s["sheet"] or "").startswith("Activity("),
               "completed stage opens its detailed activity sheet")
    phone.capture("completed-activity")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda s: s["sheet"] is None and s["state"] == "Done",
               "activity details close without changing the conversation")

    phone.fixture("tool-output")
    state = phone.settled_state()
    close_before, _ = phone.bounds("close-sheet", state=state)
    viewport = next(bounds for key, bounds in state["bounds"].items()
                    if key.startswith("scroll:") and "sheet-body-Activity" in key)
    x, y, width, height = viewport
    phone.swipe((x + width / 2, y + height * 0.8), (x + width / 2, y + height * 0.2))
    phone.wait(lambda s: (s["sheet"] or "").startswith("Activity(") and s["sheet_scroll"][0] < -100,
               "long bash output scrolls inside the details sheet")
    close_after, _ = phone.bounds("close-sheet", state=phone.settled_state())
    assert abs(close_after[1] - close_before[1]) < 1, "The close button must remain in the sheet header"
    phone.capture("long-tool-output-scrolled")
    phone.tap("close-sheet")
    phone.wait(lambda s: s["sheet"] is None, "long output can be closed without scrolling back to the top")

    phone.fixture("long-reply")
    state = phone.settled_state()
    viewport, _ = phone.bounds('scroll:NamedInteger("thread", 1)', state=state)
    thumb, _ = phone.bounds('thumb:NamedInteger("thread", 1)', state=state)
    x, y, width, height = viewport
    tx, ty, tw, th = thumb
    phone.swipe((tx + tw / 2, ty + th / 2), (tx + tw / 2, y + height * 0.2), 650)
    state = phone.settled_state()
    thumb, _ = phone.bounds('thumb:NamedInteger("thread", 1)', state=state)
    assert thumb[1] < y + height * 0.5, "Long replies must allow returning to earlier text"
    phone.capture("long-reply-reading")
    phone.tap("latest")

    def at_latest(state):
        viewport, _ = phone.bounds('scroll:NamedInteger("thread", 1)', state=state)
        thumb, _ = phone.bounds('thumb:NamedInteger("thread", 1)', state=state)
        return abs((thumb[1] + thumb[3]) - (viewport[1] + viewport[3])) < 5

    phone.wait(at_latest, "Latest reply returns to the end of the complete long answer")


def projects(phone):
    state = phone.fixture("projects")
    sessions = state["sessions"]
    phone.tap("project", 1)
    phone.wait(lambda s: s["route"] == "Start" and s["sessions"] == sessions,
               "choosing a recent project opens the composer without starting work")
    state = phone.fixture("project-empty")
    phone.tap("use-folder")
    phone.wait(lambda s: s["route"] == "Start" and s["sessions"] == state["sessions"],
               "choosing an empty folder does not create a session")
    phone.fixture("projects")
    phone.tap("view-sessions")
    phone.wait(lambda s: s["route"] == "Sessions", "project selection can be skipped to view sessions")


def deletion(phone):
    phone.fixture("delete")
    before = phone.state()["sessions"]
    phone.tap("cancel-delete")
    phone.wait(lambda s: s["sheet"] is None and s["sessions"] == before, "cancel deletion preserves all sessions")
    phone.fixture("delete")
    phone.tap("confirm-delete")
    phone.wait(lambda s: s["sheet"] is None and len(s["sessions"]) == len(before) - 1, "confirmation deletes only the selected sample session")
    phone.capture("delete-confirmed")


def gestures(phone):
    phone.fixture("drawer")
    (x, y, width, height), _ = phone.bounds("drawer-panel")
    phone.swipe((x + width * 0.75, y + height * 0.5), (x + width * 0.75 - 30, y + height * 0.5))
    assert phone.settled_state()["drawer"], "A short drawer drag must snap back"
    phone.swipe((x + width * 0.85, y + height * 0.5), (x + 15, y + height * 0.5), 650)
    phone.wait(lambda s: not s["drawer"] and s["route"] == "Sessions", "drawer swipes closed without changing the underlying screen")
    phone.fixture("thinking")
    (x, y, width, height), _ = phone.bounds("bottom-sheet")
    phone.swipe((x + width / 2, y + 14), (x + width / 2, y + 44))
    assert phone.settled_state()["sheet"] == "Thinking", "A short sheet drag must snap back"
    phone.swipe((x + width / 2, y + 14), (x + width / 2, y + 190), 650)
    phone.wait(lambda s: s["sheet"] is None and s["route"] == "Start", "sheet swipes closed without clicking through")

    state = phone.fixture("sessions")
    sessions = state["sessions"]
    (x, y, width, height), _ = phone.bounds("session-row-4")
    phone.swipe((x + 20, y + height / 2), (x + width - 20, y + height / 2), 650)
    phone.wait(lambda s: (s["sheet"] or "").startswith("Delete(") and s["sessions"] == sessions and s["route"] == "Sessions",
               "swipe right requests confirmation without opening or deleting the row")
    phone.tap("cancel-delete")
    phone.wait(lambda s: s["sheet"] is None and s["sessions"] == sessions, "cancelled swipe deletion keeps the original list")
    phone.tap("new-session")
    phone.wait(lambda s: s["route"] == "Start", "floating New session opens only the new composer")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda s: s["route"] == "Sessions", "Back from New session returns directly to the list, with no clicked-through session")


def native_image_picker(phone):
    # Owned, synthetic PNG; never select or inspect the user's photos.
    name = f"pi-upload-test-{uuid.uuid4().hex}.png"
    destination = f"/sdcard/Download/{name}"
    chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    pixels = b"".join(b"\0" + b"".join(bytes((220 if x < 48 else 30, 200 if y < 48 else 50, 110)) for x in range(96)) for y in range(96))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 96, 96, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b"")
    source = phone.output / name
    source.write_bytes(png)
    phone.run("push", source, destination)
    try:
        phone.fixture("start")
        phone.tap("attach")
        phone.wait(lambda s: (s["sheet"] or "").startswith("Attach("), "attachment choices open")
        phone.tap("choose-files")
        nodes = phone.native_nodes()
        if not any(node.get("text") == name for node in nodes):
            # AOSP DocumentsUI; Xiaomi's picker shows the fresh fixture in Recent.
            phone.native_tap("Show roots")
            phone.native_tap("Downloads")
        phone.native_tap(name)
        nodes = phone.native_nodes()
        for label in ("OK", "Open", "Select"):
            if any(node.get("text") == label and node.get("enabled") == "true" for node in nodes):
                phone.native_tap(label)
                break
        phone.wait(lambda s: s["sheet"] is None and s["draft_images"] == 1, "system document picker imports real image bytes", timeout=20)
        phone.wait_keyboard(False)
        phone.tap("attachment-0")
        phone.wait(lambda s: (s["sheet"] or "").startswith("Image("), "imported image opens full preview")
        phone.capture("picked-image-preview")
        phone.key("KEYCODE_BACK")
        phone.wait(lambda s: s["sheet"] is None, "picked image preview closes")
        phone.tap("send")
        phone.wait(lambda s: s["route"].startswith("Thread("), "picked image-only prompt is accepted")
    finally:
        # Only this run's exact, generated file; the local evidence is retained.
        phone.run("shell", "rm", destination)


CASES = {"input": input_and_selectors, "long-input": long_input_and_stop,
         "models": model_scroll, "images": images, "delete": deletion,
         "picker": native_image_picker, "gestures": gestures, "ime": gboard_typing,
         "conversation": conversation, "projects": projects}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=CASES, action="append", help="Run selected cases; default: all (English system picker)")
    args = parser.parse_args()
    phone = Phone(args.serial, args.output)
    try:
        phone.run("shell", "pm", "path", PACKAGE)
        for name in args.case or CASES:
            try:
                CASES[name](phone)
            except Exception:
                phone.results.append({"case": name, "passed": False})
                raise
    finally:
        phone.report()
    print(f"{len(phone.results)} checks passed. Screenshots and report: {args.output}")


if __name__ == "__main__":
    main()
