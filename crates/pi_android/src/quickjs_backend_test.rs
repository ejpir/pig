//! Feature-gated, offline-only backend proof inside the actual Android app UID.
//! Does not implement a Store/Live transport or expose provider credentials.
use crate::{
    app::PhoneApp,
    theme::theme,
    ui::{self, Button},
};
use gpui::{Context, SharedString, Window, div, prelude::*, px};
use pi_quickjs_backend_poc::app_fixture::{FixtureRun, Mode};
use serde_json::json;

#[derive(Default)]
pub(crate) struct Panel {
    pub visible: bool,
    opened: bool,
    frame: u64,
    pub run: Option<FixtureRun>,
    error: Option<String>,
}
impl Panel {
    pub fn cancel(&self, reason: &str) {
        if let Some(run) = &self.run {
            run.cancel(reason);
        }
    }
}

impl PhoneApp {
    pub(crate) fn quickjs_backend_url(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let action = match url {
            "pi://test/quickjs-backend" => "open",
            "pi://test/quickjs-backend/run" => "run",
            "pi://test/quickjs-backend/stall" => "stall",
            "pi://test/quickjs-backend/cancel" => "cancel",
            _ => return false,
        };
        // Defense in depth: a release APK must not expose this fixture even if its
        // optional Cargo feature was accidentally linked. Check links as well as UI.
        if !gpui_android::activity::is_debug_build() {
            self.notify_user(
                "The local backend fixture is available only in debug builds",
                cx,
            );
            return true;
        }
        if self.connecting
            || self.resuming.is_some()
            || self.store.as_ref().is_some_and(|store| !store.is_sample())
        {
            self.notify_user(
                "Open the backend fixture without an active remote connection",
                cx,
            );
            return true;
        }
        self.close_sheet(cx);
        self.close_expanded_visual(window, cx);
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        self.quickjs_backend_test.visible = true;
        self.quickjs_backend_test.opened = true;
        match action {
            "run" => self.start_quickjs_backend_test(Mode::RoundTrip, cx),
            "stall" => self.start_quickjs_backend_test(Mode::WaitForCancel, cx),
            "cancel" => self.quickjs_backend_test.cancel("stop"),
            _ => (),
        }
        cx.notify();
        true
    }

    fn start_quickjs_backend_test(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if !gpui_android::activity::is_debug_build() {
            self.quickjs_backend_test.error = Some("Available only in debug builds".into());
        } else if !self.visible || !self.window_active {
            self.quickjs_backend_test.error = Some("Bring the app to the foreground first".into());
        } else if self
            .quickjs_backend_test
            .run
            .as_ref()
            .is_some_and(FixtureRun::is_running)
        {
            self.quickjs_backend_test.error =
                Some("A fixture is already running; Stop it before restarting".into());
        } else {
            // Never pass data_dir itself: it also contains SSH keys and preferences.
            match FixtureRun::start(self.data_dir.join("quickjs-backend-test"), mode) {
                Ok(run) => {
                    self.quickjs_backend_test.run = Some(run);
                    self.quickjs_backend_test.error = None;
                }
                Err(error) => self.quickjs_backend_test.error = Some(error.to_string()),
            }
        }
        cx.notify();
    }

    pub(crate) fn tick_quickjs_backend_test(&self, cx: &mut Context<Self>) {
        if !self.quickjs_backend_test.opened || self.local_private {
            return;
        }
        let state = cx.global::<crate::testing::State>();
        let path = self.data_dir.join("quickjs-backend-test");
        // Read-only synthetic telemetry, separate from both live sessions and preview state.
        let visible = self.quickjs_backend_test.visible && self.visible && self.window_active;
        let ui = json!({ "visible": visible, "pid": std::process::id(), "frame": self.quickjs_backend_test.frame,
            "scale": state.scale, "bounds": if visible { json!(state.bounds) } else { json!({}) },
            "run_id": self.quickjs_backend_test.run.as_ref().map(|run| run.snapshot()["run_id"].clone()) });
        if std::fs::create_dir_all(&path).is_ok() {
            let temporary = path.join("ui.json.new");
            if std::fs::write(&temporary, ui.to_string()).is_ok() {
                let _ = std::fs::rename(temporary, path.join("ui.json"));
            }
        }
        if visible {
            cx.notify();
        }
    }

    pub(crate) fn quickjs_backend_screen(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.quickjs_backend_test.frame = self.quickjs_backend_test.frame.wrapping_add(1);
        let colors = theme(cx);
        let state = self
            .quickjs_backend_test
            .run
            .as_ref()
            .map(FixtureRun::snapshot)
            .unwrap_or_else(|| json!({"status": "ready", "events": 0, "tools": 0, "text": ""}));
        let status = state["status"].as_str().unwrap_or("unknown").to_owned();
        let running = self
            .quickjs_backend_test
            .run
            .as_ref()
            .is_some_and(FixtureRun::is_running);
        let detail: SharedString = format!(
            "{status} · {} events · {} tools",
            state["events"], state["tools"]
        )
        .into();
        let error = self.quickjs_backend_test.error.clone().or_else(|| {
            state["error"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });
        div().size_full().flex().flex_col()
            .child(ui::appbar(
                ui::tap("quickjs-back", "back", &colors)
                    .on_click(cx.listener(|this, _, window, cx| { this.back(window, cx); })),
                "Local Pi · QuickJS", Some("Offline backend proof · experimental".into()), &colors))
            .child(div().id("quickjs-backend-content").flex_1().min_h_0().overflow_y_scroll()
                .p(px(20.)).flex().flex_col().gap(px(16.))
                .child(ui::serif("Pi runs on this phone", 28.))
                .child("Unmodified Pi agent and provider code, running in a separate QuickJS VM inside this APK. No Bun, Node, SSH, or paid model calls.")
                .child(ui::hint("The synthetic provider requests read → write → read in an isolated temporary workspace. This does not yet enable local sessions in the normal app.", &colors))
                .child(div().rounded(px(16.)).bg(colors.panel).p(px(16.))
                    .child(ui::label(detail, &colors))
                    .child(ui::mono(state["text"].as_str().unwrap_or("").to_owned(), 17.)))
                .children(error.map(|error| ui::hint(error, &colors)))
                .child(ui::button("quickjs-run", Button::Primary, Some("send"), "Run offline agent", false, &colors)
                    .when(!running, |button| button.on_click(cx.listener(|this, _, _, cx| this.start_quickjs_backend_test(Mode::RoundTrip, cx))))
                    .opacity(if running { 0.5 } else { 1.0 }))
                .child(ui::button("quickjs-stall", Button::Plain, None, "Test cancellation", false, &colors)
                    .when(!running, |button| button.on_click(cx.listener(|this, _, _, cx| this.start_quickjs_backend_test(Mode::WaitForCancel, cx))))
                    .opacity(if running { 0.5 } else { 1.0 }))
                .child(ui::button("quickjs-stop", Button::Plain, Some("stop"), "Stop", false, &colors)
                    .when(running, |button| button.on_click(cx.listener(|this, _, _, cx| { this.quickjs_backend_test.cancel("stop"); cx.notify(); })))
                    .opacity(if running { 1.0 } else { 0.5 }))
                .child(ui::hint("Leaving this screen or backgrounding the app cancels the run. No session persistence or background service yet.", &colors)))
    }
}
