//! Sessions working in the same folder (design study 05, 04). A session about to
//! start its first run where another session is working asks where to work: its
//! own jj workspace, in the automatic place or a folder you choose, or the same
//! folder, where both sessions' edits can end up in one turn. `jj.parallelSessions`
//! answers for the project; "Remember for this project" sets it.
use super::*;
use gpui::{
    EntityId, EventEmitter, FocusHandle, Global, PathPromptOptions, PromptButton, PromptHandle,
    PromptResponse, RenderablePromptHandle,
};
use std::collections::HashMap;
use std::path::Path;

/// Where each working session works: its jj workspace root, and its title.
#[derive(Default)]
struct Working(HashMap<EntityId, (PathBuf, String)>);
impl Global for Working {}

/// Records whether a session is working, and where.
pub fn set_working(cx: &mut App, session: EntityId, folder: Option<(PathBuf, String)>) {
    let working = cx.default_global::<Working>();
    match folder {
        Some(folder) => working.0.insert(session, folder),
        None => working.0.remove(&session),
    };
}

/// The title of another session working in `root`, if one is.
pub fn working_elsewhere(cx: &App, session: EntityId, root: &Path) -> Option<String> {
    cx.try_global::<Working>()?
        .0
        .iter()
        .find(|(id, (folder, _))| **id != session && folder == root)
        .map(|(_, (_, title))| title.clone())
}

/// `<parent>/<name>-ws/<words of the prompt>`, a folder not used yet.
pub fn automatic_folder(root: &Path, prompt: &str) -> PathBuf {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    let base = root.parent().unwrap_or(root).join(format!("{name}-ws"));
    let words: Vec<String> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(4)
        .map(str::to_lowercase)
        .collect();
    let mut slug = words.join("-");
    slug.truncate(32);
    let slug = slug.trim_end_matches('-').to_owned();
    let slug = if slug.is_empty() {
        "session".into()
    } else {
        slug
    };
    let mut folder = base.join(&slug);
    let mut n = 2;
    while folder.exists() {
        folder = base.join(format!("{slug}-{n}"));
        n += 1;
    }
    folder
}

#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// A new jj workspace in this folder.
    Workspace(PathBuf),
    Same,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub place: Place,
    pub remember: bool,
}

pub struct Question {
    /// The session already working there.
    pub other: String,
    pub root: PathBuf,
    pub automatic: PathBuf,
}

const TITLE: &str = "Where should this session work?";

/// The question, and where its answer goes, until the prompt builder takes it.
#[derive(Default)]
struct Pending(Option<(Question, async_channel::Sender<Option<Answer>>)>);
impl Global for Pending {}

/// Asks, as a prompt with a form. `None` when cancelled.
pub fn ask(
    question: Question,
    window: &mut Window,
    cx: &mut App,
) -> async_channel::Receiver<Option<Answer>> {
    let (sender, receiver) = async_channel::bounded(1);
    cx.set_global(Pending(Some((question, sender))));
    let prompt = window.prompt(
        gpui::PromptLevel::Info,
        TITLE,
        None,
        &[PromptButton::cancel("Cancel"), PromptButton::ok("Start")],
        cx,
    );
    cx.spawn(async move |_| {
        let _ = prompt.await;
    })
    .detach();
    receiver
}

