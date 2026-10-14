use gpui::Hsla;
pub const SANS: &str = "IBM Plex Sans";
pub const MONO: &str = "CommitMonoV143";
#[derive(Clone, Copy)]
pub struct Theme {
    pub canvas: Hsla,
    pub panel: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub selected: Hsla,
    pub text: Hsla,
    pub secondary: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub coral: Hsla,
    pub tint: Hsla,
}
