//! The app in named states that match design/android/screens, for the desktop
//! preview window (`examples/preview.rs`) and the render test. The sample
//! sessions stay still in a preview.

use crate::{
    app::{PhoneApp, Resume, Route, Sheet, Target},
    composer::Attachment,
    model::{
        Answer, Flow, LineKind, SessionId, StageKind, StageStatus, State, Summary, ToolActivity,
        Turn,
    },
    theme::Appearance,
};
use base64::Engine as _;
use gpui::{Context, Window};
use std::time::Duration;

const ADDRESS: &str = "nick@studio-mac.local";
const QWEN: SessionId = SessionId(1);

/// Every state `PhoneApp::preview` knows.
pub const SCREENS: &[&str] = &[
    "connect",
    "reconnecting",
    "unreachable",
    "sessions",
    "computers",
    "search",
    "start",
    "working",
    "waiting",
    "done",
    "many-files",
    "review",
    "typing",
    "notice",
    "details",
    "evening",
    "settings",
    "model",
    "model-long-list",
    "model-no-match",
    "thinking",
    "mentions",
    "image-input",
    "image-only",
    "attach",
    "more",
    "project",
    "models",
    "resources",
    "long-input",
    "long-reply",
    "streaming-reply",
    "long-labels",
    "empty-search",
    "failed",
    "stopped",
    "activity",
    "markdown",
    "page",
    "html-page",
    "history",
    "multi-turn",
    "tool-output",
    "follow-up-input",
    "delete",
    "delete-running",
    "projects",
    "project-empty",
    "project-error",
    "project-loading",
    "project-long-path",
    "project-tree",
    "project-search",
    "project-file",
    "tool-image",
    "media-sample",
    "subagents",
    "subagents-many",
    "subagents-done",
    "subagents-directory",
    "subagents-directory-done",
    "subagents-directory-stopped",
    "subagents-directory-failed",
    "subagents-directory-empty",
    "subagent",
    "logs",
];