/// The prompt builder's hook: the form for [`ask`], else the handle back.
pub fn build(
    message: &str,
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> Result<RenderablePromptHandle, PromptHandle> {
    if message != TITLE {
        return Err(handle);
    }
    let Some((question, answer)) = cx
        .has_global::<Pending>()
        .then(|| cx.global_mut::<Pending>().0.take())
        .flatten()
    else {
        return Err(handle);
    };
    let view = cx.new(|cx| Form {
        question,
        answer,
        choice: 0,
        chosen: None,
        remember: false,
        focus: cx.focus_handle(),
    });
    Ok(handle.with_view(view, window, cx))
}

struct Form {
    question: Question,
    answer: async_channel::Sender<Option<Answer>>,
    /// 0: automatic workspace, 1: chosen folder, 2: same folder.
    choice: usize,
    chosen: Option<PathBuf>,
    remember: bool,
    focus: FocusHandle,
}
impl EventEmitter<PromptResponse> for Form {}
impl Focusable for Form {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Form {
    fn finish(&mut self, start: bool, cx: &mut Context<Self>) {
        let place = match self.choice {
            0 => Some(Place::Workspace(self.question.automatic.clone())),
            1 => self.chosen.clone().map(Place::Workspace),
            _ => Some(Place::Same),
        };
        let answer = place.filter(|_| start).map(|place| Answer {
            place,
            remember: self.remember,
        });
        if start && answer.is_none() {
            return;
        }
        self.answer.try_send(answer).ok();
        cx.emit(PromptResponse(usize::from(start)));
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Workspace folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    this.choice = 1;
                    this.chosen = Some(path);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

impl Render for Form {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let width = (f32::from(window.viewport_size().width) - 48.).clamp(240., 560.);
        let automatic = tilde(&self.question.automatic);
        let chosen = self
            .chosen
            .as_deref()
            .map(tilde)
            .unwrap_or_else(|| "A jj workspace wherever you pick. It must be empty.".into());
        let options = [
            ("Own workspace, automatic", automatic, false),
            ("Own workspace, in a folder you choose", chosen, true),
            (
                "Same folder",
                "Edits from both sessions can end up in one turn.".to_owned(),
                false,
            ),
        ];
        div()
            .id("parallel-overlay")
            .occlude()
            .size_full()
            .bg(gpui::rgba(0x00000055))
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .id("parallel-dialog")
                    .debug_selector(|| "parallel-dialog".into())
                    .track_focus(&self.focus)
                    .key_context("PiPrompt")
                    .role(gpui::Role::Dialog)
                    .aria_label(TITLE)
                    .w(px(width))
                    .p(px(24.))
                    .gap(px(12.))
                    .rounded(px(10.))
                    .bg(theme.canvas)
                    .text_color(theme.text)
                    .border_1()
                    .border_color(theme.line)
                    .on_action(cx.listener(|this, _: &super::prompts::CancelPrompt, _, cx| {
                        cx.stop_propagation();
                        this.finish(false, cx);
                    }))
                    .on_action(cx.listener(|this, _: &super::prompts::ConfirmPrompt, _, cx| {
                        cx.stop_propagation();
                        this.finish(true, cx);
                    }))
                    .child(
                        div()
                            .text_size(px(17.))
                            .line_height(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!(
                                "{} is working in {}",
                                self.question.other,
                                tilde(&self.question.root)
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(theme.secondary)
                            .child("Running this session in the same folder mixes both sessions' edits. Where should it work?"),
                    )
                    .children(options.into_iter().enumerate().map(|(i, (title, detail, choose))| {
                        let selected = self.choice == i;
                        h_flex()
                            .id(("parallel-option", i))
                            .debug_selector(move || format!("parallel-option-{i}"))
                            .role(gpui::Role::RadioButton)
                            .aria_selected(selected)
                            .p(px(10.))
                            .gap(px(10.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.line)
                            .when(selected, |row| row.bg(theme.selected))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.choice = i;
                                if choose && this.chosen.is_none() {
                                    this.choose_folder(cx);
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(14.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(theme.line_strong)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(selected, |dot| {
                                        dot.child(div().size(px(6.)).rounded_full().bg(theme.text))
                                    }),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_size(px(12.5)).child(title))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.))
                                            .text_color(theme.muted)
                                            .when(!choose || self.chosen.is_some(), |v| {
                                                v.when(i != 2, |v| v.font_family(MONO))
                                            })
                                            .child(detail),
                                    ),
                            )
                            .when(choose, |row| {
                                row.child(
                                    button(("parallel-choose", i), "Choose…", theme).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.choose_folder(cx);
                                        }),
                                    ),
                                )
                            })
                    }))
                    .child(
                        h_flex()
                            .id("parallel-remember")
                            .debug_selector(|| "parallel-remember".into())
                            .role(gpui::Role::CheckBox)
                            .gap(px(8.))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remember = !this.remember;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(if self.remember { theme.accent } else { theme.line_strong })
                                    .when(self.remember, |b| b.bg(theme.accent)),
                            )
                            .child(div().text_size(px(12.)).text_color(theme.secondary).child("Remember for this project"))
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(10.5))
                                    .text_color(theme.faint)
                                    .child("saved in .pi/pi-desktop.json"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(11.))
                                    .text_color(theme.faint)
                                    .child("Bring its turns into the main folder later, from Changes."),
                            )
                            .child(
                                button("parallel-cancel", "Cancel", theme)
                                    .on_click(cx.listener(|this, _, _, cx| this.finish(false, cx))),
                            )
                            .child(
                                primary_button(
                                    "parallel-start",
                                    "Start",
                                    self.choice != 1 || self.chosen.is_some(),
                                    theme,
                                )
                                .debug_selector(|| "parallel-start".into())
                                .on_click(cx.listener(|this, _, _, cx| this.finish(true, cx))),
                            ),
                    ),
            )
    }
}

fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_automatic_folder_sits_beside_the_project_and_is_new() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("pi");
        let folder = automatic_folder(&root, "Add a regression test, with the faux provider!");
        assert_eq!(folder, dir.path().join("pi-ws/add-a-regression-test"));
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(
            automatic_folder(&root, "Add a regression test"),
            dir.path().join("pi-ws/add-a-regression-test-2")
        );
        assert_eq!(
            automatic_folder(&root, "!!"),
            dir.path().join("pi-ws/session")
        );
    }
}
