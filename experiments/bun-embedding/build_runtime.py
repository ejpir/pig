#!/usr/bin/env python3
"""Build pinned Bun Android ARM64 and publish bounded runtime + object relink kits.

Linux x86_64 host; bootstrap Bun 1.4.2, NDK 29, LLVM 23 and the pinned Rust
nightly (including rust-src) must already be installed. No tool installation.
The relink kit's copy of this script also implements the offline `relink` command.
"""

import argparse
import gzip
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.error
import urllib.request
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent
BUN_TAG = "bun-v1.4.3"
BUN_COMMIT = "c6da4a4d3010e5553438c60f6bd76d981976867c"
BUN_SHA256 = "2366bc56d092f8535bbe5827788e20d987fbc96798de4d683e0ca78ea286ebf4"
JSC_COMMIT = "0c06faadf65bf8e8c8ad3a5a8aca83e1e9ed653f"
RUST = "nightly-2026-09-15"
LIMIT = 2 * 1024**3
LIBRARY = "libbunembeddingpoc.so"
PROVENANCE = "BUN-RUNTIME-PROVENANCE.json"
ABI = {
    "JNI_OnLoad",
    "pi_bun_init",
    "pi_bun_eval",
    "pi_bun_pump",
    "Java_dev_pi_bunembedding_ProbeActivity_nativePid",
    "Java_dev_pi_bunembedding_ProbeActivity_nativeInit",
    "Java_dev_pi_bunembedding_ProbeActivity_nativeEval",
    "Java_dev_pi_bunembedding_ProbeActivity_nativePump",
}


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def run(command, **kwargs):
    command = list(map(str, command))
    print("+", shlex.join(command), flush=True)
    return subprocess.run(command, check=True, **kwargs)


def capture(command, **kwargs):
    return subprocess.check_output(list(map(str, command)), text=True, **kwargs).strip()


def safe_name(name):
    parts = PurePosixPath(name).parts
    if not parts or name.startswith("/") or ".." in parts or "\\" in name:
        raise ValueError(f"Unsafe archive path: {name!r}")
    return str(PurePosixPath(*parts))


