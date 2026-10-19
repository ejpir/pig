#!/usr/bin/env python3
"""Generate Pi's auditable Nerd-symbol terminal fallback.

Install the pinned generator first:

    python3 -m pip install fonttools==4.67.0
    python3 scripts/generate_terminal_symbols.py

The input is a pinned Nerd Fonts patched Meslo. The output keeps only the
reviewed symbol-set ranges, Meslo's seven explicit legacy Powerline mappings,
space and `m` (required by GPUI's font admission), while removing every mapping
from the unlicensed Font Logos source font. Weather Icons and Meslo's U+F6C5
logo mapping are excluded because their redistribution evidence is incomplete.
"""

import argparse
import hashlib
import tempfile
import urllib.request
from pathlib import Path

NERD_FONTS_REVISION = "b894ea7803af6aade63d60a4381e006098ec9c4d"
FONTTOOLS_VERSION = "4.67.0"
PATCHED_URL = (
    "https://raw.githubusercontent.com/ryanoasis/nerd-fonts/"
    f"{NERD_FONTS_REVISION}/patched-fonts/Meslo/M/"
    "MesloLGMNerdFontMono-Regular.ttf"
)
PATCHED_SHA256 = "35c4453cc6d0861e505b62fc7039b3fb8db9966ed75fd13dda2115e2ca94c7f7"
FONT_LOGOS_URL = (
    "https://raw.githubusercontent.com/ryanoasis/nerd-fonts/"
    f"{NERD_FONTS_REVISION}/src/glyphs/font-logos.ttf"
)
FONT_LOGOS_SHA256 = "25d0d6ba1c1ea4700c861c29f18022aa759079fc919b2baa01bd99f377712857"
FAE_LICENSE_REVISION = "09d80249058ee8018a45da30add4339289dbb466"
FAE_LICENSE_URL = (
    "https://raw.githubusercontent.com/AndreLZGava/font-awesome-extension/"
    f"{FAE_LICENSE_REVISION}/LICENCE"
)
FAE_LICENSE_SHA256 = "b47b15d771c4507a75ff65beca3a78168d6af4cca9b45177a1aa817139c64ba5"
BASE_URL = (
    "https://raw.githubusercontent.com/ryanoasis/nerd-fonts/"
    f"{NERD_FONTS_REVISION}/src/unpatched-fonts/Meslo/M/"
    "Meslo%20LG%20M%20Regular%20for%20Powerline.ttf"
)
BASE_SHA256 = "be991137ccbe87765db7cbcdc034447b15913aa2fad01acebabbcb65b1d4a8a8"
FAMILY = "Pi Terminal Symbols Mono"
POSTSCRIPT = "PiTerminalSymbolsMono-Regular"
ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "assets/fonts/PiTerminalSymbolsMono-Regular.ttf"
# Destination ranges in Nerd Fonts 3.5.1's font-patcher. These are Seti UI,
# Devicons, Powerline, Powerline Extra, Pomicons, Font Awesome and Extension,
# IEC Power Symbols, Material Design Icons, Octicons and Codicons. Weather
# Icons U+E300..U+E3EB and Font Logos U+F300..U+F385 are deliberately absent.
APPROVED_RANGES = (
    (0xE000, 0xE00A),
    (0xE0A0, 0xE0D7),
    (0xE200, 0xE2A9),
    (0xE5FA, 0xE6FF),
    (0xE700, 0xE958),
    (0xEA60, 0xEC84),
    (0xED00, 0xF2FF),
    (0xF400, 0xF533),
    (0xF0001, 0xF1AF0),
    (0x23FB, 0x23FE),
    (0x2630, 0x2630),
    (0x2665, 0x2665),
    (0x26A1, 0x26A1),
    (0x2B58, 0x2B58),
)
# The pinned base contains these eight PUA mappings. U+F6C5 is a remapped logo,
# not a Powerline symbol, and is intentionally not redistributed.
EXPECTED_BASE_PUA = frozenset(
    {0xE0A0, 0xE0A1, 0xE0A2, 0xE0B0, 0xE0B1, 0xE0B2, 0xE0B3, 0xF6C5}
)
LEGACY_POWERLINE = frozenset(
    {0xE0A0, 0xE0A1, 0xE0A2, 0xE0B0, 0xE0B1, 0xE0B2, 0xE0B3}
)
DENIED_FONT_LOGOS_RANGE = frozenset(range(0xF300, 0xF386))
DENIED_WEATHER_RANGE = frozenset(range(0xE300, 0xE3EC))


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def download(url: str, destination: Path, expected: str) -> None:
    partial = destination.with_suffix(destination.suffix + ".partial")
    try:
        with (
            urllib.request.urlopen(url, timeout=120) as response,
            partial.open("wb") as out,
        ):
            while block := response.read(1024 * 1024):
                out.write(block)
        if digest(partial) != expected:
            raise ValueError(f"Downloaded font hash mismatch: {url}")
        partial.replace(destination)
    finally:
        partial.unlink(missing_ok=True)


