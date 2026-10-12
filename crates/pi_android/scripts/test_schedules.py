#!/usr/bin/env python3
"""Focused native scheduler checks for the separate --ui-test APK (no real jobs).

Install a fresh x86_64 APK separately; this script never builds or installs it.
Uses the existing Phone harness, fixture-only run-as telemetry and actual ADB
input. Artifacts include exact commands, states, screenshots and process logs.
See SCHEDULER_VERIFICATION.md for the hook contract and reproducible commands.
"""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import time

from test_interactions import ACTIVITY, PACKAGE, Phone


FIXTURES = ("schedules", "schedules-evening", "schedule-offline", "schedule-setup",
            "schedule-stopped", "schedule-unsupported")
GEOMETRIES = {"small": (720, 1280, 320), "large": (1080, 2400, 420)}
# These controls live inside the declared scheduler scroll viewport. Reject
# probes behind its appbar/dock even if their rectangles are on the display.
CONTENT_CONTROLS = {"schedule-name", "schedule-project", "schedule-prompt", "schedule-cron",
                    "schedule-timezone", "schedule-preview", "schedule-consent", "schedule-consent-label", "schedule-toggle",
                    "schedule-history", "schedule-delete", "schedule-model"}


def adb_path(explicit=None):
    candidates = [explicit, shutil.which("adb")]
    for home in (os.environ.get("ANDROID_HOME"), os.environ.get("ANDROID_SDK_ROOT"),
                 str(Path.home() / "Android/Sdk")):
        if home:
            candidates.append(str(Path(home) / "platform-tools/adb"))
    for candidate in candidates:
        if candidate and Path(candidate).is_file() and os.access(candidate, os.X_OK):
            return str(Path(candidate).resolve())
    raise RuntimeError("ADB not found; use --adb or set ANDROID_HOME")


def digest(data):
    return hashlib.sha256(data).hexdigest() if data is not None else None


def require_sample(state, fixture, now=None):
    if fixture not in FIXTURES or state.get("fixture") != fixture:
        raise AssertionError("Refusing stale/unknown scheduler fixture telemetry")
    if state.get("sample") is not True or state.get("scheduler", {}).get("mode") != "sample":
        raise AssertionError("Refusing anything except isolated scheduler sample mode")
    now = time.time() if now is None else now
    timestamp = state.get("time", 0)
    if not isinstance(timestamp, (int, float)) or not math.isfinite(timestamp) or abs(now - timestamp) > 5:
        raise AssertionError("Refusing stale sample telemetry; the app may have stopped")
    return state


def control_rect(state, selector):
    matches = [bounds for key, bounds in state["bounds"].items()
               if key == selector or f'"{selector}"' in key]
    if len(matches) != 1:
        raise AssertionError(f"Expected one control {selector}, found {len(matches)}")
    bounds = matches[0]
    if len(bounds) != 4 or not all(math.isfinite(value) for value in bounds):
        raise AssertionError(f"Invalid control bounds for {selector}: {bounds}")
    return bounds


def visible(state, selector):
    try:
        x, y, width, height = control_rect(state, selector)
    except AssertionError:
        return False
    vw, vh = state["viewport"]
    inside = width > 0 and height > 0 and x >= -1 and y >= -1 and x + width <= vw + 1 and y + height <= vh + 1
    if selector in CONTENT_CONTROLS or selector.startswith(("schedule-row-", "schedule-session-")):
        scrolls = [bounds for key, bounds in state["bounds"].items() if "scroll:" in key and "schedule" in key]
        if len(scrolls) == 1:
            sx, sy, sw, sh = scrolls[0]
            inside = inside and x >= sx - 1 and y >= sy - 1 and x + width <= sx + sw + 1 and y + height <= sy + sh + 1
    return inside