def download(url, dest, expected=None):
    """Bounded atomic download; an interrupted .partial is never trusted/reused."""
    dest = Path(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    if not dest.exists():
        partial = dest.with_name(dest.name + ".partial")
        try:
            with (
                urllib.request.urlopen(url, timeout=120) as response,
                partial.open("wb") as out,
            ):
                size = 0
                while block := response.read(1024 * 1024):
                    size += len(block)
                    if size > LIMIT:
                        raise ValueError("Download exceeds 2 GiB")
                    out.write(block)
            partial.replace(dest)
        finally:
            partial.unlink(missing_ok=True)
    if dest.stat().st_size > LIMIT or (expected and sha256(dest) != expected):
        raise ValueError(f"Archive size/checksum mismatch: {dest.name}")
    return dest


def webkit_source_archive(work):
    """Export the exact JSC/WTF/bmalloc source, not WebKit's enormous test archive.

    GitHub codeload refuses this full repository (HTTP 422). A pinned sparse Git
    checkout preserves the corresponding library source plus its build tooling.
    LayoutTests, browser engines and unrelated platform suites are not linked.
    """
    repo = work / "source/WebKit-corresponding-source"
    url = "https://github.com/oven-sh/WebKit.git"
    if not (repo / ".git").exists():
        repo.mkdir(parents=True, exist_ok=True)
        run(["git", "init", "-q", repo])
        run(["git", "-C", repo, "remote", "add", "origin", url])
    if capture(["git", "-C", repo, "remote", "get-url", "origin"]) != url:
        raise ValueError("Unexpected corresponding-source remote")
    run(
        [
            "git",
            "-C",
            repo,
            "fetch",
            "--depth=1",
            "--filter=blob:none",
            "origin",
            JSC_COMMIT,
        ]
    )
    run(["git", "-C", repo, "sparse-checkout", "init", "--cone"])
    run(
        [
            "git",
            "-C",
            repo,
            "sparse-checkout",
            "set",
            "Source/JavaScriptCore",
            "Source/WTF",
            "Source/bmalloc",
            "Source/cmake",
            "Source/ThirdParty/unifdef",
            "Source/ThirdParty/gtest",
            "Source/ThirdParty/gmock",
            "Source/ThirdParty/libpas",
            "Tools/Scripts",
            "Tools/TestWebKitAPI",
            "icu",
        ]
    )
    run(["git", "-C", repo, "checkout", "--detach", JSC_COMMIT])
    run(["git", "-C", repo, "diff", "--exit-code", "HEAD"])
    if capture(["git", "-C", repo, "rev-parse", "HEAD"]) != JSC_COMMIT:
        raise ValueError("Corresponding-source commit mismatch")
    files = {}
    for name in capture(["git", "-C", repo, "ls-files", "-z"]).split("\0"):
        path = repo / name
        if name and path.is_file():
            resolved = path.resolve()
            if not resolved.is_relative_to(repo.resolve()):
                raise ValueError("Corresponding-source symlink escapes checkout")
            files[f"WebKit-{JSC_COMMIT}/" + safe_name(name)] = resolved
    if not any("Source/JavaScriptCore/" in name for name in files):
        raise ValueError("Missing corresponding JavaScriptCore source")
    archive = work / "downloads" / f"WebKit-{JSC_COMMIT}.tar.gz"
    archive.parent.mkdir(parents=True, exist_ok=True)
    make_archive(archive, files)
    return archive


def verify_prepared_source(source, archive, bridge):
    """Do not label an edited/reused scratch checkout as the pinned source."""
    prefix = "bun-" + BUN_TAG
    excluded = {"test", "bench", "docs", ".buildkite", ".github"}
    manifest = {}
    with tarfile.open(archive, "r|gz") as tar:
        for item in tar:
            parts = PurePosixPath(safe_name(item.name)).parts
            if parts[0] != prefix:
                raise ValueError("Unexpected Bun archive prefix")
            if len(parts) < 2 or parts[1] in excluded or not item.isfile():
                continue
            relative = PurePosixPath(*parts[1:]).as_posix()
            path = source / relative
            if not path.resolve().is_relative_to(source.resolve()):
                raise ValueError("Prepared source escapes source root")
            with tar.extractfile(item) as stream:
                if relative == "src/runtime/lib.rs":
                    original = stream.read()
                    if original.count(b"mod bin_entry;") != 1:
                        raise ValueError("Pinned runtime module layout changed")
                    expected = hashlib.sha256(
                        original.replace(
                            b"mod bin_entry;", b"mod bin_entry;\nmod pi_embedding;", 1
                        )
                    ).hexdigest()
                else:
                    expected = hashlib.file_digest(stream, "sha256").hexdigest()
            if sha256(path) != expected:
                raise ValueError(
                    f"Prepared source differs from pinned archive: {relative}"
                )
            manifest[relative] = expected
    if sha256(source / "src/runtime/pi_embedding.rs") != sha256(bridge):
        raise ValueError("Prepared embedding bridge differs from recorded source")
    manifest["src/runtime/pi_embedding.rs"] = sha256(bridge)
    return manifest


def make_archive(dest, files, limit=LIMIT):
    """Regular-file allowlist only, deterministic headers, no symlinks or trees."""
    names = {safe_name(name): Path(path) for name, path in files.items()}
    dest = Path(dest)
    partial = dest.with_name(dest.name + ".partial")
    try:
        with (
            partial.open("wb") as raw,
            gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as zipped,
            tarfile.open(fileobj=zipped, mode="w|", format=tarfile.PAX_FORMAT) as tar,
        ):
            for name, path in sorted(names.items()):
                if not path.is_file() or path.is_symlink():
                    raise ValueError(f"Not a regular archive input: {name}")
                info = tarfile.TarInfo(name)
                info.size = path.stat().st_size
                info.mode = 0o644
                with path.open("rb") as stream:
                    tar.addfile(info, stream)
                if raw.tell() > limit:
                    raise ValueError("Release archive exceeds 2 GiB")
        if partial.stat().st_size > limit:
            raise ValueError("Release archive exceeds 2 GiB")
        partial.replace(dest)
    finally:
        partial.unlink(missing_ok=True)


def ndk_tools(ndk):
    tools = ndk / "toolchains/llvm/prebuilt/linux-x86_64"
    if not re.search(
        r"Pkg.Revision\s*=\s*29\.", (ndk / "source.properties").read_text()
    ):
        raise ValueError("NDK 29 required")
    if not (tools / "bin/aarch64-linux-android28-clang++").is_file():
        raise ValueError("Linux x86_64 NDK host tools required")
    return tools


def make_toolchain(dest, llvm, ndk):
    """Keep system LLVM's resource directory read-only, including during relink."""
    tools = ndk_tools(ndk)
    if not re.search(
        r"clang version 23[. ]", capture([llvm / "bin/clang", "--version"])
    ):
        raise ValueError("LLVM clang 23 required")
    resource = Path(capture([llvm / "bin/clang", "-print-resource-dir"]))
    scratch = dest / "lib/clang/23"
    scratch.mkdir(parents=True)
    (scratch / "include").symlink_to(resource / "include", target_is_directory=True)
    runtime = sorted((tools / "lib/clang").glob("*/lib/linux"))
    if len(runtime) != 1:
        raise ValueError("Ambiguous NDK compiler runtime directory")
    runtime = runtime[0]
    links = {
        "lib/aarch64-unknown-linux-android28/libclang_rt.builtins.a": runtime
        / "libclang_rt.builtins-aarch64-android.a",
        "lib/aarch64-unknown-linux-android28/libunwind.a": runtime
        / "aarch64/libunwind.a",
        "lib/linux/libclang_rt.builtins-aarch64-android.a": runtime
        / "libclang_rt.builtins-aarch64-android.a",
        "lib/linux/aarch64/libunwind.a": runtime / "aarch64/libunwind.a",
    }
    for name, target in links.items():
        if not target.is_file():
            raise ValueError(f"Missing NDK runtime {target.name}")
        link = scratch / name
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(target)
    (dest / "bin").mkdir()
    for tool in (llvm / "bin").iterdir():
        if tool.name in {"clang", "clang++"}:
            wrapper = dest / "bin" / tool.name
            wrapper.write_text(
                f'#!/bin/sh\nexec {shlex.quote(str(tool))} -resource-dir={shlex.quote(str(scratch))} "$@"\n'
            )
            wrapper.chmod(0o755)
        elif tool.is_file():
            (dest / "bin" / tool.name).symlink_to(tool)
    return dest


def build_command(bun, build, jobs):
    if jobs < 1:
        raise ValueError("--jobs must be positive")
    return [
        str(bun),
        "scripts/build.ts",
        "--profile=android",
        "--arch=aarch64",
        "--ci=ON",
        "--lto=OFF",
        "--android-api-level=28",
        "--build-dir=" + str(build),
        "--target=bun",
        "-d",
        "keeprsp",
        f"-j{jobs}",
    ]


def validate_elf(header, symbols):
    if (
        not re.search(r"Class:\s+ELF64\b", header)
        or not re.search(r"Machine:\s+AArch64\b", header)
        or not re.search(r"Type:\s+DYN\b", header)
    ):
        raise ValueError("Runtime must be an ELF64 AArch64 ET_DYN shared library")
    defined = set()
    for line in symbols.splitlines():
        fields = line.split()
        if (
            len(fields) >= 8
            and fields[4] == "GLOBAL"
            and fields[5] == "DEFAULT"
            and fields[6] != "UND"
        ):
            defined.add(fields[7].split("@")[0])
    missing = ABI - defined
    if missing:
        raise ValueError("Missing runtime ABI symbols: " + ", ".join(sorted(missing)))


def inspect_elf(library):
    header = capture(["readelf", "-h", library], env={**os.environ, "LC_ALL": "C"})
    symbols = capture(
        ["readelf", "--dyn-syms", "--wide", library], env={**os.environ, "LC_ALL": "C"}
    )
    validate_elf(header, symbols)
    return {"header": header, "dynamic_symbols": symbols}


class LinkInputs:
    """Parse the recorded driver/lld command, recursively walking response files.

    File inputs are mapped individually, not by copying build/cache directories.
    Unknown path-bearing flags fail closed. Relative response inputs are relative
    to the original link cwd (LLVM convention), not the response file's parent.
    """

    FILE_OPTIONS = frozenset(
        {
            "--version-script",
            "--export-dynamic-symbol-list",
            "--dynamic-list",
            "--script",
            "-T",
        }
    )
    DIR_OPTIONS = frozenset({"--sysroot", "-L", "--library-path"})
    TOOL_OPTIONS = frozenset({"--ld-path"})
    VALUE_OPTIONS = frozenset(
        {"-soname", "--soname", "-u", "--undefined", "--wrap", "-z", "-m"}
    )

    def __init__(self, cwd, allowed, external):
        self.cwd = Path(cwd).resolve()
        self.allowed = [Path(p).resolve() for p in allowed]
        # Ordered most-specific first: scratch toolchain before enclosing work.
        self.external = [
            (Path(p).absolute(), replacement) for p, replacement in external
        ]
        self.files = {}
        self.responses = {}
        self.active = set()

    def absolute(self, name):
        return Path(os.path.abspath(self.cwd / name))

    def external_path(self, path):
        for prefix, replacement in self.external:
            if path.is_relative_to(prefix):
                return replacement + "/" + path.relative_to(prefix).as_posix()
        raise ValueError(f"Unmapped toolchain/search path: {path}")

    def file(self, name, response=False):
        path = self.absolute(name)
        if path.suffix not in {
            ".o",
            ".a",
            ".rlib",
            ".so",
            ".lds",
            ".list",
            ".exports",
            ".rsp",
        }:
            raise ValueError(f"Unaudited link input type: {path.name}")
        resolved = path.resolve(strict=True)
        if not resolved.is_file() or not any(
            resolved.is_relative_to(p) for p in self.allowed
        ):
            raise ValueError(f"Input outside explicit build roots: {path}")
        if path.suffix in {".a", ".rlib"}:
            with resolved.open("rb") as stream:
                if stream.read(8) == b"!<thin>\n":
                    raise ValueError("Thin archives require a separate member audit")
        key = str(path)
        if key not in self.files:
            suffix = re.sub(r"[^a-zA-Z0-9._-]", "_", path.name)
            self.files[key] = {
                "path": resolved,
                "member": f"inputs/{len(self.files):05d}-{suffix}",
            }
        member = self.files[key]["member"]
        if response:
            if key in self.active:
                raise ValueError("Cyclic linker response files")
            if member not in self.responses:
                self.active.add(key)
                self.responses[member] = self.translate(
                    shlex.split(resolved.read_text())
                )
                self.active.remove(key)
            return "{scratch}/" + member
        return "{bundle}/" + member

    def translate(self, tokens, driver=False):
        result = []
        i = 0
        if driver:
            result.append(self.external_path(self.absolute(tokens[0])))
            i = 1
        while i < len(tokens):
            arg = tokens[i]
            if arg.startswith("-Wl,"):
                result.append("-Wl," + ",".join(self.translate(arg[4:].split(","))))
            elif arg.startswith("@"):
                result.append("@" + self.file(arg[1:], response=True))
            elif arg == "-o":
                i += 1
                if i == len(tokens):
                    raise ValueError("Missing link output")
                result.extend(["-o", "{out}"])
            elif arg in self.VALUE_OPTIONS:
                i += 1
                if i == len(tokens) or "/" in tokens[i] or "\\" in tokens[i]:
                    raise ValueError(f"Invalid value for linker option {arg}")
                result.extend([arg, tokens[i]])
            elif arg in self.FILE_OPTIONS | self.DIR_OPTIONS | self.TOOL_OPTIONS:
                i += 1
                if i == len(tokens):
                    raise ValueError(f"Missing argument for {arg}")
                value = tokens[i]
                result.extend(
                    [
                        arg,
                        self.file(value)
                        if arg in self.FILE_OPTIONS
                        else self.external_path(self.absolute(value)),
                    ]
                )
            elif (
                "=" in arg
                and arg.split("=", 1)[0]
                in self.FILE_OPTIONS | self.DIR_OPTIONS | self.TOOL_OPTIONS
            ):
                option, value = arg.split("=", 1)
                result.append(
                    option
                    + "="
                    + (
                        self.file(value)
                        if option in self.FILE_OPTIONS
                        else self.external_path(self.absolute(value))
                    )
                )
            elif arg.startswith("-L") and arg != "-L":
                result.append("-L" + self.external_path(self.absolute(arg[2:])))
            elif not arg.startswith("-") and (
                "/" in arg
                or Path(arg).suffix in {".o", ".a", ".rlib", ".so", ".lds", ".list"}
            ):
                result.append(self.file(arg))
            elif "/" in arg or "\\" in arg or arg in {"&&", ";", "|"}:
                raise ValueError(f"Unaudited linker argument: {arg}")
            else:
                result.append(arg)
            i += 1
        return result


def relink(args):
    """Offline relocated relink; users may replace any packaged JSC .a first."""
    bundle = ROOT.parent
    plan = json.loads((bundle / "relink-plan.json").read_text())
    out = args.out.resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="bun-relink-") as tmp:
        scratch = Path(tmp)
        toolchain = make_toolchain(
            scratch / "llvm", args.llvm.resolve(), args.ndk.resolve()
        )
        replacements = {
            "{bundle}": str(bundle),
            "{scratch}": str(scratch),
            "{toolchain}": str(toolchain),
            "{llvm}": str(args.llvm.resolve()),
            "{ndk}": str(args.ndk.resolve()),
            "{out}": str(out),
        }

        def expand(arg):
            for old, new in replacements.items():
                arg = arg.replace(old, new)
            return arg

        for member, tokens in plan["responses"].items():
            path = scratch / safe_name(member)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(shlex.join([expand(t) for t in tokens]) + "\n")
        run([expand(t) for t in plan["command"]], cwd=scratch)
        run([args.llvm.resolve() / "bin/llvm-strip", "--strip-debug", out])
        inspect_elf(out)