def codepoints(font) -> set[int]:
    return {
        point
        for table in font["cmap"].tables
        if table.isUnicode()
        for point in table.cmap
    }


def glyph_closure(font, points: set[int]) -> set[str]:
    """Return mapped glyphs and recursively referenced TrueType components."""
    cmap = font.getBestCmap()
    glyphs = font["glyf"]
    retained = {cmap[point] for point in points}
    pending = list(retained)
    while pending:
        glyph = glyphs[pending.pop()]
        if not glyph.isComposite():
            continue
        for component in glyph.components:
            if component.glyphName not in retained:
                retained.add(component.glyphName)
                pending.append(component.glyphName)
    return retained


def rename(font) -> None:
    values = {
        1: FAMILY,
        2: "Regular",
        3: f"{FAMILY} 3.5.1-pi.1",
        4: f"{FAMILY} Regular",
        5: "Version 3.5.1-pi.1",
        6: POSTSCRIPT,
        16: FAMILY,
        17: "Regular",
        21: FAMILY,
        22: "Regular",
    }
    names = font["name"]
    for record in list(names.names):
        if record.nameID in values:
            names.removeNames(
                record.nameID,
                platformID=record.platformID,
                platEncID=record.platEncID,
                langID=record.langID,
            )
    for name_id, value in values.items():
        names.setName(value, name_id, 3, 1, 0x409)
        names.setName(value, name_id, 1, 0, 0)


