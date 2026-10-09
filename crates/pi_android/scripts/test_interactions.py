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
            safe_connect = self.fixture_name == "connect" and state["route"] == "Connect"
            assert state["sample"] or safe_connect, "Refusing to act on anything except sample sessions or the inert connect fixture"
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

    def advance_fixture(self, seconds):
        """Move only the frozen sample session clock; unavailable in production builds."""
        previous = self.state()["time"]
        self.run("shell", "am", "start", "-n", ACTIVITY, "-a", "android.intent.action.VIEW",
                 "-d", f"pi://test/advance/{seconds}")
        self.wait(lambda state: state["time"] > previous, f"advanced sample by {seconds}s")

    def settled_state(self):
        # InputMethodManager can report its destination before the resize
        # animation finishes. A sheet is also mounted just below the viewport
        # before its opening animation starts, so do not consider it settled
        # until its bottom edge has reached the viewport bottom. Then wait until
        # bounds have stayed unchanged; a static screen is allowed to stop
        # repainting entirely.
        deadline = time.monotonic() + 8
        previous = self.state()
        unchanged_since = time.monotonic()
        while time.monotonic() < deadline:
            state = self.state()
            sheet_bounds = state["bounds"].get("bottom-sheet")
            sheet_settled = state["sheet"] is None or sheet_bounds is None or abs(
                sheet_bounds[1] + sheet_bounds[3] - state["viewport"][1]
            ) < 1
            if not sheet_settled or state["bounds"] != previous["bounds"]:
                unchanged_since = time.monotonic()
            elif time.monotonic() - unchanged_since >= 0.6:
                return state
            previous = state
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

    def long_press(self, point, duration=650):
        scale = self.state()["scale"]
        x, y = (round(value * scale) for value in point)
        # A zero-distance swipe is one continuous injected touch stream. Separate
        # `input motionevent` processes do not preserve a contact between calls.
        self.run("shell", "input", "swipe", x, y, x, y, duration)

    def key(self, key):
        self.run("shell", "input", "keyevent", key)

    def text(self, text):
        # The smoke text deliberately contains only shell-safe ASCII characters.
        if not re.fullmatch(r"[A-Za-z0-9_./-]+", text):
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

    def wait_activity(self, name, description, timeout=12):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            dump = self.run("shell", "dumpsys", "activity", "activities").decode()
            resumed = next((line for line in dump.splitlines() if "topResumedActivity=" in line), "")
            if name in resumed:
                self.results.append({"check": description, "passed": True})
                print(f"PASS {description}", flush=True)
                return
            time.sleep(0.2)
        self.capture("activity-failure")
        raise AssertionError(f"Activity did not become {name}")

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

    def wait_native_text(self, label, description, timeout=12):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if any(label in (node.get("text"), node.get("content-desc"))
                   for node in self.native_nodes()):
                self.results.append({"check": description, "passed": True})
                print(f"PASS {description}", flush=True)
                return
            time.sleep(0.2)
        self.capture("native-text-failure")
        raise AssertionError(f"Native text did not appear: {label}")

    def report(self):
        self.output.joinpath("report.json").write_text(json.dumps(self.results, indent=2) + "\n")


def initial_paste_menu(phone):
    phone.fixture("sessions")
    phone.wait_keyboard(False)
    phone.tap("new-session")
    phone.wait(lambda s: s["route"] == "Start" and shown(s, "draft"),
               "the start bar opens the new composer")
    state = phone.settled_state()
    draft, _ = phone.bounds("draft", state=state)
    x, y, width, height = draft
    # Stay on the first text line rather than the composer's lower padding.
    point = (x + min(40, width / 2), y + min(10, height / 2))
    phone.long_press(point)
    phone.wait(lambda s: s["text_menu_open"] and shown(s, "text-menu"),
               "a first long press opens Paste without a preparatory tap")
    time.sleep(0.5)
    phone.wait(lambda s: s["text_menu_open"] and shown(s, "text-menu"),
               "Paste remains after touch release")
    phone.wait_keyboard(False)
    phone.capture("first-long-press-paste")


