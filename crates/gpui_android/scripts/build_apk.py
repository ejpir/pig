#!/usr/bin/env python3
"""Build the GPUI Android demo as an APK, without Gradle.

    python3 crates/gpui_android/scripts/build_apk.py [--debug] [--target TARGET] [--out dist/gpui-touch.apk]

Builds the `touch` example for Android (ARM64 by default), compiles the Java
activity (`java/`) to DEX, writes a binary AndroidManifest.xml, then zips and signs the
APK with the Android debug key (~/.android/debug.keystore, created if missing),
or with ANDROID_KEYSTORE (and ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS and
ANDROID_KEY_PASSWORD) when set. The version is the workspace's.
Other apps call `build` with their own `App` (see crates/pi_android/scripts).

Needs Python 3.9+, a JDK (javac, keytool) and the Android SDK, found through
ANDROID_HOME or ANDROID_SDK_ROOT: a platform (android-30 or newer), build-tools
(d8 and apksigner) and the NDK. ANDROID_JAR, D8_JAR, APKSIGNER_JAR and
ANDROID_NDK_ROOT override the lookups. Setting the Cargo linker variable for
the selected Rust target (for example CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER)
avoids the NDK lookup.
"""

import argparse
import dataclasses
import os
import platform
import re
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
TARGET_ABIS = {
    "aarch64-linux-android": "arm64-v8a",
    "x86_64-linux-android": "x86_64",
}
MIN_SDK = 30
TARGET_SDK = 37
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
    qr_scanner: bool = False
    deep_links: tuple = ()  # (scheme, host), opened in the main activity
    # The launcher icon: a folder with ic_launcher_foreground.png (432 px, the
    # adaptive icon's 108 dp layer) and ic_notification.png (a white
    # silhouette for the status bar), and the 0xAARRGGBB colour behind it.
    launcher: tuple = ()  # (folder, background)


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


def ndk_linker_env(sdk, target):
    """Environment to link the selected Android Rust target with the NDK's clang."""
    target_env = target.upper().replace("-", "_")
    linker_var = f"CARGO_TARGET_{target_env}_LINKER"
    if os.environ.get(linker_var):
        return {}
    ndk = Path(os.environ["ANDROID_NDK_ROOT"]) if os.environ.get("ANDROID_NDK_ROOT") else None
    if ndk is None and sdk is not None:
        ndk = newest(list((sdk / "ndk").glob("*")))
    if ndk is None or not ndk.exists():
        fail("the Android NDK was not found; set ANDROID_NDK_ROOT or install it with the SDK manager")
    host = {"darwin": "darwin-x86_64", "linux": "linux-x86_64", "win32": "windows-x86_64"}[sys.platform]
    bin_dir = ndk / "toolchains" / "llvm" / "prebuilt" / host / "bin"
    suffix = ".cmd" if sys.platform == "win32" else ""
    clang = bin_dir / f"{target}{MIN_SDK}-clang{suffix}"
    if not clang.exists():
        fail(f"{clang} not found; this NDK has no toolchain for {host} ({platform.machine()})")
    rust_env = target.replace("-", "_")
    return {
        linker_var: str(clang),
        f"CC_{rust_env}": str(clang),
        f"CXX_{rust_env}": str(bin_dir / f"{target}{MIN_SDK}-clang++{suffix}"),
        f"AR_{rust_env}": str(bin_dir / "llvm-ar"),
    }


def run(command, **kwargs):
    print("+", " ".join(str(part) for part in command), flush=True)
    subprocess.run([str(part) for part in command], check=True, **kwargs)


# Binary AndroidManifest.xml (Android's compiled XML format), written directly
# so no resource compiler is needed: the app has no resources.

