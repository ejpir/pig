#!/usr/bin/env python3
"""Build Pi for Android as an APK, without Gradle.

    python3 crates/pi_android/scripts/build_apk.py [--debug] [--out dist/pi.apk]

Uses gpui_android's builder (crates/gpui_android/scripts/build_apk.py), which
lists what it needs: a JDK and the Android SDK with the NDK.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "gpui_android" / "scripts"))

import build_apk  # noqa: E402

PI = build_apk.App(
    package="dev.pi.android",
    label="Pi",
    library="pi_android",
    cargo=["-p", "pi_android", "--lib"],
    output="",
    # SSH to the computer; questions and finished sessions arrive as notifications.
    permissions=("android.permission.INTERNET", "android.permission.POST_NOTIFICATIONS"),
)

if __name__ == "__main__":
    if "--ui-test" in sys.argv:
        sys.argv.remove("--ui-test")
        PI.package = "dev.pi.android.uitest"
        PI.label = "Pi UI tests"
        PI.debuggable = True
        PI.cargo += ["--features", "ui-test"]
        build_apk.main(PI, "pi-ui-test.apk")
    else:
        build_apk.main(PI, "pi.apk")