def consent_fits(state):
    """The label itself must wrap inside the padded row, not just its hitbox."""
    if not visible(state, "schedule-consent") or not visible(state, "schedule-consent-label"):
        return False
    x, y, width, height = control_rect(state, "schedule-consent")
    tx, ty, tw, th = control_rect(state, "schedule-consent-label")
    return (tx >= x + 16 + 24 + 12 - 1 and tx + tw <= x + width - 16 + 1
            and ty >= y + 16 - 1 and ty + th <= y + height - 16 + 1 and th >= 44 - 1)


def definitions(state):
    return {job["id"]: job for job in state["scheduler"]["definitions"]}


def occurrences(state):
    return state["scheduler"]["occurrences"]


def schedule_row(state, job_id):
    return f"schedule-row-{next(index for index, job in enumerate(state['scheduler']['definitions']) if job['id'] == job_id)}"


def history_link(state, occurrence_id):
    selected = state["scheduler"]["selected_id"]
    history = [item for item in occurrences(state) if item.get("job_id", selected) == selected]
    return f"schedule-session-{next(index for index, item in enumerate(history) if item['id'] == occurrence_id)}"


def mutation_snapshot(state):
    """Preview, cancellation and unavailable states must not mutate sample data."""
    scheduler = state["scheduler"]
    return {"definitions": scheduler["definitions"], "occurrences": scheduler["occurrences"],
            "sessions": state["sessions"]}


