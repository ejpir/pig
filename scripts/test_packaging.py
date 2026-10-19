"""Offline archive/publish-policy tests; synthetic headers do not prove native builds."""

import hashlib
import io
import json
import os
import plistlib
import struct
import tarfile
import tempfile
import tomllib
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

import fetch_pi
from package_desktop import TARGETS, archive_name, check_binary, package
from package_remote import asset_name
from package_remote import package as package_remote
from release_desktop import (
    ANDROID_APK,
    ANDROID_FONT,
    ANDROID_LICENSES,
    ANDROID_SIGNER_SHA256,
    BUN_RUNTIME_ASSETS,
    DEPENDENCY_SOURCE_ARCHIVES,
    manifest,
    release_android_manifest,
    validate_android_notices,
    validate_android_signature,
    validate_local_apk,
    validate_release_version,
    validate_tag,
)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="pi-packaging-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def binary(self, target):
        header = bytearray(64)
        if "linux" in target:
            header[:6] = b"\x7fELF\x02\x01"
            struct.pack_into(
                "<H", header, 18, 183 if target.startswith("aarch64") else 62
            )
        elif "apple" in target:
            header[:4] = b"\xcf\xfa\xed\xfe"
            struct.pack_into("<I", header, 4, 0x0100000C)
        else:
            header[:2] = b"MZ"
            struct.pack_into("<I", header, 60, 64)
            header.extend(b"PE\0\0\x64\x86")
        binary = self.root / target
        binary.write_bytes(header)
        return binary

    def test_android_apk_accepts_reproducible_epoch_runtime_inputs(self):
        import os
        import runpy
        import sys

        root = Path(__file__).resolve().parent.parent
        runpy.run_path(str(root / "crates/pi_android/scripts/build_apk.py"))
        builder = sys.modules["build_apk"]
        library = self.root / "libbunembeddingpoc.so"
        library.write_bytes(b"synthetic-runtime-bytes")
        os.utime(library, (0, 0))
        output = self.root / "epoch.apk"
        with builder.apk_zip(output) as apk:
            apk.write(library, "lib/arm64-v8a/" + library.name)
        with zipfile.ZipFile(output) as apk:
            name = "lib/arm64-v8a/" + library.name
            self.assertEqual(apk.read(name), library.read_bytes())
            self.assertEqual(apk.getinfo(name).date_time[:3], (1980, 1, 1))
        self.assertEqual(library.stat().st_mtime, 0)

    def test_android_apk_carries_full_notices_for_every_embedded_font(self):
        import runpy
        import sys

        # Load the real app configuration without invoking Android tools.
        root = Path(__file__).resolve().parent.parent
        app = runpy.run_path(str(root / "crates/pi_android/scripts/build_apk.py"))["PI"]
        builder = sys.modules["build_apk"]
        output = self.root / "pi.apk"
        with zipfile.ZipFile(output, "w") as apk:
            builder.write_licenses(apk, app)
        sources = {
            "NotoEmoji-OFL.txt": root / "crates/gpui_android/assets/fonts/OFL.txt",
            "IBM-PLEX-OFL.txt": root / "licenses/IBM-PLEX-OFL.txt",
            "COMMIT-MONO-OFL.txt": root / "licenses/COMMIT-MONO-OFL.txt",
            "PI-TERMINAL-SYMBOLS-NOTICES.txt": root
            / "licenses/PI-TERMINAL-SYMBOLS-NOTICES.txt",
            "QUICKJS-NG-MIT.txt": root / "licenses/QUICKJS-NG-MIT.txt",
            "RQUICKJS-MIT.txt": root / "licenses/RQUICKJS-MIT.txt",
            **{
                name: root / "licenses" / name
                for name in (
                    "ROXMLTREE-MIT.txt",
                    "SVGTYPES-MIT.txt",
                    "KURBO-MIT.txt",
                    "LYON-MIT.txt",
                )
            },
        }
        legal_sources = {
            "LICENSE": root / "LICENSE",
            "THIRD_PARTY.md": root / "THIRD_PARTY.md",
            "RUST-DEPENDENCY-NOTICES.html": root
            / "licenses/RUST-DEPENDENCY-NOTICES.html",
            "ZED-APACHE-2.0": root / "licenses/ZED-APACHE-2.0",
            "ZED-GPL-3.0": root / "licenses/ZED-GPL-3.0",
            "ZED-ICONS-ISC.txt": root / "licenses/ZED-ICONS-ISC.txt",
        }
        with zipfile.ZipFile(output) as apk:
            all_sources = sources | legal_sources
            self.assertEqual(
                all_sources,
                ANDROID_LICENSES,
                "APK builder and release validator notice inventories drifted",
            )
            self.assertEqual(
                set(apk.namelist()),
                {f"assets/licenses/{name}" for name in all_sources},
            )
            for name, source in all_sources.items():
                with self.subTest(notice=name):
                    self.assertEqual(
                        apk.read(f"assets/licenses/{name}"), source.read_bytes()
                    )
            for name in sources:
                notice = apk.read(f"assets/licenses/{name}")
                self.assertIn(b"Copyright", notice)
                if name.endswith("OFL.txt"):
                    self.assertIn(b"SIL OPEN FONT LICENSE", notice)
                elif name.startswith("PI-TERMINAL-SYMBOLS"):
                    for required in [
                        b"b894ea7803af6aade63d60a4381e006098ec9c4d",
                        b"5a0737b45bb0ec5b7ce91c35749fa9142eb4a6aef2d89df0d8a88fb6b3694202",
                        b"Font Logos source is not redistributed",
                        b"SIL OPEN FONT LICENSE Version 1.1",
                        b"Apache License",
                        b"Creative Commons Attribution 4.0 International Public License",
                    ]:
                        self.assertIn(required, notice)
                elif "APACHE-2.0" in name:
                    self.assertIn(b"Apache License", notice)
                else:
                    self.assertIn(b"Permission is hereby granted", notice)

    def android_apk(self, path, missing=None, incomplete=None):
        library = bytearray(64)
        library[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<HH", library, 16, 3, 183)
        bundle = (
            Path(__file__).parent.parent / "crates/pi_local/generated/bun-local.mjs"
        ).read_bytes()
        version = tomllib.loads(
            (Path(__file__).resolve().parent.parent / "Cargo.toml").read_text()
        )["workspace"]["package"]["version"]
        metadata = {
            "version": version,
            "status": "experimental",
            "engine": "bun",
            "bun": "1.4.3",
            "sdk": "1.3.0",
            "abi": "arm64-v8a",
            "librarySha256": hashlib.sha256(library).hexdigest(),
            "bundleSha256": hashlib.sha256(bundle).hexdigest(),
        }
        with zipfile.ZipFile(path, "w") as apk:
            apk.writestr("AndroidManifest.xml", release_android_manifest())
            apk.writestr("assets/local-runtime.json", json.dumps(metadata))
            apk.writestr("assets/bun-local.mjs", bundle)
            font = ANDROID_FONT.read_bytes()
            apk.writestr("lib/arm64-v8a/libpi_android.so", bundle + font)
            apk.writestr("lib/arm64-v8a/libbunembeddingpoc.so", library)
            for name in ["LICENSE.bun.md", "BUN-LINKED-NOTICES.txt"]:
                apk.writestr("assets/licenses/" + name, "Synthetic nonempty notice")
            for name in ["BUN-DURABLE-NOTICES.txt", "LOCAL-AUTH-NPM-NOTICES.txt"]:
                apk.writestr(
                    "assets/licenses/" + name,
                    (Path(__file__).parent.parent / "crates/pi_local/generated" / name).read_bytes(),
                )
            for name, source in ANDROID_LICENSES.items():
                if name != missing:
                    apk.writestr(
                        f"assets/licenses/{name}",
                        b"license URL only"
                        if name == incomplete
                        else source.read_bytes(),
                    )
        return bytes(library)

    def runtime_archives(self, output, library):
        notices = {
            "LICENSE.bun.md": b"Synthetic nonempty notice",
            "BUN-LINKED-NOTICES.txt": b"Synthetic nonempty notice",
        }
        relink_entries = {
            "RELINKING.md": b"instructions",
            "scripts/relink.py": b"# synthetic test",
            "inputs/native.o": b"synthetic object",
            "sources/bun-v1.4.3-source.tar.gz": b"bun source",
            "sources/WebKit-0c06faadf65bf8e8c8ad3a5a8aca83e1e9ed653f.tar.gz": b"jsc source",
            "bridge/embedding.rs": b"embedding source",
            "bridge/jni.cpp": b"jni source",
            "bridge/patched-runtime-lib.rs": b"patched runtime source",
        }
        members = {
            name: {"bytes": len(value), "sha256": hashlib.sha256(value).hexdigest()}
            for name, value in relink_entries.items()
        }
        provenance = {
            "schema": 1,
            "target": "aarch64-linux-android28",
            "library_sha256": hashlib.sha256(library).hexdigest(),
            "bun": {
                "tag": "bun-v1.4.3",
                "commit": "c6da4a4d3010e5553438c60f6bd76d981976867c",
                "source_archive_sha256": members[
                    "sources/bun-v1.4.3-source.tar.gz"
                ]["sha256"],
            },
            "jsc": {
                "commit": "0c06faadf65bf8e8c8ad3a5a8aca83e1e9ed653f",
                "source_archive_sha256": members[
                    "sources/WebKit-0c06faadf65bf8e8c8ad3a5a8aca83e1e9ed653f.tar.gz"
                ]["sha256"],
                "prebuilt_archive_sha256": hashlib.sha256(b"jsc prebuilt").hexdigest(),
            },
            "dependency_source_archives": DEPENDENCY_SOURCE_ARCHIVES,
            "bridge": {
                "embedding.rs": members["bridge/embedding.rs"]["sha256"],
                "jni.cpp": members["bridge/jni.cpp"]["sha256"],
            },
            "patched_runtime_lib_sha256": members["bridge/patched-runtime-lib.rs"][
                "sha256"
            ],
            "inputs": [
                {"member": "inputs/native.o", **members["inputs/native.o"]}
            ],
            "relink_members": members,
        }
        import release_desktop

        pinned = {
            "BUN_SOURCE_SHA256": provenance["bun"]["source_archive_sha256"],
            "JSC_SOURCE_SHA256": provenance["jsc"]["source_archive_sha256"],
            "JSC_PREBUILT_SHA256": provenance["jsc"]["prebuilt_archive_sha256"],
        }
        previous = {name: getattr(release_desktop, name) for name in pinned}
        for name, value in pinned.items():
            setattr(release_desktop, name, value)
        self.addCleanup(
            lambda: [setattr(release_desktop, name, value) for name, value in previous.items()]
        )
        provenance_bytes = json.dumps(provenance, sort_keys=True).encode()
        archives = [
            (
                "pi-bun-android-arm64-runtime.tar.gz",
                {
                    "libbunembeddingpoc.so": library,
                    **notices,
                    "BUN-RUNTIME-PROVENANCE.json": provenance_bytes,
                },
            ),
            (
                "pi-bun-android-arm64-relink.tar.gz",
                {
                    **relink_entries,
                    **notices,
                    "BUN-RUNTIME-PROVENANCE.json": provenance_bytes,
                },
            ),
        ]
        for name, entries in archives:
            with tarfile.open(output / name, "w:gz") as archive:
                for path, value in entries.items():
                    info = tarfile.TarInfo(path)
                    info.size = len(value)
                    archive.addfile(info, io.BytesIO(value))

    def test_release_requires_the_pinned_android_update_signer(self):
        sdk = self.root / "sdk"
        signer = sdk / "build-tools/1/apksigner"
        signer.parent.mkdir(parents=True)
        apk = self.root / ANDROID_APK
        apk.write_bytes(b"fixture")
        signer.write_text(
            "#!/bin/sh\necho 'Signer #1 certificate SHA-256 digest: "
            + ANDROID_SIGNER_SHA256
            + "'\n"
        )
        signer.chmod(0o755)
        with patch.dict(os.environ, {"ANDROID_HOME": str(sdk)}, clear=False):
            validate_android_signature(apk)
            signer.write_text(
                "#!/bin/sh\necho 'Signer #1 certificate SHA-256 digest: "
                + "f" * 64
                + "'\n"
            )
            with self.assertRaisesRegex(ValueError, "signer mismatch"):
                validate_android_signature(apk)

    def test_release_rejects_apk_without_the_actual_sdk_or_bun(self):
        apk = self.root / ANDROID_APK
        self.android_apk(apk)
        validate_local_apk(apk)
        broken = self.root / "broken.apk"
        with zipfile.ZipFile(apk) as original, zipfile.ZipFile(broken, "w") as archive:
            for name in original.namelist():
                archive.writestr(
                    name,
                    b"not the declared bundle"
                    if name == "lib/arm64-v8a/libpi_android.so"
                    else original.read(name),
                )
        broken.replace(apk)
        with self.assertRaises(ValueError):
            validate_local_apk(apk)

        # An APK cannot bless and embed its own replacement bundle; tagged source wins.
        self.android_apk(apk)
        with zipfile.ZipFile(apk) as original, zipfile.ZipFile(broken, "w") as archive:
            replacement = b"self-declared replacement SDK"
            metadata = json.loads(original.read("assets/local-runtime.json"))
            metadata["bundleSha256"] = hashlib.sha256(replacement).hexdigest()
            for name in original.namelist():
                value = original.read(name)
                if name == "assets/local-runtime.json":
                    value = json.dumps(metadata).encode()
                elif name == "assets/bun-local.mjs":
                    value = replacement
                elif name == "lib/arm64-v8a/libpi_android.so":
                    value = replacement + ANDROID_FONT.read_bytes()
                archive.writestr(name, value)
        broken.replace(apk)
        with self.assertRaisesRegex(ValueError, "regenerated tagged source"):
            validate_local_apk(apk)

    def test_release_refuses_missing_or_incomplete_android_font_notices(self):
        apk = self.root / ANDROID_APK
        self.android_apk(apk)
        validate_android_notices(apk)
        for name in ANDROID_LICENSES:
            for change in ["missing", "incomplete"]:
                with self.subTest(font=name, change=change):
                    self.android_apk(apk, **{change: name})
                    with self.assertRaises(ValueError):
                        validate_android_notices(apk)
        apk.write_bytes(b"not an APK")
        with self.assertRaises(ValueError):
            validate_android_notices(apk)

    def test_android_apk_refuses_missing_configured_license(self):
        import runpy
        import sys

        root = Path(__file__).resolve().parent.parent
        app = runpy.run_path(str(root / "crates/pi_android/scripts/build_apk.py"))["PI"]
        app.licenses = (self.root / "missing-OFL.txt",)
        with (
            zipfile.ZipFile(self.root / "pi.apk", "w") as apk,
            self.assertRaises(FileNotFoundError),
        ):
            sys.modules["build_apk"].write_licenses(apk, app)

    def test_supported_matrix_has_four_distinct_archives(self):
        self.assertEqual(
            {archive_name(target) for target in TARGETS},
            {
                "pi-desktop-linux-amd64.tar.gz",
                "pi-desktop-linux-arm64.tar.gz",
                "pi-desktop-macos-arm64.zip",
                "pi-desktop-windows-amd64.zip",
            },
        )

    def test_headers_refuse_mislabeled_architecture_platform_and_truncation(self):
        for target in TARGETS:
            with self.subTest(target=target):
                binary = self.binary(target)
                check_binary(binary, target)
                for other in TARGETS.keys() - {target}:
                    with self.assertRaises(ValueError):
                        check_binary(binary, other)
                binary.write_bytes(b"invalid")
                with self.assertRaises(ValueError):
                    check_binary(binary, target)

    def test_linux_archives_include_legal_icons_desktop_entry_and_executable_mode(self):
        for target in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]:
            archive = package(self.binary(target), target, self.root / "dist")
            prefix = archive.name.removesuffix(".tar.gz")
            with tarfile.open(archive) as tar:
                names = tar.getnames()
                for name in [
                    "bin/pi-desktop",
                    "LICENSE",
                    "README.md",
                    "THIRD_PARTY.md",
                    "licenses/ZED-GPL-3.0",
                    "licenses/PI-TERMINAL-SYMBOLS-NOTICES.txt",
                    "share/applications/dev.pi.desktop.desktop",
                    "share/icons/hicolor/128x128/apps/dev.pi.desktop.png",
                ]:
                    self.assertIn(f"{prefix}/{name}", names)
                self.assertEqual(
                    tar.getmember(f"{prefix}/bin/pi-desktop").mode & 0o777, 0o755
                )
                self.assertFalse(
                    any("artifacts/" in name or "target/" in name for name in names)
                )

    def test_windows_zip_is_portable_with_legal_files_no_working_tree(self):
        target = "x86_64-pc-windows-msvc"
        output = self.root / "dist"
        output.mkdir()
        (output / "scratch.log").write_text("must not enter archive")
        archive = package(self.binary(target), target, output)
        with zipfile.ZipFile(archive) as zipped:
            names = zipped.namelist()
            self.assertIn("pi-desktop-windows-amd64/pi-desktop.exe", names)
            self.assertIn("pi-desktop-windows-amd64/licenses/ZED-GPL-3.0", names)
            self.assertIn(
                "pi-desktop-windows-amd64/licenses/PI-TERMINAL-SYMBOLS-NOTICES.txt",
                names,
            )
            self.assertNotIn("scratch.log", names)
            self.assertFalse(
                any(".git/" in name or "node_modules/" in name for name in names)
            )

    def test_pi_notices_and_bundled_licenses_ship_in_licenses(self):
        target = "x86_64-unknown-linux-gnu"
        notices = self.root / "notices.txt"
        notices.write_text("== undici@8.10.2 ==\nMIT\n")
        archive = package(self.binary(target), target, self.root / "dist", notices)
        prefix = archive.name.removesuffix(".tar.gz")
        with tarfile.open(archive) as tar:
            names = tar.getnames()
            for name in ["PI-NOTICES.txt", "BUN-LICENSE.md", "PI-MIT.txt"]:
                self.assertIn(f"{prefix}/licenses/{name}", names)
            copied = tar.extractfile(f"{prefix}/licenses/PI-NOTICES.txt").read()
            self.assertEqual(copied, notices.read_bytes())

    def test_macos_stage_signs_verifies_and_keeps_the_app_bundle(self):
        target = "aarch64-apple-darwin"
        calls = []

        def command(argv, **kwargs):
            self.assertTrue(kwargs["check"])
            calls.append(argv)
            if argv[0] == "ditto":
                bundle = Path(argv[-2])
                self.assertEqual(bundle.name, "Pi Desktop.app")
                self.assertTrue((bundle / "Contents/MacOS/pi-desktop").is_file())
                self.assertTrue(
                    (bundle / "Contents/Resources/licenses/ZED-GPL-3.0").is_file()
                )
                self.assertTrue(
                    (
                        bundle
                        / "Contents/Resources/licenses/PI-TERMINAL-SYMBOLS-NOTICES.txt"
                    ).is_file()
                )
                with (bundle / "Contents/Info.plist").open("rb") as source:
                    self.assertEqual(
                        plistlib.load(source)["CFBundleExecutable"], "pi-desktop"
                    )
                Path(argv[-1]).write_bytes(b"mock archive; not a native-signing test")

        with (
            patch("package_desktop.sys.platform", "darwin"),
            patch("package_desktop.subprocess.run", side_effect=command),
        ):
            package(self.binary(target), target, self.root / "dist")
        self.assertEqual([call[0] for call in calls], ["codesign", "codesign", "ditto"])
        self.assertIn("--verify", calls[1])
        self.assertIn("--keepParent", calls[2])

    def test_the_embedded_pi_is_the_release_the_extension_pins(self):
        version = fetch_pi.pinned_version()
        hashes = fetch_pi.pinned_hashes(version)
        for platform in ["darwin-arm64", "linux-x64", "linux-arm64"]:
            self.assertRegex(hashes[f"pi-{platform}.tar.gz"], r"^[0-9a-f]{64}$")
        self.assertRegex(hashes["pi-windows-x64.zip"], r"^[0-9a-f]{64}$")
        extension = (
            fetch_pi.ROOT / "crates/pi_core/extension/pi-desktop.ts"
        ).read_text()
        self.assertIn(f'const EXTENSION_VERSION = "{version}-', extension)

    def test_pi_archives_unpack_with_the_executable_at_the_top(self):
        def tarball():
            data = io.BytesIO()
            with tarfile.open(fileobj=data, mode="w:gz") as tar:
                for name in ["pi/pi", "pi/theme/dark.json"]:
                    info = tarfile.TarInfo(name)
                    info.size = 2
                    tar.addfile(info, io.BytesIO(b"{}"))
            data.seek(0)
            return data.read()

        def zipped():
            data = io.BytesIO()
            with zipfile.ZipFile(data, "w") as archive:
                archive.writestr("pi.exe", "MZ")
                archive.writestr("theme/dark.json", "{}")
            return data.getvalue()

        for name, data, program in [
            ("pi-linux-x64.tar.gz", tarball(), "pi"),
            ("pi-windows-x64.zip", zipped(), "pi.exe"),
        ]:
            folder = self.root / name
            folder.mkdir()
            fetch_pi.unpack(data, name, folder)
            self.assertTrue((folder / program).is_file(), name)
            self.assertTrue((folder / "theme/dark.json").is_file(), name)
            self.assertEqual(
                sorted(entry.name for entry in folder.iterdir()),
                sorted([program, "theme"]),
            )

    def test_macos_refuses_non_native_packaging(self):
        with (
            patch("package_desktop.sys.platform", "linux"),
            self.assertRaises(ValueError),
        ):
            package(
                self.binary("aarch64-apple-darwin"),
                "aarch64-apple-darwin",
                self.root / "dist",
            )

    def test_remote_helpers_have_distinct_verified_native_assets(self):
        self.assertEqual(
            {asset_name(target) for target in TARGETS},
            {
                "pi-desktop-remote-linux-amd64",
                "pi-desktop-remote-linux-arm64",
                "pi-desktop-remote-macos-arm64",
                "pi-desktop-remote-windows-amd64.exe",
            },
        )
        for target in ["aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]:
            binary = self.binary(target)
            installed = package_remote(binary, target, self.root / "remote")
            self.assertEqual(installed.name, asset_name(target))
            self.assertEqual(installed.read_bytes(), binary.read_bytes())

    def test_release_tags_are_validated_not_shell_interpolated(self):
        for tag in ["v0.1.0", "v1.2.3-rc.1"]:
            validate_tag(tag)
        for tag in [
            "main",
            "v",
            "v1.2",
            "v1.2.3\n",
            "v1.2.3/../bad",
            "v1.2.3;exit",
            "v$(id)",
        ]:
            with self.assertRaises(ValueError):
                validate_tag(tag)

    def test_release_tag_matches_the_version_used_to_download_helpers(self):
        validate_release_version("v1.2.3-rc.1", "1.2.3-rc.1")
        with self.assertRaises(ValueError):
            validate_release_version("v1.2.3", "0.1.0")

    def test_release_manifest_requires_exact_complete_matrix_and_correct_digests(self):
        output = self.root / "release"
        output.mkdir()
        names = sorted(
            [archive_name(target) for target in TARGETS]
            + [asset_name(target) for target in TARGETS]
            + [ANDROID_APK]
            + sorted(BUN_RUNTIME_ASSETS)
        )
        for name in names[:-1]:
            (output / name).write_bytes(b"fixture")
        with self.assertRaises(ValueError):
            manifest(output)
        (output / names[-1]).write_bytes(b"fixture")
        (output / "scratch.log").write_text("not publishable")
        with self.assertRaises(ValueError):
            manifest(output)
        (output / "scratch.log").unlink()
        library = self.android_apk(output / ANDROID_APK)
        self.runtime_archives(output, library)
        self.assertEqual(
            manifest(output).read_text(),
            "".join(
                f"{hashlib.sha256((output / name).read_bytes()).hexdigest()}  {name}\n"
                for name in names
            ),
        )

    def test_empty_or_symlink_release_archives_are_refused(self):
        output = self.root / "release"
        output.mkdir()
        for target in TARGETS:
            (output / archive_name(target)).write_bytes(b"fixture")
            (output / asset_name(target)).write_bytes(b"fixture")
        library = self.android_apk(output / ANDROID_APK)
        self.runtime_archives(output, library)
        first = output / archive_name(next(iter(TARGETS)))
        first.write_bytes(b"")
        with self.assertRaises(ValueError):
            manifest(output)
        first.unlink()
        first.symlink_to(self.binary("aarch64-unknown-linux-gnu"))
        with self.assertRaises(ValueError):
            manifest(output)


if __name__ == "__main__":
    unittest.main()
