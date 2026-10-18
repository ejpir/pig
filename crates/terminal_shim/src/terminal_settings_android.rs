//! Minimal terminal preferences for the Android external-stream build.
//!
//! Android does not load Zed's editor settings stack (and therefore does not
//! need its filesystem/trash and VS Code import machinery). The owning app sets
//! this global before constructing a terminal.

use gpui::{App, Global};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AlternateScroll {
    Off,
    #[default]
    On,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
    Hollow,
}

#[derive(Clone)]
pub struct TerminalSettings {
    pub option_as_meta: bool,
    pub copy_on_select: bool,
    pub keep_selection_on_copy: bool,
    pub open_links_in_mouse_mode: bool,
    pub max_scroll_history_lines: Option<usize>,
    pub scroll_multiplier: f32,
    pub minimum_contrast: f32,
    pub path_hyperlink_regexes: Vec<String>,
    pub path_hyperlink_timeout_ms: u64,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            option_as_meta: false,
            copy_on_select: false,
            keep_selection_on_copy: false,
            open_links_in_mouse_mode: false,
            max_scroll_history_lines: Some(10_000),
            scroll_multiplier: 1.0,
            minimum_contrast: 0.0,
            path_hyperlink_regexes: Vec::new(),
            path_hyperlink_timeout_ms: 50,
        }
    }
}

impl Global for TerminalSettings {}

impl TerminalSettings {
    pub fn get_global(cx: &App) -> &Self {
        cx.global::<Self>()
    }
}
