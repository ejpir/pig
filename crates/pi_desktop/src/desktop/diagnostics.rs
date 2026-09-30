//! Per-session, bounded diagnostics. Never collect RPC bodies or environment values.
use super::panels::{DocumentView, heading, note};
use super::*;
use std::collections::VecDeque;

const MAX_BYTES: usize = 48 * 1024;
const MAX_ENTRIES: usize = 128;
#[derive(Default)]
pub struct DiagnosticLog {
    entries: VecDeque<String>,
    bytes: usize,
    dropped: bool,
}
impl DiagnosticLog {
    pub fn push(&mut self, kind: &str, detail: &str) {
        let safe = redact(detail);
        let mut start = safe.len().saturating_sub(8192);
        while !safe.is_char_boundary(start) {
            start += 1;
        }
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let timestamp = format!(
            "{:02}:{:02}:{:02}.{:03}Z",
            millis / 3_600_000 % 24,
            millis / 60_000 % 60,
            millis / 1_000 % 60,
            millis % 1_000
        );
        let entry = format!(
            "{} [{kind}] {}{}\n",
            timestamp,
            if start > 0 { "[tail only] " } else { "" },
            &safe[start..]
        );
        self.bytes += entry.len();
        self.entries.push_back(entry);
        while self.bytes > MAX_BYTES || self.entries.len() > MAX_ENTRIES {
            self.bytes -= self.entries.pop_front().unwrap().len();
            self.dropped = true;
        }
    }
    pub fn text(&self) -> String {
        let prefix = if self.dropped {
            "[Older diagnostics discarded by the retention limit]\n"
        } else {
            ""
        };
        format!(
            "{prefix}{}",
            self.entries.iter().rev().cloned().collect::<String>()
        )
    }
}
fn redact(detail: &str) -> String {
    detail
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "authorization:",
                "bearer ",
                "api_key=",
                "api_key\":",
                "apikey\":",
                "access_token=",
                "access_token\":",
                "password=",
                "client_secret=",
            ]
            .iter()
            .any(|key| lower.contains(key))
            {
                "[Credential-like line redacted]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct DiagnosticsView {
    controller: Entity<session::SessionController>,
    document: Entity<DocumentView>,
    focus: gpui::FocusHandle,
    dirty: bool,
    _subscription: gpui::Subscription,
}
impl DiagnosticsView {
    pub fn new(controller: Entity<session::SessionController>, cx: &mut Context<Self>) -> Self {
        let document = cx.new(|cx| DocumentView::new(cx).compact().wrapped());
        let subscription = cx.subscribe(&controller, |this, _, event, cx| {
            if matches!(event, session::SessionEvent::Changed(c) if c.intersects(session::Changes::DIAGNOSTICS | session::Changes::STATUS | session::Changes::METADATA)) {
                this.dirty = true;
                cx.notify();
            }
        });
        let mut this = Self {
            controller,
            document,
            focus: cx.focus_handle(),
            dirty: false,
            _subscription: subscription,
        };
        this.refresh(cx);
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let text = self.controller.read(cx).diagnostics();
        self.document
            .update(cx, |doc, cx| doc.set(text, Some("text"), cx));
        cx.notify();
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }
}
impl Render for DiagnosticsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.dirty) {
            self.refresh(cx);
        }
        let theme = theme(cx);
        v_flex().id("session-diagnostics").debug_selector(||"session-diagnostics".into())
            .track_focus(&self.focus).p(px(20.)).gap(px(10.)).w_full().min_w_0()
            .child(heading("Diagnostics", "This session · newest entries first · UTC", theme))
            .child(note("Request bodies and environment values are not logged. Captured stderr/errors may contain sensitive data; review before sharing.", theme))
            .child(h_flex().gap(px(8.))
                .child(button("copy-diagnostics","Copy diagnostics",theme).debug_selector(||"copy-diagnostics".into())
                    .on_click(cx.listener(|this,_,_,cx|cx.write_to_clipboard(ClipboardItem::new_string(this.controller.read(cx).diagnostics())))))
                .child(button("clear-diagnostics","Clear",theme).on_click(cx.listener(|this,_,_,cx|this.controller.update(cx,|c,cx|c.clear_diagnostics(cx))))))
            .child(note("In-memory log: up to 128 entries / 48 KiB. Process exit includes the captured stderr tail (up to 8 KiB), not the complete process output.", theme))
            .child(self.document.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_are_bounded_utf8_safe_and_redact_common_credentials() {
        let mut log = DiagnosticLog::default();
        log.push(
            "stderr",
            "Authorization: Bearer secret\nAPI_KEY=hidden\nError: ECONNRESET",
        );
        assert!(!log.text().contains("secret"));
        assert!(!log.text().contains("hidden"));
        assert!(log.text().contains("ECONNRESET"));
        for _ in 0..160 {
            log.push("stderr", &"é".repeat(5000));
        }
        assert!(log.text().len() < MAX_BYTES + 100);
        assert!(log.text().contains("Older diagnostics discarded"));
    }
}