def notice_name(name):
    parts = PurePosixPath(name).parts
    pattern = r"(?i)^(licen[sc]es?|copying|copyrights?|notices?|authors)([._-].*|$)"
    # SPDX-style LICENSES/MIT.txt and Rust's licenses/Apache-2.0.txt are full
    # notice texts too, even though their basenames do not start with LICENSE.
    return any(re.match(pattern, part) for part in parts)


def collect_notices(root, label, files):
    """Only public notice texts, never Cargo config/auth/index/cache contents."""
    root = Path(root).resolve()
    count = 0
    for directory, dirs, names in os.walk(root, followlinks=False):
        dirs[:] = sorted(
            d
            for d in dirs
            if d not in {".git", "target", "build", "__pycache__"}
            and not (Path(directory) / d).is_symlink()
        )
        for name in sorted(names):
            path = Path(directory) / name
            if (
                notice_name(path.relative_to(root).as_posix())
                and path.is_file()
                and not path.is_symlink()
            ):
                files["notices/" + label + "/" + path.relative_to(root).as_posix()] = (
                    path
                )
                count += 1
    return count


def archive_notices(archive, label, stage, files):
    count = 0
    with tarfile.open(archive, "r|gz") as tar:
        for item in tar:
            safe_name(item.name)
            if item.isfile() and notice_name(item.name):
                if item.size > 16 * 1024**2:
                    raise ValueError("Unexpected oversized license notice")
                member = safe_name("notices/" + label + "/" + item.name)
                dest = stage / member
                dest.parent.mkdir(parents=True, exist_ok=True)
                with tar.extractfile(item) as src, dest.open("wb") as out:
                    shutil.copyfileobj(src, out)
                files[member] = dest
                count += 1
    if not count:
        raise ValueError(f"No license notices in {label} source archive")