impl PhoneApp {
    /// Shows the named state with the sample sessions held still.
    pub fn preview(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "ui-test")]
        if cx.has_global::<crate::testing::State>() {
            let state = cx.global_mut::<crate::testing::State>();
            state.fixture = name.to_owned();
            state.bounds.clear();
        }
        // A cold launch may start reconnecting from saved preferences before
        // Android delivers this explicit preview URL. Its late result must not
        // replace the fixture's store or navigation.
        self.cancel_connection_attempt();
        self.paused = true;
        self.playing = false;
        self.routes = vec![Route::Connect];
        self.sheet = None;
        self.closing_sheet = None;
        self.sheet_motion = crate::motion::SwipeMotion::at(1.);
        self.expanded_turns.clear();
        self.searching = false;
        self.preview_models.clear();
        self.expanded.clear();
        self.notice = None;
        self.update_prefs(cx, |prefs| {
            *prefs = crate::prefs::Prefs::default();
            prefs.appearance = Appearance::Moonstone;
        });
        self.apply_return_sends(cx);
        self.start.update(cx, |composer, cx| composer.clear(cx));
        self.search.update(cx, |area, cx| area.set_text("", cx));
        self.apply_theme(window, cx);
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        self.manual_setup = false;
        if name == "connect" {
            self.address
                .update(cx, |address, cx| address.set_text(ADDRESS, cx));
            cx.notify();
            return;
        }
        if let Some(failed) = match name {
            "reconnecting" => Some(false),
            "unreachable" => Some(true),
            _ => None,
        } {
            self.resuming = Some(Resume {
                address: ADDRESS.into(),
                attempt: if failed { 3 } else { 2 },
                retry_at: failed.then(|| std::time::Instant::now() + Duration::from_secs(30)),
            });
            self.connecting = !failed;
            self.connect_error =
                failed.then(|| format!("Connecting to {ADDRESS}:22 timed out after 15 s").into());
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
            "mentions" | "image-input" | "image-only" => {
                self.push(Route::Start, window, cx);
                self.start.update(cx, |composer, cx| {
                    if name == "mentions" {
                        composer.set_text("Read @src/main.rs and @\"my folder/中文.rs\". Compare with @README.md, then explain the changes.", cx);
                    } else {
                        let pixels = image::ImageBuffer::from_fn(96, 96, |x, y| image::Rgb([
                            if x < 48 { 240 } else { 20 }, if y < 48 { 60 } else { 210 }, 100,
                        ]));
                        let mut png = std::io::Cursor::new(Vec::new());
                        image::DynamicImage::ImageRgb8(pixels).write_to(&mut png, image::ImageFormat::Png).unwrap();
                        let image = crate::attachments::prepare_image("Test image.png".into(), &png.into_inner()).unwrap();
                        composer.try_attach(image, cx).unwrap();
                        if name == "image-input" { composer.set_text("Describe this image and compare it with @README.md", cx); }
                    }
                });
            }
            "projects" | "project-empty" | "project-error" | "project-loading"
            | "project-long-path" => {
                self.routes = vec![Route::Sessions, Route::Projects];
                let home = crate::projects::SAMPLE_HOME;
                match name {
                    "project-empty" => {
                        self.store.as_mut().unwrap().projects.clear();
                        self.go_to_folder(format!("{home}/repos"), cx);
                    }
                    "project-long-path" => {
                        let path = format!("{home}/{}", "a-very-long-folder-name/".repeat(12));
                        self.go_to_folder(path.trim_end_matches('/').to_owned(), cx);
                    }
                    _ => self.open_project_browser(true, cx),
                }
                let browser = &mut self.project_browser;
                match name {
                    "project-error" => {
                        let path = format!("{home}/Documents");
                        browser.open.insert(path.clone());
                        browser.fail(&path, "Cannot read this folder: permission denied. Choose another location or retry.");
                    }
                    "project-loading" => {
                        let path = format!("{home}/Desktop");
                        browser.open.insert(path.clone());
                        browser.wait(&path);
                    }
                    _ => {}
                }
            }
            "project-tree" | "project-search" | "project-file" => {
                self.push(Route::Start, window, cx);
                self.open_sheet(Sheet::Project, cx);
                let pi = format!("{}/repos/pi", crate::projects::SAMPLE_HOME);
                for folder in ["packages", "packages/ai", "packages/ai/src"] {
                    self.project_browser.open.insert(format!("{pi}/{folder}"));
                }
                match name {
                    "project-search" => {
                        self.folder.update(cx, |area, cx| area.set_text("prov", cx))
                    }
                    "project-file" => {
                        let node = self
                            .project_browser
                            .find("retry.ts")
                            .into_iter()
                            .next()
                            .unwrap();
                        self.open_file(&node, window, cx);
                    }
                    _ => {}
                }
            }
            "follow-up-input" => {
                finish(self);
                self.show_session(QWEN, window, cx);
                let composer = self.thread_composer(QWEN, window, cx);
                composer.update(cx, |composer, cx| composer.set_text(&long_text(), cx));
            }
            "delete" | "delete-running" => {
                if name == "delete" {
                    finish(self);
                }
                self.open_sheet(Sheet::Delete(QWEN), cx);
            }
            "logs" => {
                log::info!("Showing the debug log");
                log::warn!("A sample warning, so its colour shows");
                self.routes = vec![Route::Sessions, Route::Settings];
                self.open_sheet(Sheet::Logs, cx);
            }
            "computers" => {
                self.entered_preview(window, cx);
                self.open_sheet(Sheet::Computers, cx);
            }
            "activity" | "tool-output" => {
                finish(self);
                if name == "tool-output"
                    && let Some(session) = self
                        .store
                        .as_mut()
                        .and_then(|store| store.sessions.iter_mut().find(|s| s.id == QWEN))
                {
                    let stage = &mut session.turns[0].stages[2];
                    stage.tools.push(ToolActivity {
                        id: "test-command".into(),
                        name: "bash".into(),
                        target: "cargo test --workspace --all-features".into(),
                        output: format!(
                            "{}\nFINAL: 120 tests passed",
                            (1..=120)
                                .map(|i| format!("test {i}: passed — café 中文"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        ),
                        finished: true,
                        failed: false,
                    });
                }
                self.show_session(QWEN, window, cx);
                self.open_sheet(
                    Sheet::Activity(
                        QWEN,
                        0,
                        if name == "tool-output" {
                            StageKind::Verify
                        } else {
                            StageKind::Understand
                        },
                    ),
                    cx,
                );
            }
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
            "long-input" => {
                self.push(Route::Start, window, cx);
                self.start
                    .update(cx, |composer, cx| composer.set_text(&long_text(), cx));
            }
            "long-labels" => {
                let label = "a-project-with-a-very-long-name-and-no-short-alias";
                if let Some(store) = &mut self.store {
                    store.computer.name = label.repeat(3);
                    store.projects[0].name = label.repeat(3);
                    store.projects[0].folder = format!("/projects/{}/src", label.repeat(5));
                }
                self.update_prefs(cx, |prefs| prefs.model = label.repeat(3));
                self.push(Route::Start, window, cx);
            }
            "empty-search" => {
                self.entered_preview(window, cx);
                self.searching = true;
                self.search.update(cx, |area, cx| {
                    area.set_text("no-matching-session-".repeat(20), cx)
                });
            }
            "long-reply" | "streaming-reply" | "failed" | "stopped" | "markdown" | "page"
            | "multi-turn" => {
                finish(self);
                if let Some(session) = self
                    .store
                    .as_mut()
                    .and_then(|store| store.sessions.iter_mut().find(|s| s.id == QWEN))
                {
                    session.state = match name {
                        "streaming-reply" => State::Working,
                        "failed" => State::Failed,
                        "stopped" => State::Stopped,
                        _ => State::Done,
                    };
                    let turn = session.turns.last_mut().unwrap();
                    turn.prompt = "Please explain each change and include the entire reply.".into();
                    let source = if matches!(name, "markdown" | "multi-turn") {
                        let svg = base64::engine::general_purpose::STANDARD.encode(
                            br##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="96" viewBox="0 0 320 96"><rect width="320" height="96" rx="18" fill="#253d50"/><circle cx="54" cy="48" r="25" fill="#80bf95"/><path d="M105 29h166M105 48h128M105 67h148" stroke="#ebe7e4" stroke-width="9" stroke-linecap="round"/></svg>"##,
                        );
                        format!(
                            "## Ready to review\n\n**All changes are saved.** This is an *explanation* with `inline code` and ~~old text~~.\n\n1. Read the files.\n2. Run the checks.\n\n```rust\nfn main() {{\n    println!(\"hello, café 中文 👩🏽‍💻\");\n}}\n```\n\n![Generated SVG](data:image/svg+xml;base64,{svg})\n\n```mermaid\ngraph LR\n  Prompt --> Work\n  Work --> Review\n```\n\nSee the [documentation](https://example.com)."
                        )
                    } else {
                        long_text()
                    };
                    turn.summary = Some(Summary {
                        source: Some(source),
                        headline: "A complete, long reply".into(),
                        body: long_text(),
                    });
                    if name == "page" {
                        turn.pages = vec![pi_markdown::Page {
                            path: "demo/aurora.html".into(),
                            html: Some(format!(
                                "<!doctype html><html><head><title>Aurora</title></head><body style=\"font-family:sans-serif;padding:2rem\"><h1>Aurora</h1><p>A page Pi made.</p><button style=\"min-height:48px;padding:0 16px\" onclick=\"document.getElementById('js-status').textContent='JavaScript executed'\">Run interaction</button><p id=\"js-status\">Waiting for interaction</p>{}<p>End of the page</p></body></html>",
                                "<p>A long page scrolls under the finger.</p>".repeat(60)
                            )),
                        }];
                    }
                    if name == "streaming-reply" {
                        session.activity = "Writing the reply".into();
                        turn.stage_mut(StageKind::HandOff).status = StageStatus::Live;
                    }
                    if name == "multi-turn" {
                        let mut follow_up = Turn::new("Thanks. What changed?", "09:46");
                        follow_up.stages.clear();
                        follow_up.summary = Some(Summary {
                            headline: "Two files.".into(),
                            body: String::new(),
                            source: Some("Two files changed, and the tests passed.".into()),
                        });
                        session.turns.push(follow_up);
                        session.files.clear();
                        session.check = None;
                    }
                }
                self.show_session(QWEN, window, cx);
            }
            "working" => self.show_session(QWEN, window, cx),
            "waiting" => {
                if let Some(store) = &mut self.store {
                    store.tick(Duration::from_secs(10));
                }
                self.show_session(QWEN, window, cx);
                self.choice = Some(Answer::AllowOnce);
            }
            "html-page" => self.show_session(crate::model::SessionId(7), window, cx),
            "media-sample" => {
                finish(self);
                if let Some(shown) = self
                    .store
                    .as_mut()
                    .and_then(|store| store.sessions.iter_mut().find(|s| s.id == QWEN))
                {
                    let mut sample = media_sample();
                    sample.title = shown.title.clone();
                    *shown = sample;
                }
                self.show_session(QWEN, window, cx);
            }
            "subagents" | "subagents-many" | "subagents-done" | "subagent" => {
                finish(self);
                if let Some(store) = &mut self.store {
                    if let Some(shown) = store.sessions.iter_mut().find(|s| s.id == QWEN) {
                        let mut sample = subagent_sample(match name {
                            "subagents-done" => Crew::Reported,
                            "subagents-many" => Crew::Many,
                            _ => Crew::Working,
                        });
                        sample.title = "Provider retries".into();
                        *shown = sample;
                    }
                    store.sample_subagents.insert("12".into(), scout_sample());
                }
                self.show_session(QWEN, window, cx);
                if name == "subagent" {
                    self.open_subagent(
                        QWEN,
                        crate::app::Pick {
                            turn: 0,
                            handoff: 0,
                            index: 1,
                        },
                        window,
                        cx,
                    );
                }
            }
            "subagents-directory"
            | "subagents-directory-done"
            | "subagents-directory-stopped"
            | "subagents-directory-failed"
            | "subagents-directory-empty" => {
                finish(self);
                let empty = name == "subagents-directory-empty";
                if let Some(store) = &mut self.store {
                    if !empty && let Some(shown) = store.sessions.iter_mut().find(|s| s.id == QWEN)
                    {
                        let mut sample = subagent_directory_sample(match name {
                            "subagents-directory-done" => DirectoryCrew::Done,
                            "subagents-directory-stopped" => DirectoryCrew::Stopped,
                            "subagents-directory-failed" => DirectoryCrew::Failed,
                            _ => DirectoryCrew::Mixed,
                        });
                        sample.title = "Provider retries".into();
                        *shown = sample;
                    }
                    store.sample_subagents.insert("12".into(), scout_sample());
                }
                self.show_session(QWEN, window, cx);
                if empty {
                    self.open_sheet(Sheet::Details(QWEN), cx);
                } else {
                    self.push(Route::Subagents(QWEN), window, cx);
                }
            }
            "tool-image" => {
                finish(self);
                // A page screenshot: a header, a hero and three cards.
                let pixels = image::ImageBuffer::from_fn(640, 400, |x, y| {
                    let card = y > 250 && y < 370 && (x % 210) > 20 && (x % 210) < 200;
                    image::Rgb(match (y, card) {
                        (0..48, _) => [24, 28, 38],
                        (_, true) => [250, 250, 252],
                        (48..230, _) => [40 + (x / 8) as u8, 60 + (y / 4) as u8, 140],
                        _ => [232, 234, 240],
                    })
                });
                let mut png = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgb8(pixels)
                    .write_to(&mut png, image::ImageFormat::Png)
                    .unwrap();
                use base64::Engine as _;
                let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
                if let Some(turn) = self
                    .store
                    .as_mut()
                    .and_then(|store| store.sessions.iter_mut().find(|s| s.id == QWEN))
                    .and_then(|session| session.turns.last_mut())
                {
                    turn.images.push(crate::model::ToolImage {
                        key: "sample-screenshot".into(),
                        name: "localhost-3000.png".into(),
                        mime: "image/png".into(),
                        inline: Some(data),
                    });
                }
                self.show_session(QWEN, window, cx);
            }
            "done" => {
                finish(self);
                self.show_session(QWEN, window, cx);
            }
            "many-files" => {
                finish(self);
                if let Some(session) = self
                    .store
                    .as_mut()
                    .and_then(|store| store.sessions.iter_mut().find(|s| s.id == QWEN))
                {
                    let file = session.files[0].clone();
                    session.files.extend((1..=7).map(|n| {
                        let mut file = file.clone();
                        file.path = format!("packages/ai/src/providers/provider-{n}.ts");
                        file.added = n;
                        file.removed = n % 3;
                        file
                    }));
                }
                self.show_session(QWEN, window, cx);
            }
            "evening" => {
                self.update_prefs(cx, |prefs| prefs.appearance = Appearance::Evening);
                self.apply_theme(window, cx);
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
                            contents: "A sample build log for the UI fixture.".into(),
                        },
                        cx,
                    );
                    composer.set_text("Same for DeepSeek, see the log. Start with @deep", cx);
                    composer.focus(window, cx);
                });
            }
            "notice" => {
                finish(self);
                self.show_session(QWEN, window, cx);
                self.notify_user(
                    "This session is still using an older helper. Start a new session to use manual compaction; your draft has been kept.",
                    cx,
                );
            }
            "details" => {
                finish(self);
                self.show_session(QWEN, window, cx);
                self.open_sheet(Sheet::Details(QWEN), cx);
            }
            "history" => {
                finish(self);
                self.show_session(QWEN, window, cx);
                self.open_history(QWEN, window, cx);
            }
            "settings" => self.push(Route::Settings, window, cx),
            "model" | "model-long-list" | "model-no-match" | "thinking" => {
                self.push(Route::Start, window, cx);
                if name == "model-long-list" || name == "model-no-match" {
                    self.preview_models = (1..=150)
                        .map(|i| pi_core::protocol::Model {
                            id: format!("model-{i:03}"),
                            name: Some(format!("Research model {i:03} · 中文 café")),
                            provider: if i % 2 == 0 {
                                "North".into()
                            } else {
                                "South".into()
                            },
                            ..Default::default()
                        })
                        .collect();
                }
                self.open_sheet(
                    if name == "thinking" {
                        Sheet::Thinking
                    } else {
                        Sheet::Model
                    },
                    cx,
                );
                if name == "model-no-match" {
                    self.model_search
                        .update(cx, |area, cx| area.set_text("nonexistent-model", cx));
                }
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

    #[cfg(feature = "ui-test")]
    pub(crate) fn write_fixture_state(&self, cx: &gpui::App) {
        if !self.paused
            || self.prefs_path.is_none()
            || self.store.as_ref().is_some_and(|store| !store.is_sample())
        {
            return;
        }
        let Some(probes) = cx.try_global::<crate::testing::State>() else {
            return;
        };
        let session = self
            .model_session()
            .and_then(|id| self.store.as_ref()?.session(id));
        let composer = match self.route() {
            Route::Thread(id) => self.threads.get(&id),
            Route::Review(_) => Some(&self.review),
            _ => Some(&self.start),
        };
        let (model, thinking) = self.model_settings(cx);
        let snapshot = serde_json::json!({
            "fixture": probes.fixture,
            "time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64(),
            "scale": probes.scale, "bounds": probes.bounds, "viewport": probes.viewport,
            "pointer": probes.pointer,
            "route": format!("{:?}", self.route()), "sheet": self.sheet.map(|s| format!("{s:?}")),
            "project": self.store.as_ref().and_then(|s| s.projects.get(self.project)).map(|p| p.path.clone()),
            "picked": self.project_browser.picked.clone(),
            "open_folders": self.project_browser.open.len(),
            "file": self.file_view.as_ref().map(|view| view.path.clone()),
            "start_draft": self.start.read(cx).area.read(cx).text().to_owned(),
            "sample": self.store.as_ref().is_some_and(|store| store.is_sample()),
            "draft_chars": composer.map(|c| c.read(cx).area.read(cx).text().chars().count()),
            "text_menu_open": composer.is_some_and(|c| c.read(cx).area.read(cx).menu_open()),
            "draft_attachments": composer.map(|c| c.read(cx).attachments().len()),
            "draft_images": composer.map(|c| c.read(cx).attachments().iter().filter(|a| matches!(a, Attachment::Image { .. })).count()),
            "model_search_chars": self.model_search.read(cx).text().chars().count(),
            "model": model, "thinking": thinking,
            "state": session.map(|s| format!("{:?}", s.state)),
            "sessions": self.store.as_ref().map(|s| s.sessions.iter().map(|s| s.id.0).collect::<Vec<_>>()),
            "turns": session.map(|s| s.turns.len()),
            "expanded_activities": self.expanded_turns.values().filter(|expanded| **expanded).count(),
            "prompt_chars": session.and_then(|s| s.turns.last()).map(|t| t.prompt.chars().count()),
            "sheet_scroll": [f32::from(self.sheet_scroll.offset().y), f32::from(self.sheet_scroll.max_offset().y)],
        });
        if let Ok(bytes) = serde_json::to_vec(&snapshot) {
            let temporary = self.data_dir.join("ui-test-state.tmp");
            if std::fs::write(&temporary, bytes).is_ok() {
                let _ = std::fs::rename(temporary, self.data_dir.join("ui-test-state.json"));
            }
        }
    }
}

