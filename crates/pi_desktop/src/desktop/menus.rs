use super::composer::ComposerView;
use super::*;
use gpui::{ScrollStrategy, deferred, uniform_list};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Picker {
    Model,
    Thinking,
}
#[derive(Clone)]
pub(super) enum Choice {
    Model(pi_core::protocol::Model),
    Thinking(String),
}
impl Choice {
    fn label(&self) -> &str {
        match self {
            Self::Model(m) => &m.id,
            Self::Thinking(level) => level,
        }
    }
    fn detail(&self) -> &str {
        match self {
            Self::Model(m) => &m.provider,
            Self::Thinking(_) => "",
        }
    }
}

impl ComposerView {
    pub(super) fn toggle_picker(
        &mut self,
        picker: Picker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.picker == Some(picker) {
            self.picker = None;
            self.focus(window, cx);
            cx.notify();
            return;
        }
        self.dismiss_slash(cx);
        self.picker = Some(picker);
        self.picker_index = 0;
        self.picker_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.picker_filter
            .update(cx, |input, cx| input.set_content("", cx));
        if picker != Picker::Thinking {
            self.picker_filter.focus_handle(cx).focus(window, cx);
        } else {
            // Unlike the model picker, this menu has no filter to take focus.
            // Keep its keyboard actions in Composer even when opened elsewhere.
            self.focus(window, cx);
        }
        self.refresh_choices(cx);
        self.picker_pending = self.command(
            match picker {
                Picker::Model => Command::GetAvailableModels,
                Picker::Thinking => Command::GetAvailableThinkingLevels,
            },
            cx,
        );
        cx.notify();
    }
    pub(super) fn refresh_choices(&mut self, cx: &App) {
        let model = self.controller.read(cx).model();
        let filter = self.picker_filter.read(cx).content().to_lowercase();
        self.choices = match self.picker {
            Some(Picker::Model) => model
                .available_models
                .iter()
                .filter(|model| {
                    format!("{} {}", model.id, model.provider)
                        .to_lowercase()
                        .contains(&filter)
                })
                .cloned()
                .map(Choice::Model)
                .collect(),
            Some(Picker::Thinking) => model
                .thinking_levels
                .iter()
                .filter(|level| level.to_lowercase().contains(&filter))
                .cloned()
                .map(Choice::Thinking)
                .collect(),
            None => vec![],
        };
        self.picker_index = self.picker_index.min(self.choices.len().saturating_sub(1));
    }
    pub(super) fn move_choice(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.picker.is_some() && !self.choices.is_empty() {
            self.picker_index = (self.picker_index as isize + direction)
                .rem_euclid(self.choices.len() as isize) as usize;
            self.picker_scroll
                .scroll_to_item(self.picker_index, ScrollStrategy::Nearest);
            cx.notify();
        }
    }
    pub(super) fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker_pending.is_some() {
            return;
        }
        let Some(choice) = self.choices.get(index).cloned() else {
            return;
        };
        self.command(
            match choice {
                Choice::Model(model) => Command::SetModel {
                    provider: model.provider,
                    model_id: model.id,
                    persist: false,
                },
                Choice::Thinking(level) => Command::SetThinkingLevel { level },
            },
            cx,
        );
        self.picker = None;
        self.focus(window, cx);
        cx.notify();
    }
    fn picker_row(&self, index: usize, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let Some(choice) = self.choices.get(index) else {
            return div().into_any_element();
        };
        let state = &self.controller.read(cx).model().state;
        let selected = match choice {
            Choice::Model(m) => state
                .model
                .as_ref()
                .is_some_and(|current| current.id == m.id && current.provider == m.provider),
            Choice::Thinking(level) => state.thinking_level == *level,
        };
        let detail = choice.detail();
        h_flex()
            .w_full()
            .id(("picker-choice", index))
            .debug_selector(move || format!("picker-choice-{index}"))
            .h(px(if self.picker == Some(Picker::Model) {
                50.
            } else {
                30.
            }))
            .px(px(10.))
            .py(px(5.))
            .gap(px(9.))
            .rounded(px(5.))
            .cursor_pointer()
            .when(index == self.picker_index, |row| row.bg(theme.selected))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.picker_index != index {
                    this.picker_index = index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.choose(index, window, cx)))
            .when_some(
                match choice {
                    Choice::Thinking(level) => Some(level),
                    _ => None,
                },
                |row, level| row.child(div().size(px(7.)).rounded_full().bg(theme.thinking(level))),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .truncate()
                            .child(choice.label().to_owned()),
                    )
                    .when(!detail.is_empty(), |column| {
                        column.child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme.faint)
                                .truncate()
                                .child(detail.to_owned()),
                        )
                    }),
            )
            .when(selected, |row| row.child(icon("check", theme.accent)))
            .into_any_element()
    }
    pub(super) fn picker_view(&self, cx: &Context<Self>, theme: Theme) -> Option<AnyElement> {
        let picker = self.picker?;
        let loading = self.picker_pending.is_some();
        let list = if loading || self.choices.is_empty() {
            div()
                .p(px(12.))
                .text_color(theme.faint)
                .child(if loading {
                    "Loading from pi…"
                } else {
                    "No available choices"
                })
                .into_any_element()
        } else {
            let view = cx.entity().downgrade();
            uniform_list("picker-list", self.choices.len(), move |range, _, cx| {
                view.update(cx, |view, cx| {
                    range
                        .map(|index| view.picker_row(index, cx, theme))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
            })
            .debug_selector(|| "picker-list".into())
            .h(px((self.choices.len() as f32
                * if picker == Picker::Model { 50. } else { 30. })
            .min(260.)))
            .w_full()
            .track_scroll(&self.picker_scroll)
            .into_any_element()
        };
        Some(
            deferred(
                v_flex()
                    .id("composer-picker")
                    .absolute()
                    .map(|menu| {
                        if self.menus_below() {
                            menu.top(relative(1.)).mt(px(6.))
                        } else {
                            menu.bottom(px(47.))
                        }
                    })
                    .left(px(if picker == Picker::Thinking {
                        264.
                    } else {
                        106.
                    }))
                    .w(px(if picker == Picker::Thinking {
                        210.
                    } else {
                        350.
                    }))
                    .p(px(6.))
                    .gap(px(5.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.chip_line)
                    .bg(if theme.light { theme.chip } else { theme.bar })
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                        this.picker = None;
                        this.focus(window, cx);
                        cx.notify();
                    }))
                    .child(
                        label(
                            if picker == Picker::Model {
                                "MODEL"
                            } else {
                                "THINKING LEVEL"
                            },
                            theme,
                        )
                        .px(px(10.))
                        .py(px(5.)),
                    )
                    .when(picker != Picker::Thinking, |menu| {
                        menu.child(
                            div()
                                .mx(px(4.))
                                .px(px(8.))
                                .py(px(3.))
                                .rounded(px(5.))
                                .border_1()
                                .border_color(theme.line)
                                .bg(theme.canvas)
                                .child(self.picker_filter.clone()),
                        )
                    })
                    .child(list)
                    .child(
                        div()
                            .px(px(10.))
                            .text_size(px(10.))
                            .text_color(theme.faint)
                            .child("↑ ↓ select · Enter choose · Esc dismiss"),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