class SchedulerPhone(Phone):
    def __init__(self, serial, output, adb):
        super().__init__(serial, output)
        self.adb[0] = adb
        self.serial = serial
        self.started = time.time()
        self.commands = self.output / "commands.jsonl"
        self.settings_before = None
        self.settings_checked = False
        self.clock_offset = 0
        self.metadata = {"serial": serial, "started": self.started, "package": PACKAGE}

    def run(self, *args):
        args = list(map(str, args))
        # Even inherited swipe/key helpers must recheck the sample boundary.
        if args[:2] == ["shell", "input"]:
            self.state()
        command = self.adb + args
        entry = {"time": time.time(), "argv": command, "command": shlex.join(command)}
        try:
            result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=25)
            entry["returncode"] = result.returncode
            if result.returncode:
                entry["error"] = result.stdout.decode(errors="replace")[-2000:]
                raise subprocess.CalledProcessError(result.returncode, command, result.stdout)
            return result.stdout
        finally:
            with self.commands.open("a") as log:
                log.write(json.dumps(entry) + "\n")

    def raw_state(self):
        return json.loads(self.run("exec-out", "run-as", PACKAGE, "cat", "files/ui-test-state.json"))

    def state(self):
        return require_sample(self.raw_state(), self.fixture_name, time.time() + self.clock_offset)

    def fixture(self, name):
        if name not in FIXTURES:
            raise ValueError(f"Not an approved scheduler fixture: {name}")
        try:
            previous = self.raw_state().get("time", 0)
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            previous = 0
        self.fixture_name = name
        self.run("shell", "am", "start", "-n", ACTIVITY, "-a", "android.intent.action.VIEW",
                 "-d", f"pi://preview/{name}")
        # A previous fixture is normal during launch, but never authorize input
        # from it. Once loaded, every subsequent action uses strict state().
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                state = self.raw_state()
                if state.get("fixture") == name and state.get("time", 0) > previous:
                    require_sample(state, name, time.time() + self.clock_offset)
                    self.check(True, f"loaded isolated {name}")
                    return self.settled_state()
            except (subprocess.CalledProcessError, json.JSONDecodeError):
                pass
            time.sleep(0.2)
        raise AssertionError(f"Fresh APK did not expose {name}; do not treat an older APK as verified")

    def check(self, condition, description):
        self.results.append({"check": description, "passed": bool(condition)})
        if not condition:
            raise AssertionError(description)
        print(f"PASS [{self.serial}] {description}", flush=True)

    def reveal(self, name):
        # Form/detail content is scrollable on small phones. Scroll only its
        # declared viewport; never blindly tap an offscreen/stale rectangle.
        for _ in range(8):
            state = self.settled_state()
            if visible(state, name):
                return state
            scrolls = [key for key in state["bounds"] if "scroll:" in key and "schedule" in key]
            if len(scrolls) != 1:
                raise AssertionError(f"Control {name} unavailable and no unique schedule scroll viewport")
            x, y, width, height = state["bounds"][scrolls[0]]
            try:
                target_y = control_rect(state, name)[1]
            except AssertionError:
                target_y = y + height
            start, end = (0.25, 0.75) if target_y < y else (0.75, 0.25)
            self.swipe((x + width / 2, y + height * start), (x + width / 2, y + height * end))
        raise AssertionError(f"Control {name} could not be scrolled into view")

    def tap(self, name, index=None):
        if index is not None:
            raise ValueError("Use scheduler IDs mapped to delivered selectors, or the existing picker harness")
        self.reveal(name)
        return super().tap(name)

    def replace_text(self, selector, text):
        # Native edit, not a deep-link that replaces draft contents. Android's
        # keycombination sends Ctrl+A as one chord to GPUI's SelectAll binding.
        if not re.fullmatch(r"[A-Za-z0-9_./:* -]+", text):
            raise ValueError("Only harmless ASCII scheduler sample input is allowed")
        self.tap(selector)
        self.wait_keyboard(True)
        self.run("shell", "input", "keycombination", "KEYCODE_CTRL_LEFT", "KEYCODE_A")
        self.key("KEYCODE_DEL")
        self.run("shell", "input", "text", shlex.quote(text.replace(" ", "%s")))
        self.key("KEYCODE_BACK")
        self.wait_keyboard(False)
        self.settled_state()

    def capture(self, name):
        state = self.settled_state()
        png = self.run("exec-out", "screencap", "-p")
        if not png.startswith(b"\x89PNG\r\n\x1a\n"):
            raise AssertionError("ADB returned an invalid PNG")
        (self.output / f"{name}.png").write_bytes(png)
        (self.output / f"{name}.json").write_text(json.dumps(state, indent=2) + "\n")

    def read_settings(self):
        # ls distinguishes absent settings from a non-debuggable/unusable APK.
        files = self.run("exec-out", "run-as", PACKAGE, "ls", "files").decode().splitlines()
        if "settings.json" not in files:
            return None
        return self.run("exec-out", "run-as", PACKAGE, "cat", "files/settings.json")

    def preflight(self):
        self.run("shell", "pm", "path", PACKAGE)
        self.settings_before = self.read_settings()
        if self.settings_before is not None:
            settings = json.loads(self.settings_before)
            if settings.get("computer") and settings.get("sample") is not True:
                raise RuntimeError("UI-test app has real connection preferences; refusing launch. Do not clear its data.")
        self.settings_checked = True
        before = time.time()
        device_time = float(self.run("shell", "date", "+%s").strip())
        self.clock_offset = device_time - (before + time.time()) / 2
        self.metadata["device_clock_offset_seconds"] = self.clock_offset
        for key, command in {
            "api": ("shell", "getprop", "ro.build.version.sdk"),
            "abi": ("shell", "getprop", "ro.product.cpu.abi"),
            "size": ("shell", "wm", "size"), "density": ("shell", "wm", "density"),
            "package_dump": ("shell", "dumpsys", "package", PACKAGE),
        }.items():
            self.metadata[key] = self.run(*command).decode()
        self.run("shell", "am", "force-stop", PACKAGE)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                running = self.run("shell", "pidof", PACKAGE).strip()
            except subprocess.CalledProcessError:
                running = b""
            if not running:
                return
            time.sleep(0.1)
        raise RuntimeError("Previous isolated app process did not stop")

    def finish(self):
        errors = []
        try:
            # Do not clear the shared logcat buffer. Read this app's PID only.
            pid = self.run("shell", "pidof", PACKAGE).decode().strip().split()
            if pid:
                log = self.run("logcat", "-d", f"--pid={pid[0]}", "-t", "2000")
                (self.output / "logcat.txt").write_bytes(log)
                self.check(b"FATAL EXCEPTION" not in log and b"panicked at" not in log,
                           "no native/Java panic in isolated process log")
            else:
                self.check(False, "isolated native app is still running")
        except Exception as error:
            errors.append(str(error))
        try:
            if self.settings_checked:
                after = self.read_settings()
                self.metadata["settings_sha256_before"] = digest(self.settings_before)
                self.metadata["settings_sha256_after"] = digest(after)
                self.check(self.settings_before == after, "fixtures preserved app preferences byte-for-byte")
        except Exception as error:
            errors.append(str(error))
        self.metadata["finished"] = time.time()
        self.metadata["cleanup_errors"] = errors
        self.report()
        (self.output / "environment.json").write_text(json.dumps(self.metadata, indent=2) + "\n")
        if errors:
            raise AssertionError("; ".join(errors))


