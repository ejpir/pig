use gpui::{App, Global, Hsla, rgb};

pub const SANS: &str = "IBM Plex Sans";
pub const MONO: &str = "CommitMonoV143";
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub const SERIF: &str = "Georgia";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const SERIF: &str = "DejaVu Serif";

/// Semantic surfaces from design/pi_study_common.py. Avoid substituting raised/bar
/// for every control: Moonstone's white composer and warm chrome are intentional.
#[derive(Clone, Copy)]
pub struct Theme {
    pub light: bool,
    pub deep: Hsla,
    pub work_code: Hsla,
    pub canvas: Hsla,
    pub panel: Hsla,
    pub bar: Hsla,
    pub raised: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub edge: Hsla,
    pub status: Hsla,
    pub hover: Hsla,
    pub chip: Hsla,
    pub chip_line: Hsla,
    pub composer: Hsla,
    pub focus: Hsla,
    pub track: Hsla,
    pub queue: Hsla,
    pub queue_line: Hsla,
    pub danger: Hsla,
    pub danger_line: Hsla,
    pub text: Hsla,
    pub secondary: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    /// Accent-filled buttons under the pointer: the accent, slightly lighter.
    pub accent_hover: Hsla,
    pub selected: Hsla,
    pub on_accent: Hsla,
    pub steel: Hsla,
    pub amber: Hsla,
    pub coral: Hsla,
    pub green: Hsla,
    /// Activity hues from design/visual-workflow: search and change steps.
    /// Reading uses `steel`, running uses `accent`, waiting uses `amber`.
    pub violet: Hsla,
    pub orange: Hsla,
    pub added: Hsla,
    pub removed: Hsla,
    pub code: Hsla,
    pub keyword: Hsla,
    pub string: Hsla,
    pub plain: Hsla,
}

impl Global for Theme {}

impl Theme {
    pub fn new(light: bool) -> Self {
        let color = |dark, day| rgb(if light { day } else { dark }).into();
        Self {
            light,
            deep: color(0x0d1116, 0xeef0f2),
            work_code: color(0x1b232e, 0xf3f1ed),
            canvas: color(0x161d27, 0xfaf9f7),
            panel: color(0x1a212b, 0xf0ede8),
            bar: color(0x1f2630, 0xebe7e2),
            raised: color(0x252f3d, 0xe4ded8),
            line: color(0x363d46, 0xddd7d0),
            line_strong: color(0x424954, 0xcbc3bb),
            edge: color(0x0d1116, 0xcbc3bb),
            status: color(0x1f2630, 0xebe7e2),
            hover: color(0x222a35, 0xe8e3de),
            chip: color(0x29313c, 0xffffff),
            chip_line: color(0x3a434f, 0xd3ccc5),
            composer: color(0x202731, 0xffffff),
            focus: color(0x424954, 0xcbc3bb),
            track: color(0x252d38, 0xe3ddd7),
            queue: color(0x292b2d, 0xf8f3e9),
            queue_line: color(0x454137, 0xf2e7d5),
            danger: color(0x343138, 0xf9eeec),
            danger_line: color(0x684b4d, 0xe9c5be),
            text: color(0xebe7e4, 0x252f3d),
            secondary: color(0xd5d8db, 0x3e4753),
            muted: color(0x9ca2aa, 0x665f59),
            faint: color(0x9ca2aa, 0x665f59),
            accent: color(0x8caecb, 0x4b607c),
            accent_hover: color(0x7eadd5, 0x5b7190),
            selected: color(0x2d3239, 0xe3dfd9),
            on_accent: color(0x111820, 0xffffff),
            steel: color(0x4d9abf, 0x2f7fa8),
            amber: color(0xe5bd73, 0x8b631f),
            coral: color(0xefa08c, 0xa44835),
            green: color(0x80bf95, 0x2e7950),
            violet: color(0xa596e0, 0x6b5ca5),
            orange: color(0xe3a35c, 0xb0661a),
            added: color(0x243b30, 0xe4eee5),
            removed: color(0x422e30, 0xf2e4df),
            code: color(0x9cc2e0, 0x2f6f9e),
            keyword: color(0x8fb6dc, 0x2f5f9e),
            string: color(0xe1b06e, 0x9a5b17),
            plain: color(0xb9bec5, 0x3f4854),
        }
    }

    /// Selected text, in inputs and in the transcript.
    pub fn selection(self) -> Hsla {
        self.accent.opacity(if self.light { 0.22 } else { 0.32 })
    }

    /// A hue's quiet background, for pills and selected steps.
    pub fn tint(self, hue: Hsla) -> Hsla {
        hue.opacity(if self.light { 0.11 } else { 0.16 })
    }

    pub fn thinking(self, level: &str) -> Hsla {
        let index = match level {
            "minimal" | "min" => 1,
            "low" => 2,
            "medium" | "med" => 3,
            "high" => 4,
            "xhigh" => 5,
            "max" => 6,
            _ => 0,
        };
        let ramp = if self.light {
            [
                0x9aa0a8, 0x4b607c, 0x2f7fa8, 0x7d7e45, 0xc28a1c, 0xc96a58, 0xc0442a,
            ]
        } else {
            [
                0x5a616b, 0x4b607c, 0x4d9abf, 0xa3a473, 0xf1be58, 0xf09082, 0xe8704f,
            ]
        };
        rgb(ramp[index]).into()
    }
}

pub fn theme(cx: &App) -> Theme {
    *cx.global::<Theme>()
}
