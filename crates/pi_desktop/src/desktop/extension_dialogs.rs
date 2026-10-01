//! Standard Pi confirm/select requests. Decisions belong to the extension, not an OS sandbox.
use super::*;
use serde_json::{Value, json};

impl super::session_view::SessionView {
    pub(super) fn show_extension_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cancel_extension && window.has_active_prompt() {
            self.cancel_extension = false;
            window.dispatch_action(Box::new(super::prompts::CancelPrompt), cx);
        }
        if window.has_active_prompt() || self.active_extension.is_some() {
            return;
        }
        let Some(request) = self.extension_dialogs.pop_front() else {
            return;
        };
        let id = text(&request, "id");
        let choices: Vec<String> = request["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let confirm = request["method"] == "confirm";
        if id.is_empty()
            || text(&request, "title").len() > 4096
            || text(&request, "message").len() > 16384
            || choices.len() > 16
            || choices.iter().any(|choice| choice.len() > 1024)
            || !confirm && choices.is_empty()
        {
            self.controller.update(cx, |controller, cx| {
                controller.answer_extension(
                    json!({"type":"extension_ui_response", "id":id, "cancelled":true}),
                    cx,
                );
                controller.notice(
                    "Extension request cancelled: its dialog cannot be displayed safely.",
                    cx,
                );
            });
            cx.notify();
            return;
        }
        let mut buttons = vec![gpui::PromptButton::cancel("Cancel")];
        if confirm {
            buttons.push(gpui::PromptButton::ok("Confirm"));
        } else {
            buttons.extend(choices.iter().cloned().map(gpui::PromptButton::new));
        }
        let detail = format!(
            "{}\n\nPi extension request. The extension owns this decision; Pi does not enforce per-extension filesystem or network permissions.",
            text(&request, "message")
        );
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &text(&request, "title"),
            Some(&detail),
            &buttons,
            cx,
        );
        let controller = self.controller.clone();
        let session_id = controller.read(cx).model().state.session_id.clone();
        self.active_extension = Some(id.clone());
        if let Some(timeout) = request["timeout"].as_u64().filter(|timeout| *timeout > 0) {
            let timeout_id = id.clone();
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(timeout))
                    .await;
                this.update_in(cx, |this, window, cx| {
                    if this.active_extension.as_deref() == Some(timeout_id.as_str())
                        && window.has_active_prompt()
                    {
                        window.dispatch_action(Box::new(super::prompts::CancelPrompt), cx);
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.spawn(async move |this, cx| {
            let answer = answer.await.ok().unwrap_or(0);
            // Respond to the owning controller/request, never whichever tab is now selected.
            controller.update(cx, |controller, cx| {
                let same =
                    session_id.is_none() || controller.model().state.session_id == session_id;
                let mut response = json!({"type":"extension_ui_response", "id":id});
                if answer == 0 || !same || !controller.is_connected() {
                    response["cancelled"] = true.into();
                } else if confirm {
                    response["confirmed"] = true.into();
                } else if let Some(value) = choices.get(answer - 1) {
                    response["value"] = value.clone().into();
                } else {
                    response["cancelled"] = true.into();
                }
                controller.answer_extension(response, cx);
            });
            this.update(cx, |this, cx| {
                this.active_extension = None;
                this.cancel_extension = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