@contextmanager
def geometry(phone, preset):
    if preset == "native":
        yield
        return
    if not phone.serial.startswith("emulator-") or phone.run("shell", "getprop", "ro.kernel.qemu").strip() != b"1":
        raise RuntimeError("Display overrides are permitted only on explicitly selected Android emulators")
    previous = {}
    for prop in ("size", "density"):
        value = phone.run("shell", "wm", prop).decode()
        match = re.search(rf"Override {prop}: (\S+)", value)
        previous[prop] = match.group(1) if match else "reset"
    (phone.output / "display-before.json").write_text(json.dumps(previous, indent=2) + "\n")
    width, height, density = GEOMETRIES[preset]
    try:
        phone.run("shell", "wm", "size", f"{width}x{height}")
        phone.run("shell", "wm", "density", density)
        time.sleep(0.8)
        yield
    finally:
        # Narrow restoration only. No AVD files, app data or global settings.
        errors = []
        for prop in ("density", "size"):
            try:
                phone.run("shell", "wm", prop, previous[prop])
            except Exception as error:
                errors.append(str(error))
        if errors:
            raise RuntimeError("Display restoration failed; inspect display-before.json: " + "; ".join(errors))


def navigation(phone):
    state = phone.fixture("schedules")
    before = mutation_snapshot(state)
    phone.tap("schedule-back")
    state = phone.wait(lambda s: s["route"] == "Sessions", "Back from Schedules returns to Sessions")
    phone.check(not visible(state, "home-sessions") and not visible(state, "home-schedules"),
                "Home has no Sessions/Schedules button row")
    phone.tap("computers")
    phone.wait(lambda s: s["sheet"] == "Computers" and visible(s, "open-schedules"),
               "the Computers menu exposes Schedules")
    phone.capture("computers-schedules-menu")
    phone.tap("open-schedules")
    state = phone.wait(lambda s: s["route"] == "Schedules" and s["sheet"] is None,
                       "menu navigation opens Schedules and closes the Computers sheet")
    phone.check(mutation_snapshot(state) == before, "navigation keeps the same computer's jobs and sessions")
    phone.check(not visible(state, "home-sessions") and not visible(state, "home-schedules"),
                "Schedules uses its back arrow instead of a button row")
    phone.capture("navigation")