ANDROID_NS = "http://schemas.android.com/apk/res/android"
ATTRIBUTE_IDS = {  # android.R.attr
    "theme": 0x01010000, "label": 0x01010001, "icon": 0x01010002, "name": 0x01010003, "hasCode": 0x0101000C,
    "debuggable": 0x0101000F, "exported": 0x01010010, "authorities": 0x01010018,
    "grantUriPermissions": 0x0101001B, "launchMode": 0x0101001D, "configChanges": 0x0101001F,
    "value": 0x01010024, "scheme": 0x01010027, "host": 0x01010028,
    "minSdkVersion": 0x0101020C, "versionCode": 0x0101021B,
    "versionName": 0x0101021C, "targetSdkVersion": 0x01010270,
    "drawable": 0x01010199, "extractNativeLibs": 0x010104EA, "roundIcon": 0x0101052C,
    "enableOnBackInvokedCallback": 0x0101066C,
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


def workspace_version():
    """The workspace version, and a version code that grows with it (1.2.3 is 1002003)."""
    text = (WORKSPACE / "Cargo.toml").read_text()
    version = re.search(r'^\[workspace\.package\][^\[]*?^version = "([^"]+)"', text, re.M | re.S).group(1)
    major, minor, patch = (int(part) for part in re.match(r"(\d+)\.(\d+)\.(\d+)", version).groups())
    return version, major * 1_000_000 + minor * 1_000 + patch


def manifest(app, debuggable):
    version, code = workspace_version()
    return element("manifest", [
        attribute("package", STRING, app.package, android=False),
        attribute("versionCode", INT_DEC, code),
        attribute("versionName", STRING, version),
    ], [
        element("uses-sdk", [
            attribute("minSdkVersion", INT_DEC, MIN_SDK),
            attribute("targetSdkVersion", INT_DEC, TARGET_SDK),
        ]),
        *(element("uses-permission", [attribute("name", STRING, permission)])
          for permission in app.permissions),
        element("application", [
            attribute("label", STRING, app.label),
            *([attribute("icon", REFERENCE, LAUNCHER_ICON),
               attribute("roundIcon", REFERENCE, LAUNCHER_ICON)] if app.launcher else []),
            attribute("hasCode", BOOLEAN, True),
            attribute("debuggable", BOOLEAN, debuggable),
            attribute("extractNativeLibs", BOOLEAN, True),
            attribute("theme", REFERENCE, THEME_NO_ACTION_BAR),
            # Back reaches the app as the "back" key, which Android 16+ sends
            # only to apps that leave predictive back off.
            attribute("enableOnBackInvokedCallback", BOOLEAN, False),
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
                *(element("intent-filter", [], [
                    element("action", [attribute("name", STRING, "android.intent.action.VIEW")]),
                    element("category", [attribute("name", STRING, "android.intent.category.DEFAULT")]),
                    element("category", [attribute("name", STRING, "android.intent.category.BROWSABLE")]),
                    element("data", [
                        attribute("scheme", STRING, scheme),
                        attribute("host", STRING, host),
                    ]),
                ]) for scheme, host in app.deep_links),
            ]),
            *( [element("activity", [
                attribute("name", STRING, "dev.pi.gpui.PairScannerActivity"),
                attribute("exported", BOOLEAN, False),
                attribute("label", STRING, "Scan computer"),
                attribute("theme", REFERENCE, THEME_NO_ACTION_BAR),
                attribute("configChanges", INT_HEX, CONFIG_CHANGES),
            ])] if app.qr_scanner else []),
            # HTML pages an app shows full screen (activity::show_page).
            element("activity", [
                attribute("name", STRING, "dev.pi.gpui.PageActivity"),
                attribute("exported", BOOLEAN, False),
                attribute("label", STRING, "Page"),
                attribute("theme", REFERENCE, THEME_NO_ACTION_BAR),
                attribute("configChanges", INT_HEX, CONFIG_CHANGES),
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


# The resources an app with a launcher icon has, in a table built here as the
# manifest is: no aapt. Ids are 0x7f TT EEEE, by type then entry, as listed.
RESOURCE_TYPES = (
    ("color", ("ic_launcher_background",)),
    ("drawable", ("ic_launcher_foreground", "ic_notification")),
    ("mipmap", ("ic_launcher",)),
)
LAUNCHER_ICON = 0x7F030000
TYPE_INT_COLOR_ARGB8 = 0x1C


def adaptive_icon():
    """res/mipmap/ic_launcher.xml: the colour behind the foreground."""
    return element("adaptive-icon", [], [
        element("background", [attribute("drawable", REFERENCE, 0x7F010000)]),
        element("foreground", [attribute("drawable", REFERENCE, 0x7F020000)]),
    ])


def string_pool(strings):
    def length(n):
        return bytes([n]) if n < 0x80 else bytes([0x80 | (n >> 8), n & 0xFF])

    data, offsets = b"", []
    for text in strings:
        encoded = text.encode("utf-8")
        offsets.append(len(data))
        data += length(len(text)) + length(len(encoded)) + encoded + b"\0"
    data += b"\0" * (-len(data) % 4)
    strings_start = 28 + 4 * len(strings)
    header = struct.pack("<HHIIIIII", 0x0001, 28, strings_start + len(data), len(strings), 0, 0x100, strings_start, 0)
    return header + b"".join(struct.pack("<I", offset) for offset in offsets) + data


def resource_table(package, background):
    """resources.arsc for RESOURCE_TYPES, in the default configuration; and
    the files it names, as (path in the APK, source or bytes)."""
    files = {
        "ic_launcher_foreground": "res/drawable/ic_launcher_foreground.png",
        "ic_notification": "res/drawable/ic_notification.png",
        "ic_launcher": "res/mipmap/ic_launcher.xml",
    }
    paths = list(files.values())
    keys = [name for _, names in RESOURCE_TYPES for name in names]
    types = [name for name, _ in RESOURCE_TYPES]
    type_pool, key_pool = string_pool(types), string_pool(keys)
    chunks = b""
    for type_id, (kind, names) in enumerate(RESOURCE_TYPES, 1):
        chunks += struct.pack("<HHIBBHI", 0x0202, 16, 16 + 4 * len(names), type_id, 0, 0, len(names))
        chunks += b"\0" * 4 * len(names)
        entries = b""
        offsets = []
        for name in names:
            offsets.append(len(entries))
            if kind == "color":
                value = struct.pack("<HBBI", 8, 0, TYPE_INT_COLOR_ARGB8, background)
            else:
                value = struct.pack("<HBBI", 8, 0, STRING, paths.index(files[name]))
            entries += struct.pack("<HHI", 8, 0, keys.index(name)) + value
        config = struct.pack("<I", 64) + b"\0" * 60
        header_size = 20 + len(config)
        entries_start = header_size + 4 * len(names)
        chunks += struct.pack("<HHIBBHII", 0x0201, header_size, entries_start + len(entries),
                              type_id, 0, 0, len(names), entries_start)
        chunks += config + b"".join(struct.pack("<I", o) for o in offsets) + entries
    name = package.encode("utf-16-le")[:254].ljust(256, b"\0")
    header_size = 288
    package_chunk = struct.pack("<HHII", 0x0200, header_size, header_size + len(type_pool) + len(key_pool) + len(chunks), 0x7F)
    package_chunk += name + struct.pack("<IIIII", header_size, len(types), header_size + len(type_pool), len(keys), 0)
    package_chunk += type_pool + key_pool + chunks
    body = string_pool(paths) + package_chunk
    return struct.pack("<HHII", 0x0002, 12, 12 + len(body), 1) + body


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

    pool = string_pool(strings)
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


def debug_keystore(keytool="keytool"):
    keystore = Path.home() / ".android" / "debug.keystore"
    if not keystore.exists():
        keystore.parent.mkdir(parents=True, exist_ok=True)
        run([keytool, "-genkeypair", "-keystore", keystore, "-storepass", "android",
             "-keypass", "android", "-alias", "androiddebugkey", "-keyalg", "RSA",
             "-keysize", "2048", "-validity", "10000", "-dname", "CN=Android Debug,O=Android,C=US"])
    return keystore


def signing_key(keytool):
    """A release key from the environment, or the debug key."""
    keystore = os.environ.get("ANDROID_KEYSTORE")
    if not keystore:
        return ["--ks", debug_keystore(keytool), "--ks-pass", "pass:android"]
    key = ["--ks", keystore, "--ks-pass", "env:ANDROID_KEYSTORE_PASSWORD"]
    if os.environ.get("ANDROID_KEY_ALIAS"):
        key += ["--ks-key-alias", os.environ["ANDROID_KEY_ALIAS"]]
    if os.environ.get("ANDROID_KEY_PASSWORD"):
        key += ["--key-pass", "env:ANDROID_KEY_PASSWORD"]
    return key


def build(app, debug, out, target=TARGET):
    sdk = sdk_root()
    android_jar = tool("ANDROID_JAR", lambda: sdk and find_android_jar(sdk), f"android.jar (API {MIN_SDK}+)")
    d8 = tool("D8_JAR", lambda: sdk and find_build_tool(sdk, "d8.jar"), "d8.jar (build-tools)")
    apksigner = tool("APKSIGNER_JAR", lambda: sdk and find_build_tool(sdk, "apksigner.jar"), "apksigner.jar (build-tools)")
    java = os.environ.get("JAVA", "java")
    javac = os.environ.get("JAVAC", "javac")
    keytool = os.environ.get("KEYTOOL", "keytool")
    for program in (javac, java, keytool):
        if not Path(program).is_file() and not shutil.which(program):
            fail(f"{program} not found; install a JDK (Android Studio's is in its jbr folder)")

    env = {**os.environ, **ndk_linker_env(sdk, target)}
    cargo = env.get("CARGO", "cargo")
    profile = [] if debug else ["--release"]
    run([cargo, "build", *app.cargo, "--target", target, *profile], cwd=WORKSPACE, env=env)
    target_dir = Path(env.get("CARGO_TARGET_DIR", WORKSPACE / "target"))
    library = target_dir / target / ("debug" if debug else "release") / app.output / f"lib{app.library}.so"

    with tempfile.TemporaryDirectory(prefix="gpui-apk-") as scratch:
        scratch = Path(scratch)
        classes = scratch / "classes"
        sources = sorted((CRATE / "java").rglob("*.java"))
        run([javac, "--release", "11", "-classpath", android_jar, "-d", classes, *sources])
        run([java, "-cp", d8, "com.android.tools.r8.D8", "--min-api", MIN_SDK, "--lib", android_jar,
             "--output", scratch, *sorted(classes.rglob("*.class"))])
        unsigned = scratch / "unsigned.apk"
        with zipfile.ZipFile(unsigned, "w", zipfile.ZIP_DEFLATED) as apk:
            apk.writestr("AndroidManifest.xml", encode_xml(manifest(app, debuggable=debug or app.debuggable)))
            apk.write(scratch / "classes.dex", "classes.dex")
            apk.write(library, f"lib/{TARGET_ABIS[target]}/lib{app.library}.so")
            apk.write(CRATE / "assets/fonts/OFL.txt", "assets/licenses/NotoEmoji-OFL.txt")
            if app.launcher:
                folder, background = app.launcher
                # Stored, not compressed: Android maps resources.arsc directly.
                apk.writestr(zipfile.ZipInfo("resources.arsc"), resource_table(app.package, background))
                for name in ("ic_launcher_foreground", "ic_notification"):
                    apk.write(Path(folder) / f"{name}.png", f"res/drawable/{name}.png", zipfile.ZIP_STORED)
                apk.writestr("res/mipmap/ic_launcher.xml", encode_xml(adaptive_icon()))
        out.parent.mkdir(parents=True, exist_ok=True)
        run([java, "-jar", apksigner, "sign", *signing_key(keytool), "--min-sdk-version", MIN_SDK, "--out", out, unsigned])
    print(f"Built {out}\nInstall it with: adb install -r {out}")


def main(app=TOUCH, default_out="gpui-touch.apk"):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--debug", action="store_true", help="build Rust without optimizations")
    parser.add_argument("--target", choices=TARGET_ABIS, default=TARGET,
                        help="Rust Android target (ARM64 by default; x86_64 is useful for emulators)")
    parser.add_argument("--out", type=Path, default=WORKSPACE / "dist" / default_out)
    args = parser.parse_args()
    build(app, args.debug, args.out, args.target)


if __name__ == "__main__":
    main()
