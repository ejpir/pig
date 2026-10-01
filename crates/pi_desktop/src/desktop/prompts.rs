//! A real GPUI prompt owner: wrapped warnings and an explicit cancel keyboard
//! boundary. GPUI's minimal fallback clips details and has no Escape handling.
use super::*;
use gpui::{EventEmitter, FocusHandle, PromptButton, PromptResponse};

gpui::actions!(
    desktop_prompts,
    [CancelPrompt, ConfirmPrompt, NextPrompt, PreviousPrompt]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CancelPrompt, Some("PiPrompt")),
        KeyBinding::new("enter", ConfirmPrompt, Some("PiPrompt")),
        KeyBinding::new("tab", NextPrompt, Some("PiPrompt")),
        KeyBinding::new("right", NextPrompt, Some("PiPrompt")),
        KeyBinding::new("shift-tab", PreviousPrompt, Some("PiPrompt")),
        KeyBinding::new("left", PreviousPrompt, Some("PiPrompt")),
    ]);
    cx.set_prompt_builder(|_, message, detail, buttons, handle, window, cx| {
        let handle = match super::parallel::build(message, handle, window, cx) {
            Ok(prompt) => return prompt,
            Err(handle) => handle,
        };
        let handle = match super::new_session::build(message, detail, handle, window, cx) {
            Ok(prompt) => return prompt,
            Err(handle) => handle,
        };
        let view = cx.new(|cx| ConfirmationPrompt {
            message: message.into(),
            detail: detail.map(str::to_owned),
            buttons: buttons.to_vec(),
            selected: cancel_index(buttons).unwrap_or(0),
            focus: cx.focus_handle(),
        });
        handle.with_view(view, window, cx)
    });
}
fn cancel_index(buttons: &[PromptButton]) -> Option<usize> {
    buttons
        .iter()
        .position(|b| b.is_cancel() || b.label().eq_ignore_ascii_case("cancel"))
        .or_else(|| {
            (buttons.len() == 1 && buttons[0].label().eq_ignore_ascii_case("ok")).then_some(0)
        })
}
struct ConfirmationPrompt {
    message: String,
    detail: Option<String>,
    buttons: Vec<PromptButton>,
    selected: usize,
    focus: FocusHandle,
}
impl EventEmitter<PromptResponse> for ConfirmationPrompt {}
impl Focusable for ConfirmationPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for ConfirmationPrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let width = (f32::from(window.viewport_size().width) - 48.).clamp(240., 560.);
        div()
            .id("confirmation-overlay")
            .occlude()
            .size_full()
            .bg(gpui::rgba(0x00000055))
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .id("confirmation-dialog")
                    .debug_selector(|| "confirmation-dialog".into())
                    .track_focus(&self.focus)
                    .key_context("PiPrompt")
                    .role(gpui::Role::Dialog)
                    .aria_label(self.message.clone())
                    .w(px(width))
                    .max_h(window.viewport_size().height - px(48.))
                    .overflow_y_scroll()
                    .p(px(24.))
                    .gap(px(16.))
                    .rounded(px(10.))
                    .bg(theme.canvas)
                    .text_color(theme.text)
                    .border_1()
                    .border_color(theme.line)
                    .on_action(
                        cx.listener(|_, _: &super::FocusSearch, _, cx| cx.stop_propagation()),
                    )
                    .on_action(cx.listener(|_, _: &super::NewSession, _, cx| cx.stop_propagation()))
                    .on_action(cx.listener(|_, _: &super::OpenFolder, _, cx| cx.stop_propagation()))
                    .on_action(
                        cx.listener(|_, _: &super::ToggleTerminal, _, cx| cx.stop_propagation()),
                    )
                    .on_action(
                        cx.listener(|_, _: &super::ShowDiagnostics, _, cx| cx.stop_propagation()),
                    )
                    .on_action(cx.listener(|_, _: &super::ShowFiles, _, cx| cx.stop_propagation()))
                    .on_action(
                        cx.listener(|_, _: &super::ToggleInspector, _, cx| cx.stop_propagation()),
                    )
                    .on_action(
                        cx.listener(|_, _: &super::ToggleSidebar, _, cx| cx.stop_propagation()),
                    )
                    .on_action(
                        cx.listener(|_, _: &super::ExpandMessageEditor, _, cx| {
                            cx.stop_propagation()
                        }),
                    )
                    .on_action(cx.listener(|this, _: &CancelPrompt, _, cx| {
                        cx.stop_propagation();
                        if let Some(index) = cancel_index(&this.buttons) {
                            cx.emit(PromptResponse(index));
                        }
                    }))
                    .on_action(cx.listener(|this, _: &ConfirmPrompt, _, cx| {
                        cx.stop_propagation();
                        if this.selected < this.buttons.len() {
                            cx.emit(PromptResponse(this.selected));
                        }
                    }))
                    .on_action(cx.listener(|this, _: &NextPrompt, _, cx| {
                        cx.stop_propagation();
                        this.selected = (this.selected + 1) % this.buttons.len().max(1);
                        cx.notify();
                    }))
                    .on_action(cx.listener(|this, _: &PreviousPrompt, _, cx| {
                        cx.stop_propagation();
                        let len = this.buttons.len().max(1);
                        this.selected = (this.selected + len - 1) % len;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w_full()
                            .text_size(px(17.))
                            .line_height(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(self.message.clone()),
                    )
                    .when_some(self.detail.clone(), |v, detail| {
                        v.child(
                            div()
                                .w_full()
                                .text_size(px(13.))
                                .line_height(px(20.))
                                .text_color(theme.secondary)
                                .child(detail),
                        )
                    })
                    .child(
                        h_flex()
                            .justify_end()
                            .gap(px(8.))
                            .when(self.buttons.len() > 2, |row| row.flex_col().items_stretch())
                            .children(self.buttons.iter().enumerate().map(|(index, choice)| {
                                let selected = index == self.selected;
                                button(("prompt-choice", index), choice.label().clone(), theme)
                                    .debug_selector(move || format!("prompt-choice-{index}"))
                                    .h_auto()
                                    .min_h(px(28.))
                                    .py(px(4.))
                                    .when(selected, |b| {
                                        b.border_color(theme.line).bg(theme.selected)
                                    })
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(PromptResponse(index));
                                    }))
                            })),
                    ),
            )
    }
}