def create_edit(phone):
    state = phone.fixture("schedules")
    before = mutation_snapshot(state)
    phone.tap("schedule-new")
    phone.wait(lambda s: s["scheduler"]["draft"] is not None, "New schedule opens native editable form")
    phone.replace_text("schedule-name", "EmulatorCheck")
    phone.replace_text("schedule-prompt", "SampleOnlyNoProvider")
    phone.wait(lambda s: s["scheduler"]["draft"]["name_chars"] == len("EmulatorCheck")
               and s["scheduler"]["draft"]["prompt_chars"] == len("SampleOnlyNoProvider"),
               "native fields retain the exact sample name/prompt character counts")
    phone.tap("schedule-model")
    phone.wait(lambda s: s["sheet"] == "Model", "schedule chooses model through the existing native picker")
    Phone.tap(phone, "model", 0)
    phone.wait(lambda s: s["sheet"] is None and bool(s["model"]), "model selection returns to intact schedule draft")
    phone.replace_text("schedule-cron", "invalid")
    phone.tap("schedule-preview")
    phone.wait(lambda s: bool(s["scheduler"]["preview_error"]), "invalid cron is rejected by preview")
    phone.check(mutation_snapshot(phone.state()) == before, "invalid preview creates no schedule or session")
    phone.tap("schedule-review")
    phone.check(phone.state()["scheduler"]["confirmation"] is None, "invalid preview cannot reach enable confirmation")
    phone.capture("invalid-preview")
    phone.replace_text("schedule-cron", "15 10 * * 1-5")
    phone.replace_text("schedule-timezone", "UTC")
    phone.tap("schedule-preview")
    state = phone.wait(lambda s: len(s["scheduler"]["preview"]) >= 3 and not s["scheduler"]["preview_error"],
                       "valid cron/timezone previews upcoming occurrences")
    phone.check(mutation_snapshot(state) == before, "preview is read-only")
    phone.check(all(first < second for first, second in zip(state["scheduler"]["preview"], state["scheduler"]["preview"][1:])),
                "preview instants are strictly increasing")
    phone.replace_text("schedule-timezone", "Europe/Amsterdam")
    phone.tap("schedule-review")
    phone.check(phone.state()["scheduler"]["confirmation"] is None, "changing timezone invalidates prior preview authorization")
    phone.tap("schedule-preview")
    phone.wait(lambda s: len(s["scheduler"]["preview"]) >= 3 and not s["scheduler"]["preview_error"],
               "changed timezone requires a fresh preview before review")
    phone.capture("valid-preview")
    phone.tap("schedule-review")
    state = phone.wait(lambda s: s["scheduler"]["confirmation"] == "save", "review asks for explicit enable confirmation")
    phone.check(not state["scheduler"]["consent"], "unattended execution starts without consent")
    phone.tap("schedule-confirm")
    phone.check(mutation_snapshot(phone.state()) == before, "confirmation without consent cannot enable or create")
    phone.capture("enable-confirmation")
    phone.tap("schedule-consent")
    phone.tap("schedule-confirm")
    state = phone.wait(lambda s: len(definitions(s)) == len(before["definitions"]) + 1,
                       "explicit consent creates one enabled sample schedule")
    new_id = (set(definitions(state)) - {job["id"] for job in before["definitions"]}).pop()
    phone.check(definitions(state)[new_id]["enabled"], "new sample schedule is enabled only after consent")
    phone.check(definitions(state)[new_id]["name"] == "EmulatorCheck"
                and definitions(state)[new_id]["cron"] == "15 10 * * 1-5"
                and definitions(state)[new_id]["timezone"] == "Europe/Amsterdam",
                "creation stores the exact reviewed native draft settings")
    phone.check(state["sessions"] == before["sessions"], "creation does not admit a session")
    phone.tap("schedule-toggle")
    phone.wait(lambda s: not definitions(s)[new_id]["enabled"], "Pause stops future occurrences")
    paused = mutation_snapshot(phone.state())
    phone.tap("schedule-edit")
    phone.wait(lambda s: s["scheduler"]["draft"] is not None, "Edit opens existing schedule")
    phone.replace_text("schedule-name", "EmulatorEdited")
    phone.replace_text("schedule-cron", "30 11 * * 1-5")
    phone.tap("schedule-preview")
    phone.wait(lambda s: len(s["scheduler"]["preview"]) >= 3, "edited schedule previews new future times")
    phone.tap("schedule-review")
    phone.wait(lambda s: s["scheduler"]["confirmation"] == "save", "edit requires reviewed confirmation")
    phone.tap("schedule-consent")
    phone.tap("schedule-confirm")
    state = phone.wait(lambda s: definitions(s)[new_id]["revision"] > next(
        job["revision"] for job in paused["definitions"] if job["id"] == new_id), "edit increments saved revision")
    phone.check(not definitions(state)[new_id]["enabled"], "editing a paused schedule keeps it paused")
    phone.check(definitions(state)[new_id]["name"] == "EmulatorEdited"
                and definitions(state)[new_id]["cron"] == "30 11 * * 1-5",
                "edit stores the changed reviewed settings")
    phone.check(state["sessions"] == before["sessions"], "editing and pausing preserve existing sessions")
    phone.capture("edited-paused")
    phone.tap("schedule-toggle")
    phone.wait(lambda s: definitions(s)[new_id]["enabled"], "Resume enables future occurrences")
    phone.capture("resumed")