def verify_prebuilt(archive, graph):
    """Bind the downloaded JSC archive hash to the actual cached link inputs."""
    wanted = {
        p["path"].name: sha256(p["path"])
        for p in graph.files.values()
        if "webkit-" in str(p["path"]) and p["path"].suffix == ".a"
    }
    if "libJavaScriptCore.a" not in wanted:
        raise ValueError("Missing matching static JavaScriptCore link input")
    found = set()
    with tarfile.open(archive, "r|gz") as tar:
        for item in tar:
            safe_name(item.name)
            name = PurePosixPath(item.name).name
            if item.isfile() and name in wanted:
                with tar.extractfile(item) as stream:
                    digest = hashlib.file_digest(stream, "sha256").hexdigest()
                if digest != wanted[name] or name in found:
                    raise ValueError(
                        "Cached JSC library differs from pinned prebuilt archive"
                    )
                found.add(name)
    if found != wanted.keys():
        raise ValueError("Pinned JSC prebuilt archive is incomplete")


def upstream_crate_notices(package, root, stage, files):
    """Some published monorepo crates omit their root license. Fetch that exact VCS revision."""
    repository = re.fullmatch(
        r"https://github\.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/?",
        package.get("repository") or "",
    )
    vcs_file = root / ".cargo_vcs_info.json"
    if not repository or not vcs_file.is_file():
        raise ValueError(
            "Missing full notice and pinned public GitHub source: " + package["name"]
        )
    revision = json.loads(vcs_file.read_text())["git"]["sha1"]
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("Invalid pinned crate source revision")
    owner, repo = repository.groups()
    repo = repo.removesuffix(".git")
    label = "notices/rust/" + safe_name(package["name"] + "-" + package["version"])
    sources = []
    for name in [
        "LICENSE",
        "LICENSE.md",
        "LICENSE.txt",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "COPYING",
    ]:
        url = f"https://raw.githubusercontent.com/{owner}/{repo}/{revision}/{name}"
        dest = stage / label / ("upstream-" + name)
        try:
            download(url, dest)
        except urllib.error.HTTPError as error:
            if error.code == 404:
                continue
            raise
        files[label + "/upstream-" + name] = dest
        sources.append({"url": url, "revision": revision, "sha256": sha256(dest)})
    if not sources:
        raise ValueError("Pinned source has no full license texts: " + package["name"])
    record = stage / label / "upstream-license-source.json"
    write_json(record, sources)
    files[label + "/upstream-license-source.json"] = record
    return len(sources)


