//! The device's fonts, memory-mapped.
//!
//! Android has no font-config, and fontdb does not look in `/system/fonts`.
//! Mapping every file costs address space, not memory: fontdb reads each
//! face's tables once, and glyph outlines are paged in only when drawn, which
//! keeps CJK and emoji fallback available without loading them.

use memmap2::Mmap;
use std::{borrow::Cow, fs::File, path::Path};

const FONT_DIRECTORIES: [&str; 2] = ["/system/fonts", "/product/fonts"];

pub(crate) fn system_fonts() -> Vec<Cow<'static, [u8]>> {
    let mut fonts = Vec::new();
    for directory in FONT_DIRECTORIES {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if !is_font(&path) {
                continue;
            }
            match File::open(&path).and_then(|file| unsafe { Mmap::map(&file) }) {
                // The text system keeps fonts for the process lifetime.
                Ok(map) => fonts.push(Cow::Borrowed(&Box::leak(Box::new(map))[..])),
                Err(error) => log::warn!("Skipping font {}: {error}", path.display()),
            }
        }
    }
    fonts
}

fn is_font(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("ttf" | "otf" | "ttc")
    )
}