def run_history_delete(phone):
    state = phone.fixture("schedules")
    enabled = next(job for job in definitions(state).values() if job["enabled"])
    job_id = enabled["id"]
    phone.tap(schedule_row(state, job_id))
    phone.wait(lambda s: s["scheduler"]["selected_id"] == job_id, "schedule row opens detail")
    prior_occurrences, prior_sessions = occurrences(state), state["sessions"]
    phone.tap("schedule-toggle")
    phone.wait(lambda s: not definitions(s)[job_id]["enabled"], "run-now test uses a paused schedule")
    phone.check(occurrences(phone.state()) == prior_occurrences and phone.state()["sessions"] == prior_sessions,
                "pausing future work does not stop or remove existing occurrences/sessions")
    before = mutation_snapshot(phone.state())
    phone.tap("schedule-run-now")
    phone.wait(lambda s: s["scheduler"]["confirmation"] == "run", "Run now asks before admission")
    phone.check(mutation_snapshot(phone.state()) == before, "opening Run now does not submit")
    phone.tap("schedule-cancel")
    phone.wait(lambda s: s["scheduler"]["confirmation"] is None, "Run now cancellation closes confirmation")
    phone.check(mutation_snapshot(phone.state()) == before, "cancel creates no occurrence/session")
    phone.tap("schedule-run-now")
    phone.wait(lambda s: s["scheduler"]["confirmation"] == "run", "second Run now confirmation is explicit")
    phone.tap("schedule-consent")
    phone.tap("schedule-confirm")
    state = phone.wait(lambda s: len(occurrences(s)) == len(before["occurrences"]) + 1,
                       "confirmed Run now admits exactly one sample occurrence")
    new = next(item for item in occurrences(state) if item["id"] not in {old["id"] for old in before["occurrences"]})
    phone.check(new["status"] == "accepted", "sample admission is accepted, not completed")
    phone.check(new["session_id"] is None and state["sessions"] == before["sessions"],
                "inert sample admission starts no provider or new session")
    phone.check(definitions(state)[job_id] == next(job for job in before["definitions"] if job["id"] == job_id),
                "Run now leaves pause state/revision/next recurring time unchanged")
    phone.capture("admitted-not-completed")
    if "ScheduleHistory" not in phone.state()["route"]:
        phone.tap("schedule-history")
    phone.wait(lambda s: "ScheduleHistory" in s["route"], "occurrence history opens")
    statuses = {item["status"] for item in occurrences(phone.state())}
    phone.check({"accepted", "running", "completed", "interrupted", "skipped", "missed"} <= statuses,
                "history keeps accepted, completion, interruption and non-execution distinct")
    phone.capture("history")
    for status in ("completed", "interrupted"):
        item = next(item for item in occurrences(phone.state()) if item["status"] == status and item["session_id"] is not None)
        phone.reveal(history_link(phone.state(), item["id"]))
        phone.capture(f"history-{status}")
    linked = next(item for item in occurrences(phone.state()) if item["status"] == "completed" and item["session_id"] is not None)
    phone.tap(history_link(phone.state(), linked["id"]))
    phone.wait(lambda s: s["route"] == f"Thread(SessionId({linked['session_id']}))",
               "history opens the exact linked normal session")
    phone.capture("history-session")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda s: "ScheduleHistory" in s["route"], "session Back returns to occurrence history")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda s: s["scheduler"]["selected_id"] == job_id and "ScheduleHistory" not in s["route"],
               "history Back returns to schedule")
    sessions = phone.state()["sessions"]
    phone.tap("schedule-delete")
    phone.wait(lambda s: s["scheduler"]["confirmation"] == "delete", "Delete requires explicit confirmation")
    phone.tap("schedule-cancel")
    phone.check(job_id in definitions(phone.state()), "delete cancellation preserves definition")
    phone.tap("schedule-delete")
    phone.wait(lambda s: s["scheduler"]["confirmation"] == "delete", "second Delete confirmation is explicit")
    phone.tap("schedule-consent")
    phone.tap("schedule-confirm")
    state = phone.wait(lambda s: job_id not in definitions(s), "confirmed deletion removes only future scheduling")
    phone.check(state["sessions"] == sessions, "deleting schedule preserves every linked session")
    phone.capture("deleted-session-preserved")
    phone.tap("schedule-back")
    phone.wait(lambda s: s["route"] == "Sessions", "deleted schedule leaves normal Sessions accessible")
    phone.tap(f"session-row-{linked['session_id']}")
    phone.wait(lambda s: s["route"] == f"Thread(SessionId({linked['session_id']}))",
               "preserved historical session still opens after schedule deletion")
    phone.capture("preserved-session-reopened")


