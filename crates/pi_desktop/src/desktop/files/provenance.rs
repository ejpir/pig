//! What this session observed about the open file (design/workbench-vision 03):
//! its edit and write calls, the lines the latest edit wrote shaded in the
//! editor, and the inspector's provenance details. Observed tool calls only,
//! never a full working-tree history; details are opened, never pushed.
use super::*;
use std::path::Path;

/// Shaded rows of the latest observed edit.
struct EditedRows;

/// One edit or write call this session made to the open file.
pub(super) struct ObservedEdit {
    pub id: String,
    pub write: bool,
    /// The message's time, epoch milliseconds.
    pub at: Option<u64>,
    pub new_texts: Vec<String>,
    pub finished: bool,
    pub failed: bool,
}

/// Edit and write calls to `path` in this session, oldest first.
pub(super) fn observed_edits(
    model: &pi_core::session::Session,
    root: &Path,
    path: &Path,
) -> Vec<ObservedEdit> {
    let mut edits = vec![];
    for message in model.messages.iter().filter(|m| m["role"] == "assistant") {
        for block in message["content"].as_array().into_iter().flatten() {
            if block["type"] != "toolCall" {
                continue;
            }
            let id = block["id"].as_str().unwrap_or_default();
            let tool = model.tools.iter().find(|tool| tool.id == id);
            let name = block["name"]
                .as_str()
                .or(tool.map(|tool| tool.name.as_str()))
                .unwrap_or_default();
            if !matches!(name, "edit" | "write") {
                continue;
            }
            let args = if block["arguments"].is_object() {
                &block["arguments"]
            } else {
                tool.map_or(&block["arguments"], |tool| &tool.args)
            };
            let Some(target) = args["path"].as_str() else {
                continue;
            };
            let target = Path::new(target.trim_start_matches("./"));
            if root.join(target) != path && target != path {
                continue;
            }
            let new_texts = args["newText"]
                .as_str()
                .map(str::to_owned)
                .into_iter()
                .chain(
                    args["edits"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|edit| edit["newText"].as_str().map(str::to_owned)),
                )
                .collect();
            edits.push(ObservedEdit {
                id: id.to_owned(),
                write: name == "write",
                at: message["timestamp"].as_u64(),
                new_texts,
                finished: tool.is_some_and(|tool| tool.finished),
                failed: tool.is_some_and(|tool| tool.is_error),
            });
        }
    }
    edits
}

/// The rows an edit's new text occupies in `text` now, when that text is still
/// there exactly once. A file changed since shades nothing rather than guessing.
pub(super) fn rows_of(text: &str, new_texts: &[String]) -> Vec<std::ops::Range<u32>> {
    new_texts
        .iter()
        .filter_map(|new| {
            let new = new.trim_matches('\n');
            if new.trim().is_empty() || text.matches(new).count() != 1 {
                return None;
            }
            let start = text.find(new)?;
            let row = text[..start].matches('\n').count() as u32;
            Some(row..row + new.matches('\n').count() as u32 + 1)
        })
        .collect()
}

fn clock(timestamp: u64) -> String {
    format!(
        "{:02}:{:02}",
        timestamp / 3_600_000 % 24,
        timestamp / 60_000 % 60
    )
}

impl FilesView {
    fn controller(&self) -> Option<Entity<super::super::session::SessionController>> {
        self.bars.session.as_ref()?.upgrade()
    }

    pub(super) fn active_edits(&self, cx: &App) -> Vec<ObservedEdit> {
        let (Some(controller), Some(tab)) = (self.controller(), self.tab()) else {
            return vec![];
        };
        observed_edits(controller.read(cx).model(), &self.root, &tab.path)
    }

    /// Shades what the latest successful edit wrote, when the open file still
    /// holds it. Recomputed only when the file, its length or the edit changes.
    pub(super) fn shade_edits(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tab() else {
            return;
        };
        let Some(editor) = tab.editor.clone() else {
            return;
        };
        let latest = self
            .active_edits(cx)
            .into_iter()
            .rfind(|edit| !edit.write && edit.finished && !edit.failed);
        let key = (
            tab.path.clone(),
            tab.buffer.read(cx).len(),
            latest.as_ref().map(|edit| edit.id.clone()),
        );
        if self.shaded.as_ref() == Some(&key) {
            return;
        }
        let rows = latest
            .map(|edit| rows_of(&tab.buffer.read(cx).text(), &edit.new_texts))
            .unwrap_or_default();
        self.shaded = Some(key);
        pi_editor::shade_rows::<EditedRows>(&editor, &rows, |cx| theme(cx).raised, cx);
    }

