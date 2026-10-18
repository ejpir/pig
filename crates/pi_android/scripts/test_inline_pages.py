#!/usr/bin/env python3
"""Native inline WebView regression checks on the isolated --ui-test APK.

Only inert, paused sample fixtures are used. No providers or SSH connections.
GPUI controls use fixture telemetry; HTML controls use Android accessibility
or read-only DOM geometry inside the visible native clip. Readback is initiated
by the fixture harness, not a page bridge, and never changes document inputs.
Outputs include native screenshots, document readbacks and a report.
"""
import argparse
import re
import json
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import time
from pathlib import Path

from test_interactions import ACTIVITY, PACKAGE, Phone

SCROLL = 'scroll:NamedInteger("thread", 1)'


def focus_page(phone):
    """Focus the actual browser surface, not a key, before accessibility lookup."""
    state = phone.settled_state()
    (x, y, width, height), scale = phone.bounds("inline-page-clip", state=state)
    assert height > 40 and width > 80, "No visible browser surface"
    phone.run("shell", "input", "tap", round((x + width / 2) * scale), round((y + 20) * scale))


def document(phone, timeout=3):
    assert phone.state()["sample"] and phone.fixture_name.startswith("inline-")
    request = str(time.monotonic_ns())
    phone.run("shell", "am", "broadcast", "-a", "dev.pi.gpui.INLINE_INSPECT", "-p", PACKAGE, "--es", "request", request)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            state = json.loads(phone.run("exec-out", "run-as", PACKAGE, "cat", "files/ui-test-inline.json"))
            if state.get("request") == request and state.get("ready") == "complete":
                with phone.output.joinpath("documents.jsonl").open("a") as out:
                    out.write(json.dumps(state) + "\n")
                return state
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            pass
        time.sleep(.1)
    return None