def generate(destination: Path) -> None:
    try:
        import fontTools
        from fontTools import subset
        from fontTools.ttLib import TTFont
    except ImportError as error:
        raise SystemExit(
            f"Install the generator with: python3 -m pip install fonttools=={FONTTOOLS_VERSION}"
        ) from error
    if fontTools.__version__ != FONTTOOLS_VERSION:
        raise SystemExit(
            f"fonttools {FONTTOOLS_VERSION} required, found {fontTools.__version__}"
        )
    with tempfile.TemporaryDirectory(prefix="pi-terminal-symbols-") as temporary:
        temporary = Path(temporary)
        patched_path = temporary / "patched.ttf"
        logos_path = temporary / "font-logos.ttf"
        base_path = temporary / "base.ttf"
        fae_license_path = temporary / "font-awesome-extension-LICENCE"
        download(PATCHED_URL, patched_path, PATCHED_SHA256)
        download(FONT_LOGOS_URL, logos_path, FONT_LOGOS_SHA256)
        download(BASE_URL, base_path, BASE_SHA256)
        # Pin the exact upstream MIT grant copied into the combined notice.
        download(FAE_LICENSE_URL, fae_license_path, FAE_LICENSE_SHA256)

        patched = TTFont(patched_path, recalcTimestamp=False)
        base = TTFont(base_path, recalcTimestamp=False)
        logos = TTFont(logos_path, recalcTimestamp=False)
        patched_points = codepoints(patched)
        base_points = codepoints(base)
        logo_cmap = logos.getBestCmap()
        logo_points = set(logo_cmap) - {0x20}
        denied_logo_points = logo_points | DENIED_FONT_LOGOS_RANGE
        # Font-patcher renames Font Logos glyphs, so deny both the source names
        # and the names assigned to the same mappings in the patched input.
        logo_names = {logo_cmap[point] for point in logo_points} | {
            patched.getBestCmap()[point]
            for point in logo_points
            if point in patched.getBestCmap()
        }
        base_pua = {
            point
            for point in base_points
            if 0xE000 <= point <= 0xF8FF or 0xF0000 <= point <= 0x10FFFD
        }
        if base_pua != EXPECTED_BASE_PUA:
            raise ValueError(
                f"Pinned Meslo base PUA changed: {sorted(base_pua)} != "
                f"{sorted(EXPECTED_BASE_PUA)}"
            )
        if not LEGACY_POWERLINE <= patched_points:
            raise ValueError("Patched input is missing an approved legacy Powerline mapping")
        reviewed_symbols = {
            point
            for point in patched_points
            if any(start <= point <= end for start, end in APPROVED_RANGES)
        }
        allowed = reviewed_symbols | LEGACY_POWERLINE | {0x20, ord("m")}
        denied_points = denied_logo_points | DENIED_WEATHER_RANGE | {0xF6C5}
        if allowed & denied_points:
            raise ValueError("A denied mapping entered the approved symbol set")
        retained_names = glyph_closure(patched, allowed)
        denied_names = retained_names & logo_names
        if denied_names:
            raise ValueError(f"Font Logos glyphs entered the approved set: {denied_names}")
        # Deny the glyphs and TrueType components behind the other
        # licensing-driven exclusions too, not merely their cmap entries.
        excluded_outline_names = glyph_closure(
            patched, (DENIED_WEATHER_RANGE | {0xF6C5}) & patched_points
        )
        denied_names = retained_names & excluded_outline_names
        if denied_names:
            raise ValueError(f"Excluded glyph outlines entered the approved set: {denied_names}")

        options = subset.Options()
        options.recalc_timestamp = False
        options.canonical_order = True
        options.name_IDs = ["*"]
        options.name_languages = ["*"]
        options.notdef_outline = True
        options.recommended_glyphs = True
        subsetter = subset.Subsetter(options=options)
        subsetter.populate(unicodes=allowed)
        subsetter.subset(patched)
        subset_points = codepoints(patched)
        subset_names = set(patched.getGlyphOrder())
        denied_names = subset_names & logo_names
        if subset_points & denied_points:
            raise ValueError("Subset font retained denied mappings")
        if denied_names:
            raise ValueError(f"Subset font retained Font Logos glyphs: {denied_names}")
        denied_names = subset_names & excluded_outline_names
        if denied_names:
            raise ValueError(f"Subset font retained excluded glyph outlines: {denied_names}")
        rename(patched)
        if "DSIG" in patched:
            del patched["DSIG"]
        destination.parent.mkdir(parents=True, exist_ok=True)
        patched.save(destination, reorderTables=True)

        result = TTFont(destination, recalcTimestamp=False)
        result_points = codepoints(result)
        if result_points != allowed:
            raise ValueError("Generated font cmap differs from the approved set")
        if result_points & denied_points:
            raise ValueError("Generated font contains denied mappings")
        widths = {
            result["hmtx"].metrics[glyph_name][0]
            for glyph_name in result.getGlyphOrder()
        }
        if len(widths) != 1:
            raise ValueError(f"Generated terminal symbols are not mono-width: {widths}")
        if destination.stat().st_size > 3 * 1024 * 1024:
            raise ValueError("Generated terminal fallback exceeds its 3 MiB budget")
        print(
            f"Generated {destination}: {len(result_points)} mappings, "
            f"{destination.stat().st_size} bytes, sha256={digest(destination)}"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.check:
        with tempfile.TemporaryDirectory(
            prefix="pi-terminal-symbols-check-"
        ) as temporary:
            generated = Path(temporary) / args.output.name
            generate(generated)
            if (
                not args.output.is_file()
                or generated.read_bytes() != args.output.read_bytes()
            ):
                raise SystemExit(f"Generated terminal fallback is stale: {args.output}")
            print(f"Verified generated terminal fallback: {args.output}")
    else:
        generate(args.output)


if __name__ == "__main__":
    main()