def focus_long_press_during_ime_reflow(phone):
    phone.fixture("start")
    phone.wait_keyboard(False)
    state = phone.settled_state()
    draft, scale = phone.bounds("draft", state=state)
    x, y, width, height = draft
    point = (x + min(40, width / 2), y + min(10, height / 2))
    physical = tuple(round(value * scale) for value in point)
    # Keep both injected gestures in one asynchronous device shell. Host-side
    # settling or telemetry between a tap and long press would hide the race.
    command = (
        f"input tap {physical[0]} {physical[1]}; "
        f"exec input swipe {physical[0]} {physical[1]} "
        f"{physical[0]} {physical[1]} 1200"
    )
    process = subprocess.Popen(phone.adb + ["shell", command], stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT)
    moved_during_hold = False
    try:
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline and process.poll() is None:
            current = phone.state()
            current_draft, _ = phone.bounds("draft", state=current)
            if abs(current_draft[1] - y) > 1:
                moved_during_hold = process.poll() is None
                break
            time.sleep(0.05)
        output = process.communicate(timeout=5)[0]
    except Exception:
        process.kill()
        process.communicate()
        raise
    if process.returncode:
        raise subprocess.CalledProcessError(process.returncode, process.args, output)
    assert moved_during_hold, "The composer did not reflow while the second touch was held"
    phone.results.append({"check": "IME reflow occurred during the held touch", "passed": True})
    print("PASS IME reflow occurred during the held touch", flush=True)
    phone.wait(lambda s: s["draft_chars"] == 0 and s["text_menu_open"],
               "the reflowed long press opens Paste without entering text")
    phone.wait_keyboard(True)
    phone.capture("focus-long-press-during-ime-reflow")


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


def pairing_scanner(phone):
    phone.fixture("connect")
    phone.bounds("scan-computer")
    phone.capture("pairing-connect")
    phone.run("shell", "pm", "grant", PACKAGE, "android.permission.CAMERA")
    phone.tap("scan-computer")
    phone.wait_activity("dev.pi.gpui.PairScannerActivity", "pairing opens the native offline QR scanner")
    phone.capture("pairing-scanner")
    phone.native_tap("Close scanner")
    phone.wait_activity("dev.pi.gpui.GpuiActivity", "scanner close returns to the same app")
    phone.wait(lambda s: s["fixture"] == "connect" and s["route"] == "Connect",
               "closing the scanner preserves the connect screen")


