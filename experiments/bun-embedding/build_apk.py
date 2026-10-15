#!/usr/bin/env python3
"""Package a genuinely source-linked Bun/JNI library in a separate offline APK.

This script never embeds/executes the Bun executable. --library must contain
both the full Bun runtime and our JNI bridge, linked from PIC objects.
"""

import argparse
import hashlib
import importlib.util
import subprocess
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKSPACE = ROOT.parent.parent
PACKAGE = "dev.pi.bunembedding"
ACTIVITY = PACKAGE + ".ProbeActivity"


def run(*args):
    print("+", *map(str, args), flush=True)
    subprocess.run(list(map(str, args)), check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", type=Path, required=True)
    parser.add_argument("--abi", choices=["x86_64", "arm64-v8a"], default="x86_64")
    parser.add_argument(
        "--out", type=Path, default=WORKSPACE / "dist/pi-bun-embedding-poc.apk"
    )
    parser.add_argument("--sdk", type=Path, default=Path.home() / "Android/Sdk")
    parser.add_argument(
        "--sdk-bundle",
        type=Path,
        help="Trusted offline Durable SDK probe to include as an APK asset",
    )
    parser.add_argument(
        "--sdk-notices", type=Path, help="Dependency notices accompanying --sdk-bundle"
    )
    args = parser.parse_args()
    if args.sdk_bundle and (
        not args.sdk_bundle.is_file()
        or not args.sdk_notices
        or not args.sdk_notices.is_file()
    ):
        raise SystemExit(
            "SDK bundling requires an existing probe bundle and dependency notices"
        )
    library = args.library.resolve()
    header = subprocess.check_output(["readelf", "-h", str(library)], text=True)
    dynamic = subprocess.check_output(["readelf", "-d", str(library)], text=True)
    symbols = subprocess.check_output(
        ["readelf", "--dyn-syms", "--wide", str(library)], text=True
    )
    if (
        "DYN (Shared object file)" not in header
        or "FLAGS_1" in dynamic
        and "PIE" in dynamic
    ):
        raise SystemExit(
            "A genuine linked shared library is required, not a PIE executable"
        )
    if "Java_dev_pi_bunembedding_ProbeActivity_nativeEval" not in symbols:
        raise SystemExit("Missing linked JNI evaluation entry")
    builder = WORKSPACE / "crates/gpui_android/scripts/build_apk.py"
    spec = importlib.util.spec_from_file_location("gpui_apk_builder", builder)
    module = importlib.util.module_from_spec(spec)
    # dataclasses resolves class annotations through sys.modules.
    import sys

    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    element, attr = module.element, module.attribute
    manifest = element(
        "manifest",
        [
            attr("package", module.STRING, PACKAGE, android=False),
            attr("versionCode", module.INT_DEC, 1),
            attr("versionName", module.STRING, "0.1"),
        ],
        [
            element(
                "uses-sdk",
                [
                    attr("minSdkVersion", module.INT_DEC, 30),
                    attr("targetSdkVersion", module.INT_DEC, 37),
                ],
            ),
            element(
                "application",
                [
                    attr("label", module.STRING, "Bun linked-runtime PoC"),
                    attr("debuggable", module.BOOLEAN, True),
                    attr("hasCode", module.BOOLEAN, True),
                    attr("extractNativeLibs", module.BOOLEAN, True),
                    attr("theme", module.REFERENCE, module.THEME_NO_ACTION_BAR),
                ],
                [
                    element(
                        "activity",
                        [
                            attr("name", module.STRING, ACTIVITY),
                            attr("exported", module.BOOLEAN, True),
                        ],
                        [
                            element(
                                "intent-filter",
                                [],
                                [
                                    element(
                                        "action",
                                        [
                                            attr(
                                                "name",
                                                module.STRING,
                                                "android.intent.action.MAIN",
                                            )
                                        ],
                                    ),
                                    element(
                                        "category",
                                        [
                                            attr(
                                                "name",
                                                module.STRING,
                                                "android.intent.category.LAUNCHER",
                                            )
                                        ],
                                    ),
                                ],
                            )
                        ],
                    )
                ],
            ),
        ],
    )
    platform = module.find_android_jar(args.sdk)
    if not platform:
        raise SystemExit("Android SDK platform not found")
    d8 = module.find_build_tool(args.sdk, "d8.jar")
    signer = module.find_build_tool(args.sdk, "apksigner.jar")
    args.out = args.out.resolve()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pi-bun-apk-") as temp:
        temp = Path(temp)
        classes = temp / "classes"
        run(
            "javac",
            "--release",
            "11",
            "-classpath",
            platform,
            "-d",
            classes,
            *sorted((ROOT / "java").rglob("*.java")),
        )
        run(
            "java",
            "-cp",
            d8,
            "com.android.tools.r8.D8",
            "--min-api",
            "30",
            "--lib",
            platform,
            "--output",
            temp,
            *sorted(classes.rglob("*.class")),
        )
        unsigned = temp / "unsigned.apk"
        with zipfile.ZipFile(unsigned, "w", zipfile.ZIP_DEFLATED) as apk:
            apk.writestr("AndroidManifest.xml", module.encode_xml(manifest))
            apk.write(temp / "classes.dex", "classes.dex")
            apk.write(library, f"lib/{args.abi}/libbunembeddingpoc.so")
            # Include exact upstream Bun license when --library's build supplied it.
            license_path = library.parent / "LICENSE.bun.md"
            if not license_path.exists():
                raise SystemExit("Place the pinned Bun license beside the library")
            apk.write(license_path, "assets/licenses/Bun-LICENSE.md")
            if args.sdk_bundle:
                apk.write(args.sdk_bundle, "assets/durable-probe.bundle.mjs")
                apk.write(args.sdk_notices, "assets/licenses/Durable-NOTICES.txt")
                apk.writestr(
                    "assets/durable-probe.sha256",
                    hashlib.sha256(args.sdk_bundle.read_bytes()).hexdigest() + "\n",
                )
        run(
            "java",
            "-jar",
            signer,
            "sign",
            "--ks",
            Path.home() / ".android/debug.keystore",
            "--ks-pass",
            "pass:android",
            "--min-sdk-version",
            "30",
            "--out",
            args.out,
            unsigned,
        )
    print(
        "Built", args.out, "SHA256", hashlib.sha256(args.out.read_bytes()).hexdigest()
    )


if __name__ == "__main__":
    main()