/// The sample both apps' tests draw (`pi_markdown::sample`), as a session
/// from the computer: pictures, raw SVG, a diagram and a page.
fn media_sample() -> crate::model::Session {
    use pi_markdown::sample;
    let mut pi = pi_core::session::Session::new(sample::CWD.into());
    pi.apply(&sample::record()).expect("valid media sample");
    crate::projection::project(
        &pi,
        crate::projection::Facts {
            id: QWEN,
            cwd: sample::CWD,
            folder: "~/repo".into(),
            question: None,
            outbox: &[],
            key: "0123456789abcdef0123456789abcdef",
        },
    )
}

enum Crew {
    /// Three scouts side by side, one done, while Pi waits for nothing.
    Working,
    /// Twenty-four side by side, most still to come.
    Many,
    /// A chain picked up after a restart, reported back and answered.
    Reported,
}

#[derive(Clone, Copy)]
enum DirectoryCrew {
    Mixed,
    Done,
    Stopped,
    Failed,
}

fn subagent_directory_sample(state: DirectoryCrew) -> crate::model::Session {
    use pi_core::subagent::{Handoff, Status};
    use serde_json::json;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut session = subagent_sample(Crew::Working);
    let handoff = Handoff::from_details(&json!({
        "version": 1,
        "mode": "parallel",
        "background": true,
        "results": [
            {"index":0,"agent":"planner","task":"Plan the shared retry helper","status":"waiting"},
            {"index":1,"agent":"worker","task":"Stop the duplicate retry paths","status":"stopped","conversationId":"31","now":"Stopped before editing","startedAt":now - 42_000,"endedAt":now - 18_000,"cost":0.03},
            {"index":2,"agent":"reviewer","task":"Run the provider regression checks","status":"failed","conversationId":"32","now":"Two provider tests failed","startedAt":now - 36_000,"endedAt":now - 12_000,"cost":0.04}
        ]
    }))
    .expect("directory handoff");
    let mut turn = Turn::new("Keep the hand-offs together.", "09:48");
    turn.stages.clear();
    turn.flow.push(Flow::Handoff(0));
    turn.handoffs.push(handoff);
    session.turns.push(turn);

    if !matches!(state, DirectoryCrew::Mixed) {
        for subagent in session
            .turns
            .iter_mut()
            .flat_map(|turn| &mut turn.handoffs)
            .flat_map(|handoff| &mut handoff.subagents)
        {
            subagent.status = Status::Done;
            subagent.now = Some(
                subagent
                    .output
                    .clone()
                    .unwrap_or_else(|| "Finished the task".into()),
            );
            subagent.ended_at.get_or_insert(now - 8_000);
        }
        let exceptional = session.turns[1].handoffs[0]
            .subagents
            .get_mut(1)
            .expect("sample subagent");
        match state {
            DirectoryCrew::Stopped => {
                exceptional.status = Status::Stopped;
                exceptional.now = Some("Stopped before editing".into());
            }
            DirectoryCrew::Failed => {
                exceptional.status = Status::Failed;
                exceptional.now = Some("Two provider tests failed".into());
            }
            DirectoryCrew::Mixed | DirectoryCrew::Done => {}
        }
    }
    session
}

