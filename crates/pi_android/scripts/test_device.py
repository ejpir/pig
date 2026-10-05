#!/usr/bin/env python3
"""Capture every phone UI fixture on an ADB device, without using real sessions.

First install the APK built with build_apk.py --ui-test. Screenshots and the
test app's logcat are saved for visual inspection; this is a smoke test, not
an automated assertion that each screenshot looks correct.
"""

import argparse
import json
from pathlib import Path
import re
import subprocess
import time


PACKAGE = "dev.pi.android.uitest"
ACTIVITY = f"{PACKAGE}/dev.pi.gpui.GpuiActivity"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", help="ADB device serial")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--screen", action="append", help="only these named fixtures")
    args = parser.parse_args()
    adb = ["adb"] + (["-s", args.serial] if args.serial else [])

    def run(*command):
        return subprocess.check_output(adb + list(command), stderr=subprocess.STDOUT)

    source = (Path(__file__).resolve().parents[1] / "src" / "preview.rs").read_text()
    declaration = source.split("pub const SCREENS:", 1)[1].split("];", 1)[0]
    available = re.findall(r'"([a-z-]+)"', declaration)
    screens = args.screen or available
    if unknown := set(screens) - set(available):
        parser.error(f"unknown screens: {sorted(unknown)}")
    args.output.mkdir(parents=True, exist_ok=True)
    run("shell", "pm", "path", PACKAGE)
    for name in screens:
        launched = time.time()
        run("shell", "am", "start", "-n", ACTIVITY, "-a",
            "android.intent.action.VIEW", "-d", f"pi://preview/{name}")
        deadline = time.monotonic() + 10
        previous = None
        while True:
            try:
                state = json.loads(run("exec-out", "run-as", PACKAGE, "cat", "files/ui-test-state.json"))
                if state["fixture"] == name and state["time"] > launched:
                    if previous and state["time"] > previous["time"] and state["bounds"] == previous["bounds"]:
                        break
                    previous = state
            except (subprocess.CalledProcessError, json.JSONDecodeError):
                pass
            if time.monotonic() > deadline:
                raise RuntimeError(f"fixture {name} did not render and settle")
            time.sleep(0.2)
        pid = run("shell", "pidof", PACKAGE).decode().strip()
        if not pid:
            raise RuntimeError(f"test app stopped on {name}")
        screenshot = run("exec-out", "screencap", "-p")
        if not screenshot.startswith(b"\x89PNG\r\n\x1a\n"):
            raise RuntimeError(f"invalid screenshot on {name}")
        (args.output / f"{name}.png").write_bytes(screenshot)
        print(f"Captured {name}", flush=True)
    log = run("logcat", "-d", f"--pid={pid}", "-t", "2000")
    (args.output / "logcat.txt").write_bytes(log)
    if b"FATAL EXCEPTION" in log or b"panicked at" in log:
        raise RuntimeError(f"app crash recorded; see {args.output / 'logcat.txt'}")
    print(f"{len(screens)} screens captured in {args.output}; inspect them visually.")


if __name__ == "__main__":
    main()
