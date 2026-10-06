//! Moonstone (light) and Evening (dark), from Pi Desktop's `theme.rs`, with the
//! phone's stage hues from design/android.

use gpui::{App, Global, Hsla, Rgba, rgb, rgba};
use serde::{Deserialize, Serialize};

pub const SANS: &str = "IBM Plex Sans";
pub const MONO: &str = "CommitMonoV143";
/// Headlines are a serif italic, as on the desktop; Android ships Noto Serif.
#[cfg(target_os = "android")]
pub const SERIF: &str = "Noto Serif";
/// The desktop preview's: Georgia, as in the mocks, or DejaVu Serif on Linux.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub const SERIF: &str = "Georgia";
#[cfg(not(any(target_os = "android", target_os = "macos", target_os = "windows")))]
pub const SERIF: &str = "DejaVu Serif";

/// The theme setting; `System` follows the phone's dark mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Evening,
    Moonstone,
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub canvas: Hsla,
    pub panel: Hsla,
    pub raised: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub selected: Hsla,
    pub composer: Hsla,
    pub chip: Hsla,
    pub text: Hsla,
    pub secondary: Hsla,
    pub muted: Hsla,
    /// Placeholders, stages ahead and quiet chevrons.
    pub faint: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub amber: Hsla,
    pub coral: Hsla,
    pub green: Hsla,
    pub added: Hsla,
    pub removed: Hsla,
    pub keyword: Hsla,
    pub string: Hsla,
    pub plain: Hsla,
    /// Stage hues: reading, changing, checking and waiting for you.
    pub read: Hsla,
    pub edit: Hsla,
    pub check: Hsla,
    pub wait: Hsla,
    pub scrim: Hsla,
    pub shadow: Hsla,
}

impl Global for Theme {}

impl Theme {
    pub fn new(dark: bool) -> Self {
        let color =
            |dark_color, light_color| rgb(if dark { dark_color } else { light_color }).into();
        let alpha = |dark_color, light_color| -> Hsla {
            let value: Rgba = rgba(if dark { dark_color } else { light_color });
            value.into()
        };
        Self {
            dark,
            canvas: color(0x161d27, 0xfaf9f7),
            panel: color(0x1a212b, 0xf0ede8),
            raised: color(0x252f3d, 0xe4ded8),
            line: color(0x363d46, 0xddd7d0),
            line_strong: color(0x424954, 0xcbc3bb),
            selected: color(0x2d3239, 0xe3dfd9),
            composer: color(0x202731, 0xffffff),
            chip: color(0x29313c, 0xffffff),
            text: color(0xebe7e4, 0x252f3d),
            secondary: color(0xd5d8db, 0x3e4753),
            muted: color(0x9ca2aa, 0x665f59),
            faint: color(0x7d848d, 0x8a837c),
            accent: color(0x8caecb, 0x4b607c),
            on_accent: color(0x111820, 0xffffff),
            amber: color(0xe5bd73, 0x8b631f),
            coral: color(0xefa08c, 0xa44835),
            green: color(0x80bf95, 0x2e7950),
            added: color(0x243b30, 0xe4eee5),
            removed: color(0x422e30, 0xf2e4df),
            keyword: color(0x8fb6dc, 0x2f5f9e),
            string: color(0xe1b06e, 0x9a5b17),
            plain: color(0xb9bec5, 0x3f4854),
            read: color(0x5fa9d0, 0x2f7fa8),
            edit: color(0xe3a35c, 0xb0661a),
            check: color(0x80bf95, 0x2e7950),
            wait: color(0xe5bd73, 0xa57514),
            scrim: alpha(0x00000080, 0x252f3d52),
            shadow: alpha(0x00000066, 0x252f3d24),
        }
    }

    /// A hue's quiet background, for tiles, badges and status rings.
    pub fn tint(self, hue: Hsla) -> Hsla {
        hue.opacity(if self.dark { 0.16 } else { 0.14 })
    }

    /// 0xRRGGBB, for Android's notification accent.
    pub fn accent_rgb(self) -> u32 {
        let rgba = self.accent.to_rgb();
        let channel = |value: f32| (value * 255.).round() as u32;
        channel(rgba.r) << 16 | channel(rgba.g) << 8 | channel(rgba.b)
    }

    pub fn syntax_palette(self) -> pi_markdown::SyntaxPalette {
        pi_markdown::SyntaxPalette {
            text: self.text,
            plain: self.plain,
            muted: self.muted,
            faint: self.faint,
            accent: self.accent,
            steel: self.read,
            amber: self.amber,
            coral: self.coral,
            green: self.green,
            keyword: self.keyword,
            string: self.string,
            added: self.added,
            removed: self.removed,
            selected: self.selected,
        }
    }
}

pub fn theme(cx: &App) -> Theme {
    *cx.global::<Theme>()
}

#[cfg(test)]
pub fn install_for_tests(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_global(Theme::new(false)));
}