def widget_tap(phone, label, timeout=12, class_name=None):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        state = phone.state()
        clip, scale = phone.bounds("inline-page-clip", state=state)
        x, y, width, height = (value * scale for value in clip)
        matches = []
        for node in phone.native_nodes():
            if class_name:
                if node.get("class") != class_name:
                    continue
            elif label not in (node.get("text"), node.get("content-desc")):
                continue
            if node.get("enabled") != "true" or node.get("clickable") != "true":
                continue
            bounds = list(map(int, re.findall(r"-?\d+", node.get("bounds", ""))))
            if len(bounds) != 4:
                continue
            left, top, right, bottom = bounds
            if right > left and bottom > top and x <= (left+right)/2 <= x+width and y <= (top+bottom)/2 <= y+height:
                matches.append(bounds)
        if not matches:
            # Chromium sometimes exposes only a fresh replacement's title until
            # its first DOM change. Use measured native DOM geometry instead of
            # guessing a key's position or mutating inputs to warm accessibility.
            doc = document(phone)
            if doc:
                for control in doc["controls"]:
                    if (class_name and control["type"] != "INPUT") or (not class_name and control["label"] != label):
                        continue
                    dx, dy, dw, dh = control["rect"]
                    left, top = (doc["origin"][0]+dx*doc["scale"], doc["origin"][1]+dy*doc["scale"])
                    right, bottom = left+dw*doc["scale"], top+dh*doc["scale"]
                    if dw > 0 and dh > 0 and x <= (left+right)/2 <= x+width and y <= (top+bottom)/2 <= y+height:
                        matches.append([round(left), round(top), round(right), round(bottom)])
        if len(matches) == 1:
            left, top, right, bottom = matches[0]
            phone.run("shell", "input", "tap", (left+right)//2, (top+bottom)//2)
            return
        if len(matches) > 1:
            raise AssertionError(f"Ambiguous visible HTML control: {label}")
        time.sleep(.2)
    phone.capture("widget-control-failure")
    raise AssertionError(f"HTML control not found inside the live page clip: {label}")


def value(phone, expected, description):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        # A replacement WebView may finish loading after the close-sheet
        # animation and its first tap. Re-focus the display, never clear/press a
        # key: a reset assertion must not manufacture its expected zero value.
        focus_page(phone)
        doc = document(phone)
        if doc and doc.get("result") == expected:
            phone.results.append({"check": description, "passed": True})
            print(f"PASS {description}", flush=True)
            return
        time.sleep(.2)
    phone.capture("native-value-failure")
    raise AssertionError(f"Running calculator result did not become {expected}")


def mutation(phone, action):
    before = phone.state()["time"]
    phone.run("shell", "am", "start", "-n", ACTIVITY, "-a", "android.intent.action.VIEW", "-d", f"pi://test/{action}")
    phone.wait(lambda state: state["time"] > before, f"sample mutation {action} observed")


def calculator(phone):
    phone.fixture("inline-calculator")
    phone.wait(lambda state: state["route"] == "Thread(SessionId(1))" and "inline-page-clip" in state["bounds"], "calculator is painted inside the ordinary chat")
    value(phone, "0", "local calculator loads without opening PageActivity")
    for key in ["Clear all", "1", "2", "Multiply", "8", "Equals"]:
        widget_tap(phone, key)
    value(phone, "96", "inline HTML buttons execute 12 × 8 = 96")
    phone.capture("01-inline-calculator")
    mutation(phone, "inline-evening")
    phone.wait(lambda state: state["scheduler"]["appearance"] == "dark", "Evening changes the native presentation")
    value(phone, "96", "theme changes retain the document and its inputs")
    phone.capture("01b-inline-evening")
    mutation(phone, "inline-light")
    phone.wait(lambda state: state["scheduler"]["appearance"] == "light", "Moonstone restores the native presentation")
    value(phone, "96", "returning to Moonstone does not reload HTML")
    phone.tap("page-0-0")
    phone.wait(lambda state: state["inline_expanded"], "Expand stays in the native session")
    value(phone, "96", "expansion retains the same running calculation")
    phone.wait_activity("dev.pi.gpui.GpuiActivity", "expansion does not launch a separate activity")
    phone.capture("02-expanded-calculator")
    phone.tap("page-source-0-0")
    phone.wait(lambda state: state["sheet"] == "InlineSource", "source opens as a GPUI bottom sheet")
    phone.capture("03-source-sheet")
    assert not any(node.get("class") == "android.webkit.WebView" for node in phone.native_nodes()), "Native host must not occlude the GPUI sheet"
    phone.key("KEYCODE_BACK")
    phone.wait(lambda state: state["sheet"] is None, "Back closes source before collapsing the page")
    phone.settled_state()
    value(phone, "96", "reading source leaves the document intact")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda state: not state["inline_expanded"], "Android Back returns to chat without leaving the session")
    value(phone, "96", "returning to the chat retains the calculation")
    phone.tap("inline-permissions")
    phone.wait(lambda state: state["sheet"] == "InlinePermissions", "local capability boundary is inspectable")
    phone.capture("04-permissions")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda state: state["sheet"] is None, "permissions sheet closes")
    phone.settled_state()
    value(phone, "96", "permissions inspection does not reload the calculator")
    mutation(phone, "inline-update")
    phone.wait(lambda state: any('"inline-update"' in key for key in state["bounds"]), "new HTML offers an explicit update")
    value(phone, "96", "new HTML does not silently reset the running document")
    phone.tap("inline-update")
    phone.wait(lambda state: state["sheet"] == "InlineUpdate", "revision replacement requires confirmation")
    phone.tap("inline-keep")
    phone.wait(lambda state: state["sheet"] is None, "Keep current cancels revision replacement")
    phone.settled_state()
    value(phone, "96", "cancelled update retains the running inputs")
    phone.tap("inline-update")
    phone.tap("inline-load")
    phone.wait(lambda state: state["sheet"] is None, "explicit update closes its confirmation")
    phone.settled_state()
    value(phone, "0", "explicit Load update starts a fresh document")
    for key in ["1", "Add", "2", "Equals"]:
        widget_tap(phone, key)
    value(phone, "3", "the explicitly loaded revision remains genuinely interactive")
    mutation(phone, "inline-unavailable")
    phone.wait(lambda state: "inline-page-clip" not in state["bounds"], "unknown revision removes the current browser surface")
    phone.capture("05-unavailable-revision")
    phone.tap("inline-saved")
    phone.wait(lambda state: "inline-page-clip" in state["bounds"], "saved version requires an explicit action")
    value(phone, "3", "explicit saved snapshot keeps its running inputs and remains usable")
    phone.capture("06-saved-snapshot")