    /// The inspector for the open file: where its observed edits came from and
    /// what history can do about them. Opened on request only.
    pub fn details(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let Some(tab) = self.tab() else {
            return v_flex()
                .p(px(20.))
                .child(note("No file is open.", theme))
                .into_any_element();
        };
        let path = tab.path.clone();
        let relative = path
            .strip_prefix(&self.root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| relative.clone());
        let edits = self.active_edits(cx);
        let controller = self.controller();
        let (session, recording, records, context, run) = controller
            .as_ref()
            .map(|controller| {
                let controller = controller.read(cx);
                let model = controller.model();
                let records = controller
                    .jj()
                    .records
                    .iter()
                    .filter(|record| record.diff.iter().any(|file| file.path == relative))
                    .count();
                let context = model
                    .stats
                    .context_usage
                    .as_ref()
                    .and_then(|usage| usage.percent)
                    .map(|percent| format!("{percent:.0}% used"));
                let run = [
                    model
                        .state
                        .model
                        .as_ref()
                        .map(|m| m.name.clone().unwrap_or_else(|| m.id.clone())),
                    Some(model.state.thinking_level.clone()).filter(|level| !level.is_empty()),
                    Some(model.run.label().to_owned()),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                (
                    model.title().to_owned(),
                    controller.jj().project.is_some(),
                    records,
                    context,
                    run,
                )
            })
            .unwrap_or_default();
        let latest = edits.last();
        let rule = || div().h(px(1.)).my(px(14.)).bg(theme.line);
        let heading = |text: &'static str| {
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.secondary)
                .child(text)
        };
        let small = |text: String, color| {
            div()
                .text_size(px(12.))
                .line_height(px(20.))
                .text_color(color)
                .child(text)
        };
        let disclosure = |key: &'static str,
                          title: &'static str,
                          value: Option<String>,
                          body: AnyElement,
                          cx: &mut Context<Self>| {
            let open = self.details_open.contains(key);
            v_flex()
                .child(
                    h_flex()
                        .id(SharedString::from(format!("file-details-{key}")))
                        .debug_selector(move || format!("file-details-{key}"))
                        .role(gpui::Role::Button)
                        .aria_expanded(open)
                        .h(px(36.))
                        .gap(px(8.))
                        .cursor_pointer()
                        .child(
                            icon(
                                if open {
                                    "chevron_down"
                                } else {
                                    "chevron_right"
                                },
                                theme.muted,
                            )
                            .size(px(12.)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .text_color(theme.text)
                                .child(title),
                        )
                        .children(value.map(|value| small(value, theme.muted)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.details_open.remove(key) {
                                this.details_open.insert(key);
                            }
                            cx.notify();
                        })),
                )
                .when(open, |v| v.child(div().pl(px(20.)).pb(px(8.)).child(body)))
        };
        let from_session = v_flex()
            .gap(px(6.))
            .child(heading("From this session"))
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme.text)
                    .child(session.clone()),
            )
            .child(match latest {
                None => small(
                    "No edits observed for this file in this session.".into(),
                    theme.muted,
                ),
                Some(edit) => small(
                    format!(
                        "{}{}{}",
                        if edit.failed {
                            "Failed edit"
                        } else if edit.write {
                            "Written"
                        } else {
                            "Recorded edit"
                        },
                        edit.at
                            .map(|at| format!(" · {}", clock(at)))
                            .unwrap_or_default(),
                        if edits.len() > 1 {
                            format!(" · {} calls", edits.len())
                        } else {
                            String::new()
                        }
                    ),
                    if edit.failed {
                        theme.coral
                    } else {
                        theme.muted
                    },
                ),
            })
            .children(latest.map(|edit| {
                let id = edit.id.clone();
                div()
                    .id("file-details-reveal")
                    .debug_selector(|| "file-details-reveal".into())
                    .text_size(px(13.))
                    .text_color(theme.accent)
                    .cursor_pointer()
                    .hover(move |link| link.text_color(theme.text))
                    .child("Reveal in thread →")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(controller) = this.controller() {
                            controller.update(cx, |c, cx| c.reveal_tool(id.clone(), cx));
                        }
                    }))
            }));
        let history = v_flex()
            .debug_selector(|| "file-details-history".into())
            .gap(px(6.))
            .child(heading("File history"))
            .child(if records > 0 {
                small(
                    format!(
                        "jj recorded this file in {records} turn{}. Changes shows each snapshot.",
                        if records == 1 { "" } else { "s" }
                    ),
                    theme.muted,
                )
            } else if recording {
                small("No turn recorded this file yet.".into(), theme.muted)
            } else {
                small(
                    if latest.is_some() {
                        "No snapshot was recorded for this edit."
                    } else {
                        "No file snapshots are recorded for this project."
                    }
                    .into(),
                    theme.muted,
                )
            })
            .when(records == 0, |v| {
                v.child(small("Restore is unavailable.".into(), theme.amber))
            });
        let full_path = disclosure(
            "path",
            "Full path",
            None,
            div()
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(theme.muted)
                .child(path.display().to_string())
                .into_any_element(),
            cx,
        );
        let session_context = disclosure(
            "context",
            "Session context",
            context.clone(),
            small(
                context
                    .map(|c| format!("{c} of the model's context window."))
                    .unwrap_or_else(|| "Pi has not reported context usage yet.".into()),
                theme.muted,
            )
            .into_any_element(),
            cx,
        );
        let tree = disclosure(
            "tree",
            "Session tree",
            None,
            div()
                .id("file-details-open-tree")
                .text_size(px(12.5))
                .text_color(theme.accent)
                .cursor_pointer()
                .child("Open the session tree →")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(FileEvent::ShowTree)))
                .into_any_element(),
            cx,
        );
        let run_details = disclosure(
            "run",
            "Run details",
            None,
            small(
                if run.is_empty() {
                    "Not reported".into()
                } else {
                    run
                },
                theme.muted,
            )
            .into_any_element(),
            cx,
        );
        v_flex()
            .id("file-details")
            .debug_selector(|| "file-details".into())
            .min_h((window.viewport_size().height - px(76.)).max(px(400.)))
            .p(px(20.))
            .child(
                div()
                    .font_family(SERIF)
                    .italic()
                    .text_size(px(26.))
                    .line_height(px(32.))
                    .child(if latest.is_some() {
                        "Selected edit"
                    } else {
                        "File details"
                    }),
            )
            .child(
                div()
                    .mt(px(4.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name),
            )
            .child(rule())
            .child(from_session)
            .child(rule())
            .child(history)
            .child(rule())
            .child(full_path)
            .child(session_context)
            .child(tree)
            .child(run_details)
            .child(div().flex_1().min_h(px(24.)))
            .child(small("Details are optional.".into(), theme.muted))
            .child(small(
                "File and revision actions stay in the work area.".into(),
                theme.muted,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{observed_edits, rows_of};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn the_latest_edit_is_shaded_only_where_its_text_still_is() {
        let text = "a\n  if (!signature && isAnthropic(model)) {\n    throw x;\n  }\nb\n";
        let new = |s: &str| vec![s.to_owned()];
        assert_eq!(
            rows_of(
                text,
                &new("  if (!signature && isAnthropic(model)) {\n    throw x;")
            ),
            vec![1..3]
        );
        assert!(
            rows_of(text, &new("gone")).is_empty(),
            "a file changed since shades nothing"
        );
        assert!(
            rows_of("x\nx\n", &new("x")).is_empty(),
            "ambiguous text shades nothing"
        );
        assert!(rows_of(text, &new("\n")).is_empty());
    }

    #[test]
    fn only_this_files_edit_and_write_calls_are_observed() {
        let mut model = pi_core::session::Session::new("/p".into());
        model.messages = vec![json!({"role":"assistant","timestamp":1000,"content":[
            {"type":"toolCall","id":"a","name":"edit","arguments":{"path":"src/x.ts","newText":"new"}},
            {"type":"toolCall","id":"b","name":"edit","arguments":{"path":"src/y.ts","newText":"other"}},
            {"type":"toolCall","id":"c","name":"read","arguments":{"path":"src/x.ts"}},
            {"type":"toolCall","id":"d","name":"write","arguments":{"path":"/p/src/x.ts","content":"all"}}
        ]})];
        let edits = observed_edits(&model, Path::new("/p"), Path::new("/p/src/x.ts"));
        let ids: Vec<_> = edits
            .iter()
            .map(|e| (e.id.as_str(), e.write, e.at))
            .collect();
        assert_eq!(ids, [("a", false, Some(1000)), ("d", true, Some(1000))]);
        assert_eq!(edits[0].new_texts, ["new"]);
    }
}