def unavailable(phone):
    for fixture, status in (("schedule-offline", "offline"), ("schedule-setup", "setup_required"),
                            ("schedule-stopped", "setup_required"), ("schedule-unsupported", "unsupported")):
        state = phone.fixture(fixture)
        phone.check(state["scheduler"]["status"] == status, f"{status} is explicit, not paused/live")
        before = mutation_snapshot(state)
        for selector in ("schedule-new", "schedule-toggle", "schedule-run-now", "schedule-edit", "schedule-delete"):
            if visible(state, selector):
                phone.tap(selector)
                phone.check(mutation_snapshot(phone.state()) == before, f"{status} cannot mutate via {selector}")
        phone.check(mutation_snapshot(phone.state()) == before, f"{status} does not queue a hidden retry")
        phone.capture(fixture)


def consent_wrapping(phone):
    for fixture in ("schedules", "schedules-evening"):
        for kind in ("save", "run", "delete"):
            state = phone.fixture(fixture)
            before = mutation_snapshot(state)
            phone.tap(schedule_row(state, next(iter(definitions(state)))))
            if kind == "save":
                phone.tap("schedule-edit")
                phone.tap("schedule-preview")
                phone.wait(lambda s: len(s["scheduler"]["preview"]) >= 3, "existing sample draft previews")
                phone.tap("schedule-review")
            else:
                phone.tap("schedule-run-now" if kind == "run" else "schedule-delete")
            phone.wait(lambda s: s["scheduler"]["confirmation"] == kind, f"{kind} asks for explicit consent")
            state = phone.reveal("schedule-consent")
            phone.check(consent_fits(state), f"{fixture} {kind}: full consent label wraps inside its row")
            phone.check(not state["scheduler"]["consent"], "consent is not preselected")
            x, y, width, height = control_rect(state, "schedule-consent-label")
            scale = state["scale"]
            phone.run("shell", "input", "tap", round((x + width / 2) * scale), round((y + height - 11) * scale))
            phone.wait(lambda s: s["scheduler"]["consent"], "the last wrapped line toggles consent")
            phone.capture(f"{fixture}-{kind}-consent")
            phone.tap("schedule-consent")
            phone.wait(lambda s: not s["scheduler"]["consent"], "consent can be withdrawn")
            phone.tap("schedule-confirm")
            phone.check(mutation_snapshot(phone.state()) == before, "without consent no job or session is changed")
            phone.tap("schedule-cancel")
            phone.wait(lambda s: s["route"] != "ScheduleConfirm", "cancel leaves the confirmation")


