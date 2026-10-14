#!/usr/bin/env python3
"""Verify the offline QuickJS backend inside the separately packaged Android app UID.

Build/install --quickjs-backend-test first. Never targets dev.pi.android or clears data.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

PACKAGE = "dev.pi.android.quickjstest"
ACTIVITY = f"{PACKAGE}/dev.pi.gpui.GpuiActivity"
DIRECTORY = "files/quickjs-backend-test"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--adb", default="adb")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    adb = [args.adb, "-s", args.serial]

    def command(*parts):
        return subprocess.check_output(adb + list(parts), stderr=subprocess.STDOUT, timeout=15)

    def private_json(name):
        try:
            return json.loads(command("exec-out", "run-as", PACKAGE, "cat", f"{DIRECTORY}/{name}.json"))
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            return None

    def wait(predicate, name="report", timeout=20):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            last = private_json(name)
            if last and predicate(last):
                return last
            time.sleep(0.1)
        raise AssertionError(f"{name} did not reach expected state; last={last}")

    def open_screen(suffix=""):
        command("shell", "am", "start", "-n", ACTIVITY, "-a", "android.intent.action.VIEW",
                "-d", "pi://test/quickjs-backend" + suffix)
        time.sleep(0.4)

    def tap(name):
        def bounds(state):
            return next((value for key, value in state.get("bounds", {}).items()
                         if name in key and value[2] > 0 and value[3] > 0), None)
        pid = int(command("shell", "pidof", PACKAGE).decode().split()[0])
        before = private_json("ui") or {}
        frame = before.get("frame", -1) if before.get("pid") == pid else -1
        state = wait(lambda state: state.get("pid") == pid and state.get("visible") is True
                     and state.get("frame", -1) > frame and bounds(state) is not None, name="ui")
        x, y, width, height = bounds(state)
        scale = state["scale"]
        command("shell", "input", "tap", str(round((x + width / 2) * scale)),
                str(round((y + height / 2) * scale)))

    def start(button, completed=False):
        before = private_json("report")
        old_id = before and before["run_id"]
        tap(button)
        report = wait(lambda report: report["run_id"] != old_id and
                      (report["status"] == "passed" if completed else report["request_seen"] and report["status"] == "running"))
        assert report["os"] == "android" and report["runtime"] == "QuickJS"
        assert report["uid"] == uid and report["uid"] >= 10000
        assert str(report["pid"]) in command("shell", "pidof", PACKAGE).decode().split()
        assert report["live_provider"] is False and report["provider"] == "loopback-fixture"
        if completed:
            assert report["tools"] == 3 and report["agent_ends"] == 1
            assert report["text"] == "Saved 🐷" and report["fixture_files_cleaned"]
            assert report["transport_closed"]
        return report

    def stopped(run_id, reason):
        report = wait(lambda report: report["run_id"] == run_id and report["status"] == "cancelled")
        assert report["cancel_reason"] == reason, report
        assert report["transport_closed"] and report["fixture_files_cleaned"], report
        return report

    results = {}
    command("shell", "pm", "path", PACKAGE)
    uid = int(command("shell", "run-as", PACKAGE, "id", "-u").strip())
    command("shell", "am", "force-stop", PACKAGE)
    try:
        open_screen()
        results["round_trip"] = start("quickjs-run", completed=True)
        time.sleep(0.4)
        (args.output / "passed.png").write_bytes(command("exec-out", "screencap", "-p"))

        run = start("quickjs-stall")
        open_screen("/run")  # Duplicate admission must not replace the in-flight fixture.
        assert private_json("report")["run_id"] == run["run_id"]
        tap("quickjs-stop")
        results["stop_and_duplicate"] = stopped(run["run_id"], "stop")

        run = start("quickjs-stall")
        command("shell", "input", "keyevent", "KEYCODE_HOME")
        results["background"] = stopped(run["run_id"], "background")
        wait(lambda state: state["visible"] is False and state["bounds"] == {}, name="ui")
        open_screen()
        assert private_json("report")["run_id"] == run["run_id"], "resume must not restart a run"

        run = start("quickjs-stall")
        command("shell", "input", "keyevent", "KEYCODE_BACK")
        results["back"] = stopped(run["run_id"], "back")
        wait(lambda state: state["visible"] is False and state["bounds"] == {}, name="ui")
        open_screen()
        results["restart"] = start("quickjs-run", completed=True)
        children = command("exec-out", "run-as", PACKAGE, "ls", "-a", DIRECTORY).decode().split()
        assert not any(name.startswith(".tmp") for name in children), children
        (args.output / "report.json").write_text(json.dumps(results, indent=2) + "\n")
        print(f"PASS: APK UID {uid}: streamed tool round-trip, duplicate guard, Stop, background, Back, restart")
    finally:
        try:
            pid = command("shell", "pidof", PACKAGE).decode().strip().split()[0]
            log = command("logcat", "-d", f"--pid={pid}", "-t", "1000")
            (args.output / "logcat.txt").write_bytes(log)
            assert b"FATAL EXCEPTION" not in log and b"panicked at" not in log
        except (subprocess.SubprocessError, IndexError) as error:
            print(f"Could not collect app log: {error}")


if __name__ == "__main__":
    main()
