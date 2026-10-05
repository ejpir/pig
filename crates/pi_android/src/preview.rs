//! The app in named states that match design/android/screens, for the desktop
//! preview window (`examples/preview.rs`) and the render test. The sample
//! sessions stay still in a preview.

use crate::{
    app::{PhoneApp, Route, Sheet, Target},
    composer::Attachment,
    model::{Answer, LineKind, SessionId},
    theme::Appearance,
};
use gpui::{Context, Window};
use std::time::Duration;

const ADDRESS: &str = "nick@studio-mac.local";
const QWEN: SessionId = SessionId(1);

/// Every state `PhoneApp::preview` knows.
pub const SCREENS: &[&str] = &[
    "connect",
    "sessions",
    "search",
    "start",
    "working",
    "waiting",
    "done",
    "review",
    "typing",
    "details",
    "evening",
    "settings",
    "model",
    "attach",
    "more",
    "project",
    "models",
    "resources",
];

impl PhoneApp {
    /// Shows the named state with the sample sessions held still.
    pub fn preview(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.paused = true;
        if name == "connect" {
            self.address
                .update(cx, |address, cx| address.set_text(ADDRESS, cx));
            cx.notify();
            return;
        }
        self.open_store(ADDRESS);
        let finish = |this: &mut Self| {
            if let Some(store) = &mut this.store {
                store.tick(Duration::from_secs(10));
                store.answer(QWEN, Answer::AllowOnce);
                store.tick(Duration::from_secs(10));
            }
        };
        match name {
            "sessions" | "search" => {
                if let Some(store) = &mut self.store {
                    store.tick(Duration::from_secs(10));
                }
                self.entered_preview(window, cx);
                if name == "search" {
                    self.searching = true;
                    self.search
                        .update(cx, |search, cx| search.set_text("qwen", cx));
                }
            }
            "start" => self.push(Route::Start, window, cx),
            "working" => self.show_session(QWEN, window, cx),
            "waiting" => {
                if let Some(store) = &mut self.store {
                    store.tick(Duration::from_secs(10));
                }
                self.show_session(QWEN, window, cx);
                self.choice = Some(Answer::AllowOnce);
            }
            "done" | "evening" => {
                finish(self);
                if name == "evening" {
                    self.update_prefs(cx, |prefs| prefs.appearance = Appearance::Evening);
                    self.apply_theme(window, cx);
                }
                self.show_session(QWEN, window, cx);
            }
            "review" => {
                finish(self);
                self.show_session(QWEN, window, cx);
                self.open_review(QWEN, 0, window, cx);
                let picked: Vec<_> =
                    self.store
                        .as_ref()
                        .and_then(|store| store.session(QWEN))
                        .and_then(|session| session.files.first())
                        .map(|file| {
                            file.hunks
                                .iter()
                                .enumerate()
                                .flat_map(|(hunk, lines)| {
                                    lines.lines.iter().enumerate().filter_map(
                                        move |(index, line)| {
                                            (line.kind != LineKind::Context
                                                && (211..=212).contains(&line.number))
                                            .then_some((hunk, index))
                                        },
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                for (hunk, line) in picked {
                    self.toggle_line(QWEN, hunk, line, cx);
                }
                self.review.update(cx, |review, cx| {
                    review.set_text("Also accept it for DeepSeek", cx)
                });
            }
            "typing" => {
                finish(self);
                self.show_session(QWEN, window, cx);
                let composer = self.thread_composer(QWEN, window, cx);
                composer.update(cx, |composer, cx| {
                    composer.attach(
                        Attachment::File {
                            name: "build.log".into(),
                            size: "12 KB".into(),
                        },
                        cx,
                    );
                    composer.set_text("Same for DeepSeek, see the log. Start with @deep", cx);
                    composer.focus(window, cx);
                });
            }
            "details" => {
                finish(self);
                self.show_session(QWEN, window, cx);
                self.open_sheet(Sheet::Details(QWEN), cx);
            }
            "settings" => self.push(Route::Settings, window, cx),
            "model" => {
                self.push(Route::Start, window, cx);
                self.open_sheet(Sheet::Model, cx);
            }
            "attach" => {
                self.push(Route::Start, window, cx);
                self.open_sheet(Sheet::Attach(Target::Start), cx);
            }
            "project" => {
                self.push(Route::Start, window, cx);
                self.open_sheet(Sheet::Project, cx);
            }
            "more" => {
                self.show_session(QWEN, window, cx);
                self.open_sheet(Sheet::More(QWEN), cx);
            }
            "models" | "resources" => {
                self.push(Route::Settings, window, cx);
                self.open_sheet(
                    if name == "models" {
                        Sheet::Models
                    } else {
                        Sheet::Resources
                    },
                    cx,
                );
            }
            _ => log::warn!("No preview named {name}; showing the sessions"),
        }
        cx.notify();
    }

    fn entered_preview(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.routes = vec![Route::Sessions];
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn every_screen_renders(cx: &mut TestAppContext) {
        for name in SCREENS {
            let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
            cx.update(|window, cx| app.update(cx, |app, cx| app.preview(name, window, cx)));
            cx.run_until_parked();
            app.read_with(cx, |app, _| {
                let expected = match *name {
                    "connect" => Route::Connect,
                    "sessions" | "search" => Route::Sessions,
                    "start" | "model" | "attach" | "project" => Route::Start,
                    "review" => Route::Review(QWEN),
                    "settings" | "models" | "resources" => Route::Settings,
                    _ => Route::Thread(QWEN),
                };
                assert_eq!(app.route(), expected, "{name}");
            });
        }
    }

    #[gpui::test]
    fn back_closes_the_sheet_then_the_screen(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("details", window, cx)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.sheet.is_some());
                assert!(app.back(window, cx));
                assert!(app.sheet.is_none());
                assert!(app.back(window, cx));
                assert_eq!(app.route(), Route::Sessions);
                assert!(!app.back(window, cx), "the first screen leaves the app");
            })
        });
    }

    #[gpui::test]
    fn a_notification_link_answers_and_opens_the_session(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("sessions", window, cx);
                app.open_url("pi://session/1/question/allow-once", window, cx);
                assert_eq!(app.route(), Route::Thread(QWEN));
                assert!(app.sheet.is_none());
                let session = app.store.as_ref().unwrap().session(QWEN).unwrap();
                assert_eq!(session.state, crate::model::State::Working);
            })
        });
    }
}