def scrolling_and_lifecycle(phone):
    phone.fixture("inline-scroll")
    phone.scroll_into_view("inline-page-0-0", SCROLL)
    value(phone, "0", "an actually visible page mounts after scrolling into view")
    for key in ["Clear all", "4", "2", "Add", "8", "Equals"]:
        widget_tap(phone, key)
    value(phone, "50", "scroll fixture calculator works")
    state = phone.settled_state()
    bounds, _ = phone.bounds("inline-page-0-0", state=state)
    x, y, width, height = bounds
    before = y
    phone.swipe((x+width/2, y+height*.65), (x+width/2, y+height*.25), 600)
    phone.wait(lambda state: abs(phone.bounds("inline-page-0-0", state=state)[0][1]-before) > 20, "a vertical drag starting in the HTML scrolls the chat")
    phone.capture("07-scrolled-and-clipped")
    phone.tap("page-source-0-0")
    phone.wait(lambda state: state["sheet"] == "InlineSource", "a partially clipped browser does not cover native footer actions")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda state: state["sheet"] is None, "clipped-page source closes normally")
    phone.scroll_into_view("inline-page-0-0", SCROLL)
    value(phone, "50", "scrolling does not reload or accidentally press a calculator key")
    phone.tap("back")
    phone.wait(lambda state: state["route"] == "Sessions" and "inline-page-clip" not in state["bounds"], "leaving the thread removes the native browser window")
    phone.capture("07b-session-list")
    if "session-row-1" not in phone.settled_state()["bounds"]:
        phone.scroll_into_view("seen-today", 'scroll:Name("sessions")')
        phone.tap("seen-today")
        phone.wait(
            lambda state: "session-row-1" in state["bounds"],
            "the cached session group opens before selecting its retained row",
        )
    phone.scroll_into_view("session-row-1", 'scroll:Name("sessions")')
    phone.tap("session-row-1")
    phone.wait(lambda state: state["route"] == "Thread(SessionId(1))", "cached session reopens")
    phone.scroll_into_view("inline-page-0-0", SCROLL)
    value(phone, "50", "route changes retain cached inputs without an orphaned popup")
    phone.key("KEYCODE_HOME")
    time.sleep(.5)
    phone.run("shell", "am", "start", "-n", ACTIVITY)
    phone.wait_activity("dev.pi.gpui.GpuiActivity", "returning from background restores the native app")
    phone.wait(lambda state: state["fixture"] == "inline-scroll", "backgrounding did not reset the fixture")
    phone.settled_state()
    value(phone, "50", "background/resume retains the cached document")
    phone.capture("08-resumed")


def keyboard(phone):
    phone.fixture("inline-input")
    phone.tap("page-0-0")
    phone.wait(lambda state: state["inline_expanded"], "HTML input fixture expands")
    focus_page(phone)
    before_height = phone.bounds("inline-page-clip", state=phone.settled_state())[0][3]
    # Inputs are native HTML controls; do not use GPUI editor telemetry for them.
    widget_tap(phone, "Page note", class_name="android.widget.EditText")
    phone.wait_keyboard(True)
    phone.wait(lambda state: phone.bounds("inline-page-clip", state=state)[0][3] < before_height-100, "native content resizes above the HTML keyboard without inset feedback")
    phone.text("WidgetNote42")
    phone.wait_native_text("WidgetNote42", "WebView owns its input connection")
    assert document(phone)["result"] == "0", "Typing digits in an HTML field must not activate calculator shortcuts"
    phone.capture("09-html-keyboard")
    assert phone.state()["draft_chars"] == 0, "HTML typing must not write to the composer mirror"
    phone.key("KEYCODE_BACK")
    phone.wait_keyboard(False)
    assert phone.state()["inline_expanded"], "Back hides the HTML keyboard before collapsing the page"
    widget_tap(phone, "Page note", class_name="android.widget.EditText")
    phone.wait_keyboard(True)
    phone.tap("page-source-0-0")
    phone.wait(lambda state: state["sheet"] == "InlineSource", "source remains reachable while an HTML field has focus")
    phone.wait_keyboard(False)
    phone.key("KEYCODE_BACK")
    phone.wait(lambda state: state["sheet"] is None, "source closes back to the retained input page")
    phone.settled_state()
    phone.tap("inline-back")
    phone.wait(lambda state: not state["inline_expanded"], "HTML input returns to the conversation")
    phone.tap("draft")
    phone.wait_keyboard(True)
    phone.text("ComposerNote")
    phone.wait(lambda state: state["draft_chars"] == len("ComposerNote"), "first composer tap reclaims focus and its own input connection")
    phone.capture("10-composer-keyboard")
    phone.key("KEYCODE_BACK")
    phone.wait_keyboard(False)
    phone.tap("page-0-0")
    phone.wait(lambda state: state["inline_expanded"], "calculator expands again after composer typing")
    focus_page(phone)
    phone.wait_native_text("WidgetNote42", "HTML input survives composer focus and re-expansion")
    phone.capture("11-input-retained")