def gboard_typing(phone):
    """English, portrait four-row Gboard (no number row or suggestion toolbar).

    Real touchscreen keys exercise InputConnection composition. Bounds come
    from keyboard occlusion, not this device's pixel dimensions. Other keyboard
    layouts can run the non-IME cases without changing their system settings.
    """
    method = phone.run("shell", "settings", "get", "secure", "default_input_method").decode()
    assert "com.google.android.inputmethod.latin" in method, "The ime case requires Gboard"
    # New session is a sheet that sits on the keyboard: the keyboard begins
    # under its last row (the starting points, then 12 dp of padding) and
    # reaches the bottom of the screen.
    phone.fixture("start")
    phone.tap("draft")
    phone.wait_keyboard(True)

    def lifted(state):
        starters, _ = phone.bounds("starters", state=state)
        return starters[1] + starters[3] < state["viewport"][1] - 150

    phone.wait(lifted, "the keyboard lifts the new-session sheet")
    shown = phone.settled_state()
    starters, scale = phone.bounds("starters", state=shown)
    top = starters[1] + starters[3] + 12
    bottom = shown["viewport"][1]
    width = shown["viewport"][0]
    assert bottom - top > 100, "Keyboard must occlude the lower viewport"
    positions = {letter: ((i + 0.5) / 10, 0.125) for i, letter in enumerate("qwertyuiop")}
    positions.update({letter: ((i + 1) / 10, 0.375) for i, letter in enumerate("asdfghjkl")})
    positions.update({letter: ((i + 2) / 10, 0.625) for i, letter in enumerate("zxcvbnm")})
    positions.update({" ": (0.5, 0.875), "delete": (0.925, 0.625)})

    def touch(letter):
        x, y = positions[letter]
        phone.run("shell", "input", "tap", round(width * x * scale), round((top + (bottom - top) * y) * scale))
        # Key by key, as a person types: each tap is its own composition update.
        time.sleep(0.15)

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
    # Autocorrect replaces a word as space is pressed, not with the next key.
    for letter in " wrld ":
        touch(letter)
    phone.wait(lambda s: s["draft_chars"] == 12, "Gboard's autocorrect (wrld to world) arrives with the space")
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
    phone.wait(lambda s: s["model_search_chars"] == 6 and s["draft_chars"] == 12,
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
    # The thumb intentionally fades while idle. Reveal it again immediately
    # before testing its direct-drag affordance.
    phone.swipe((x + width / 2, y + height * 0.53),
                (x + width / 2, y + height * 0.48), duration=700)
    time.sleep(0.02)
    state = phone.state()
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
    phone.fixture("working")
    phone.wait(lambda s: any("turn-0-stage-1-live-diff-3" in key for key in s["bounds"]),
               "expanded main activity shows the current live file diff")
    phone.advance_fixture(4)
    phone.wait(lambda s: any("turn-0-stage-1-live-diff-4" in key for key in s["bounds"]),
               "expanded main activity streams the next file update")
    phone.capture("inline-live-edit-streamed")

    phone.fixture("working")
    phone.tap("turn-0-stage-1")
    phone.wait(lambda s: (s["sheet"] or "").startswith("Activity("),
               "live change stage opens its detailed activity sheet")
    phone.wait(lambda s: any("activity-live-diff-3" in key for key in s["bounds"]),
               "live edit details show the current diff")
    phone.advance_fixture(4)
    phone.wait(lambda s: any("activity-live-diff-4" in key for key in s["bounds"])
               and (s["sheet"] or "").startswith("Activity("),
               "open edit details stream the next file update")
    phone.capture("live-edit-streamed")
    phone.key("KEYCODE_BACK")

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
    x, y, width, height = viewport
    # Scrollbars intentionally rest hidden. A short content scroll reveals the
    # thumb before testing its direct-drag affordance.
    phone.swipe((x + width / 2, y + height * 0.48),
                (x + width / 2, y + height * 0.53), 700)
    time.sleep(0.02)
    state = phone.state()
    thumb, _ = phone.bounds('thumb:NamedInteger("thread", 1)', state=state)
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


def rich_content(phone):
    """Exercise the shared desktop Markdown renderer and Pi-made page viewer."""
    phone.fixture("markdown")
    phone.wait(lambda s: s["route"].startswith("Thread("),
               "Markdown fixture opens as a normal conversation")
    phone.wait(lambda s: any("markdown-image" in key for key in s["bounds"]),
               "embedded SVG renders through the Markdown image path")
    phone.wait(lambda s: any("markdown-mermaid" in key for key in s["bounds"]),
               "Mermaid renders through the shared diagram engine")
    phone.capture("markdown-shared-renderer")

    phone.fixture("page")
    phone.tap("page-0-0")
    phone.wait_activity("dev.pi.gpui.PageActivity", "page card opens the sandboxed preview")
    # topResumedActivity changes before WebView's first frame reaches SurfaceFlinger.
    time.sleep(0.4)
    phone.capture("page-preview")
    phone.native_tap("Run interaction")
    phone.wait_native_text("JavaScript executed", "page-local JavaScript executes")
    phone.native_tap("Source")
    phone.capture("page-source")
    phone.native_tap("Page")
    phone.key("KEYCODE_BACK")
    phone.wait_activity("dev.pi.gpui.GpuiActivity", "closing a page returns to its conversation")
    phone.wait(lambda s: s["fixture"] == "page" and s["route"].startswith("Thread("),
               "page viewer preserves the source conversation")


def history(phone):
    phone.fixture("history")
    phone.wait(lambda s: s["route"].startswith("History("),
               "file history opens as a dedicated screen")
    phone.capture("jj-history")
    phone.tap("restore-operation", 0)
    phone.wait(lambda s: (s["sheet"] or "").startswith("RestoreHistory("),
               "restoring history requires confirmation")
    phone.capture("jj-restore-confirmation")
    phone.tap("cancel-restore")
    phone.wait(lambda s: s["sheet"] is None and s["route"].startswith("History("),
               "cancelling restore leaves project files and history untouched")


def tool_images(phone):
    state = phone.fixture("tool-image")
    name = next(key for key in state["bounds"] if key.startswith("tool-image-"))
    phone.tap(name)
    phone.wait(lambda s: (s["sheet"] or "").startswith("ToolImage("),
               "tapping a screenshot Pi read shows it whole")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda s: s["sheet"] is None, "back closes the image")