def rendering(phone):
    for fixture, appearance in (("schedules", "light"), ("schedules-evening", "dark")):
        state = phone.fixture(fixture)
        phone.check(state["scheduler"]["appearance"] == appearance, f"{appearance} fixture renders native scheduler")
        for selector in ("schedule-back", "schedule-refresh", "schedule-new"):
            phone.check(visible(state, selector), f"{appearance} {selector} is inside viewport")
        phone.capture(f"{appearance}-schedules")
        job_id = next(iter(definitions(state)))
        phone.tap(schedule_row(state, job_id))
        phone.capture(f"{appearance}-detail")


CASES = {"navigation": navigation, "create-edit": create_edit,
         "run-history-delete": run_history_delete, "unavailable": unavailable, "rendering": rendering,
         "consent-wrapping": consent_wrapping}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", action="append", required=True, help="Repeat for each available emulator/device")
    parser.add_argument("--adb", help="Explicit platform-tools/adb path; SDK/PATH otherwise")
    parser.add_argument("--output", type=Path, required=True, help="New directory beneath ignored artifacts/")
    parser.add_argument("--case", choices=CASES, action="append", help="Default: all focused native cases")
    parser.add_argument("--geometry", choices=("native", *GEOMETRIES), action="append", help="Default: native; overrides restored; emulator only")
    parser.add_argument("--apk", type=Path, help="Optional installed APK path to record its SHA-256; never installed by this script")
    args = parser.parse_args()
    artifacts = Path(__file__).resolve().parents[3] / "artifacts"
    output = args.output.resolve()
    if not output.is_relative_to(artifacts.resolve()) or output == artifacts.resolve():
        parser.error("Keep verification outputs in a new directory under artifacts/")
    if output.exists():
        parser.error("Output already exists; preserve prior evidence and choose a new directory")
    adb = adb_path(args.adb)
    output.mkdir(parents=True)
    invocation = {"argv": [sys.executable, *sys.argv], "command": shlex.join([sys.executable, *sys.argv]),
                  "started": datetime.now(timezone.utc).isoformat(), "visual_review": "required; automated checks do not certify appearance"}
    if args.apk:
        invocation["apk"] = str(args.apk.resolve())
        invocation["apk_sha256"] = digest(args.apk.read_bytes())
    (output / "invocation.json").write_text(json.dumps(invocation, indent=2) + "\n")
    runs = []
    for serial in dict.fromkeys(args.serial):
        if not re.fullmatch(r"[A-Za-z0-9_.:-]+", serial):
            parser.error("Unsafe ADB serial")
        for preset in dict.fromkeys(args.geometry or ["native"]):
            phone = SchedulerPhone(serial, output / serial / preset, adb)
            result = {"serial": serial, "geometry": preset, "passed": False}
            try:
                phone.preflight()
                with geometry(phone, preset):
                    for name in args.case or CASES:
                        try:
                            CASES[name](phone)
                        except Exception:
                            # Capture at the failing geometry before restoration.
                            try:
                                phone.capture(f"failure-{name}")
                            except Exception as capture_error:
                                result["capture_error"] = str(capture_error)
                            raise
                result["passed"] = True
            except Exception as error:
                result["error"] = str(error)
                try:
                    phone.capture("failure")
                except Exception as capture_error:
                    result["capture_error"] = str(capture_error)
            finally:
                try:
                    phone.finish()
                except Exception as error:
                    result["passed"] = False
                    result["finalization_error"] = str(error)
                runs.append(result)
                (output / "report.json").write_text(json.dumps(runs, indent=2) + "\n")
            print(json.dumps(result), flush=True)
    if not all(run["passed"] for run in runs):
        raise SystemExit(1)
    print(f"PASS {len(runs)} device/geometry runs. Visually inspect screenshots in {output}.")


if __name__ == "__main__":
    main()