fn subagent_sample(crew: Crew) -> crate::model::Session {
    use serde_json::json;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let running = !matches!(crew, Crew::Reported);
    let (args, details) = if let Crew::Many = crew {
        let areas = [
            "auth", "billing", "cache", "cli", "config", "db", "events", "export", "files", "http",
            "i18n", "import", "jobs", "logging", "mail", "metrics", "queue", "search", "settings",
            "storage", "sync", "ui", "users", "webhooks",
        ];
        let status = |n: usize| match n {
            0..=8 => "done",
            9..=16 => "running",
            _ => "waiting",
        };
        (
            json!({"tasks": areas.iter().map(|area| json!({"agent":"scout","task":format!("Review the {area} module")})).collect::<Vec<_>>()}),
            json!({"version":1,"mode":"parallel","background":true,"results": areas.iter().enumerate().map(|(n, area)| {
                let mut result = json!({"index":n,"agent":"scout","task":format!("Review the {area} module"),"status":status(n)});
                if status(n) != "waiting" {
                    result["conversationId"] = json!(format!("{}", 100 + n));
                    result["startedAt"] = json!(now - 90_000);
                    result["now"] = json!(if status(n) == "done" { format!("{area}: no blocking issues") } else { format!("Reading {area}.ts") });
                    result["cost"] = json!(0.004);
                }
                if status(n) == "done" {
                    result["endedAt"] = json!(now - 30_000 + n as u64 * 1000);
                }
                result
            }).collect::<Vec<_>>()}),
        )
    } else if running {
        (
            json!({"tasks":[
                {"agent":"scout","task":"Anthropic and Bedrock"},
                {"agent":"scout","task":"OpenAI and OpenCode"},
                {"agent":"scout","task":"Google and Mistral"}]}),
            json!({"version":1,"mode":"parallel","background":true,"results":[
                {"index":0,"agent":"scout","task":"Anthropic and Bedrock","status":"done","conversationId":"11","model":"anthropic/claude-haiku-4-5","now":"4 call sites, one backoff","output":"4 call sites, one backoff","startedAt":now - 62_000,"endedAt":now - 14_000,"cost":0.012},
                {"index":1,"agent":"scout","task":"OpenAI and OpenCode","status":"running","conversationId":"12","model":"anthropic/claude-haiku-4-5","now":"Searching for retryAfter","startedAt":now - 62_000,"cost":0.009},
                {"index":2,"agent":"scout","task":"Google and Mistral","status":"running","conversationId":"13","model":"anthropic/claude-haiku-4-5","now":"Reading google-gemini.ts","startedAt":now - 62_000,"cost":0.007}]}),
        )
    } else {
        (
            json!({"chain":[
                {"agent":"worker","task":"Build the plan: one withRetry in retry.ts"},
                {"agent":"reviewer","task":"Review the worker's changes: {previous}"}]}),
            json!({"version":1,"mode":"chain","resumed":true,"results":[
                {"index":0,"agent":"worker","task":"Build the plan: one withRetry in retry.ts","status":"done","conversationId":"21","model":"anthropic/claude-opus-5-5","now":"Moved the backoff into retry.ts; 5 providers call it","output":"Moved the backoff into retry.ts; 5 providers call it","startedAt":now - 400_000,"endedAt":now - 88_000,"cost":0.42,"interrupted":["pnpm test"]},
                {"index":1,"agent":"reviewer","task":"Review the worker's changes","status":"done","conversationId":"22","model":"anthropic/claude-sonnet-5-5","modelNote":"claude-sonnet-4-5 isn't set up on this computer; using anthropic/claude-sonnet-5-5","now":"No blocking issues; one naming nit","output":"No blocking issues; one naming nit","startedAt":now - 88_000,"endedAt":now - 22_000,"cost":0.11}]}),
        )
    };
    // The call returns at once; the subagents carry on, and report back when done.
    let mut messages = vec![
        json!({"role":"user","content":"Every provider retries differently. Use scouts to find how each one retries, then plan one shared helper.","timestamp":now - 80_000}),
        json!({"role":"assistant","content":[{"type":"toolCall","id":"hand","name":"subagent","arguments":args}],"timestamp":now - 76_000}),
        json!({"role":"toolResult","toolCallId":"hand","toolName":"subagent","content":[{"type":"text","text":"Started the subagents in the background."}],"details":details,"isError":false,"timestamp":now - 75_000}),
        json!({"role":"assistant","content":[{"type":"text","text":"The scouts are on it. I'll put their findings together when they report back; ask me anything meanwhile."}],"timestamp":now - 74_000}),
    ];
    if !running {
        messages.push(json!({"role":"user","content":"<subagent_report call=\"hand\">\nThe subagents you started have finished. This is their report, not a message from the user.\n\nNo blocking issues; one naming nit\n</subagent_report>","timestamp":now - 20_000}));
        messages.push(json!({"role":"assistant","content":[{"type":"text","text":"The worker moved every provider onto one `withRetry`, and the reviewer found no blocking issues."}],"timestamp":now - 18_000}));
    }
    let mut pi = pi_core::session::Session::new("/repo".into());
    pi.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":messages}}))
        .unwrap();
    crate::projection::project(
        &pi,
        crate::projection::Facts {
            id: QWEN,
            cwd: "/repo",
            folder: "~/repo".into(),
            question: None,
            outbox: &[],
            key: "0123456789abcdef0123456789abcdef",
        },
    )
}

/// The second scout's own run, as `get_subagent` sends it.
fn scout_sample() -> pi_core::session::Session {
    pi_core::subagent::session(
        &serde_json::json!({"busy":true,"messages":[
            {"role":"user","content":"Find every place the OpenAI and OpenCode providers retry a request. For each, note what triggers it, the delay, the limit, and whether it honours Retry-After. Don't change files."},
            {"role":"assistant","content":[
                {"type":"toolCall","id":"r1","name":"read","arguments":{"path":"/repo/src/openai-completions.ts"}},
                {"type":"toolCall","id":"r2","name":"read","arguments":{"path":"/repo/src/openai-responses.ts"}},
                {"type":"toolCall","id":"r3","name":"read","arguments":{"path":"/repo/src/opencode.ts"}}]},
            {"role":"toolResult","toolCallId":"r1","toolName":"read","content":[{"type":"text","text":"…"}],"isError":false},
            {"role":"toolResult","toolCallId":"r2","toolName":"read","content":[{"type":"text","text":"…"}],"isError":false},
            {"role":"toolResult","toolCallId":"r3","toolName":"read","content":[{"type":"text","text":"…"}],"isError":false},
            {"role":"assistant","content":[{"type":"toolCall","id":"b1","name":"bash","arguments":{"command":"rg -n \"retryAfter|retry-after\" src"}}]}
        ],"tools":[{"callId":"b1","name":"bash","output":"src/openai-completions.ts:88: retryAfter\nsrc/opencode.ts:41: retry-after\n"}]}),
        "/repo".into(),
    )
    .unwrap()
}

