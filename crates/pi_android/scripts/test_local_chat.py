#!/usr/bin/env python3
"""Synthetic-only local chat UI test. Uses only the isolated test APK/synthetic vault."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import time
import xml.etree.ElementTree as ET

PACKAGE = "dev.pi.android.quickjstest"
ACTIVITY = PACKAGE + "/dev.pi.gpui.GpuiActivity"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adb", default="adb")
    parser.add_argument("--serial", default="emulator-5554")
    parser.add_argument("--output", default="artifacts/local-chat-emulator")
    args = parser.parse_args()
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=True)
    pids = set()

    def command(*parts, check=True):
        return subprocess.run([args.adb, "-s", args.serial, *parts], stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, timeout=25, check=check).stdout

    def state(kind="chat"):
        try:
            folder = "quickjs-chat-test" if kind == "chat" else "quickjs-provider-test"
            value = json.loads(command("shell", "run-as", PACKAGE, "cat", f"files/{folder}/ui.json"))
            assert value["synthetic"] is True
            return value
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            return {}

    def wait(predicate, kind="chat", seconds=20):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            value = state(kind)
            if value and predicate(value):
                return value
            time.sleep(.08)
        raise AssertionError(f"{kind} timed out: {json.dumps(state(kind))}")

    def tap(name, kind="chat"):
        # Shared sheets animate into place; fresh telemetry alone need not be
        # final geometry. Let the preceding transition settle before sampling.
        if kind == "chat":
            time.sleep(.55)
        pid = int(command("shell", "pidof", PACKAGE).decode().split()[0])
        before = state(kind)
        frame = before.get("frame", -1) if before.get("pid") == pid else -1
        def rect(value):
            return next((v for k, v in value.get("bounds", {}).items()
                         if k in [name, f'Name("{name}")']), None)
        value = wait(lambda s: s["pid"] == pid and s["visible"] and s["frame"] > frame and rect(s), kind)
        x, y, w, h = rect(value)
        scale = value["scale"]
        command("shell", "input", "tap", str(round((x+w/2)*scale)), str(round((y+h/2)*scale)))

    def launch():
        command("shell", "am", "force-stop", PACKAGE)
        command("shell", "am", "start", "-W", "-a", "android.intent.action.VIEW", "-d",
                "pi://test/local-providers/synthetic", "-n", ACTIVITY)
        pid = int(command("shell", "pidof", PACKAGE).decode().split()[0])
        pids.add(pid)
        wait(lambda s: s["pid"] == pid and s["visible"] and not s["accounts"]["busy"], "auth")

    def logout(provider):
        tap(f"provider-{provider}-logout", "auth")
        wait(lambda s: not s["accounts"]["busy"] and not s["accounts"][provider]["saved"], "auth")

    def login(provider):
        tap(f"provider-{provider}-login", "auth")
        if provider == "anthropic":
            wait(lambda s: s["status"] == "prompt", "auth")
            tap("provider-code", "auth")
            wait(lambda s: s["dialog"], "auth")
            time.sleep(.4)
            command("shell", "input", "text", "synthetic-code")
            dump = "/data/local/tmp/pi-synthetic-chat-dialog.xml"
            try:
                command("shell", "uiautomator", "dump", dump)
                tree = ET.fromstring(command("shell", "cat", dump).decode())
            finally:
                command("shell", "rm", "-f", dump)
            button = next(n for n in tree.iter("node") if n.get("resource-id") == "android:id/button1"
                          and n.get("package") == PACKAGE)
            x1, y1, x2, y2 = map(int, re.findall(r"\d+", button.get("bounds")))
            command("shell", "input", "tap", str((x1+x2)//2), str((y1+y2)//2))
        wait(lambda s: s["provider"] == provider and s["status"] == "connected"
             and not s["accounts"]["busy"] and s["accounts"][provider]["saved"], "auth")

    def open_chat(provider):
        wait(lambda s: s["visible"] and not s["accounts"]["busy"] and s["accounts"][provider]["saved"], "auth")
        tap(f"provider-{provider}-chat", "auth")
        return wait(lambda s: s["visible"] and s["provider"] == provider)

    def send(text="fixture-question"):
        tap("draft")
        # Android's IME animation moves the shared bottom composer. Do not tap
        # the previous frame's Send coordinates while the keyboard is opening.
        time.sleep(.65)
        command("shell", "input", "text", text)
        time.sleep(.25)
        # Honor the user's return-sends preference; use the explicit Send control.
        tap("send")

    def link(url):
        command("shell", "am", "start", "-W", "-a", "android.intent.action.VIEW", "-d", url, "-n", ACTIVITY)

    def mode(name):
        link("pi://test/local-chat/" + name.lower())
        wait(lambda s: s["mode"] == name)

    def back():
        # The ordinary thread Back goes to the session list. Account management
        # lives in that list's existing Computers menu.
        tap("back")
        wait(lambda s: s["route"] == "Sessions" and not s["running"])
        tap("backend-picker")
        tap("local-accounts")
        wait(lambda s: s["visible"], "auth")

    launch()
    for provider in ["anthropic", "openai-codex"]:
        logout(provider)
        login(provider)
        initial = open_chat(provider)
        assert initial["status"] == "idle" and initial["turns"] == 0 and not initial["running"]
        time.sleep(.4)
        assert state()["status"] == "idle", "opening chat must not infer"
        assert "attach" in state()["bounds"] and "composer-thinking" in state()["bounds"]
        tap("backend-picker")
        wait(lambda s: s["sheet"] == "Computers")
        (out / (provider + "-backends.png")).write_bytes(command("exec-out", "screencap", "-p"))
        tap("backend-local")
        wait(lambda s: s["route"] == initial["route"] and s["sheet"] is None)
        tap("composer-thinking")
        tap("thinking-choice-3")
        wait(lambda s: s["thinking"] == "Medium" and s["sheet"] is None)
        send()
        wait(lambda s: s["status"] == "done" and s["context_messages"] == 2 and s["reply_verified"])
        send("fixture-followup")
        wait(lambda s: s["status"] == "done" and s["context_messages"] == 4 and s["reply_verified"])
        (out / (provider + ".png")).write_bytes(command("exec-out", "screencap", "-p"))
        tap("composer-model")
        tap("model-choice-1")
        wait(lambda s: s["model"] != initial["model"] and s["turns"] == 2 and s["context_messages"] == 4)
        mode("Stall")
        send("fixture-stop")
        wait(lambda s: s["running"] and s["reply_verified"])
        tap("stop")
        wait(lambda s: s["status"] == "stopped" and not s["running"])
        assert state()["context_messages"] == 4
        send("fixture-background")
        wait(lambda s: s["running"] and s["reply_verified"])
        command("shell", "input", "keyevent", "KEYCODE_HOME")
        wait(lambda s: not s["visible"] and s["status"] == "stopped" and not s["running"])
        command("shell", "am", "start", "-W", "-n", ACTIVITY)
        wait(lambda s: s["visible"] and not s["running"])
        assert state()["status"] == "stopped", "resume must not replay inference"
        send("fixture-back")
        wait(lambda s: s["running"] and s["reply_verified"])
        back()
        wait(lambda s: not s["visible"] and not s["running"])
        open_chat(provider)
        mode("Reject")
        send("fixture-rejected")
        wait(lambda s: s["status"] == "auth_error" and not s["running"])
        assert state()["context_messages"] == 4, "reopening the same account must retain conversations"
        mode("Reply")
        tap("back")
        wait(lambda s: s["route"] == "Sessions")
        tap("new-session")
        wait(lambda s: s["route"] == "Start")
        send("fixture-restart")
        wait(lambda s: s["status"] == "done" and s["reply_verified"])
        back()
    # Logout/new login must not retain the previous account's chat, even with the same provider.
    logout("openai-codex")
    login("openai-codex")
    assert open_chat("openai-codex")["turns"] == 0
    launch()
    assert open_chat("openai-codex")["turns"] == 0, "ephemeral chat must not persist in prefs"
    back()
    for provider in ["anthropic", "openai-codex"]:
        logout(provider)
    logs = b"".join(command("logcat", "-d", "--pid", str(pid)) for pid in pids)
    telemetry = json.dumps([state(), state("auth")]).encode()
    for secret in [b"sk-ant-oat", b"synthetic-chat-rotated", b"synthetic-SECRET", b"fixture-question", b"fixture-followup", b"Hello from the phone"]:
        assert secret not in logs + telemetry, "chat/account content leaked into telemetry/logs"
    (out / "logcat.txt").write_bytes(logs)
    (out / "report.json").write_text(json.dumps({"passed": True, "synthetic_only": True,
        "pids": sorted(pids), "checks": ["Claude OAuth SSE", "Codex OAuth SSE", "two-turn context", "model selection",
        "Stop", "background cancellation", "resume without replay", "Back cancellation", "redacted provider error",
        "shared New session", "shared thread/composer/model picker/Stop", "thinking selection", "top-right Pi backend picker", "attachment control present", "account isolation", "ephemeral restart", "log redaction"]}, indent=2) + "\n")
    print("PASS: local Claude/Codex chat, context, model selection, cancellation, isolation and redaction")


if __name__ == "__main__":
    main()
