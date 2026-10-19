"""Structural and provenance tests for the generated Android terminal fallback."""

import hashlib
import json
import runpy
import struct
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FONT = ROOT / "assets/fonts/PiTerminalSymbolsMono-Regular.ttf"
PROVENANCE = ROOT / "assets/fonts/PiTerminalSymbolsMono-Regular.provenance.json"
NOTICE = ROOT / "licenses/PI-TERMINAL-SYMBOLS-NOTICES.txt"
LEGACY_POWERLINE = {
    0xE0A0,
    0xE0A1,
    0xE0A2,
    0xE0B0,
    0xE0B1,
    0xE0B2,
    0xE0B3,
}
REQUIRED_FASTFETCH = {
    0xF0443,
    0xEBC6,
    0xF013,
    0xF03D6,
    0xF489,
    0xF0322,
    0xF0EE0,
    0xF0ED1,
    0xF0379,
    0xF0F86,
    0xF04E1,
    0xF0150,
    0xF028,
    0xF0960,
    0xF075A,
    0xF488,
    0xF027C,
    0xF003B,
    0xE623,
    0xF031,
    0xE795,
    0xF192,
}


def u16(data: bytes, offset: int) -> int:
    return struct.unpack_from(">H", data, offset)[0]


def u32(data: bytes, offset: int) -> int:
    return struct.unpack_from(">I", data, offset)[0]


class Sfnt:
    """The small subset of TrueType parsing needed for dependency-free CI checks."""

    def __init__(self, path: Path):
        self.data = path.read_bytes()
        count = u16(self.data, 4)
        self.tables = {}
        for index in range(count):
            offset = 12 + index * 16
            tag = self.data[offset : offset + 4].decode("ascii")
            table_offset = u32(self.data, offset + 8)
            length = u32(self.data, offset + 12)
            self.tables[tag] = self.data[table_offset : table_offset + length]

    def cmap(self) -> dict[int, int]:
        table = self.tables["cmap"]
        mappings = {}
        for index in range(u16(table, 2)):
            record = 4 + index * 8
            offset = u32(table, record + 4)
            format_ = u16(table, offset)
            if format_ == 4:
                mappings.update(self._cmap4(table, offset))
            elif format_ == 12:
                mappings.update(self._cmap12(table, offset))
        return mappings

    @staticmethod
    def _cmap12(table: bytes, offset: int) -> dict[int, int]:
        mappings = {}
        groups = u32(table, offset + 12)
        for index in range(groups):
            group = offset + 16 + index * 12
            start, end, glyph = struct.unpack_from(">III", table, group)
            for point in range(start, end + 1):
                mappings[point] = glyph + point - start
        return mappings

    @staticmethod
    def _cmap4(table: bytes, offset: int) -> dict[int, int]:
        mappings = {}
        segments = u16(table, offset + 6) // 2
        end_codes = offset + 14
        start_codes = end_codes + 2 * segments + 2
        deltas = start_codes + 2 * segments
        range_offsets = deltas + 2 * segments
        for index in range(segments):
            start = u16(table, start_codes + 2 * index)
            end = u16(table, end_codes + 2 * index)
            delta = u16(table, deltas + 2 * index)
            range_offset_position = range_offsets + 2 * index
            range_offset = u16(table, range_offset_position)
            for point in range(start, end + 1):
                if point == 0xFFFF:
                    continue
                if range_offset:
                    glyph_position = (
                        range_offset_position + range_offset + 2 * (point - start)
                    )
                    glyph = u16(table, glyph_position)
                    if glyph:
                        glyph = (glyph + delta) & 0xFFFF
                else:
                    glyph = (point + delta) & 0xFFFF
                if glyph:
                    mappings[point] = glyph
        return mappings

    def names(self, name_id: int) -> set[str]:
        table = self.tables["name"]
        count = u16(table, 2)
        strings = u16(table, 4)
        values = set()
        for index in range(count):
            record = 6 + index * 12
            platform = u16(table, record)
            current_id = u16(table, record + 6)
            if current_id != name_id:
                continue
            length = u16(table, record + 8)
            offset = u16(table, record + 10)
            raw = table[strings + offset : strings + offset + length]
            encoding = "utf-16-be" if platform in (0, 3) else "mac_roman"
            values.add(raw.decode(encoding))
        return values

    def advances(self) -> list[int]:
        glyph_count = u16(self.tables["maxp"], 4)
        metric_count = u16(self.tables["hhea"], 34)
        metrics = self.tables["hmtx"]
        widths = [u16(metrics, index * 4) for index in range(metric_count)]
        return widths + [widths[-1]] * (glyph_count - metric_count)


class TerminalSymbolsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.font = Sfnt(FONT)
        cls.cmap = cls.font.cmap()
        cls.provenance = json.loads(PROVENANCE.read_text())

    def test_output_identity_family_and_budget(self):
        output = self.provenance["output"]
        data = FONT.read_bytes()
        self.assertEqual(hashlib.sha256(data).hexdigest(), output["sha256"])
        self.assertEqual(len(data), output["bytes"])
        self.assertLessEqual(len(data), self.provenance["policy"]["maximumBytes"])
        self.assertEqual(self.font.names(1), {"Pi Terminal Symbols Mono"})
        self.assertEqual(self.font.names(2), {"Regular"})
        self.assertEqual(self.font.names(6), {"PiTerminalSymbolsMono-Regular"})
        self.assertEqual(self.font.names(21), {"Pi Terminal Symbols Mono"})
        self.assertEqual(self.font.names(22), {"Regular"})
        self.assertEqual(u32(self.font.tables["post"], 0), 0x00030000)
        self.assertEqual(len(self.font.tables["post"]), 32)

    def test_cmap_is_fixed_width_and_matches_provenance(self):
        points = sorted(self.cmap)
        output = self.provenance["output"]
        fingerprint = hashlib.sha256(
            "".join(f"U+{point:06X}\n" for point in points).encode()
        ).hexdigest()
        self.assertEqual(len(points), output["unicodeMappings"])
        self.assertEqual(fingerprint, output["unicodeCmapSha256"])
        advances = self.font.advances()
        self.assertEqual(
            set(advances),
            {output["advanceWidth"]},
            "every serialized glyph, including unencoded closure glyphs, must be fixed-width",
        )

    def test_fastfetch_fixture_maps_to_real_glyphs_including_supplementary_pua(self):
        self.assertTrue(REQUIRED_FASTFETCH <= self.cmap.keys())
        self.assertTrue(all(self.cmap[point] != 0 for point in REQUIRED_FASTFETCH))
        self.assertTrue(any(point > 0xFFFF for point in REQUIRED_FASTFETCH))
        self.assertTrue(any(point > 0xFFFF for point in self.cmap))

    def test_font_logos_and_unreviewed_generated_sets_are_absent(self):
        self.assertFalse(set(range(0xF300, 0xF386)) & self.cmap.keys())
        self.assertFalse(set(range(0xE300, 0xE3EC)) & self.cmap.keys())
        self.assertNotIn(0xF6C5, self.cmap)
        self.assertTrue(
            self.provenance["policy"]["fontLogosGlyphNamesExcluded"]
        )
        self.assertFalse(set(range(0x276C, 0x2772)) & self.cmap.keys())
        self.assertFalse(set(range(0x2800, 0x2900)) & self.cmap.keys())
        self.assertEqual({point for point in self.cmap if point < 0x80}, {0x20, 0x6D})
        generator = runpy.run_path(str(ROOT / "scripts/generate_terminal_symbols.py"))
        self.assertEqual(LEGACY_POWERLINE, set(generator["LEGACY_POWERLINE"]))
        self.assertEqual(
            [f"U+{point:04X}" for point in sorted(LEGACY_POWERLINE)],
            self.provenance["policy"]["legacyPowerlineMappings"],
        )
        self.assertTrue(LEGACY_POWERLINE <= self.cmap.keys())

    def test_generation_inputs_and_full_notices_are_pinned(self):
        generator = runpy.run_path(str(ROOT / "scripts/generate_terminal_symbols.py"))
        upstream = self.provenance["upstream"]
        self.assertEqual(generator["NERD_FONTS_REVISION"], upstream["revision"])
        self.assertEqual(
            generator["FONTTOOLS_VERSION"],
            self.provenance["generator"]["fonttools"],
        )
        self.assertEqual(
            {
                generator["PATCHED_SHA256"],
                generator["BASE_SHA256"],
                generator["FONT_LOGOS_SHA256"],
            },
            {source["sha256"] for source in upstream["inputs"]},
        )
        self.assertEqual(
            [
                {
                    "repository": "https://github.com/AndreLZGava/font-awesome-extension",
                    "revision": generator["FAE_LICENSE_REVISION"],
                    "path": "LICENCE",
                    "sha256": generator["FAE_LICENSE_SHA256"],
                }
            ],
            upstream["licenseInputs"],
        )
        notice = NOTICE.read_text()
        output = self.provenance["output"]
        for required in (
            upstream["revision"],
            output["sha256"],
            f'{output["bytes"]:,} bytes',
            f'Unicode cmap mappings: {output["unicodeMappings"]:,}',
            output["unicodeCmapSha256"],
            f'{output["advanceWidth"]:,}-unit advance width',
            "Font Logos source is not redistributed",
            "SIL OPEN FONT LICENSE Version 1.1",
            "Apache License",
            "Creative Commons Attribution 4.0 International Public License",
            "Copyright (c) 2015 konpa",
            "Copyright (c) 2014 Jesse Weed",
            "Copyright (c) 2013 Joe Loughry",
            "FONT AWESOME EXTENSION 0.0.3 MIT",
            "Copyright (c) 2017 André Luiz Gava",
            generator["FAE_LICENSE_REVISION"],
        ):
            self.assertIn(required, notice)
        for placeholder in (
            "<dates>",
            "<Erik Flowers>",
            "<URL|email>",
            "<Reserved Font Name>",
            "<additional Copyright Holder>",
        ):
            self.assertNotIn(placeholder, notice)
        self.assertNotIn("WEATHER ICONS SIL OFL 1.1", notice)


if __name__ == "__main__":
    unittest.main()
