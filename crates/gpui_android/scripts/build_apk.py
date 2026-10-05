#!/usr/bin/env python3
"""Build the GPUI Android demo as an APK, without Gradle.

    python3 crates/gpui_android/scripts/build_apk.py [--debug] [--out dist/gpui-touch.apk]

Builds the `touch` example for arm64 Android, compiles the Java activity
(`java/`) to DEX, writes a binary AndroidManifest.xml, then zips and signs the
APK with the Android debug key (~/.android/debug.keystore, created if missing).
Other apps call `build` with their own `App` (see crates/pi_android/scripts).

Needs Python 3.9+, a JDK (javac, keytool) and the Android SDK, found through
ANDROID_HOME or ANDROID_SDK_ROOT: a platform (android-30 or newer), build-tools
(d8 and apksigner) and the NDK. ANDROID_JAR, D8_JAR, APKSIGNER_JAR and
ANDROID_NDK_ROOT override the lookups. If CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER
is set, the NDK is not needed.
"""

import argparse
import dataclasses
import os
import platform
import shutil
import struct
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

CRATE = Path(__file__).resolve().parent.parent
WORKSPACE = CRATE.parent.parent
TARGET = "aarch64-linux-android"
MIN_SDK = 30
TARGET_SDK = 35
ACTIVITY = "dev.pi.gpui.GpuiActivity"


@dataclasses.dataclass
class App:
    package: str  # the Android application id
    label: str
    library: str  # lib<library>.so, which defines android_main
    cargo: list  # cargo build arguments that build the library
    output: str  # the library's folder under target/<triple>/<profile>
    permissions: tuple = ()
    debuggable: bool = False  # permits read-only run-as fixture telemetry, independent of Rust optimization


TOUCH = App(
    package="dev.pi.gpui_touch",
    label="GPUI touch",
    library="touch",
    cargo=["-p", "gpui_android", "--example", "touch"],
    output="examples",
)


def fail(message):
    sys.exit(f"build_apk: {message}")


def version_key(path):
    parts = []
    for piece in path.name.replace("android-", "").replace("-", ".").split("."):
        parts.append(int(piece) if piece.isdigit() else -1)
    return parts


def newest(paths):
    paths = sorted(paths, key=version_key)
    return paths[-1] if paths else None


def sdk_root():
    for name in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        if os.environ.get(name):
            return Path(os.environ[name])
    default = Path.home() / ("Library/Android/sdk" if sys.platform == "darwin" else "Android/Sdk")
    return default if default.exists() else None


def tool(env, finder, what):
    if os.environ.get(env):
        return Path(os.environ[env])
    found = finder()
    if not found or not found.exists():
        fail(f"{what} not found; set {env} or ANDROID_HOME")
    return found


def find_android_jar(sdk):
    platforms = [p for p in (sdk / "platforms").glob("android-*") if (p / "android.jar").exists()]
    platforms = [p for p in platforms if version_key(p)[0] >= MIN_SDK]
    best = newest(platforms)
    return best / "android.jar" if best else None


def find_build_tool(sdk, jar):
    best = newest([p for p in (sdk / "build-tools").glob("*") if (p / "lib" / jar).exists()])
    return best / "lib" / jar if best else None


def ndk_linker_env(sdk):
    """Environment to link for Android with the NDK's clang."""
    if os.environ.get("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"):
        return {}
    ndk = Path(os.environ["ANDROID_NDK_ROOT"]) if os.environ.get("ANDROID_NDK_ROOT") else None
    if ndk is None and sdk is not None:
        ndk = newest(list((sdk / "ndk").glob("*")))
    if ndk is None or not ndk.exists():
        fail("the Android NDK was not found; set ANDROID_NDK_ROOT or install it with the SDK manager")
    host = {"darwin": "darwin-x86_64", "linux": "linux-x86_64", "win32": "windows-x86_64"}[sys.platform]
    bin_dir = ndk / "toolchains" / "llvm" / "prebuilt" / host / "bin"
    suffix = ".cmd" if sys.platform == "win32" else ""
    clang = bin_dir / f"{TARGET}{MIN_SDK}-clang{suffix}"
    if not clang.exists():
        fail(f"{clang} not found; this NDK has no toolchain for {host} ({platform.machine()})")
    return {
        "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER": str(clang),
        "CC_aarch64_linux_android": str(clang),
        "CXX_aarch64_linux_android": str(bin_dir / f"{TARGET}{MIN_SDK}-clang++{suffix}"),
        "AR_aarch64_linux_android": str(bin_dir / "llvm-ar"),
    }


def run(command, **kwargs):
    print("+", " ".join(str(part) for part in command), flush=True)
    subprocess.run([str(part) for part in command], check=True, **kwargs)


# Binary AndroidManifest.xml (Android's compiled XML format), written directly
# so no resource compiler is needed: the app has no resources.

