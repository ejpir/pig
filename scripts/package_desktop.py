#!/usr/bin/env python3
"""Stage only distributable files; archive the native binary for a supported target."""
import argparse
import plistlib
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "x86_64-unknown-linux-gnu": ("linux", "amd64", "pi-desktop"),
    "aarch64-unknown-linux-gnu": ("linux", "arm64", "pi-desktop"),
    "aarch64-apple-darwin": ("macos", "arm64", "pi-desktop"),
    "x86_64-pc-windows-msvc": ("windows", "amd64", "pi-desktop.exe"),
}


def archive_name(target: str) -> str:
    """Use unique release names, not the hosted runner's architecture label."""
    platform, arch, _ = TARGETS[target]
    suffix = "tar.gz" if platform == "linux" else "zip"
    return f"pi-desktop-{platform}-{arch}.{suffix}"


def check_binary(binary: Path, target: str) -> None:
    """Refuse mislabeled ELF, PE and Mach-O binaries before publishing."""
    platform, arch, _ = TARGETS[target]
    with binary.open("rb") as source:
        header = source.read(64)
        if platform == "linux":
            valid = (
                len(header) == 64 and header[:6] == b"\x7fELF\x02\x01"
                and struct.unpack_from("<H", header, 18)[0] == (62 if arch == "amd64" else 183)
            )
        elif platform == "macos":
            valid = (
                len(header) >= 8 and header[:4] == b"\xcf\xfa\xed\xfe"
                and struct.unpack_from("<I", header, 4)[0] == 0x0100000C
            )
        else:
            valid = len(header) == 64 and header[:2] == b"MZ"
            if valid:
                source.seek(struct.unpack_from("<I", header, 60)[0])
                pe = source.read(6)
                valid = pe == b"PE\0\0\x64\x86"
    if not valid:
        raise ValueError(f"Binary is not a native {target} executable: {binary}")


def copy_legal(destination: Path, notices: Path | None = None) -> None:
    """Keep licenses and runtime prerequisites alongside every executable.

    `notices` lists the licenses of the npm packages bundled into the built-in
    backend (from packages/pi-desktop-backend/scripts/build-binary.mjs)."""
    destination.mkdir(parents=True, exist_ok=True)
    for name in ["LICENSE", "THIRD_PARTY.md", "README.md"]:
        shutil.copy2(ROOT / name, destination / name)
    shutil.copytree(ROOT / "licenses", destination / "licenses")
    if notices is not None:
        shutil.copy2(notices, destination / "licenses" / "PI-DESKTOP-BACKEND-NOTICES.txt")


def package(binary: Path, target: str, output: Path, notices: Path | None = None) -> Path:
    """Package a prebuilt binary without ever archiving a working tree or logs."""
    check_binary(binary, target)
    platform, arch, executable = TARGETS[target]
    if platform == "macos" and sys.platform != "darwin":
        raise ValueError("macOS packaging requires native codesign and ditto")
    output.mkdir(parents=True, exist_ok=True)
    archive = output / archive_name(target)
    # Old files in artifacts/dist cannot leak into this fresh, private staging tree.
    with tempfile.TemporaryDirectory(prefix="pi-desktop-package-") as temporary:
        staging = Path(temporary)
        if platform == "macos":
            bundle = staging / "Pi Desktop.app"
            contents = bundle / "Contents"
            (contents / "MacOS").mkdir(parents=True)
            copy_legal(contents / "Resources", notices)
            shutil.copy2(binary, contents / "MacOS" / executable)
            (contents / "MacOS" / executable).chmod(0o755)
            shutil.copy2(ROOT / "packaging/macos/Info.plist", contents / "Info.plist")
            with (contents / "Info.plist").open("rb") as source:
                plistlib.load(source)
            shutil.copy2(ROOT / "packaging/macos/AppIcon.icns", contents / "Resources/AppIcon.icns")
            subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(bundle)], check=True)
            subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
            subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(bundle), str(archive)], check=True)
        else:
            folder = staging / f"pi-desktop-{platform}-{arch}"
            copy_legal(folder, notices)
            installed = folder / ("bin" if platform == "linux" else "") / executable
            installed.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(binary, installed)
            if platform == "linux":
                installed.chmod(0o755)
                desktop = folder / "share/applications"
                desktop.mkdir(parents=True)
                shutil.copy2(ROOT / "packaging/linux/dev.pi.desktop.desktop", desktop)
                for size in [128, 512]:
                    icon = folder / f"share/icons/hicolor/{size}x{size}/apps"
                    icon.mkdir(parents=True)
                    shutil.copy2(ROOT / f"assets/app-icon/app-icon-{size}.png", icon / "dev.pi.desktop.png")
                with tarfile.open(archive, "w:gz") as tar:
                    tar.add(folder, arcname=folder.name)
            else:
                with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zipped:
                    for file in sorted(folder.rglob("*")):
                        if file.is_file():
                            zipped.write(file, file.relative_to(staging).as_posix())
    return archive


def main() -> None:
    """CLI used by native packaging wrappers and CI."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    parser.add_argument("--notices", type=Path, help="the built-in backend's npm license notices")
    args = parser.parse_args()
    notices = args.notices.resolve() if args.notices else None
    print(package(args.binary.resolve(), args.target, args.output.resolve(), notices))


if __name__ == "__main__":
    main()
