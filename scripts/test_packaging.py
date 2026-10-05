"""Offline archive/publish-policy tests; synthetic headers do not prove native builds."""
import hashlib
import io
import plistlib
import struct
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

import fetch_pi
from package_desktop import TARGETS, archive_name, check_binary, package
from release_desktop import manifest, validate_tag, validate_release_version
from package_remote import asset_name, package as package_remote


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="pi-packaging-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def binary(self, target):
        header = bytearray(64)
        if "linux" in target:
            header[:6] = b"\x7fELF\x02\x01"
            struct.pack_into("<H", header, 18, 183 if target.startswith("aarch64") else 62)
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

    def test_supported_matrix_has_four_distinct_archives(self):
        self.assertEqual(set(archive_name(target) for target in TARGETS), {
            "pi-desktop-linux-amd64.tar.gz", "pi-desktop-linux-arm64.tar.gz",
            "pi-desktop-macos-arm64.zip", "pi-desktop-windows-amd64.zip",
        })

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
                for name in ["bin/pi-desktop", "LICENSE", "README.md", "THIRD_PARTY.md",
                             "licenses/ZED-GPL-3.0", "share/applications/dev.pi.desktop.desktop",
                             "share/icons/hicolor/128x128/apps/dev.pi.desktop.png"]:
                    self.assertIn(f"{prefix}/{name}", names)
                self.assertEqual(tar.getmember(f"{prefix}/bin/pi-desktop").mode & 0o777, 0o755)
                self.assertFalse(any("artifacts/" in name or "target/" in name for name in names))

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
            self.assertNotIn("scratch.log", names)
            self.assertFalse(any(".git/" in name or "node_modules/" in name for name in names))

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
                self.assertTrue((bundle / "Contents/Resources/licenses/ZED-GPL-3.0").is_file())
                with (bundle / "Contents/Info.plist").open("rb") as source:
                    self.assertEqual(plistlib.load(source)["CFBundleExecutable"], "pi-desktop")
                Path(argv[-1]).write_bytes(b"mock archive; not a native-signing test")

        with patch("package_desktop.sys.platform", "darwin"), patch("package_desktop.subprocess.run", side_effect=command):
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
        extension = (fetch_pi.ROOT / "crates/pi_core/extension/pi-desktop.ts").read_text()
        self.assertIn(f'const EXTENSION_VERSION = "{version}-', extension)

    def test_pi_archives_unpack_with_the_executable_at_the_top(self):
        def tarball():
            data = tempfile.SpooledTemporaryFile()
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

        for name, data, program in [("pi-linux-x64.tar.gz", tarball(), "pi"), ("pi-windows-x64.zip", zipped(), "pi.exe")]:
            folder = self.root / name
            folder.mkdir()
            fetch_pi.unpack(data, name, folder)
            self.assertTrue((folder / program).is_file(), name)
            self.assertTrue((folder / "theme/dark.json").is_file(), name)
            self.assertEqual(sorted(entry.name for entry in folder.iterdir()), sorted([program, "theme"]))

    def test_macos_refuses_non_native_packaging(self):
        with patch("package_desktop.sys.platform", "linux"):
            with self.assertRaises(ValueError):
                package(self.binary("aarch64-apple-darwin"), "aarch64-apple-darwin", self.root / "dist")

    def test_remote_helpers_have_distinct_verified_native_assets(self):
        self.assertEqual({asset_name(target) for target in TARGETS}, {
            "pi-desktop-remote-linux-amd64", "pi-desktop-remote-linux-arm64",
            "pi-desktop-remote-macos-arm64", "pi-desktop-remote-windows-amd64.exe",
        })
        for target in ["aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]:
            binary = self.binary(target)
            installed = package_remote(binary, target, self.root / "remote")
            self.assertEqual(installed.name, asset_name(target))
            self.assertEqual(installed.read_bytes(), binary.read_bytes())

    def test_release_tags_are_validated_not_shell_interpolated(self):
        for tag in ["v0.1.0", "v1.2.3-rc.1"]:
            validate_tag(tag)
        for tag in ["main", "v", "v1.2", "v1.2.3\n", "v1.2.3/../bad", "v1.2.3;exit", "v$(id)"]:
            with self.assertRaises(ValueError):
                validate_tag(tag)

    def test_release_tag_matches_the_version_used_to_download_helpers(self):
        validate_release_version("v1.2.3-rc.1", "1.2.3-rc.1")
        with self.assertRaises(ValueError):
            validate_release_version("v1.2.3", "0.1.0")

    def test_release_manifest_requires_exact_complete_matrix_and_correct_digests(self):
        output = self.root / "release"
        output.mkdir()
        names = sorted([archive_name(target) for target in TARGETS] + [asset_name(target) for target in TARGETS])
        for name in names[:-1]:
            (output / name).write_bytes(b"fixture")
        with self.assertRaises(ValueError):
            manifest(output)
        (output / names[-1]).write_bytes(b"fixture")
        (output / "scratch.log").write_text("not publishable")
        with self.assertRaises(ValueError):
            manifest(output)
        (output / "scratch.log").unlink()
        self.assertEqual(manifest(output).read_text(), "".join(
            f"{hashlib.sha256(b'fixture').hexdigest()}  {name}\n" for name in names))

    def test_empty_or_symlink_release_archives_are_refused(self):
        output = self.root / "release"
        output.mkdir()
        for target in TARGETS:
            (output / archive_name(target)).write_bytes(b"fixture")
            (output / asset_name(target)).write_bytes(b"fixture")
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