ANDROID_NS = "http://schemas.android.com/apk/res/android"
ATTRIBUTE_IDS = {  # android.R.attr
    "theme": 0x01010000, "label": 0x01010001, "name": 0x01010003, "hasCode": 0x0101000C,
    "debuggable": 0x0101000F, "exported": 0x01010010, "authorities": 0x01010018,
    "grantUriPermissions": 0x0101001B, "launchMode": 0x0101001D, "configChanges": 0x0101001F,
    "value": 0x01010024, "minSdkVersion": 0x0101020C, "versionCode": 0x0101021B,
    "versionName": 0x0101021C, "targetSdkVersion": 0x01010270,
    "extractNativeLibs": 0x010104EA,
}
THEME_NO_ACTION_BAR = 0x01030129  # @android:style/Theme.DeviceDefault.NoActionBar
# ActivityInfo.CONFIG_*: rotation, resizing, density, dark mode and keyboards
# reach the app as resizes instead of restarting the activity.
CONFIG_CHANGES = 0x10 | 0x20 | 0x40 | 0x80 | 0x100 | 0x200 | 0x400 | 0x800 | 0x1000
# One activity per task: notification and link taps reach it through onNewIntent.
LAUNCH_SINGLE_TASK = 2
STRING, INT_DEC, INT_HEX, BOOLEAN, REFERENCE = 0x03, 0x10, 0x11, 0x12, 0x01


def element(tag, attributes=(), children=()):
    return tag, list(attributes), list(children)


def attribute(name, kind, value, android=True):
    return name, kind, value, android


def manifest(app, debuggable):
    return element("manifest", [
        attribute("package", STRING, app.package, android=False),
        attribute("versionCode", INT_DEC, 1),
        attribute("versionName", STRING, "0.1"),
    ], [
        element("uses-sdk", [
            attribute("minSdkVersion", INT_DEC, MIN_SDK),
            attribute("targetSdkVersion", INT_DEC, TARGET_SDK),
        ]),
        *(element("uses-permission", [attribute("name", STRING, permission)])
          for permission in app.permissions),
        element("application", [
            attribute("label", STRING, app.label),
            attribute("hasCode", BOOLEAN, True),
            attribute("debuggable", BOOLEAN, debuggable),
            attribute("extractNativeLibs", BOOLEAN, True),
            attribute("theme", REFERENCE, THEME_NO_ACTION_BAR),
        ], [
            element("activity", [
                attribute("name", STRING, ACTIVITY),
                attribute("exported", BOOLEAN, True),
                attribute("label", STRING, app.label),
                attribute("launchMode", INT_DEC, LAUNCH_SINGLE_TASK),
                attribute("configChanges", INT_HEX, CONFIG_CHANGES),
            ], [
                element("meta-data", [
                    attribute("name", STRING, "android.app.lib_name"),
                    attribute("value", STRING, app.library),
                ]),
                element("intent-filter", [], [
                    element("action", [attribute("name", STRING, "android.intent.action.MAIN")]),
                    element("category", [attribute("name", STRING, "android.intent.category.LAUNCHER")]),
                ]),
            ]),
            # Serves copied images to the apps that paste them.
            element("provider", [
                attribute("name", STRING, "dev.pi.gpui.ClipboardProvider"),
                attribute("authorities", STRING, f"{app.package}.gpui.clipboard"),
                attribute("exported", BOOLEAN, False),
                attribute("grantUriPermissions", BOOLEAN, True),
            ]),
        ]),
    ])


def walk(node):
    yield node
    for child in node[2]:
        yield from walk(child)


