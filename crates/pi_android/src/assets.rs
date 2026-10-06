//! Icons and fonts, built into the library: the phone app has no files of its own.

use gpui::{App, AssetSource, SharedString};
use std::borrow::Cow;

pub struct Assets;

macro_rules! icons {
    ($($name:ident),* $(,)?) => {
        &[$((
            stringify!($name),
            concat!("icons/", stringify!($name), ".svg"),
            include_bytes!(concat!("../assets/icons/", stringify!($name), ".svg")) as &[u8],
        )),*]
    };
}

/// Stroke icons on a 24-unit grid, from design/android.
const ICONS: &[(&str, &str, &[u8])] = icons![
    back, search, plus, folder, chat, file, chev_r, chev_d, chev_u, check, x, term, pencil, eye,
    stop, clock, alert, clip, slash, send, spark, dots, settings, info, shield, computer, server,
    key, copy, image, bell, palette, layers, diff, queue, sun, moon, hand, pi, menu, trash, scan,
    restore, chev_l, open, refresh, code, wrap,
];

/// The asset path of a bundled icon.
pub fn icon_path(name: &str) -> &'static str {
    ICONS
        .iter()
        .find(|(icon, _, _)| *icon == name)
        .map_or("icons/missing.svg", |(_, path, _)| path)
}

const PLEX_SANS: &[u8] = include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf");

/// The font GPUI draws text in SVGs with, such as diagram labels: Android has
/// no system fonts it can find, so without it the labels are left out.
const SVG_FONT: &str = "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf";

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if path == SVG_FONT {
            return Ok(Some(Cow::Borrowed(PLEX_SANS)));
        }
        Ok(ICONS
            .iter()
            .find(|(_, icon, _)| *icon == path)
            .map(|(_, _, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, _path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(_, path, _)| (*path).into()).collect())
    }
}

/// IBM Plex Sans and Commit Mono, as on the desktop.
pub fn load_fonts(cx: &App) -> anyhow::Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(PLEX_SANS),
        Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/IBMPlexSans-SemiBold.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/IBMPlexSans-Italic.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/CommitMono-Regular.otf"
        )),
    ])
}

#[cfg(test)]
mod tests {
    use super::ICONS;

    /// Every icon and glyph argument in the views names a bundled icon.
    #[test]
    fn every_icon_used_is_bundled() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![source.clone()];
        let mut checked = 0;
        while let Some(path) = files.pop() {
            if path.is_dir() {
                files.extend(
                    std::fs::read_dir(&path)
                        .unwrap()
                        .map(|entry| entry.unwrap().path()),
                );
                continue;
            }
            // Views only: elsewhere `Some("…")` is not a glyph.
            let view = path.components().any(|part| part.as_os_str() == "screens")
                || ["app.rs", "composer.rs", "ui.rs"]
                    .iter()
                    .any(|file| path.ends_with(file));
            if !view {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for marker in ["icon(\"", "Some(\"", "tap(\""] {
                for (index, _) in text.match_indices(marker) {
                    let rest = &text[index + marker.len()..];
                    let name = &rest[..rest.find('"').unwrap()];
                    let glyph_like = !name.is_empty()
                        && name.len() <= 10
                        && name.chars().all(|c| c.is_ascii_lowercase() || c == '_');
                    if marker == "icon(\"" || (marker == "Some(\"" && glyph_like) {
                        checked += 1;
                        assert!(
                            ICONS.iter().any(|(icon, _, _)| *icon == name),
                            "{} uses the icon {name}, which is not bundled",
                            path.display()
                        );
                    }
                }
            }
        }
        assert!(checked > 20);
    }
}
