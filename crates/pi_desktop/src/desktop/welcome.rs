//! Empty workspace after the last session closes. Project selection survives;
//! choosing it does not start an RPC process until New Session is invoked.
use super::*;
pub struct WelcomeView {
    workspace: Entity<WorkspaceController>,
    focus: gpui::FocusHandle,
    _subscription: gpui::Subscription,
}
impl WelcomeView {
    pub fn new(workspace: Entity<WorkspaceController>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&workspace, |_, _, event, cx| {
            if matches!(
                event,
                WorkspaceEvent::Navigation | WorkspaceEvent::Selection(_)
            ) {
                cx.notify();
            }
        });
        Self {
            workspace,
            focus: cx.focus_handle(),
            _subscription: subscription,
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
}
impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let project = self.workspace.read(cx).selected_project.clone();
        v_flex()
            .id("workspace-welcome")
            .debug_selector(|| "workspace-welcome".into())
            .track_focus(&self.focus)
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(18.))
            .p(px(32.))
            .child(brand_mark_sized(48.))
            .child(
                div()
                    .font_family(SERIF)
                    .italic()
                    .text_size(px(26.))
                    .child("A quiet place to work."),
            )
            .child(
                div()
                    .text_color(theme.muted)
                    .text_center()
                    .child(if project.is_some() {
                        "Start a new session in the selected project."
                    } else {
                        "Select a project on the left, or open a folder."
                    }),
            )
            .when_some(project.clone(), |v, path| {
                v.child(
                    div()
                        .font_family(MONO)
                        .text_size(px(12.))
                        .text_color(theme.muted)
                        .child(path.display().to_string()),
                )
            })
            .child(
                primary_button(
                    "welcome-open",
                    if project.is_some() {
                        "New session"
                    } else {
                        "Open project…"
                    },
                    true,
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    super::new_session::show(this.workspace.clone(), window, cx);
                })),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.faint)
                    .child("No active sessions · no Pi processes"),
            )
    }
}
