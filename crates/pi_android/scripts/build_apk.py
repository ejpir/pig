#!/usr/bin/env python3
"""Build Pi for Android as an APK, without Gradle.

    python3 crates/pi_android/scripts/build_apk.py [--debug] [--out dist/pi.apk]

Uses gpui_android's builder (crates/gpui_android/scripts/build_apk.py), which
lists what it needs: a JDK and the Android SDK with the NDK.
"""

import sys
import subprocess
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
    permissions=(
        "android.permission.INTERNET",
        "android.permission.POST_NOTIFICATIONS",
        "android.permission.CAMERA",
    ),
    profileable=True,
    qr_scanner=True,
    deep_links=(("pi", "pair"),),
    launcher=(Path(__file__).resolve().parents[1] / "assets" / "launcher", 0xFF161D27),
    licenses=tuple(build_apk.WORKSPACE / "licenses" / name for name in (
        "IBM-PLEX-OFL.txt", "COMMIT-MONO-OFL.txt", "RQUICKJS-MIT.txt", "QUICKJS-NG-MIT.txt",
        "ROXMLTREE-MIT.txt", "SVGTYPES-MIT.txt", "KURBO-MIT.txt", "LYON-MIT.txt",
    )),
)

if __name__ == "__main__":
    if "--bun-backend-test" in sys.argv:
        sys.argv.remove("--bun-backend-test")
        if "--bun-library" not in sys.argv:
            raise SystemExit("--bun-backend-test requires --bun-library <architecture-matching libbunembeddingpoc.so>")
        index = sys.argv.index("--bun-library")
        library = Path(sys.argv[index + 1]).resolve()
        del sys.argv[index:index + 2]
        if library.name != "libbunembeddingpoc.so" or not library.is_file():
            raise SystemExit("Expected the source-linked libbunembeddingpoc.so")
        symbols = subprocess.check_output(["readelf", "--dyn-syms", "--wide", str(library)], text=True)
        if not all(name in symbols for name in ("pi_bun_init", "pi_bun_eval", "pi_bun_pump")):
            raise SystemExit("Missing the source embedding ABI")
        experiment = build_apk.WORKSPACE / "experiments" / "quickjs-backend"
        subprocess.run(["bun", "run", "build"], cwd=experiment, check=True)
        subprocess.run([sys.executable, str(build_apk.WORKSPACE / "experiments/bun-embedding/build_local.py")], check=True)
        license_path = library.parent / "LICENSE.bun.md"
        if not license_path.is_file():
            raise SystemExit("Place the pinned Bun LICENSE.bun.md beside the shared library")
        PI.package = "dev.pi.android.quickjstest"
        PI.label = "Pi local backend tests"
        PI.debuggable = True
        PI.cargo += ["--features", "bun-backend-test"]
        PI.native_libraries = (library,)
        PI.licenses += (license_path, experiment / "generated/BUN-DURABLE-NOTICES.txt", experiment / "generated/QUICKJS-BACKEND-NPM-NOTICES.txt")
        build_apk.main(PI, "pi-bun-local-test.apk")
    elif "--quickjs-backend-test" in sys.argv:
        sys.argv.remove("--quickjs-backend-test")
        if "--ui-test" in sys.argv:
            sys.argv.remove("--ui-test")
        experiment = build_apk.WORKSPACE / "experiments" / "quickjs-backend"
        if not (experiment / "node_modules").is_dir():
            raise SystemExit("First run: cd experiments/quickjs-backend && npm ci --ignore-scripts")
        subprocess.run(["bun", "run", "build"], cwd=experiment, check=True)
        PI.package = "dev.pi.android.quickjstest"
        PI.label = "Pi local backend tests"
        PI.debuggable = True
        PI.cargo += ["--features", "quickjs-backend-test"]
        PI.licenses += (experiment / "generated" / "QUICKJS-BACKEND-NPM-NOTICES.txt",)
        build_apk.main(PI, "pi-quickjs-test.apk")
    elif "--ui-test" in sys.argv:
        sys.argv.remove("--ui-test")
        PI.package = "dev.pi.android.uitest"
        PI.label = "Pi UI tests"
        PI.debuggable = True
        PI.cargo += ["--features", "ui-test"]
        build_apk.main(PI, "pi-ui-test.apk")
    else:
        build_apk.main(PI, "pi.apk")