def encode_xml(root):
    # Attribute names that have resource ids come first, in resource-map order.
    android_names = sorted(
        {name for node in walk(root) for name, _, _, android in node[1] if android},
        key=ATTRIBUTE_IDS.__getitem__,
    )
    strings = list(android_names)

    def index(text):
        if text not in strings:
            strings.append(text)
        return strings.index(text)

    prefix, namespace = index("android"), index(ANDROID_NS)
    for node in walk(root):
        index(node[0])
        for name, kind, value, _ in node[1]:
            index(name)
            if kind == STRING:
                index(value)

    def length(n):
        return bytes([n]) if n < 0x80 else bytes([0x80 | (n >> 8), n & 0xFF])

    data, offsets = b"", []
    for text in strings:
        encoded = text.encode("utf-8")
        offsets.append(len(data))
        data += length(len(text)) + length(len(encoded)) + encoded + b"\0"
    data += b"\0" * (-len(data) % 4)
    strings_start = 28 + 4 * len(strings)
    pool = struct.pack("<HHIIIIII", 0x0001, 28, strings_start + len(data), len(strings), 0, 0x100, strings_start, 0)
    pool += b"".join(struct.pack("<I", offset) for offset in offsets) + data
    ids = [ATTRIBUTE_IDS[name] for name in android_names]
    resource_map = struct.pack("<HHI", 0x0180, 8, 8 + 4 * len(ids)) + b"".join(struct.pack("<I", i) for i in ids)

    none = 0xFFFFFFFF
    body = [struct.pack("<HHIIIII", 0x0100, 16, 24, 1, none, prefix, namespace)]

    def emit(node):
        tag, attributes, children = node
        # Android looks attributes up by resource id, so they are sorted by it.
        attributes = sorted(attributes, key=lambda a: ATTRIBUTE_IDS.get(a[0], none) if a[3] else none)
        encoded = b""
        for name, kind, value, android in attributes:
            raw = index(value) if kind == STRING else none
            datum = index(value) if kind == STRING else (none if value else 0) if kind == BOOLEAN else value
            encoded += struct.pack("<IIIHBBI", namespace if android else none, index(name), raw, 8, 0, kind, datum)
        body.append(struct.pack("<HHIII", 0x0102, 16, 36 + len(encoded), 1, none))
        body.append(struct.pack("<IIHHHHHH", none, index(tag), 20, 20, len(attributes), 0, 0, 0) + encoded)
        for child in children:
            emit(child)
        body.append(struct.pack("<HHIIIII", 0x0103, 16, 24, 1, none, none, index(tag)))

    emit(root)
    body.append(struct.pack("<HHIIIII", 0x0101, 16, 24, 1, none, prefix, namespace))
    chunks = pool + resource_map + b"".join(body)
    return struct.pack("<HHI", 0x0003, 8, 8 + len(chunks)) + chunks


def debug_keystore():
    keystore = Path.home() / ".android" / "debug.keystore"
    if not keystore.exists():
        keystore.parent.mkdir(parents=True, exist_ok=True)
        run(["keytool", "-genkeypair", "-keystore", keystore, "-storepass", "android",
             "-keypass", "android", "-alias", "androiddebugkey", "-keyalg", "RSA",
             "-keysize", "2048", "-validity", "10000", "-dname", "CN=Android Debug,O=Android,C=US"])
    return keystore


def build(app, debug, out):
    sdk = sdk_root()
    android_jar = tool("ANDROID_JAR", lambda: sdk and find_android_jar(sdk), f"android.jar (API {MIN_SDK}+)")
    d8 = tool("D8_JAR", lambda: sdk and find_build_tool(sdk, "d8.jar"), "d8.jar (build-tools)")
    apksigner = tool("APKSIGNER_JAR", lambda: sdk and find_build_tool(sdk, "apksigner.jar"), "apksigner.jar (build-tools)")
    for program in ("javac", "java", "keytool"):
        if not shutil.which(program):
            fail(f"{program} not found; install a JDK (Android Studio's is in its jbr folder)")

    env = {**os.environ, **ndk_linker_env(sdk)}
    profile = [] if debug else ["--release"]
    run(["cargo", "build", *app.cargo, "--target", TARGET, *profile], cwd=WORKSPACE, env=env)
    target_dir = Path(env.get("CARGO_TARGET_DIR", WORKSPACE / "target"))
    library = target_dir / TARGET / ("debug" if debug else "release") / app.output / f"lib{app.library}.so"

    with tempfile.TemporaryDirectory(prefix="gpui-apk-") as scratch:
        scratch = Path(scratch)
        classes = scratch / "classes"
        sources = sorted((CRATE / "java").rglob("*.java"))
        run(["javac", "--release", "11", "-classpath", android_jar, "-d", classes, *sources])
        run(["java", "-cp", d8, "com.android.tools.r8.D8", "--min-api", MIN_SDK, "--lib", android_jar,
             "--output", scratch, *sorted(classes.rglob("*.class"))])
        unsigned = scratch / "unsigned.apk"
        with zipfile.ZipFile(unsigned, "w", zipfile.ZIP_DEFLATED) as apk:
            apk.writestr("AndroidManifest.xml", encode_xml(manifest(app, debuggable=debug or app.debuggable)))
            apk.write(scratch / "classes.dex", "classes.dex")
            apk.write(library, f"lib/arm64-v8a/lib{app.library}.so")
            apk.write(CRATE / "assets/fonts/OFL.txt", "assets/licenses/NotoEmoji-OFL.txt")
        out.parent.mkdir(parents=True, exist_ok=True)
        run(["java", "-jar", apksigner, "sign", "--ks", debug_keystore(), "--ks-pass", "pass:android",
             "--min-sdk-version", MIN_SDK, "--out", out, unsigned])
    print(f"Built {out}\nInstall it with: adb install -r {out}")


def main(app=TOUCH, default_out="gpui-touch.apk"):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--debug", action="store_true", help="build Rust without optimizations")
    parser.add_argument("--out", type=Path, default=WORKSPACE / "dist" / default_out)
    args = parser.parse_args()
    build(app, args.debug, args.out)


if __name__ == "__main__":
    main()