def dependency_notices(source, env, stage, files):
    if not collect_notices(source, "bun", files):
        raise ValueError("Missing Bun dependency notices")
    # Upstream's actual unit graph excludes unrelated workspace/dev crates and
    # includes the exact target/build dependencies used by this native link.
    # Never scan Cargo config, credentials, registry indexes or cache dumps.
    plan = json.loads(
        (source / "build/android-arm64/rust-target/plan.json").read_text()
    )
    package_ids = {unit["pkg_id"] for unit in plan["unitGraph"]["units"]}
    for package_id in sorted(package_ids):
        package = plan["packages"][package_id]
        if package.get("source") is None:
            continue
        root = Path(package["manifest_path"]).parent
        label = "rust/" + safe_name(package["name"] + "-" + package["version"])
        count = collect_notices(root, label, files)
        if package.get("license_file"):
            license_file = (root / package["license_file"]).resolve(strict=True)
            if not license_file.is_relative_to(root.resolve()):
                raise ValueError("Cargo license-file escapes package")
            files["notices/" + label + "/declared-license.txt"] = license_file
            count += 1
        if package.get("license") == "MPL-2.0":
            # MPL crates often carry their notices only in source headers. Ship
            # the covered Rust source form as well as the complete standard terms.
            license_text = ROOT.parent.parent / "licenses/MPL-2.0.txt"
            files["notices/" + label + "/MPL-2.0.txt"] = license_text
            for path in root.rglob("*.rs"):
                if path.is_file() and not path.is_symlink():
                    files[
                        "notices/"
                        + label
                        + "/source/"
                        + path.relative_to(root).as_posix()
                    ] = path
            count += 1
        if not count:
            upstream_crate_notices(package, root, stage, files)
    sysroot = Path(capture(["rustc", "--print", "sysroot"], env=env))
    for root, label in [
        (sysroot / "share/doc/rust", "rust-toolchain"),
        (sysroot / "lib/rustlib/src/rust", "rust-src"),
    ]:
        if not collect_notices(root, label, files):
            raise ValueError(f"Rust full license notices missing: {label}")
    # SQLite and JS polyfills can carry their only notice inline. Preserve all
    # of these small source files rather than inventing shortened license text.
    for path in [
        source / "src/jsc/bindings/sqlite/sqlite3.c",
        source / "vendor/picohttpparser/picohttpparser.h",
        *sorted((source / "src/js/thirdparty").rglob("*")),
    ]:
        if path.is_file() and not path.is_symlink():
            files["notices/inline/" + path.relative_to(source).as_posix()] = path


