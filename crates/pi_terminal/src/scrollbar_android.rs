use crate::terminal::Terminal;

/// Android uses touch/terminal wheel scrolling directly and does not link
/// Zed's desktop `ui` scrollbar stack.
pub struct TerminalScrollHandle {
    pub(crate) future_display_offset: Option<usize>,
}

impl TerminalScrollHandle {
    pub fn new(_: &Terminal) -> Self {
        Self {
            future_display_offset: None,
        }
    }

    pub fn update(&mut self, _: &Terminal) {}
}