def commands(phone):
    phone.fixture("resources")
    phone.wait(lambda s: all(shown(s, f"command-{name}") for name in ("review", "fix-tests", "skill:lint")),
               "Resources lists prompt templates and skills")
    phone.capture("resources-commands")
    phone.key("KEYCODE_BACK")
    phone.fixture("start")
    phone.tap("draft")
    phone.wait_keyboard(True)
    phone.text("/")
    phone.wait(lambda s: shown(s, "suggestion-/review") and shown(s, "suggestion-/skill:lint"),
               "typing / offers prompt templates and skills")
    phone.capture("command-suggestions")
    # Typing narrows the strip to what matches, as it would for a person.
    phone.text("sk")
    # Bounds outlive what left the screen; the skill moving first shows the strip narrowed.
    phone.wait(lambda s: s["bounds"]["suggestion-/skill:lint"][0] == s["bounds"]['NamedInteger("suggestion", 0)'][0],
               "typing narrows the commands to the skill")
    phone.tap("suggestion-/skill:lint")
    phone.wait(lambda s: s["draft_chars"] == len("/skill:lint "), "choosing a skill puts its command in the draft")
    phone.key("KEYCODE_BACK")
    phone.wait_keyboard(False)


def subagents(phone):
    phone.fixture("subagents")
    phone.wait(lambda s: all(shown(s, f"subagent-0-0-{n}") for n in range(3)),
               "work handed to three scouts shows a row for each")
    phone.capture("subagents")
    phone.tap("subagent-0-0-1")
    phone.wait(lambda s: s["route"].startswith("Subagent(") and shown(s, "stop-subagent"),
               "a scout's row opens its own screen, with Stop while it works")
    phone.capture("subagent")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda s: s["route"].startswith("Thread(") and shown(s, "subagent-0-0-1"),
               "back returns to the session's hand-off")
    phone.fixture("subagents-done")
    phone.wait(lambda s: shown(s, "subagent-0-0-0") and shown(s, "subagent-0-0-1"),
               "a finished chain lists each step")
    phone.capture("subagents-done")
    # A big crew: the first few, those at work first, then all on request.
    phone.fixture("subagents-many")
    phone.wait(lambda s: all(shown(s, f"subagent-0-0-{n}") for n in range(9, 14)) and shown(s, "handoff-all-0-0"),
               "twenty-four scouts show the five at work, and Show all")
    phone.capture("subagents-many")
    phone.tap("handoff-all-0-0")
    phone.wait(lambda s: shown(s, "subagent-0-0-0") and shown(s, "subagent-0-0-14"),
               "Show all lists the rest")
    phone.capture("subagents-all")
    phone.tap("handoff-0-0")
    time.sleep(0.5)
    phone.capture("subagents-folded")


def shown(state, name):
    return any(key == name or f'"{name}"' in key for key in state["bounds"])


