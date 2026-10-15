#!/usr/bin/env python3
"""Offline, small-fixture tests; no Bun build, network, NDK or LLVM required."""

import io
import json
import os
import shlex
import tarfile
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

import build_runtime as runtime


class RuntimeToolingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="runtime-tooling-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.build = self.root / "source/build/android arm64"
        self.build.mkdir(parents=True)
        self.ndk = self.root / "ndk"
        self.llvm = self.root / "llvm"
        self.ndk.mkdir()
        self.llvm.mkdir()

    def put(self, name, content="object"):
        path = self.build / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def graph(self):
        return runtime.LinkInputs(
            self.build, [self.root], [(self.ndk, "{ndk}"), (self.llvm, "{llvm}")]
        )

    def source_tar(self, name, entries):
        path = self.root / name
        with tarfile.open(path, "w:gz") as tar:
            for member, data in entries.items():
                data = data.encode()
                info = tarfile.TarInfo(member)
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
        return path

    def test_recursive_response_and_linker_scripts(self):
        self.put("obj/one.o")
        self.put("obj/two spaced.o")
        self.put("rust/dependency.rlib")
        archive = self.root / "cache/webkit/lib/libJavaScriptCore.a"
        archive.parent.mkdir(parents=True)
        archive.write_text("archive")
        self.put("nested.rsp", '"obj/two spaced.o" rust/dependency.rlib')
        original = "obj/one.o @nested.rsp " + shlex.quote(
            os.path.relpath(archive, self.build)
        )
        rsp = self.put("bun-debug.rsp", original)
        self.put("bun-debug.lazy.rsp", "--start-lib obj/one.o --end-lib")
        script = self.put("exports.list", "{ symbols; };")
        graph = self.graph()
        translated = graph.translate(
            [
                str(self.llvm / "bin/clang++"),
                "@bun-debug.rsp",
                "-Wl,@bun-debug.lazy.rsp",
                "-Wl,--version-script=" + str(script),
                "--sysroot=" + str(self.ndk / "sysroot"),
                "--ld-path=" + str(self.llvm / "bin/ld.lld"),
                "-L" + str(self.ndk / "lib"),
                "-shared",
                "-lc",
                "-Wl,-soname,libbunembeddingpoc.so",
                "-o",
                "output.so",
            ],
            driver=True,
        )
        self.assertEqual(translated[0], "{llvm}/bin/clang++")
        self.assertIn("--sysroot={ndk}/sysroot", translated)
        self.assertIn("--ld-path={llvm}/bin/ld.lld", translated)
        self.assertIn("-L{ndk}/lib", translated)
        self.assertIn("-Wl,-soname,libbunembeddingpoc.so", translated)
        self.assertEqual(translated[-2:], ["-o", "{out}"])
        self.assertEqual(len(graph.responses), 3)
        self.assertEqual(len(graph.files), 8)
        self.assertEqual(rsp.read_text(), original)
        self.assertIn(str(archive), graph.files)
        # No original build/cache prefix remains in executable templates.
        self.assertNotIn(str(self.root), json.dumps([translated, graph.responses]))
        self.assertEqual(
            sum(v["path"].name == "one.o" for v in graph.files.values()), 1
        )

    def test_missing_inputs_fail_closed_including_lazy_rsp(self):
        self.put("bun-debug.rsp", "missing.o")
        with self.assertRaises(FileNotFoundError):
            self.graph().translate(["@bun-debug.rsp"])
        self.put("bun-debug.rsp", "")
        with self.assertRaises(FileNotFoundError):
            self.graph().translate(["@bun-debug.rsp", "-Wl,@bun-debug.lazy.rsp"])

    def test_response_cycle_rejected(self):
        self.put("one.rsp", "@two.rsp")
        self.put("two.rsp", "@one.rsp")
        with self.assertRaisesRegex(ValueError, "Cyclic"):
            self.graph().translate(["@one.rsp"])

    def test_outside_root_and_private_files_rejected(self):
        self.put("credentials", "not a link input")
        with self.assertRaisesRegex(ValueError, "input type"):
            self.graph().file("credentials")
        graph = runtime.LinkInputs(self.build, [self.build], [])
        foreign = self.root / "foreign.o"
        foreign.write_text("foreign")
        with self.assertRaisesRegex(ValueError, "outside"):
            graph.file(str(foreign))
        link = self.put("escape.o")
        link.unlink()
        link.symlink_to(foreign)
        with self.assertRaisesRegex(ValueError, "outside"):
            graph.file("escape.o")
        with self.assertRaisesRegex(ValueError, "Unaudited"):
            self.graph().translate(["--unknown-path=/private/config"])
        with self.assertRaises(ValueError):
            self.graph().translate(["-L/unmapped/system/lib"])

    def test_thin_archive_fails_closed(self):
        self.put("thin.a", "!<thin>\nexternal-member.o")
        with self.assertRaisesRegex(ValueError, "Thin archives"):
            self.graph().file("thin.a")

    def test_relink_relocates_rsp_in_fresh_scratch(self):
        self.put("object.o")
        self.put("bun-debug.rsp", "object.o")
        self.put("bun-debug.lazy.rsp", "--start-lib object.o --end-lib")
        graph = self.graph()
        command = graph.translate(
            [
                str(self.llvm / "bin/clang++"),
                "@bun-debug.rsp",
                "-Wl,@bun-debug.lazy.rsp",
                "--sysroot=" + str(self.ndk / "sysroot"),
                "-shared",
                "-o",
                "old.so",
            ],
            driver=True,
        )
        bundle = self.root / "unpacked elsewhere"
        (bundle / "scripts").mkdir(parents=True)
        for record in graph.files.values():
            dest = bundle / record["member"]
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(record["path"].read_bytes())
        runtime.write_json(
            bundle / "relink-plan.json",
            {"command": command, "responses": graph.responses},
        )
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            if "cwd" in kwargs:
                self.assertNotIn(str(self.build), json.dumps(argv))
                self.assertIn("--sysroot=" + str(self.ndk / "sysroot"), argv)
                for arg in argv:
                    if arg.startswith(("@", "-Wl,@")):
                        path = Path(arg.split("@", 1)[1])
                        self.assertTrue(path.is_relative_to(kwargs["cwd"]))
                        tokens = shlex.split(path.read_text())
                        self.assertIn(
                            str(
                                bundle
                                / graph.files[str(self.build / "object.o")]["member"]
                            ),
                            tokens,
                        )

        args = Namespace(ndk=self.ndk, llvm=self.llvm, out=self.root / "result.so")
        with (
            patch.object(runtime, "ROOT", bundle / "scripts"),
            patch.object(runtime, "make_toolchain", return_value=self.llvm),
            patch.object(runtime, "run", side_effect=fake_run),
            patch.object(runtime, "inspect_elf") as inspect,
        ):
            runtime.relink(args)
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[-1][-2:], ["--strip-debug", args.out])
        inspect.assert_called_once_with(args.out)

    def test_shared_library_architecture_and_abi(self):
        header = "Class: ELF64\nType: DYN (Shared object file)\nMachine: AArch64\n"
        symbols = "\n".join(
            f"1: 000012 40 FUNC GLOBAL DEFAULT 12 {name}" for name in runtime.ABI
        )
        runtime.validate_elf(header, symbols)
        for wrong in (
            header.replace("AArch64", "Advanced Micro Devices X86-64"),
            header.replace("DYN", "EXEC"),
            header.replace("ELF64", "ELF32"),
        ):
            with self.assertRaisesRegex(ValueError, "AArch64 ET_DYN"):
                runtime.validate_elf(wrong, symbols)
        for wrong in (
            symbols.replace("pi_bun_init", "wrong"),
            symbols.replace("DEFAULT", "HIDDEN"),
            symbols.replace("12 ", "UND "),
        ):
            with self.assertRaisesRegex(ValueError, "Missing runtime ABI"):
                runtime.validate_elf(header, wrong)

    def test_build_command_has_ci_pins_and_keep_rsp(self):
        command = runtime.build_command("bun", self.build, 2)
        self.assertEqual(command[:2], ["bun", "scripts/build.ts"])
        for arg in (
            "--profile=android",
            "--arch=aarch64",
            "--ci=ON",
            "--lto=OFF",
            "--android-api-level=28",
            "--target=bun",
        ):
            self.assertIn(arg, command)
        self.assertEqual(command[-3:], ["-d", "keeprsp", "-j2"])
        with self.assertRaises(ValueError):
            runtime.build_command("bun", self.build, 0)

    def test_archives_are_allowlisted_normalized_and_repeatable(self):
        obj = self.put("selected.o")
        self.put("credentials", "excluded")
        files = {"inputs/selected.o": obj}
        a, b = self.root / "a.tar.gz", self.root / "b.tar.gz"
        runtime.make_archive(a, files)
        obj.touch()
        runtime.make_archive(b, files)
        self.assertEqual(a.read_bytes(), b.read_bytes())
        with tarfile.open(a) as tar:
            self.assertEqual(tar.getnames(), ["inputs/selected.o"])
            entry = tar.getmembers()[0]
            self.assertEqual(
                (entry.uid, entry.gid, entry.mtime, entry.mode), (0, 0, 0, 0o644)
            )
            self.assertTrue(entry.isfile())
        for name in ("/absolute", "../escape", "ok/../../escape", "evil\\escape", "."):
            with self.assertRaises(ValueError):
                runtime.make_archive(b, {name: obj})
        symlink = self.build / "link.o"
        symlink.symlink_to(obj)
        with self.assertRaises(ValueError):
            runtime.make_archive(b, {"link.o": symlink})
        too_small = self.root / "small.tar.gz"
        with self.assertRaisesRegex(ValueError, "exceeds"):
            runtime.make_archive(too_small, files, limit=1)
        self.assertFalse(too_small.exists())
        self.assertFalse(too_small.with_name(too_small.name + ".partial").exists())

    def test_archive_notices_do_not_extract_source_or_traversal(self):
        archive = self.source_tar(
            "source.tar.gz", {"src/LICENSE": "full license", "src/code.c": "code"}
        )
        stage = self.root / "stage"
        files = {}
        runtime.archive_notices(archive, "dependency", stage, files)
        self.assertEqual(list(files), ["notices/dependency/src/LICENSE"])
        self.assertEqual(next(iter(files.values())).read_text(), "full license")
        self.assertFalse((stage / "src/code.c").exists())
        bad = self.source_tar("bad.tar.gz", {"../LICENSE": "bad"})
        with self.assertRaises(ValueError):
            runtime.archive_notices(bad, "dependency", stage, {})

    def test_license_collection_never_reads_cargo_credentials(self):
        root = self.root / "public-package"
        root.mkdir()
        (root / "LICENSE-MIT").write_text("full text")
        (root / "credentials.toml").write_text("excluded")
        (root / "LICENSES").mkdir()
        (root / "LICENSES/Apache-2.0.txt").write_text("full Apache text")
        (root / "target").mkdir()
        (root / "target/LICENSE").write_text("unrelated")
        files = {}
        self.assertEqual(runtime.collect_notices(root, "package", files), 2)
        self.assertEqual(
            list(files),
            ["notices/package/LICENSE-MIT", "notices/package/LICENSES/Apache-2.0.txt"],
        )

    def test_prebuilt_hash_bound_to_actual_jsc_input(self):
        archive_path = self.root / "cache/webkit-pin/lib/libJavaScriptCore.a"
        archive_path.parent.mkdir(parents=True)
        archive_path.write_text("matching archive")
        graph = self.graph()
        graph.file(str(archive_path))
        tar = self.source_tar(
            "prebuilt.tar.gz", {"lib/libJavaScriptCore.a": "matching archive"}
        )
        runtime.verify_prebuilt(tar, graph)
        archive_path.write_text("different")
        with self.assertRaisesRegex(ValueError, "differs"):
            runtime.verify_prebuilt(tar, graph)

    def test_prepared_source_rejects_reused_modified_sources(self):
        prefix = "bun-" + runtime.BUN_TAG
        archive = self.source_tar(
            "bun.tar.gz",
            {
                prefix + "/src/runtime/lib.rs": "mod bin_entry;\n",
                prefix + "/src/file.rs": "original",
                prefix + "/test/excluded.rs": "not extracted",
            },
        )
        source = self.root / "prepared"
        (source / "src/runtime").mkdir(parents=True)
        (source / "src/runtime/lib.rs").write_text(
            "mod bin_entry;\nmod pi_embedding;\n"
        )
        (source / "src/file.rs").write_text("original")
        bridge = self.root / "embedding.rs"
        bridge.write_text("bridge")
        (source / "src/runtime/pi_embedding.rs").write_text("bridge")
        manifest = runtime.verify_prepared_source(source, archive, bridge)
        self.assertEqual(len(manifest), 3)
        (source / "src/file.rs").write_text("modified")
        with self.assertRaisesRegex(ValueError, "differs"):
            runtime.verify_prepared_source(source, archive, bridge)

    def test_cached_download_checks_hash_without_network(self):
        archive = self.put("cached.a")
        with patch.object(
            runtime.urllib.request, "urlopen", side_effect=AssertionError("network")
        ):
            runtime.download(
                "https://example.invalid", archive, runtime.sha256(archive)
            )
            with self.assertRaisesRegex(ValueError, "checksum"):
                runtime.download("https://example.invalid", archive, "0" * 64)


if __name__ == "__main__":
    unittest.main()
