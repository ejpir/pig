#!/usr/bin/env python3
"""Run only the offline fixture as adb shell, not inside the production APK."""
import argparse
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--serial", required=True, help="explicit emulator/device serial")
parser.add_argument("--target", choices=["x86_64-linux-android", "aarch64-linux-android"], required=True)
parser.add_argument("--adb", default="adb")
parser.add_argument("--output", type=Path, default=Path("generated/android-fixture.txt"))
args = parser.parse_args()
root = Path(__file__).resolve().parent
binary = root / "target" / args.target / "release/pi-quickjs-backend-poc"
if not binary.is_file():
    parser.error(f"build first: {binary}")
adb = [args.adb, "-s", args.serial]

def shell(command, **kwargs):
    kwargs.setdefault("timeout", 15)
    kwargs.setdefault("check", True)
    return subprocess.run(adb + ["shell", command], text=True, **kwargs)

abi = shell("getprop ro.product.cpu.abi", capture_output=True).stdout.strip()
expected = {"x86_64-linux-android": "x86_64", "aarch64-linux-android": "arm64-v8a"}[args.target]
if abi != expected:
    parser.error(f"device ABI is {abi}, not {expected}")
remote = shell("mktemp -d /data/local/tmp/pi-quickjs-poc.XXXXXX", capture_output=True).stdout.strip()
if not re.fullmatch(r"/data/local/tmp/pi-quickjs-poc\.[A-Za-z0-9]+", remote):
    raise RuntimeError(f"unexpected temporary path: {remote!r}")
print(f"Isolated fixture directory: {remote}", flush=True)
try:
    subprocess.run(adb + ["push", str(binary), remote + "/run"], check=True, timeout=30)
    shell(f"chmod 700 {shlex.quote(remote + '/run')}")
    result = shell(f"TMPDIR={shlex.quote(remote)} {shlex.quote(remote + '/run')} --fixture", capture_output=True, timeout=45, check=False)
    evidence = result.stdout + result.stderr
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(evidence)
    result.check_returncode()
    events = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
    assert sum(event["type"] == "agent_end" for event in events) == 1
    assert sum(event["type"] == "tool_execution_end" for event in events) == 3
    assert "PASS: upstream Pi + OpenAI SDK in QuickJS" in evidence
    print(f"PASS: Android {abi}, {len(events)} upstream events; evidence: {args.output}")
finally:
    # Remove only our known executable; Rust cleans its own fixture workspace.
    # Deliberately do not recursively remove unknown files after an interrupted run.
    try:
        shell(f"rm -f {shlex.quote(remote + '/run')}; rmdir {shlex.quote(remote)}")
    except (subprocess.SubprocessError, OSError) as error:
        print(f"Could not clean {remote}; inspect it before retrying: {error}", file=sys.stderr)
