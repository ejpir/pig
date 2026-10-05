//! The pre-conversation surface (design/workbench-vision 08): lead with the task
//! and the composer. Starting points fill the draft; nothing here sends a prompt.
//! Catalog metadata is not a context manifest. This owner keeps its scroll and
//! notifications separate from the transcript and draft.
use super::composer::ComposerView;
use super::session::{Changes, SessionController, SessionEvent};
use super::*;
use gpui::{Div, ElementId, EventEmitter, Stateful, Subscription};

pub enum LandingEvent {
    UseCommand(SlashCommand),
    /// Text for the draft, from a starting point. Never sent by itself.
    Draft(String),
}

/// Starting points that suit any project. They fill the draft only.
const STARTERS: [(&str, &str); 2] = [
    (
        "Review the local changes",
        "Review the local changes in this project and point out anything risky.",
    ),
    (
        "Explain this project",
        "Explain how this project is organized and where I should start.",
    ),
];

pub struct LandingView {
    controller: Entity<SessionController>,
    composer: Entity<ComposerView>,
    scroll: ScrollHandle,
    composer_focus: gpui::FocusHandle,
    _subscription: Subscription,
}
impl EventEmitter<LandingEvent> for LandingView {}
impl LandingView {
    pub fn new(
        controller: Entity<SessionController>,
        composer: Entity<ComposerView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&controller, |_, _, event, cx| {
            if matches!(event, SessionEvent::Changed(c) if c.intersects(Changes::CATALOG | Changes::METADATA | Changes::RUN | Changes::JJ)) {
                cx.notify();
            }
        });
        let composer_focus = composer.read(cx).input.focus_handle(cx);
        Self {
            controller,
            composer,
            composer_focus,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }
}

/// One starting point: a quiet row with a hairline below, as in the study.
fn starter(
    id: impl Into<ElementId>,
    title: String,
    hint: Option<String>,
    theme: Theme,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(title.clone())
        .w_full()
        .h(px(44.))
        .px(px(2.))
        .gap(px(12.))
        .border_b_1()
        .border_color(theme.line)
        .cursor_pointer()
        .hover(move |row| row.bg(theme.hover))
        .child(icon("thread", theme.muted).size(px(14.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(14.))
                .text_color(theme.text)
                .child(title),
        )
        .children(hint.map(|hint| {
            div()
                .flex_shrink_0()
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(theme.faint)
                .child(hint)
        }))
        .child(icon("chevron_right", theme.faint).size(px(12.)))
}

/// `~/repos/pi` for a folder under the home directory; the full path otherwise.
fn home_relative(path: &std::path::Path) -> String {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|home| path.strip_prefix(home).ok().map(|rest| rest.to_path_buf()))
        .map(|rest| format!("~/{}", rest.display()))
        .unwrap_or_else(|| path.display().to_string())
}

impl Render for LandingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let controller = self.controller.read(cx);
        let model = controller.model();
        let ready = controller.ready();
        let demo = controller.is_demo();
        let recording = controller.jj().project.is_some();
        let project = home_relative(&model.cwd);
        let commands: Vec<_> = model
            .commands
            .iter()
            .filter(|c| matches!(c.source.as_str(), "prompt" | "skill"))
            .take(2)
            .cloned()
            .collect();
        let gutter = WORK_GUTTER;
        let starters = v_flex()
            .debug_selector(|| "landing-starters".into())
            .px(gutter)
            .mt(px(52.))
            .child(
                div()
                    .h(px(28.))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.secondary)
                    .child("A starting point"),
            )
            .child(div().h(px(1.)).bg(theme.line))
            .children(STARTERS.iter().enumerate().map(|(i, (title, draft))| {
                let draft = draft.to_string();
                starter(("landing-starter", i), title.to_string(), None, theme)
                    .debug_selector(move || format!("landing-starter-{i}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.emit(LandingEvent::Draft(draft.clone()));
                        this.composer_focus.focus(window, cx);
                    }))
            }))
            // Pi's own templates and skills attach as a command; the draft is kept.
            .children(commands.into_iter().enumerate().map(|(i, command)| {
                let title = command
                    .description
                    .clone()
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| command.name.clone());
                starter(
                    ("landing-command", i),
                    title,
                    Some(format!("/{}", command.name)),
                    theme,
                )
                .debug_selector(move || format!("landing-command-{i}"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.controller.read(cx).ready() {
                        cx.emit(LandingEvent::UseCommand(command.clone()));
                        this.composer_focus.focus(window, cx);
                    }
                }))
            }));
        let body = v_flex()
            .w_full()
            .pt(px(88.))
            .pb(px(32.))
            .flex_shrink_0()
            .child(
                v_flex()
                    .px(gutter)
                    .child(
                        div()
                            .font_family(SERIF)
                            .italic()
                            .text_size(px(36.))
                            .line_height(px(44.))
                            .text_color(theme.text)
                            .child("What should we change?"),
                    )
                    .child(
                        div()
                            .mt(px(12.))
                            .text_size(px(15.))
                            .line_height(px(22.))
                            .text_color(theme.muted)
                            .child(
                                "Start with the task. Bring in files and details as you need them.",
                            ),
                    ),
            )
            .child(div().mt(px(26.)).child(self.composer.clone()))
            .child(
                div()
                    .px(gutter)
                    .mt(px(4.))
                    .text_size(px(12.))
                    .text_color(theme.muted)
                    .child(if ready || demo {
                        "Type / for commands or @ to include a file."
                    } else {
                        "Waiting for Pi to report this project's session and commands."
                    }),
            )
            .child(starters)
            .child(
                v_flex()
                    .debug_selector(|| "landing-project".into())
                    .px(gutter)
                    .mt(px(44.))
                    .gap(px(10.))
                    .child(
                        h_flex()
                            .gap(px(24.))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.secondary)
                                    .child("Project"),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(px(12.))
                                    .text_color(theme.muted)
                                    .child(project),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.muted)
                            .child(if recording {
                                "File history is on. jj records each turn that edits files."
                            } else {
                                "File history is off. Past edits cannot be restored."
                            }),
                    )
                    .when(demo, |v| {
                        v.child(
                            div().text_size(px(12.)).text_color(theme.faint).child(
                                "Offline sample · no prompts are sent or project files read.",
                            ),
                        )
                    }),
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
