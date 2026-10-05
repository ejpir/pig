//! One honest run-state surface, reused with the composer in Thread and Changes.
use super::*;

impl ComposerView {
    pub(super) fn work_status(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let controller = self.controller.read(cx);
        let model = controller.model();
        let last_reply = model
            .messages
            .iter()
            .rev()
            .find(|m| m["role"] == "assistant");
        let failed_tools = super::super::session::latest_failed_tools(model).len();
        if self.contextual_issue && failed_tools > 0 {
            return div()
                .debug_selector(|| "contextual-work-status".into())
                .into_any_element();
        }
        let completed_check = model
            .tools
            .iter()
            .rev()
            .find(|tool| tool.finished && tool.name == "bash" && tool.target().contains("check"))
            .and_then(|tool| {
                tool.output
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
            })
            .map(str::trim);
        let (title, detail, working, warning) = if controller.connecting() {
            ("Connecting", "Preparing the session", true, false)
        } else if !controller.is_connected() {
            (
                "Disconnected",
                if controller.is_remote() {
                    "Remote work may still be running"
                } else {
                    "Draft retained"
                },
                false,
                true,
            )
        } else if controller.bootstrap_failed() {
            ("Setup failed", "See session diagnostics", false, true)
        } else if self.waiting {
            (
                "Waiting for you",
                "Answer or cancel the open question",
                false,
                false,
            )
        } else if controller.working() {
            let detail = match model.run {
                pi_core::session::RunState::Compacting => "Compacting context",
                pi_core::session::RunState::Retrying => "Retrying the request",
                _ if model.shell_running() => "Running shell command",
                _ if !model.busy() => "Starting the run",
                _ if model.tools.iter().any(|tool| {
                    !tool.finished && tool.name == "bash" && tool.target().contains("check")
                }) =>
                {
                    "Checking the workspace"
                }
                _ => "Working in this project",
            };
            ("Working", detail, true, false)
        } else if model.error.is_some() || last_reply.is_some_and(|m| m["stopReason"] == "error") {
            ("Needs attention", "See the error above", false, true)
        } else if last_reply.is_some_and(|m| m["stopReason"] == "aborted") {
            (
                "Stopped",
                "You can continue with a new prompt",
                false,
                false,
            )
        } else if model.tools.iter().any(|tool| !tool.finished) {
            (
                "Incomplete",
                "Some tool calls have no reported result",
                false,
                true,
            )
        } else if failed_tools > 0 {
            (
                if failed_tools == 1 {
                    "Completed with 1 issue"
                } else {
                    "Completed with issues"
                },
                if failed_tools == 1 {
                    "One tool call failed"
                } else {
                    "Some tool calls failed"
                },
                false,
                true,
            )
        } else if last_reply.is_some() && model.messages.last().is_none_or(|m| m["role"] != "user")
        {
            (
                "Finished",
                completed_check.unwrap_or("Run completed"),
                false,
                false,
            )
        } else {
            (
                "Ready",
                "Your draft is not sent until you submit",
                false,
                false,
            )
        };
        let detail = controller
            .remote_target()
            .map(|target| format!("SSH · {} · {detail}", target.host))
            .unwrap_or_else(|| detail.to_owned());
        h_flex()
            .debug_selector(|| "work-status".into())
            .mx(WORK_GUTTER)
            .mb(px(8.))
            .min_h(px(32.))
            .gap(px(10.))
            .text_size(px(13.))
            .child(if working {
                status_dot("composer-working", theme.steel, true)
            } else if self.waiting {
                status_dot("composer-waiting", theme.amber, true)
            } else if title == "Finished" {
                icon("check", theme.green).size(px(16.)).into_any_element()
            } else if warning {
                icon("warning", theme.coral)
                    .size(px(14.))
                    .into_any_element()
            } else {
                div()
                    .size(px(6.))
                    .rounded_full()
                    .bg(theme.muted)
                    .into_any_element()
            })
            .child(
                div()
                    .debug_selector(|| "work-status-label".into())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.muted)
                    .child(detail),
            )
            .when(
                controller.is_connected()
                    && (controller.working()
                        || !model.follow_up.is_empty()
                        || !model.steering.is_empty()),
                |v| {
                    v.child(
                        work_button("stop", "Stop", theme)
                            .debug_selector(|| "stop".into())
                            .tooltip(ui::Tooltip::text(
                                "Stop the run and restore queued drafts; keep the current draft",
                            ))
                            .on_click(cx.listener(|this, _, _, cx| this.clear_queue(true, cx))),
                    )
                },
            )
            .when(
                controller.is_remote() && !controller.is_connected() && !controller.connecting(),
                |row| {
                    row.child(work_button("reconnect-ssh", "Reconnect", theme).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.controller
                                .update(cx, |controller, cx| controller.connect_remote(cx))
                        }),
                    ))
                },
            )
            .into_any_element()
    }
}