fn long_text() -> String {
    let paragraphs = (1..=40).map(|number| format!(
        "Paragraph {number:02}: Check a long reply with short words, a long path /projects/a-very-long-project-name/src/deeply/nested/module/file.rs, Unicode café 中文 日本語 👩🏽‍💻, and punctuation. Every paragraph must remain readable and editable."
    )).collect::<Vec<_>>().join("\n\n");
    format!(
        "START OF LONG TEXT\n\n{paragraphs}\n\nEND OF LONG TEXT — all forty paragraphs are present."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Focusable, TestAppContext};

    #[gpui::test]
    fn preview_fixtures_never_replace_persisted_phone_settings(cx: &mut TestAppContext) {
        let directory =
            std::env::temp_dir().join(format!("pi-android-preview-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        let persisted = crate::prefs::Prefs {
            computer: Some("nick@real-computer.local".into()),
            sample: true,
            host_keys: [("nick@real-computer.local".into(), "SHA256:real".into())]
                .into_iter()
                .collect(),
            ..crate::prefs::Prefs::default()
        };
        persisted.save(&path);
        let original = std::fs::read(&path).unwrap();

        let (app, cx) =
            cx.add_window_view(|window, cx| PhoneApp::new(Some(path.clone()), window, cx));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("evening", window, cx)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("long-labels", window, cx)));
        cx.run_until_parked();

        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_dir_all(directory).ok();
    }

    #[gpui::test]
    fn a_different_computer_leaves_reconnecting_for_setup(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("unreachable", window, cx)));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let resume = app.resuming.as_ref().expect("reconnecting");
            assert!(resume.retry_at.is_some() && app.connect_error.is_some());
        });
        cx.update(|_, cx| app.update(cx, |app, cx| app.leave_resume(cx)));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.resuming.is_none() && app.connect_error.is_none());
            assert_eq!(app.route(), Route::Connect);
        });
    }

    #[gpui::test]
    fn failed_compaction_restores_its_command_without_overwriting_new_text(
        cx: &mut TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("done", window, cx)));
        cx.run_until_parked();
        let id = app.read_with(cx, |app, _| match app.route() {
            Route::Thread(id) => id,
            route => panic!("expected thread, got {route:?}"),
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.restore_compact_draft(Target::Thread(id), "/compact keep API names", cx)
            })
        });
        app.read_with(cx, |app, cx| {
            assert_eq!(
                app.threads[&id].read(cx).area.read(cx).text(),
                "/compact keep API names"
            );
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.threads[&id].update(cx, |composer, cx| composer.set_text("new draft", cx));
                app.restore_compact_draft(Target::Thread(id), "/compact old", cx);
            })
        });
        app.read_with(cx, |app, cx| {
            assert_eq!(app.threads[&id].read(cx).area.read(cx).text(), "new draft");
        });
    }

    #[gpui::test]
    fn every_screen_renders(cx: &mut TestAppContext) {
        for (width, height) in [(320., 640.), (384., 854.), (640., 360.)] {
            for name in SCREENS {
                let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
                cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
                cx.update(|window, cx| app.update(cx, |app, cx| app.preview(name, window, cx)));
                cx.run_until_parked();
                app.read_with(cx, |app, _| {
                    let expected = match *name {
                        "connect" | "reconnecting" | "unreachable" => Route::Connect,
                        "sessions" | "computers" | "search" | "empty-search" | "delete"
                        | "delete-running" => Route::Sessions,
                        "start" | "model" | "model-long-list" | "model-no-match" | "thinking"
                        | "mentions" | "image-input" | "image-only" | "attach" | "project"
                        | "long-input" | "long-labels" | "project-tree" | "project-search" => {
                            Route::Start
                        }
                        "project-file" => Route::File,
                        "review" => Route::Review(QWEN),
                        "html-page" => Route::Thread(crate::model::SessionId(7)),
                        "history" => Route::History(QWEN),
                        "subagents-directory"
                        | "subagents-directory-done"
                        | "subagents-directory-stopped"
                        | "subagents-directory-failed" => Route::Subagents(QWEN),
                        "subagent" => Route::Subagent(
                            QWEN,
                            crate::app::Pick {
                                turn: 0,
                                handoff: 0,
                                index: 1,
                            },
                        ),
                        "projects" | "project-empty" | "project-error" | "project-loading"
                        | "project-long-path" => Route::Projects,
                        "settings" | "models" | "resources" | "logs" => Route::Settings,
                        "subagents-directory-empty" => Route::Thread(QWEN),
                        _ => Route::Thread(QWEN),
                    };
                    assert_eq!(app.route(), expected, "{name}");
                });
            }
        }
    }

    /// A big crew shows its first few, all on request, or just its header.
    #[gpui::test]
    fn a_crew_folds(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("subagents-many", window, cx)));
        cx.run_until_parked();
        let rows = |cx: &mut gpui::VisualTestContext| {
            (0..24)
                .filter(|n| {
                    cx.debug_bounds(format!("subagent-0-0-{n}").leak())
                        .is_some()
                })
                .count()
        };
        assert_eq!(rows(cx), 5);
        let all = cx.debug_bounds("handoff-all-0-0").unwrap();
        cx.simulate_click(all.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(rows(cx), 24);
        let header = cx.debug_bounds("handoff-0-0").unwrap();
        cx.simulate_click(header.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(rows(cx), 0);
    }

    /// Work handed to subagents shows as a card whose rows open each one.
    #[gpui::test]
    fn a_subagent_opens_from_its_row(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("subagents", window, cx)));
        cx.run_until_parked();
        for row in ["subagent-0-0-0", "subagent-0-0-1", "subagent-0-0-2"] {
            assert!(cx.debug_bounds(row).is_some(), "{row} is not shown");
        }
        let row = cx.debug_bounds("subagent-0-0-1").unwrap();
        cx.simulate_click(row.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.route(),
                Route::Subagent(
                    QWEN,
                    crate::app::Pick {
                        turn: 0,
                        handoff: 0,
                        index: 1
                    }
                )
            )
        });
        assert!(
            cx.debug_bounds("stop-subagent").is_some(),
            "a running subagent can be stopped"
        );
    }

    /// Details exposes one live session-wide directory, whose rows keep their behavior.
    #[gpui::test]
    fn details_opens_the_ordered_subagent_directory(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(320.), gpui::px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("subagents-directory", window, cx);
                app.back(window, cx);
                app.open_sheet(Sheet::Details(QWEN), cx);
                app.sheet_motion.finish();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let session = app.store.as_ref().unwrap().session(QWEN).unwrap();
            assert_eq!(app.route(), Route::Thread(QWEN));
            assert_eq!(app.sheet, Some(Sheet::Details(QWEN)));
            assert_eq!(
                crate::model::subagent_summary(&session.turns).as_deref(),
                Some("2 working · 1 waiting · 3 finished (1 stopped, 1 failed)")
            );
            assert_eq!(
                crate::model::subagent_details_summary(&session.turns).as_deref(),
                Some("6 total · 2 working · 1 waiting")
            );
        });
        let details = cx
            .debug_bounds("details-subagents")
            .expect("sessions with subagents show the Details row");
        let sheet = cx.debug_bounds("bottom-sheet").unwrap();
        let summary = cx.debug_bounds("details-subagents-value").unwrap();
        let chevron = cx.debug_bounds("details-subagents-chevron").unwrap();
        assert!(
            details.left() >= sheet.left() && details.right() <= sheet.right(),
            "Details row is outside the sheet: row={details:?} sheet={sheet:?}"
        );
        assert!(
            summary.left() >= details.left()
                && summary.right() <= chevron.left()
                && chevron.right() <= details.right(),
            "compact summary and chevron overflow the row: row={details:?} summary={summary:?} chevron={chevron:?}"
        );
        assert_eq!(
            chevron.size.width,
            gpui::px(16.),
            "the disclosure chevron must not shrink"
        );
        cx.simulate_click(details.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.route(),
                Route::Subagents(QWEN),
                "row={details:?} sheet={sheet:?} sheet_state={:?}",
                app.sheet
            )
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.sheet_motion.finish();
                cx.notify();
            })
        });
        cx.run_until_parked();

        let bounds = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            cx.debug_bounds(selector).unwrap().top()
        };
        let ordered = [
            "directory-subagent-0-0-1",
            "directory-subagent-0-0-2",
            "directory-subagent-1-0-0",
            "directory-subagent-0-0-0",
            "directory-subagent-1-0-1",
            "directory-subagent-1-0-2",
        ];
        assert!(
            ordered
                .windows(2)
                .all(|pair| bounds(cx, pair[0]) < bounds(cx, pair[1]))
        );

        let running = cx.debug_bounds(ordered[0]).unwrap();
        cx.simulate_click(running.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.route(),
                Route::Subagent(
                    QWEN,
                    crate::app::Pick {
                        turn: 0,
                        handoff: 0,
                        index: 1,
                    }
                )
            )
        });
        assert!(
            cx.debug_bounds("stop-subagent").is_some(),
            "directory rows retain the existing stop action"
        );
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.back(window, cx));
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| assert_eq!(app.route(), Route::Subagents(QWEN)));
    }

    #[gpui::test]
    fn terminal_subagent_details_summaries_only_show_the_total(cx: &mut TestAppContext) {
        for (fixture, directory) in [
            ("subagents-directory-done", "6 subagents"),
            ("subagents-directory-stopped", "6 subagents · 1 stopped"),
            ("subagents-directory-failed", "6 subagents · 1 failed"),
        ] {
            let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
            cx.update(|window, cx| app.update(cx, |app, cx| app.preview(fixture, window, cx)));
            cx.run_until_parked();
            app.read_with(cx, |app, _| {
                let session = app.store.as_ref().unwrap().session(QWEN).unwrap();
                assert_eq!(
                    crate::model::subagent_summary(&session.turns).as_deref(),
                    Some(directory),
                    "{fixture} keeps terminal status detail in the directory"
                );
                assert_eq!(
                    crate::model::subagent_details_summary(&session.turns).as_deref(),
                    Some("6 subagents"),
                    "{fixture} keeps the Details row compact"
                );
            });
        }
    }

    #[gpui::test]
    fn details_hides_subagents_for_an_empty_session(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("subagents-directory-empty", window, cx)
            })
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("bottom-sheet").is_some());
        assert!(cx.debug_bounds("details-subagents").is_none());
    }

    /// The shared sample (`pi_markdown::sample`), as a session from the
    /// computer: each kind of picture and page in it shows on the phone.
    #[gpui::test]
    fn the_shared_sample_shows_every_picture_and_page(cx: &mut TestAppContext) {
        use pi_markdown::sample::{self, Item};
        let session = media_sample();
        let turn = session.turns.last().unwrap();
        assert_eq!(turn.images[0].name, sample::TOOL_IMAGE_NAME);
        assert_eq!(turn.pages[0].path, sample::PAGE_PATH);
        assert!(
            turn.pages[0]
                .html
                .as_deref()
                .unwrap()
                .contains(sample::PAGE_BODY)
        );
        let reply = pi_markdown::blocks(turn.summary.as_ref().unwrap().source.as_deref().unwrap());
        let image = |format| {
            reply.iter().position(|block| {
                matches!(&block.media, Some(pi_markdown::Media::Image(image)) if image.image.format == format)
            })
        };
        let selectors = |item| match item {
            Item::InlineSvg | Item::SvgBlock | Item::SvgMarkup => {
                let svgs: Vec<usize> = reply
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| {
                        matches!(&block.media, Some(pi_markdown::Media::Image(image)) if image.image.format == gpui::ImageFormat::Svg)
                    })
                    .map(|(index, _)| index)
                    .collect();
                format!("markdown-image-{}", svgs[sample::svg_order(item).unwrap()])
            }
            Item::InlinePng => format!("markdown-image-{}", image(gpui::ImageFormat::Png).unwrap()),
            Item::Mermaid => format!(
                "markdown-mermaid-{}",
                reply
                    .iter()
                    .position(|block| matches!(block.media, Some(pi_markdown::Media::Mermaid(_))))
                    .unwrap()
            ),
            Item::ToolImage => "tool-image-0-0".into(),
            Item::Page => "page-poster-0-0".into(),
        };

        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("media-sample", window, cx)));
        cx.run_until_parked();
        for item in sample::ITEMS {
            // `debug_bounds` takes a static name.
            let selector: &'static str = selectors(item).leak();
            assert!(
                cx.debug_bounds(selector).is_some(),
                "{item:?} ({selector}) is not shown"
            );
        }
    }

    #[gpui::test]
    fn new_session_does_not_open_the_row_underneath(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(640.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("sessions", window, cx)));
        cx.run_until_parked();
        let button = cx.debug_bounds("new-session").expect("floating button");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.routes, [Route::Sessions, Route::Start])
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.back(window, cx);
            })
        });
        app.read_with(cx, |app, _| assert_eq!(app.route(), Route::Sessions));
    }

    fn swipe(
        cx: &mut gpui::VisualTestContext,
        from: gpui::Point<gpui::Pixels>,
        to: gpui::Point<gpui::Pixels>,
    ) {
        drag(cx, from, to, gpui::TouchPhase::Ended);
    }

    fn drag(
        cx: &mut gpui::VisualTestContext,
        from: gpui::Point<gpui::Pixels>,
        to: gpui::Point<gpui::Pixels>,
        finish: gpui::TouchPhase,
    ) {
        use gpui::InputEvent;
        for step in 0..=12 {
            let event = gpui::TouchEvent {
                id: gpui::TouchId(77),
                phase: if step == 0 {
                    gpui::TouchPhase::Started
                } else if step == 12 {
                    finish
                } else {
                    gpui::TouchPhase::Moved
                },
                position: from + (to - from) * (step as f32 / 12.),
                ..Default::default()
            };
            cx.update(|window, cx| {
                window.dispatch_event(event.to_platform_input(), cx);
            });
            cx.run_until_parked();
        }
    }

    #[gpui::test]
    fn project_choice_follows_connection_and_never_starts_a_session(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("projects", window, cx)));
        cx.run_until_parked();
        let count = app.read_with(cx, |app, _| app.store.as_ref().unwrap().sessions.len());
        let choice = cx.debug_bounds("project-choice-1").unwrap();
        cx.simulate_click(choice.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.route(), Route::Start);
            assert_eq!(app.project, 1);
            assert_eq!(app.store.as_ref().unwrap().sessions.len(), count);
        });
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("project-empty", window, cx)));
        cx.run_until_parked();
        let use_folder = cx.debug_bounds("use-folder").unwrap();
        cx.simulate_click(use_folder.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.route(), Route::Start);
            assert_eq!(
                app.store.as_ref().unwrap().projects[app.project].path,
                "/Users/nick/repos"
            );
            assert_eq!(app.store.as_ref().unwrap().sessions.len(), count);
        });
    }

    #[gpui::test]
    fn the_tree_rolls_out_opens_files_and_comes_back(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("project", window, cx)));
        cx.run_until_parked();
        let click = |cx: &mut gpui::VisualTestContext, name: &'static str| {
            let bounds = cx
                .debug_bounds(name)
                .unwrap_or_else(|| panic!("{name} is shown"));
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.run_until_parked();
        };
        // pi is rolled out and picked; its README is a file in it.
        assert!(cx.debug_bounds("node:~/repos/pi/README.md").is_some());
        assert!(cx.debug_bounds("node:~/repos/pi/packages/ai").is_none());
        click(cx, "roll:~/repos/pi/packages");
        assert!(cx.debug_bounds("node:~/repos/pi/packages/ai").is_some());
        click(cx, "roll:~/repos/pi/packages");
        assert!(cx.debug_bounds("node:~/repos/pi/packages/ai").is_none());
        click(cx, "node:~/repos/minivm");
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.project_browser.picked.as_deref(),
                Some("/Users/nick/repos/minivm")
            );
            assert_eq!(app.route(), Route::Start);
        });
        click(cx, "node:~/repos/pi/README.md");
        app.read_with(cx, |app, _| {
            assert_eq!(app.route(), Route::File);
            assert_eq!(app.file_view.as_ref().unwrap().path, "README.md");
        });
        cx.update(|window, cx| app.update(cx, |app, cx| app.back(window, cx)));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.route(), Route::Start);
            assert_eq!(app.sheet, Some(Sheet::Project));
        });
        let count = app.read_with(cx, |app, _| app.store.as_ref().unwrap().sessions.len());
        click(cx, "use-folder");
        app.read_with(cx, |app, _| {
            let store = app.store.as_ref().unwrap();
            assert_eq!(store.projects[app.project].path, "/Users/nick/repos/minivm");
            assert_eq!(store.sessions.len(), count);
            assert_eq!(app.sheet, None);
        });
    }

    #[gpui::test]
    fn partial_and_cancelled_swipes_do_not_delete_or_navigate(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        for (distance, finish) in [
            (50., gpui::TouchPhase::Ended),
            (240., gpui::TouchPhase::Cancelled),
        ] {
            let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
            cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
            cx.update(|window, cx| app.update(cx, |app, cx| app.preview("sessions", window, cx)));
            cx.run_until_parked();
            let row = cx.debug_bounds("session-row-2").unwrap();
            let from = gpui::point(gpui::px(45.), row.center().y);
            drag(
                cx,
                from,
                from + gpui::point(gpui::px(distance), gpui::px(0.)),
                finish,
            );
            app.read_with(cx, |app, _| {
                assert_eq!(app.route(), Route::Sessions);
                assert!(app.sheet.is_none());
                assert!(app.swiping_session.is_none());
                assert!(
                    app.store
                        .as_ref()
                        .unwrap()
                        .session(crate::model::SessionId(2))
                        .is_some()
                );
            });
        }
    }

    #[gpui::test]
    fn panels_remain_mounted_and_follow_the_finger_until_release(cx: &mut TestAppContext) {
        use gpui::InputEvent;
        for fixture in ["computers", "model"] {
            let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
            cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.preview(fixture, window, cx);
                    app.sheet_motion.finish();
                })
            });
            cx.run_until_parked();
            let bounds = cx.debug_bounds("bottom-sheet").unwrap();
            let from = gpui::point(bounds.center().x, bounds.top() + gpui::px(14.));
            let delta = gpui::point(gpui::px(0.), gpui::px(160.));
            for step in 0..=12 {
                cx.update(|window, cx| {
                    window.dispatch_event(
                        gpui::TouchEvent {
                            id: gpui::TouchId(91),
                            phase: if step == 0 {
                                gpui::TouchPhase::Started
                            } else if step == 12 {
                                gpui::TouchPhase::Ended
                            } else {
                                gpui::TouchPhase::Moved
                            },
                            position: from + delta * (step as f32 / 12.),
                            ..Default::default()
                        }
                        .to_platform_input(),
                        cx,
                    );
                });
                cx.run_until_parked();
                if step < 12 {
                    app.read_with(cx, |app, _| {
                        assert!(app.sheet.is_some(), "must wait for release")
                    });
                }
            }
            app.read_with(cx, |app, _| {
                assert!(app.sheet.is_none());
                assert!(app.closing_sheet.is_some());
                assert!(app.sheet_motion.animating());
            });
        }
    }

    #[gpui::test]
    fn follow_up_and_review_have_the_shared_model_picker(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        for name in ["done", "review", "follow-up-input"] {
            cx.update(|window, cx| app.update(cx, |app, cx| app.preview(name, window, cx)));
            cx.run_until_parked();
            let picker = cx.debug_bounds("composer-model").unwrap();
            let send = cx.debug_bounds("send").unwrap();
            assert!(send.bottom() < gpui::px(854.));
            cx.simulate_click(picker.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            app.read_with(cx, |app, _| assert_eq!(app.sheet, Some(Sheet::Model)));
            let choice = cx.debug_bounds("model-choice-1").unwrap();
            cx.simulate_click(choice.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            app.read_with(cx, |app, cx| {
                assert_eq!(app.model_settings(cx).0, "Sonnet 5.5");
                assert_eq!(
                    app.prefs(cx).model,
                    "Opus 5.5",
                    "session model is not a global default"
                );
            });
        }
    }

    #[gpui::test]
    fn swiping_right_confirms_before_deleting_and_does_not_open_the_row(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("delete", window, cx);
                app.close_sheet(cx);
            })
        });
        cx.run_until_parked();
        let row = cx.debug_bounds("session-row-1").unwrap();
        swipe(
            cx,
            gpui::point(gpui::px(45.), row.center().y),
            gpui::point(gpui::px(330.), row.center().y),
        );
        app.read_with(cx, |app, _| {
            assert_eq!(app.route(), Route::Sessions);
            assert_eq!(app.sheet, Some(Sheet::Delete(QWEN)));
            assert!(app.store.as_ref().unwrap().session(QWEN).is_some());
        });
        let confirm = cx.debug_bounds("confirm-delete").unwrap();
        cx.simulate_click(confirm.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                app.sheet.is_none(),
                "confirmation bounds: {confirm:?}; state: {:?}; notice: {:?}",
                app.store.as_ref().unwrap().session(QWEN).map(|s| s.state),
                app.notice
            );
            assert!(app.store.as_ref().unwrap().session(QWEN).is_none());
        });
    }

    #[gpui::test]
    fn bottom_sheets_close_by_swipe_without_navigation(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("computers", window, cx)));
        cx.run_until_parked();
        let sheet = cx.debug_bounds("bottom-sheet").unwrap();
        let from = gpui::point(sheet.center().x, sheet.top() + gpui::px(14.));
        swipe(cx, from, from + gpui::point(gpui::px(0.), gpui::px(150.)));
        app.read_with(cx, |app, _| {
            assert!(app.sheet.is_none());
            assert_eq!(app.route(), Route::Sessions);
        });
        // A fresh window isolates this second gesture from the first swipe's
        // simulated fling (both otherwise run in the same instant).
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("model", window, cx)));
        cx.run_until_parked();
        let sheet = cx.debug_bounds("bottom-sheet").unwrap();
        let from = gpui::point(sheet.center().x, sheet.top() + gpui::px(14.));
        swipe(cx, from, from + gpui::point(gpui::px(0.), gpui::px(150.)));
        app.read_with(cx, |app, _| {
            assert!(app.sheet.is_none());
            assert_eq!(app.route(), Route::Start);
        });
    }

    #[gpui::test]
    fn dragging_a_scrollbar_owns_the_touch_without_dismissing_its_sheet(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.preview("model-long-list", window, cx))
        });
        cx.run_until_parked();
        let viewport = app.read_with(cx, |app, _| app.sheet_scroll.bounds());
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-40.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let (from, to) = app.read_with(cx, |app, _| {
            let scroll = &app.sheet_scroll;
            let g = crate::scroll::Geometry::new(
                scroll.bounds(),
                scroll.max_offset().y,
                scroll.offset().y,
            )
            .unwrap();
            (
                g.thumb.center(),
                gpui::point(
                    g.thumb.center().x,
                    g.track.top() + g.track.size.height * 0.8,
                ),
            )
        });
        drag(cx, from, to, gpui::TouchPhase::Ended);
        app.read_with(cx, |app, _| {
            assert_eq!(app.sheet, Some(Sheet::Model));
            assert!(-app.sheet_scroll.offset().y > app.sheet_scroll.max_offset().y * 0.5);
        });
    }

    #[gpui::test]
    fn image_preview_and_removal_have_separate_touch_targets(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(320.), gpui::px(640.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("image-only", window, cx)));
        cx.run_until_parked();
        let preview = cx.debug_bounds("attachment-0").unwrap();
        let remove = cx.debug_bounds("remove-attachment-0").unwrap();
        let draft = cx.debug_bounds("composer-draft").unwrap();
        assert!(preview.right() <= remove.left());
        assert!(
            preview.bottom() <= draft.top(),
            "{preview:?} overlaps {draft:?}"
        );
        assert!(remove.size.width >= gpui::px(44.) && remove.size.height >= gpui::px(44.));
        cx.simulate_click(preview.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert!(matches!(app.sheet, Some(Sheet::Image(_, 0))));
            assert_eq!(app.start.read(cx).attachments().len(), 1);
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.back(window, cx);
            })
        });
        cx.run_until_parked();
        cx.simulate_click(remove.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert!(app.sheet.is_none());
            assert!(app.start.read(cx).attachments().is_empty());
            assert!(!app.start.read(cx).can_send(cx));
        });
    }

    #[gpui::test]
    fn notices_unroll_below_the_app_bar_wrap_and_dismiss(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(320.), gpui::px(640.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("image-input", window, cx);
                app.notify_user(
                    "This session is still using an older helper. Start a new session to use manual compaction; your draft has been kept.",
                    cx,
                );
            })
        });
        cx.run_until_parked();
        let drawer = cx.debug_bounds("notice-drawer").unwrap();
        assert!(drawer.left() >= gpui::px(12.));
        assert!(drawer.right() <= gpui::px(308.));
        assert!(drawer.top() >= gpui::px(68.));
        assert!(
            drawer.size.height > gpui::px(56.),
            "long notice should wrap: {drawer:?}"
        );
        cx.simulate_click(drawer.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| assert!(app.notice.is_none()));
        assert!(cx.debug_bounds("notice-drawer").is_none());
    }

    #[gpui::test]
    fn recovering_a_failed_prompt_preserves_bytes_and_never_overwrites_a_draft(
        cx: &mut TestAppContext,
    ) {
        use base64::Engine;
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("image-input", window, cx)));
        cx.update(|_, cx| {
            let composer = app.read(cx).start.clone();
            composer.update(cx, |composer, cx| {
                let Attachment::Image { image, .. } = &composer.attachments()[0] else {
                    panic!("image fixture")
                };
                let image = pi_core::protocol::ImageContent::new(
                    base64::engine::general_purpose::STANDARD.encode(&image.bytes),
                    image.format.mime_type(),
                );
                let prompt =
                    crate::prompt::Prompt::new("Recovered café 中文".into(), vec![image.clone()]);
                let existing = composer.area.read(cx).text().to_owned();
                assert!(composer.restore_prompt(&prompt, cx).is_err());
                assert_eq!(composer.area.read(cx).text(), existing);
                composer.clear(cx);
                let old_import = composer.begin_import(cx);
                assert!(composer.restore_prompt(&prompt, cx).is_err());
                composer.clear(cx);
                assert!(
                    !composer.finish_import(old_import, cx),
                    "a cleared import cannot refill the draft"
                );
                composer.restore_prompt(&prompt, cx).unwrap();
                assert_eq!(composer.area.read(cx).text(), prompt.message);
                let Attachment::Image {
                    image: restored, ..
                } = &composer.attachments()[0]
                else {
                    panic!("restored image")
                };
                assert_eq!(restored.format.mime_type(), image.mime_type);
                assert_eq!(
                    base64::engine::general_purpose::STANDARD.encode(&restored.bytes),
                    image.data
                );
                assert!(composer.can_send(cx));
            });
        });
    }

    #[gpui::test]
    fn model_search_receives_input_after_a_composer_had_focus(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("start", window, cx)));
        cx.run_until_parked();
        let at = cx.debug_bounds("composer-draft").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.simulate_input("Hello world");
        let at = cx.debug_bounds("composer-model").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        let at = cx.debug_bounds("model-search").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.simulate_input("Sonnet");
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(app.model_search.read(cx).text(), "Sonnet");
            assert_eq!(app.start.read(cx).area.read(cx).text(), "Hello world");
        });
    }

    #[gpui::test]
    fn the_empty_draft_padding_focuses_the_editor(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("start", window, cx)));
        cx.run_until_parked();
        let draft = cx.debug_bounds("composer-draft").unwrap();
        cx.simulate_click(draft.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.read(cx)
                .start
                .read(cx)
                .area
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        })
        .then_some(())
        .expect("padding is part of the editable touch target");
    }

    #[gpui::test]
    fn starter_cards_do_not_replace_a_draft_being_edited(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("start", window, cx);
                app.start.update(cx, |composer, cx| {
                    composer.set_text("My task", cx);
                    composer.focus(window, cx);
                });
            })
        });
        cx.run_until_parked();
        let starter = cx.debug_bounds("starter-1").unwrap();
        cx.simulate_click(starter.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(app.start.read(cx).area.read(cx).text(), "My task")
        });
    }

    #[gpui::test]
    fn sheet_scrim_does_not_activate_underlying_controls(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("model", window, cx)));
        cx.run_until_parked();
        // The close button for New session is behind the scrim.
        cx.simulate_click(
            gpui::point(gpui::px(28.), gpui::px(28.)),
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.sheet.is_none());
            assert_eq!(app.route(), Route::Start);
        });
    }

    #[gpui::test]
    fn long_labels_leave_send_inside_the_phone(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(320.), gpui::px(640.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("long-labels", window, cx)));
        cx.run_until_parked();
        let send = cx.debug_bounds("send").expect("send button");
        assert!(
            send.left() >= gpui::px(0.) && send.right() <= gpui::px(320.),
            "{send:?}"
        );
        assert!(send.bottom() <= gpui::px(640.), "{send:?}");
    }

    #[gpui::test]
    fn a_long_sheet_keeps_its_header_outside_the_scrolling_content(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("tool-output", window, cx)));
        cx.run_until_parked();
        let before = cx.debug_bounds("sheet-header").unwrap();
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.sheet_scroll
                    .set_offset(gpui::point(gpui::px(0.), gpui::px(-400.)));
                cx.notify();
            })
        });
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("sheet-header").unwrap(), before);
        assert!(before.top() >= gpui::px(0.) && before.bottom() <= gpui::px(854.));
    }

    #[gpui::test]
    fn finished_activity_expands_and_opens_details(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("done", window, cx)));
        cx.run_until_parked();
        let toggle = cx.debug_bounds("expand-turn").unwrap();
        cx.simulate_click(toggle.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.expanded_turns.get(&(QWEN, 0)), Some(&true))
        });
        let stage = cx.debug_bounds("stage-details").unwrap();
        cx.simulate_click(stage.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(matches!(app.sheet, Some(Sheet::Activity(QWEN, 0, _))))
        });
    }

    #[gpui::test]
    fn back_closes_the_computers_without_leaving_home(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("computers", window, cx);
                assert_eq!(app.sheet, Some(Sheet::Computers));
                assert!(app.back(window, cx));
                assert!(app.sheet.is_none());
                assert_eq!(app.route(), Route::Sessions);
            })
        });
    }

    #[gpui::test]
    fn reading_a_long_reply_does_not_snap_back_on_render(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(gpui::px(384.), gpui::px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("long-reply", window, cx)));
        cx.run_until_parked();
        let scroll = cx.update(|_, cx| app.update(cx, |app, _| app.scroll(Route::Thread(QWEN))));
        let reading = gpui::point(gpui::px(0.), -scroll.max_offset().y + gpui::px(30.));
        scroll.set_offset(reading);
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
        assert_eq!(
            scroll.offset(),
            reading,
            "even a small upward scroll belongs to the reader"
        );
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