def projects(phone):
    state = phone.fixture("projects")
    sessions = state["sessions"]
    phone.tap("project", 1)
    phone.wait(lambda s: s["route"] == "Start" and s["sessions"] == sessions,
               "choosing a recent project opens the composer without starting work")
    state = phone.fixture("project-empty")
    phone.tap("use-folder")
    phone.wait(lambda s: s["route"] == "Start" and s["sessions"] == state["sessions"]
               and s["project"] == "/Users/nick/repos",
               "choosing an empty folder does not create a session")
    phone.fixture("projects")
    phone.tap("view-sessions")
    phone.wait(lambda s: s["route"] == "Sessions", "project selection can be skipped to view sessions")

    # The tree: the caret rolls a folder out and back in, a name picks it.
    state = phone.fixture("project")
    opened = state["open_folders"]
    phone.tap("roll:~/repos/pi/packages")
    state = phone.wait(lambda s: s["open_folders"] == opened + 1
                       and shown(s, "node:~/repos/pi/packages/ai"),
                       "the caret rolls a folder out in place")
    phone.tap("roll:~/repos/pi/packages")
    phone.wait(lambda s: s["open_folders"] == opened, "the caret rolls it back in")
    phone.tap("node:~/repos/minivm")
    phone.wait(lambda s: s["picked"] == "/Users/nick/repos/minivm" and s["sheet"] == "Project",
               "tapping a folder's name picks it without leaving the sheet")
    phone.tap("hidden-toggle")
    phone.wait(lambda s: shown(s, "node:~/repos/pi/.gitignore"), "Hidden shows dot-files")
    phone.swipe((192, 700), (192, 300))
    phone.wait(lambda s: s["sheet_scroll"][0] < -100 or s["sheet_scroll"][0] > 100,
               "the tree scrolls inside the sheet")
    phone.tap("node:~/repos/pi/README.md")
    phone.wait(lambda s: s["route"] == "File" and s["file"] == "README.md", "tapping a file opens it read-only")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda s: s["route"] == "Start" and s["sheet"] == "Project"
               and s["picked"] == "/Users/nick/repos/minivm",
               "back from a file returns to the tree as it was")
    phone.tap("use-folder")
    phone.wait(lambda s: s["sheet"] is None and s["project"] == "/Users/nick/repos/minivm",
               "New session uses the picked folder")

    # Find and go to.
    phone.fixture("project")
    phone.tap("project-search")
    phone.wait_keyboard(True)
    phone.text("prov")
    phone.wait(lambda s: any('"found"' in key for key in s["bounds"]),
               "typing finds folders and files listed in the tree")
    phone.tap("found", 0)
    phone.wait(lambda s: s["picked"] == "/Users/nick/repos/pi/packages/ai/src/providers",
               "a found folder is revealed and picked in the tree")
    phone.tap("project-search")
    phone.text("/Users/nick/repos/zed")
    phone.key("KEYCODE_ENTER")
    phone.wait(lambda s: shown(s, "node:~/repos/zed/crates"),
               "a typed path starts the tree there")

    # A file: Ask about it starts a new session with it mentioned.
    phone.fixture("project-file")
    phone.tap("ask-about-file")
    phone.wait(lambda s: s["route"] == "Start" and s["sheet"] is None
               and s["start_draft"].startswith("@packages/ai/src/retry.ts")
               and s["project"] == "/Users/nick/repos/pi",
               "Ask about it starts a session in the file's project with the file mentioned")


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
    phone.fixture("computers")
    (x, y, width, height), _ = phone.bounds("bottom-sheet")
    phone.swipe((x + width / 2, y + 14), (x + width / 2, y + 44))
    assert phone.settled_state()["sheet"] == "Computers", "A short computers drag must snap back"
    phone.swipe((x + width / 2, y + 14), (x + width / 2, y + 190), 650)
    phone.wait(lambda s: s["sheet"] is None and s["route"] == "Sessions", "computers swipe closed without changing Home")
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
    phone.wait(lambda s: s["route"] == "Start", "the start bar opens only the new composer")
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


CASES = {"pairing": pairing_scanner, "initial-paste": initial_paste_menu,
         "focus-long-press": focus_long_press_during_ime_reflow,
         "input": input_and_selectors, "long-input": long_input_and_stop,
         "models": model_scroll, "images": images, "delete": deletion,
         "picker": native_image_picker, "gestures": gestures, "ime": gboard_typing,
         "conversation": conversation, "rich-content": rich_content, "history": history,
         "projects": projects, "tool-images": tool_images, "commands": commands,
         "subagents": subagents}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=CASES, action="append", help="Run selected cases; default: all (English system picker)")
    args = parser.parse_args()
    phone = Phone(args.serial, args.output)
    try:
        phone.run("shell", "pm", "path", PACKAGE)
        # Incremental installs can leave the previous native process alive.
        # Start every run from this APK without clearing its paired/settings data.
        phone.run("shell", "am", "force-stop", PACKAGE)
        stopped_by = time.monotonic() + 5
        while True:
            try:
                running = phone.run("shell", "pidof", PACKAGE).strip()
            except subprocess.CalledProcessError:
                running = b""
            if not running:
                break
            if time.monotonic() >= stopped_by:
                raise RuntimeError("The previous isolated app process did not stop")
            time.sleep(0.05)
        phone.run("shell", "input", "keyevent", "KEYCODE_WAKEUP")
        phone.run("shell", "wm", "dismiss-keyguard")
        time.sleep(0.5)
        policy = phone.run("shell", "dumpsys", "window", "policy").decode()
        if "mIsShowing=true" in policy and "mKeyguardOccluded=false" in policy:
            raise RuntimeError("Unlock the phone before running physical interaction tests")
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
