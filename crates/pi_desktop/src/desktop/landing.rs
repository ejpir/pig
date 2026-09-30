//! The pre-conversation surface. Catalog metadata is not a context manifest.
//! This owner keeps its scroll/notifications separate from the transcript and draft.
use super::session::{Changes, SessionController, SessionEvent};
use super::*;
use gpui::{Div, EventEmitter, Subscription};

pub enum LandingEvent {
    UseCommand(SlashCommand),
}

pub struct LandingView {
    controller: Entity<SessionController>,
    scroll: ScrollHandle,
    composer_focus: gpui::FocusHandle,
    _subscription: Subscription,
}
impl EventEmitter<LandingEvent> for LandingView {}
impl LandingView {
    pub fn new(
        controller: Entity<SessionController>,
        composer_focus: gpui::FocusHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&controller, |_, _, event, cx| {
            if matches!(event, SessionEvent::Changed(c) if c.intersects(Changes::CATALOG | Changes::METADATA | Changes::RUN)) {
                cx.notify();
            }
        });
        Self {
            controller,
            composer_focus,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }

    fn catalog(&self, source: &str, title: &str, cx: &Context<Self>, theme: Theme) -> Div {
        let commands: Vec<_> = self
            .controller
            .read(cx)
            .model()
            .commands
            .iter()
            .filter(|c| c.source == source)
            .collect();
        card(title, &commands.len().to_string(), theme)
            .children(commands.iter().take(3).map(|command| {
                h_flex()
                    .h(px(20.))
                    .gap(px(10.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(11.5))
                            .child(if source == "skill" {
                                command.name.trim_start_matches("skill:").to_owned()
                            } else {
                                format!("/{}", command.name)
                            }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_right()
                            .text_size(px(11.))
                            .text_color(theme.faint)
                            .child(command.description.clone().unwrap_or_default()),
                    )
            }))
            .when(commands.is_empty(), |v| {
                v.child(note("None reported by Pi", theme))
            })
            .when(commands.len() > 3, |v| {
                v.child(note(
                    format!("{} more in / commands", commands.len() - 3),
                    theme,
                ))
            })
    }
}

fn note(text: impl Into<SharedString>, theme: Theme) -> Div {
    div()
        .text_size(px(11.))
        .line_height(px(18.))
        .text_color(theme.faint)
        .child(text.into())
}
fn card(title: &str, count: &str, theme: Theme) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .min_h(px(102.))
        .flex_shrink_0()
        .px(px(14.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(theme.panel)
        .border_1()
        .border_color(theme.line)
        .child(section(title, count, theme).mb(px(4.)))
}

impl Render for LandingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let controller = self.controller.read(cx);
        let model = controller.model();
        let ready = controller.ready();
        let commands: Vec<_> = model
            .commands
            .iter()
            .filter(|c| matches!(c.source.as_str(), "prompt" | "skill"))
            .take(4)
            .cloned()
            .collect();
        let subtitle = if controller.is_demo() {
            "Offline sample catalog · no prompts are sent or project files read.".to_owned()
        } else if ready {
            format!(
                "Available in {}. Choose a starting point, then send when ready.",
                model.cwd.display()
            )
        } else {
            "Waiting for Pi to report this project's session and commands.".to_owned()
        };
        let body = v_flex()
            .w_full()
            .px(px(32.))
            .pt(px(26.))
            .pb(px(18.))
            .flex_shrink_0()
            .child(
                v_flex()
                    .items_center()
                    .child(brand_mark_sized(48.))
                    .child(
                        div()
                            .mt(px(22.))
                            .font_family(SERIF)
                            .italic()
                            .text_size(px(24.))
                            .line_height(px(30.))
                            .text_center()
                            .child("Start with a prompt, a template, or a skill."),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_center()
                            .text_size(px(12.5))
                            .line_height(px(20.))
                            .text_color(theme.muted)
                            .child(subtitle),
                    ),
            )
            .child(
                label("REPORTED FOR THIS PROJECT", theme)
                    .mt(px(22.))
                    .mb(px(6.)),
            )
            .child(
                h_flex()
                    .items_stretch()
                    .gap(px(8.))
                    .child(
                        card("CONTEXT FILES", "—", theme)
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(11.5))
                                    .child("Manifest not exposed by RPC"),
                            )
                            .child(note("Pi assembles context for each request.", theme))
                            .child(note("No file list is inferred from the project.", theme)),
                    )
                    .child(self.catalog("skill", "SKILLS", cx, theme)),
            )
            .child(
                h_flex()
                    .items_stretch()
                    .gap(px(8.))
                    .mt(px(8.))
                    .child(self.catalog("prompt", "PROMPT TEMPLATES", cx, theme))
                    .child(self.catalog("extension", "EXTENSION COMMANDS", cx, theme)),
            )
            .child(label("START FROM", theme).mt(px(24.)).mb(px(2.)))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .min_h(px(24.))
                    .when(commands.is_empty(), |v| {
                        v.child(note(
                            "Write a prompt below, or type / to browse commands.",
                            theme,
                        ))
                    })
                    .children(commands.into_iter().enumerate().map(|(i, command)| {
                        button(("landing-command", i), format!("/{}", command.name), theme)
                            .debug_selector(move || format!("landing-command-{i}"))
                            .font_family(MONO)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.controller.read(cx).ready() {
                                    cx.emit(LandingEvent::UseCommand(command.clone()));
                                    this.composer_focus.focus(window, cx);
                                }
                            }))
                    })),
            );
        div()
            .id("landing")
            .debug_selector(|| "landing".into())
            .size_full()
            .relative()
            .child(
                div()
                    .id("landing-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(body),
            )
            .custom_scrollbars(
                scrollbar("landing-scrollbar", &self.scroll, None),
                window,
                cx,
            )
    }
}