def build(args):
    # prepare.main is deliberately invoked in its own interpreter/process.
    import prepare

    if (prepare.TAG, prepare.COMMIT, prepare.SOURCE_SHA256) != (
        BUN_TAG,
        BUN_COMMIT,
        BUN_SHA256,
    ):
        raise ValueError("prepare.py pin changed: audit runtime helper")
    work, ndk, llvm, out = (
        p.resolve() for p in (args.work_dir, args.ndk, args.llvm, args.out_dir)
    )
    if args.jobs < 1:
        raise ValueError("--jobs must be positive")
    if out == work or work.is_relative_to(out):
        raise ValueError("Output must not enclose the work directory")
    work.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    # Refuse to overwrite completed/partial release outputs; retries use a fresh
    # out-dir, while Ninja can reuse the explicit scratch build/cache directory.
    if any(out.iterdir()):
        raise ValueError("--out-dir must be empty (use a new directory after failure)")
    tools = ndk_tools(ndk)
    bun = shutil.which("bun")
    if not bun or capture([bun, "--version"]) != "1.4.2":
        raise ValueError("Bootstrap Bun 1.4.2 required on PATH")
    env = {
        **os.environ,
        "RUSTUP_TOOLCHAIN": RUST,
        "GIT_SHA": BUN_COMMIT,
        "BUN_BUILD_CACHE_DIR": str(work / "cache"),
        "ANDROID_NDK_ROOT": str(ndk),
    }
    # Do not inherit knobs capable of silently changing source pins/ABI/build mode.
    for key in (
        "BUN_WEBKIT_PATH",
        "BUN_WEBKIT_VERSION",
        "BUN_BUILD_DIR",
        "BUN_PREFETCH_DIR",
    ):
        env.pop(key, None)
    capture(["rustc", "--version"], env=env)
    sysroot = Path(capture(["rustc", "--print", "sysroot"], env=env))
    if not (sysroot / "lib/rustlib/src/rust/library/Cargo.toml").is_file():
        raise ValueError("Workflow must provide pinned nightly with rust-src")
    run([sys.executable, ROOT / "prepare.py", "--work-dir", work], env=env)
    source = work / "source" / ("bun-" + BUN_TAG)
    build_dir = source / "build/android-arm64"
    bun_archive = work / "downloads" / (BUN_TAG + "-source.tar.gz")
    if sha256(bun_archive) != BUN_SHA256:
        raise ValueError("Pinned Bun source archive checksum mismatch")
    source_manifest = verify_prepared_source(
        source, bun_archive, ROOT / "native/embedding.rs"
    )
    if not re.search(
        r'WEBKIT_VERSION = "' + JSC_COMMIT + r'"',
        (source / "scripts/build/deps/webkit.ts").read_text(),
    ):
        raise ValueError("Pinned JavaScriptCore source revision changed")
    with tempfile.TemporaryDirectory(prefix="runtime-", dir=work) as temporary:
        stage = Path(temporary)
        toolchain = make_toolchain(stage / "llvm", llvm, ndk)
        env["BUN_TOOLCHAIN_LLVM"] = str(toolchain)
        command = build_command(bun, build_dir, args.jobs)
        run(command, cwd=source, env=env)
        ninjas = [
            p
            for p in (work / "cache/ninja").rglob("ninja")
            if p.is_file() and os.access(p, os.X_OK)
        ]
        if len(ninjas) != 1:
            raise ValueError(
                f"Expected one downloaded Ninja in work/cache, found {len(ninjas)}"
            )
        jni = stage / "jni-arm64.o"
        compile_command = [
            tools / "bin/aarch64-linux-android28-clang++",
            "-std=c++20",
            "-fPIC",
            "-fno-exceptions",
            "-fno-rtti",
            "-c",
            ROOT / "native/jni.cpp",
            "-o",
            jni,
        ]
        run(compile_command, env=env)
        run(
            [
                sys.executable,
                ROOT / "relink.py",
                "--source",
                source,
                "--build",
                build_dir,
                "--ninja",
                ninjas[0],
                "--jni-object",
                jni,
                "--out",
                out / LIBRARY,
            ],
            env=env,
        )
        unstripped_sha = sha256(out / LIBRARY)
        run([llvm / "bin/llvm-strip", "--strip-debug", out / LIBRARY])
        elf = inspect_elf(out / LIBRARY)
        graph = LinkInputs(
            build_dir,
            [work, out, ndk, llvm],
            [(toolchain, "{toolchain}"), (ndk, "{ndk}"), (llvm, "{llvm}")],
        )
        upstream = json.loads((out / "upstream-link-command.json").read_text())
        graph.translate(upstream, driver=True)
        linked = json.loads((out / "jni-link-command.json").read_text())
        plan = {
            "command": graph.translate(linked, driver=True),
            "responses": graph.responses,
        }
        # Original command's scripts and every response are preserved even where
        # the JNI variant replaces them. Unreferenced outputs are never scanned.
        files = {record["member"]: record["path"] for record in graph.files.values()}
        for name in ("prepare.py", "relink.py", "build_runtime.py"):
            files["scripts/" + name] = ROOT / name
        for name in ("embedding.rs", "jni.cpp"):
            files["bridge/" + name] = ROOT / "native" / name
        files["bridge/patched-runtime-lib.rs"] = source / "src/runtime/lib.rs"
        write_json(stage / "prepared-source-sha256.json", source_manifest)
        files["prepared-source-sha256.json"] = stage / "prepared-source-sha256.json"
        files["sources/" + bun_archive.name] = bun_archive
        jsc_source_url = "https://github.com/oven-sh/WebKit.git#" + JSC_COMMIT
        jsc_source = webkit_source_archive(work)
        files["sources/" + jsc_source.name] = jsc_source
        jsc_binary_url = f"https://github.com/oven-sh/WebKit/releases/download/autobuild-{JSC_COMMIT}/bun-webkit-linux-arm64-android-debug.tar.gz"
        jsc_binary = download(
            jsc_binary_url,
            work / "downloads/bun-webkit-linux-arm64-android-debug.tar.gz",
        )
        verify_prebuilt(jsc_binary, graph)
        archive_notices(jsc_source, "webkit", stage, files)
        dependency_notices(source, env, stage, files)
        for name in ("NOTICE", "NOTICE.toolchain"):
            path = ndk / name
            if not path.is_file():
                raise ValueError("NDK full notices required")
            files["notices/ndk/" + name] = path
        # ICU is prebuilt with JSC. WebKit source may not vendor ICU; use the
        # full ICU license text from the matching prebuilt headers when present.
        jsc_paths = [
            v["path"]
            for v in graph.files.values()
            if v["path"].name == "libJavaScriptCore.a"
        ]
        icu_header = jsc_paths[0].parent.parent / "include/unicode/uvernum.h"
        if not icu_header.is_file():
            raise ValueError("Missing matching ICU version header")
        files["notices/icu/uvernum.h"] = icu_header
        # ICU's complete license is pinned with its bundled version, not a
        # moving main-branch license link in Bun's summary table.
        match = re.search(
            r"#define U_ICU_VERSION_MAJOR_NUM\s+(\d+)", icu_header.read_text()
        )
        if not match:
            raise ValueError("Cannot identify bundled ICU version")
        icu_major = match[1]
        minor = re.search(
            r"#define U_ICU_VERSION_MINOR_NUM\s+(\d+)", icu_header.read_text()
        )
        if not minor:
            raise ValueError("Cannot identify bundled ICU minor version")
        icu_version = f"{icu_major}.{minor[1]}"
        icu_license_url = f"https://raw.githubusercontent.com/unicode-org/icu/release-{icu_version}/icu4c/LICENSE"
        icu_license = download(
            icu_license_url, work / "downloads" / f"ICU-{icu_version}-LICENSE"
        )
        files["notices/icu/LICENSE"] = icu_license
        write_json(stage / "relink-plan.json", plan)
        files["relink-plan.json"] = stage / "relink-plan.json"
        recorded = {
            "build": command,
            "jni_compile": list(map(str, compile_command)),
            "environment": {
                k: env[k]
                for k in (
                    "RUSTUP_TOOLCHAIN",
                    "GIT_SHA",
                    "BUN_BUILD_CACHE_DIR",
                    "ANDROID_NDK_ROOT",
                    "BUN_TOOLCHAIN_LLVM",
                )
            },
            "cwd": str(build_dir),
            "upstream": upstream,
            "jni_link": linked,
        }
        write_json(stage / "recorded-commands.json", recorded)
        files["recorded-commands.json"] = stage / "recorded-commands.json"
        instructions = f"""Bun Android ARM64 static-link relink kit (LGPL JavaScriptCore)

This kit contains the actual original .o/.a/.rlib inputs, JNI bridge, both
original response files (including nested inputs), linker scripts, pinned Bun
and WebKit source archives and full dependency notices. No Cargo credentials,
registry dumps, host executables, temporary tools or unrelated build outputs.
Original absolute paths appear ONLY in recorded commands/input provenance (and
may occur in debug information); executable relinking uses an exact input map.

Linux x86_64, Python >=3.11, GNU readelf, Android NDK 29, LLVM 23 required.
From this unpacked directory (spaces in paths are supported):
  python3 scripts/build_runtime.py relink --ndk /path/to/ndk/29 --llvm /usr/lib/llvm-23 --out /path/to/{LIBRARY}
The script writes relocated response files and resource-dir wrappers in a fresh
scratch directory. Your NDK supplies implicit platform/runtime libraries; use
the original revision in provenance for matching results. It never invokes Bun
main. It strips debug only and validates AArch64 ET_DYN and exported bridge ABI.

To modify JavaScriptCore, unpack sources/WebKit-{JSC_COMMIT}.tar.gz, follow that
revision's Android build recipe (Dockerfile.android), using matching ICU and
ABI flags (debug, no LTO, API 28, ARM64, PIC, same ASAN setting). Replace the
corresponding inputs/*-libJavaScriptCore.a, libWTF.a and libbmalloc.a (and ICU
archives if modified), then run the command above. Input names and original
hashes are in BUN-RUNTIME-PROVENANCE.json. No hash gate prevents modification.
WebKit source: https://github.com/oven-sh/WebKit/tree/{JSC_COMMIT}
Bun source: https://github.com/oven-sh/bun/tree/{BUN_COMMIT}
The bridge patch is native embedding.rs plus `mod pi_embedding;` in runtime
lib.rs; patched-runtime-lib.rs is included. Build/relink/prepare scripts and
exact commands are included. Rebuilding from source additionally requires Bun
1.4.2 and {RUST} + rust-src, NASM, Git, CMake and normal Bun build prerequisites.
Release contents are deterministic tar/gzip headers, not a claim of bit-for-bit
compiler reproducibility across different host paths or toolchain revisions.
"""
        (stage / "RELINKING.md").write_text(instructions)
        files["RELINKING.md"] = stage / "RELINKING.md"
        # Record exact downloaded dependency source archives without publishing
        # the cache itself. Pins/patches live in the pinned Bun build scripts.
        dependency_archives = {
            p.name: sha256(p)
            for p in sorted((work / "cache/tarballs").glob("*.tar.gz"))
            if p.is_file()
        }
        inventory = {
            name: {"sha256": sha256(path), "bytes": path.stat().st_size}
            for name, path in sorted(files.items())
        }
        provenance = {
            "schema": 1,
            "target": "aarch64-linux-android28",
            "bun": {
                "tag": BUN_TAG,
                "commit": BUN_COMMIT,
                "source_archive_sha256": BUN_SHA256,
            },
            "jsc": {
                "commit": JSC_COMMIT,
                "source_url": jsc_source_url,
                "source_archive_sha256": sha256(jsc_source),
                "prebuilt_url": jsc_binary_url,
                "prebuilt_archive_sha256": sha256(jsc_binary),
            },
            "icu": {
                "version": icu_version,
                "license_url": icu_license_url,
                "license_sha256": sha256(icu_license),
            },
            "tools": {
                "bootstrap_bun": "1.4.2",
                "rust": capture(["rustc", "--version"], env=env),
                "rust_toolchain": RUST,
                "ndk": (ndk / "source.properties").read_text(),
                "llvm": capture([llvm / "bin/clang", "--version"]),
            },
            "library_sha256": sha256(out / LIBRARY),
            "unstripped_library_sha256": unstripped_sha,
            "bridge": {
                name: sha256(ROOT / "native" / name)
                for name in ("embedding.rs", "jni.cpp")
            },
            "patched_runtime_lib_sha256": sha256(source / "src/runtime/lib.rs"),
            "dependency_source_archives": dependency_archives,
            "inputs": [
                {
                    "original_path": key,
                    "member": value["member"],
                    **inventory[value["member"]],
                }
                for key, value in graph.files.items()
            ],
            "relink_members": inventory,
            "elf": elf,
        }
        write_json(out / PROVENANCE, provenance)
        notices = {
            name: path for name, path in files.items() if name.startswith("notices/")
        }
        # Also expose notices beside the loose library for downstream packagers.
        for name, path in notices.items():
            dest = out / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dest)
        # APK packagers need the full linked-library texts, not Bun's URL table alone.
        linked_notices = out / "BUN-LINKED-NOTICES.txt"
        with linked_notices.open("wb") as stream:
            for name, path in sorted(notices.items()):
                stream.write(("\n===== " + name + " =====\n").encode())
                stream.write(path.read_bytes())
                stream.write(b"\n")
        common = {
            "BUN-LINKED-NOTICES.txt": linked_notices,
            PROVENANCE: out / PROVENANCE,
            "LICENSE.bun.md": out / "LICENSE.bun.md",
            **notices,
        }
        make_archive(
            out / "pi-bun-android-arm64-runtime.tar.gz",
            {LIBRARY: out / LIBRARY, **common},
        )
        make_archive(out / "pi-bun-android-arm64-relink.tar.gz", {**files, **common})
    print("Runtime and object relink kit:", out)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    if len(sys.argv) > 1 and sys.argv[1] == "relink":
        p.add_argument("relink")
        p.add_argument("--ndk", type=Path, required=True)
        p.add_argument("--llvm", type=Path, default=Path("/usr/lib/llvm-23"))
        p.add_argument("--out", type=Path, required=True)
        relink(p.parse_args())
    else:
        p.add_argument("--work-dir", type=Path, required=True)
        p.add_argument("--ndk", type=Path, required=True)
        p.add_argument("--llvm", type=Path, default=Path("/usr/lib/llvm-23"))
        p.add_argument("--out-dir", type=Path, required=True)
        p.add_argument("--jobs", type=int, default=2)
        build(p.parse_args())


if __name__ == "__main__":
    main()