def boundary(phone):
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"local boundary test")

        def log_message(self, *_):
            pass

    mappings = phone.run("reverse", "--list").decode()
    if "tcp:18789 " in mappings:
        raise RuntimeError("Test port already has an adb reverse; refusing to overwrite it")
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        phone.run("reverse", "tcp:18789", f"tcp:{server.server_address[1]}")
        # Keep stdin open until adb's reverse proxy has flushed the request.
        phone.run("shell", "(printf 'GET /control HTTP/1.0\\r\\nHost: localhost\\r\\n\\r\\n'; sleep 1) | toybox nc -w 2 127.0.0.1 18789")
        assert requests == ["/control"], "Network negative test needs a working loopback endpoint"
        requests.clear()
        phone.fixture("inline-boundary")
        focus_page(phone)
        phone.wait_native_text("Ready for boundary checks", "trusted offline envelope loads before untrusted scripts")
        widget_tap(phone, "Run boundary checks")
        phone.wait_native_text("Boundary checks passed", "HTTP, WebSocket, files, cookies, native bridges and RTC realm checks are blocked")
        time.sleep(.3)
        assert not requests, f"Inline page reached the loopback endpoint: {requests}"
        phone.results.append({"check": "no HTTP/WebSocket request reached the verified local endpoint", "passed": True})
        widget_tap(phone, "Try a modal")
        phone.wait_native_text("Modal suppressed", "page scripts cannot create native app-modal dialogs")
        widget_tap(phone, "Try app navigation")
        phone.wait_native_text("App navigation attempted", "untrusted navigation remains inside the same document")
        assert phone.state()["fixture"] == "inline-boundary"
        phone.wait_activity("dev.pi.gpui.GpuiActivity", "inline scripts cannot open an external/native activity")
        phone.capture("12-native-boundary")
    finally:
        phone.run("reverse", "--remove", "tcp:18789")
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def recovery(phone):
    phone.fixture("inline-calculator")
    value(phone, "0", "recoverability fixture has a running browser renderer")
    pid = phone.run("shell", "pidof", PACKAGE).strip()
    mutation(phone, "inline-crash")
    phone.wait(lambda state: not state["inline_available"] and "inline-failed" in state["bounds"] and "inline-page-clip" not in state["bounds"], "renderer termination becomes a recoverable native placeholder, not a blank surface or retry loop")
    assert phone.run("shell", "pidof", PACKAGE).strip() == pid, "Renderer exit must not restart/kill the native app"
    assert phone.state()["turns"] == 1
    phone.wait_activity("dev.pi.gpui.GpuiActivity", "native chat survives the browser renderer exit")
    phone.capture("13-renderer-recovery")
    phone.tap("page-source-0-0")
    phone.wait(lambda state: state["sheet"] == "InlineSource", "source remains inspectable after renderer failure")
    phone.key("KEYCODE_BACK")
    phone.wait(lambda state: state["sheet"] is None, "failed-preview source can close normally")
    phone.settled_state()
    assert not phone.state()["inline_available"], "Closing source must not automatically retry a crashed document"
    phone.tap("inline-retry")
    phone.wait(lambda state: state["inline_available"] and "inline-page-clip" in state["bounds"], "explicit Retry permits a fresh browser document")
    value(phone, "0", "recovered preview starts with fresh page inputs")
    for key in ["2", "Add", "3", "Equals"]:
        widget_tap(phone, key)
    value(phone, "5", "browser interaction works after an actual renderer termination")
    phone.capture("14-recovered-calculator")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--case", choices=["calculator", "scrolling", "keyboard", "boundary", "recovery"], action="append")
    args = parser.parse_args()
    phone = Phone(args.serial, args.output)
    try:
        phone.run("shell", "pm", "path", PACKAGE)
        phone.run("shell", "am", "force-stop", PACKAGE)
        phone.wake()
        for case in args.case or ["calculator", "scrolling", "keyboard", "boundary", "recovery"]:
            {"calculator": calculator, "scrolling": scrolling_and_lifecycle, "keyboard": keyboard, "boundary": boundary, "recovery": recovery}[case](phone)
    except Exception:
        phone.capture("failure")
        phone.results.append({"check": "inline page regression", "passed": False})
        raise
    finally:
        phone.report()
    print(f"{len(phone.results)} checks passed. Evidence: {args.output}")


if __name__ == "__main__":
    main()
