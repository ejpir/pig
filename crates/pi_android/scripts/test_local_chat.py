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

    def choose_model(provider):
        model = "claude-sonnet-4-6" if provider == "anthropic" else "gpt-5.5"
        before = state()
        # An idle session composer is one line; typing in it shows the model.
        if "composer-model" not in before["bounds"]:
            tap("draft")
            time.sleep(.65)
        tap("composer-model")
        tap("model-search")
        time.sleep(.65)
        command("shell", "input", "text", model)
        time.sleep(.25)
        tap("model-choice-0")
        value = wait(lambda s: s["provider"] == provider and s["model"] == model and s["sheet"] is None)
        assert value["turns"] == before["turns"] and value["context_messages"] == before["context_messages"]
        return value

    def open_backend():
        command("shell", "input", "keyevent", "KEYCODE_BACK")
        link("pi://test/local-backend/synthetic")
        pid = int(command("shell", "pidof", PACKAGE).decode().split()[0])
        return wait(lambda s: s["pid"] == pid and s["visible"] and s["route"] == "Sessions")

    def new_chat(provider):
        tap("new-session")
        wait(lambda s: s["route"] == "Start")
        return choose_model(provider)

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

    def accounts():
        tap("computers")
        tap("local-accounts")
        wait(lambda s: s["visible"], "auth")

    launch()
    for provider in ["anthropic", "openai-codex"]:
        logout(provider)
    initial_backend = open_backend()
    assert initial_backend["sessions"] == 0 and not initial_backend["running"]
    assert "backend-picker" not in initial_backend["bounds"], "do not add another Pi/menu control"
    tap("computers")
    wait(lambda s: s["sheet"] == "Computers")
    (out / "backends.png").write_bytes(command("exec-out", "screencap", "-p"))
    tap("backend-local")
    wait(lambda s: s["route"] == "Sessions" and s["sheet"] is None)
    # Selecting a signed-out backend must not navigate to provider setup.
    accounts()
    for provider in ["anthropic", "openai-codex"]:
        login(provider)
    assert open_backend()["sessions"] == 0
    for provider in ["anthropic", "openai-codex"]:
        initial = new_chat(provider)
        assert initial["status"] == "idle" and initial["turns"] == 0 and not initial["running"]
        assert "attach" in initial["bounds"] and "composer-thinking" in initial["bounds"]
        assert "backend-picker" not in initial["bounds"]
        tap("composer-thinking")
        tap("thinking-choice-3")
        wait(lambda s: s["thinking"] == "Medium" and s["sheet"] is None)
        send()
        wait(lambda s: s["status"] == "done" and s["context_messages"] == 2 and s["reply_verified"])
        send("fixture-followup")
        wait(lambda s: s["status"] == "done" and s["context_messages"] == 4 and s["reply_verified"])
        route = state()["route"]
        other = "openai-codex" if provider == "anthropic" else "anthropic"
        choose_model(other)
        assert state()["route"] == route
        send("cross-provider-followup")
        wait(lambda s: s["status"] == "done" and s["context_messages"] == 6 and s["reply_verified"])
        choose_model(provider)
        assert state()["route"] == route and state()["context_messages"] == 6
        (out / (provider + ".png")).write_bytes(command("exec-out", "screencap", "-p"))
        mode("Stall")
        send("fixture-stop")
        wait(lambda s: s["running"] and s["reply_verified"])
        tap("stop")
        wait(lambda s: s["status"] == "stopped" and not s["running"])
        assert state()["context_messages"] == 6
        send("fixture-background")
        wait(lambda s: s["running"] and s["reply_verified"])
        command("shell", "input", "keyevent", "KEYCODE_HOME")
        wait(lambda s: not s["visible"] and s["status"] == "stopped" and not s["running"])
        command("shell", "am", "start", "-W", "-n", ACTIVITY)
        wait(lambda s: s["visible"] and not s["running"])
        assert state()["status"] == "stopped", "resume must not replay inference"
        mode("Reject")
        send("fixture-rejected")
        wait(lambda s: s["status"] == "auth_error" and not s["running"])
        assert state()["context_messages"] == 6
        tap("back")
        wait(lambda s: s["route"] == "Sessions")
        accounts()
        logout(provider)
        login(provider)
        reopened = open_backend()
        assert reopened["sessions"] >= 1, "account management is not a chat destination or session reset"
        mode("Reply")
    # Conversations stay memory-only; saved accounts survive process restart.
    launch()
    assert open_backend()["sessions"] == 0
    accounts()
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
        "shared New session", "shared thread/composer/model picker/Stop", "thinking selection", "existing Pi/computer backend selector", "in-chat cross-provider history", "account management without session replacement", "attachment control present", "credential namespace isolation", "ephemeral restart", "log redaction"]}, indent=2) + "\n")
    print("PASS: local Claude/Codex chat, context, model selection, cancellation, isolation and redaction")


if __name__ == "__main__":
    main()
