#!/usr/bin/env python3
"""Synthetic-only, app-UID OAuth/Keystore/UI test. Never accepts real codes or credentials."""
import argparse
import base64
import json
from pathlib import Path
import re
import subprocess
import time
import xml.etree.ElementTree as ET

PACKAGE = "dev.pi.android.quickjstest"
ACTIVITY = PACKAGE + "/dev.pi.gpui.GpuiActivity"
ROOT = "no_backup/local-providers/synthetic/"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adb", default="adb")
    parser.add_argument("--serial", default="emulator-5554")
    parser.add_argument("--output", default="artifacts/provider-auth-emulator")
    args = parser.parse_args()
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=True)
    pids = set()

    def command(*parts, data=None, check=True):
        return subprocess.run([args.adb, "-s", args.serial, *parts], input=data,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20, check=check).stdout

    def read(path):
        return command("shell", "run-as", PACKAGE, "cat", path)

    def ui():
        try:
            value = json.loads(read("files/quickjs-provider-test/ui.json"))
            assert value["synthetic"] is True
            return value
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            return {}

    def wait(predicate, seconds=15):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            state = ui()
            if state and predicate(state):
                return state
            time.sleep(0.08)
        raise AssertionError("synthetic provider UI timeout: " + json.dumps(ui()))

    def launch(restart=False):
        if restart:
            command("shell", "am", "force-stop", PACKAGE)
        command("shell", "am", "start", "-W", "-a", "android.intent.action.VIEW", "-d",
                "pi://test/local-providers/synthetic", "-n", ACTIVITY)
        pid = int(command("shell", "pidof", PACKAGE).decode().split()[0])
        pids.add(pid)
        wait(lambda s: s["pid"] == pid and s["visible"] and not s["accounts"]["busy"])

    def tap(name):
        pid = int(command("shell", "pidof", PACKAGE).decode().split()[0])
        before = ui()
        frame = before.get("frame", -1) if before.get("pid") == pid else -1
        def bounds(s):
            return next((v for k, v in s.get("bounds", {}).items() if k == f'Name("{name}")'), None)
        state = wait(lambda s: s["pid"] == pid and s["visible"] and s["frame"] > frame and bounds(s) is not None)
        x, y, w, h = bounds(state)
        scale = state["scale"]
        command("shell", "input", "tap", str(round((x + w / 2) * scale)), str(round((y + h / 2) * scale)))

    def saved(provider, value):
        return wait(lambda s: not s["accounts"]["busy"] and s["accounts"][provider]["saved"] is value)

    def logout(provider):
        tap(f"provider-{provider}-logout")
        saved(provider, False)
        # Account may already be empty; ensure the queued worker has had time to publish.
        time.sleep(0.15)
        wait(lambda s: not s["accounts"]["busy"])

    def enter_code():
        wait(lambda s: s["status"] == "prompt")
        tap("provider-code")
        wait(lambda s: s["dialog"])
        time.sleep(0.4)
        command("shell", "input", "text", "synthetic-code")
        # Native dialog only, and synthetic input only. No accessibility dump is retained.
        dump = "/data/local/tmp/pi-synthetic-provider-dialog.xml"
        try:
            command("shell", "uiautomator", "dump", dump)
            xml = command("shell", "cat", dump).decode()
        finally:
            command("shell", "rm", "-f", dump)
        tree = ET.fromstring(xml)
        assert any(n.get("password") == "true" for n in tree.iter("node"))
        assert "synthetic-code" not in xml, "native secret field must mask its contents"
        button = next(n for n in tree.iter("node") if n.get("resource-id") == "android:id/button1"
                      and n.get("package") == PACKAGE and n.get("text", "").lower() == "continue")
        x1, y1, x2, y2 = map(int, re.findall(r"\d+", button.get("bounds")))
        command("shell", "input", "tap", str((x1 + x2) // 2), str((y1 + y2) // 2))
        wait(lambda s: s["status"] == "connected" and not s["dialog"] and s["accounts"]["anthropic"]["saved"])

    def envelope(provider):
        raw = read(ROOT + provider + ".json")
        for secret in [b"synthetic-claude-access", b"synthetic-claude-refresh", b"synthetic-codex-refresh", b"eyJhbGciOiJub25l"]:
            assert secret not in raw, "plaintext token in credential file"
        value = json.loads(raw)
        assert len(base64.b64decode(value["iv"])) == 12
        assert len(base64.b64decode(value["ciphertext"])) > 16
        return value

    def replace(provider, value):
        # Only the separately namespaced synthetic fixture file can be modified.
        assert provider in ["anthropic", "openai-codex"]
        command("shell", "-T", "run-as", PACKAGE, "sh", "-c", f"'cat > {ROOT}{provider}.json'",
                data=json.dumps(value).encode())

    launch(restart=True)
    for provider in ["anthropic", "openai-codex"]:
        logout(provider)
    tap("provider-anthropic-login")
    wait(lambda s: s["status"] == "prompt")
    command("shell", "input", "keyevent", "KEYCODE_HOME")
    wait(lambda s: not s["visible"] and s["status"] == "prompt")
    command("shell", "am", "start", "-W", "-n", ACTIVITY)
    wait(lambda s: s["visible"] and s["status"] == "prompt")
    enter_code()
    tap("provider-openai-codex-login")
    tap("provider-copy-code")
    wait(lambda s: s["copied_code_verified"])
    wait(lambda s: s["provider"] == "openai-codex" and s["status"] == "connected" and s["accounts"]["openai-codex"]["saved"])
    before = {provider: envelope(provider) for provider in ["anthropic", "openai-codex"]}
    launch(restart=True)
    for provider in before:
        saved(provider, True)
        assert envelope(provider) == before[provider], "restart must not silently refresh or rewrite"
        tap(f"provider-{provider}-login")  # existing account: Check / refresh
        wait(lambda s: s["provider"] == provider and s["status"] == "connected")
        after = envelope(provider)
        assert after["generation"] != before[provider]["generation"] and after["iv"] != before[provider]["iv"]
    # AES-GCM authentication failure and provider isolation.
    tampered = envelope("anthropic")
    data = bytearray(base64.b64decode(tampered["ciphertext"]))
    data[0] ^= 1
    tampered["ciphertext"] = base64.b64encode(data).decode()
    replace("anthropic", tampered)
    launch(restart=True)
    wait(lambda s: s["accounts"]["anthropic"]["error"] and s["accounts"]["openai-codex"]["saved"])
    logout("anthropic")
    tap("provider-anthropic-login")
    enter_code()
    # A failed atomic write preserves the existing record (synthetic namespace only).
    original = envelope("anthropic")
    try:
        command("shell", "run-as", PACKAGE, "chmod", "500", ROOT)
        tap("provider-anthropic-login")
        wait(lambda s: s["provider"] == "anthropic" and s["status"] == "failed")
        assert envelope("anthropic") == original
    finally:
        command("shell", "run-as", PACKAGE, "chmod", "700", ROOT)
    # Restoring ciphertext after logout's key deletion must fail, never regenerate a key
    # and pretend the unreadable account was absent.
    logout("anthropic")
    replace("anthropic", original)
    launch(restart=True)
    wait(lambda s: s["accounts"]["anthropic"]["error"] and s["accounts"]["openai-codex"]["saved"])
    logout("anthropic")
    tap("provider-anthropic-login")
    enter_code()
    # Generation is authenticated data, not an independently mutable CAS counter.
    tampered = envelope("anthropic")
    tampered["generation"] = "synthetic-tampered-generation"
    replace("anthropic", tampered)
    launch(restart=True)
    wait(lambda s: s["accounts"]["anthropic"]["error"] and s["accounts"]["openai-codex"]["saved"])
    logout("anthropic")
    logout("openai-codex")
    # Dismissing the private entry dialog does not submit a stale code or end sign-in.
    tap("provider-anthropic-login")
    wait(lambda s: s["status"] == "prompt")
    tap("provider-code")
    wait(lambda s: s["dialog"])
    time.sleep(0.3)
    command("shell", "input", "keyevent", "KEYCODE_BACK")
    wait(lambda s: not s["dialog"] and s["status"] == "prompt")
    tap("provider-cancel")
    wait(lambda s: s["status"] == "cancelled")
    # Back cancels interactive login rather than retaining a hidden attempt.
    tap("provider-anthropic-login")
    wait(lambda s: s["status"] == "prompt")
    command("shell", "input", "keyevent", "KEYCODE_BACK")
    wait(lambda s: not s["visible"] and s["status"] == "cancelled")
    logs = b"\n".join(command("logcat", "-d", f"--pid={pid}") for pid in pids)
    telemetry = read("files/quickjs-provider-test/ui.json")
    for secret in [b"synthetic-claude-access", b"synthetic-claude-refresh", b"synthetic-codex-refresh", b"eyJhbGciOiJub25l", b"code_verifier", b"code_challenge", b"TEST-CODE"]:
        assert secret not in logs + telemetry, "secret in logs/telemetry"
    assert b"FATAL EXCEPTION" not in logs and b"panicked at" not in logs
    (out / "logcat.txt").write_bytes(logs)
    result = {"passed": True, "package": PACKAGE, "uid": int(command("shell", "run-as", PACKAGE, "id", "-u")), "pids": sorted(pids), "synthetic_only": True,
              "checks": ["Claude native code entry", "browser-style background survival", "Codex device flow", "explicit sign-in code copy",
                         "encrypted persistence across restart", "refresh rotation", "ciphertext tamper rejection",
                         "AAD generation tamper rejection", "missing key rejection", "failed-write preservation", "other-provider isolation", "logout/reset", "native dialog dismissal", "explicit cancellation", "Back cancellation", "log redaction"]}
    (out / "report.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS: app-UID Claude/Codex synthetic login, Keystore persistence, refresh, tamper rejection, logout and cancellation")


if __name__ == "__main__":
    main()
