//! Opt-in local accounts only. No credentials in Prefs, agent workspaces or UI telemetry.
use crate::{
    app::PhoneApp,
    theme::theme,
    ui::{self, Button},
};
use gpui::{Context, Window, div, prelude::*, px};
use pi_local::auth::{AuthTask, Operation, Provider, Vault};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
static NEXT_PANEL: AtomicU64 = AtomicU64::new(1);

// One bounded, coalescing test-telemetry writer. Never performs filesystem work on GPUI.
type Publication = (PathBuf, Option<Value>);
struct Telemetry {
    pending: Arc<Mutex<Option<Publication>>>,
    wake: mpsc::SyncSender<()>,
}
fn publish(path: PathBuf, value: Option<Value>) {
    static WRITER: OnceLock<Telemetry> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let pending = Arc::new(Mutex::new(None::<Publication>));
        let work = pending.clone();
        let (wake, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                let item = work.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some((path, value)) = item {
                    if let Some(value) = value {
                        if std::fs::create_dir_all(&path).is_ok()
                            && std::fs::write(path.join("ui.new"), value.to_string()).is_ok()
                        {
                            let _ = std::fs::rename(path.join("ui.new"), path.join("ui.json"));
                        }
                    } else {
                        let _ = std::fs::remove_file(path.join("ui.json"));
                    }
                }
            }
        });
        Telemetry { pending, wake }
    });
    *writer.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, value));
    let _ = writer.wake.try_send(());
}

#[cfg(target_os = "android")]
struct AndroidVault(
    gpui_android::secure_storage::ProviderVault,
    pi_local::auth::VaultScope,
);
#[cfg(target_os = "android")]
impl Vault for AndroidVault {
    fn scope(&self) -> pi_local::auth::VaultScope {
        self.1
    }
    fn read(&self, provider: Provider) -> anyhow::Result<Value> {
        let text = self.0.read(provider.id()).map_err(anyhow::Error::msg)?;
        serde_json::from_str(&text).map_err(|_| anyhow::anyhow!("invalid credential record"))
    }
    fn write(
        &self,
        provider: Provider,
        expected: &str,
        credential: &str,
    ) -> anyhow::Result<Option<String>> {
        self.0
            .write(provider.id(), expected, credential)
            .map_err(anyhow::Error::msg)
    }
    fn clear(&self, provider: Provider) -> anyhow::Result<()> {
        self.0.clear(provider.id()).map_err(anyhow::Error::msg)
    }
}
pub(crate) fn vault(synthetic: bool) -> Option<Arc<dyn Vault>> {
    #[cfg(target_os = "android")]
    {
        Some(Arc::new(AndroidVault(
            gpui_android::secure_storage::provider_vault(synthetic)?,
            if synthetic {
                pi_local::auth::VaultScope::Synthetic
            } else {
                pi_local::auth::VaultScope::Accounts
            },
        )))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = synthetic;
        None
    }
}

fn sign_in_code(state: &Value) -> Option<&str> {
    if state["status"] != "running" {
        return None;
    }
    state["device_code"].as_str().filter(|code| {
        !code.trim().is_empty()
            && code.len() <= 64
            && code
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ' ')
    })
}

#[derive(Default)]
pub(crate) struct Panel {
    pub visible: bool,
    generation: u64,
    revision: u64,
    synthetic: bool,
    opened: bool,
    frame: u64,
    vault: Option<Arc<dyn Vault>>,
    saved: Arc<Mutex<Value>>,
    task: Option<AuthTask>,
    task_provider: Option<Provider>,
    task_operation: Option<Operation>,
    terminal_seen: bool,
    synthetic_copy_verified: bool,
    dialog: Option<String>,
    error: Option<String>,
}
impl Panel {
    pub(crate) fn is_synthetic(&self) -> bool {
        self.synthetic
    }
    fn running(&self) -> bool {
        self.task.as_ref().is_some_and(|t| !t.is_finished())
    }
    fn dismiss_dialog(&mut self) {
        if let Some(id) = self.dialog.take() {
            gpui_android::activity::dismiss_provider_code(&id);
        }
    }
    fn manage(&mut self, clear: Option<Provider>) -> bool {
        let Some(vault) = self.vault.clone() else {
            return false;
        };
        let saved = self.saved.clone();
        {
            let mut state = saved.lock().unwrap_or_else(|e| e.into_inner());
            if state["busy"] == true {
                return false;
            }
            if state.is_null() {
                *state = json!({});
            }
            state["busy"] = json!(true);
        }
        std::thread::spawn(move || {
            let error = clear.is_some_and(|p| pi_local::auth::logout(p, vault.as_ref()).is_err());
            let mut state = json!({"busy": false, "error": error});
            for provider in [Provider::Claude, Provider::Codex] {
                state[provider.id()] = match vault.read(provider) {
                    Ok(record) => {
                        json!({"saved": record["credential"]["type"] == "oauth", "error": false})
                    }
                    Err(_) => json!({"saved": false, "error": true}),
                };
            }
            *saved.lock().unwrap_or_else(|e| e.into_inner()) = state;
        });
        true
    }
}

impl PhoneApp {
    pub(crate) fn provider_accounts_url(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let synthetic = match url {
            "pi://test/local-providers" => false,
            "pi://test/local-providers/synthetic"
                if cfg!(feature = "ui-test")
                    && (cfg!(test) || gpui_android::activity::is_debug_build()) =>
            {
                true
            }
            _ => return false,
        };
        if self.connecting || self.resuming.is_some() {
            self.notify_user("Open local providers without an active remote session", cx);
            return true;
        }
        if self.local_providers.running() {
            self.notify_user("Finish or cancel the current sign-in first", cx);
            return true;
        }
        self.close_sheet(cx);
        // Replacing the entire screen must also retire the old sheet's invisible
        // closing blocker; otherwise the first account action is swallowed.
        self.closing_sheet = None;
        self.close_expanded_visual(window, cx);
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        self.local_providers.dismiss_dialog();
        self.local_providers = Panel {
            visible: true,
            generation: NEXT_PANEL.fetch_add(1, Ordering::Relaxed),
            opened: true,
            synthetic,
            vault: vault(synthetic),
            ..Default::default()
        };
        if self.local_providers.vault.is_none() {
            self.local_providers.error = Some("Android credential storage is unavailable".into());
        }
        self.local_providers.manage(None);
        if !synthetic {
            self.local_private = true;
            publish(self.data_dir.join("quickjs-provider-test"), None);
        }
        // Screenshots are intentionally enabled for now, including sign-in codes.
        cx.notify();
        true
    }
    pub(crate) fn close_provider_accounts(&mut self) {
        if !self.local_providers.visible {
            return;
        }
        if let Some(task) = &self.local_providers.task {
            task.cancel();
        }
        self.local_providers.dismiss_dialog();
        self.local_providers.visible = false;
    }
    fn provider_action(&self, stamp: (u64, u64)) -> bool {
        self.visible
            && self.window_active
            && self.local_providers.visible
            && stamp
                == (
                    self.local_providers.generation,
                    self.local_providers.revision,
                )
    }
    fn provider_login(&mut self, provider: Provider, operation: Operation, cx: &mut Context<Self>) {
        if !self.local_providers.visible
            || !self.visible
            || !self.window_active
            || self.local_providers.running()
            || self
                .local_providers
                .saved
                .lock()
                .unwrap_or_else(|e| e.into_inner())["busy"]
                == true
        {
            return;
        }
        let Some(vault) = self.local_providers.vault.clone() else {
            return;
        };
        if operation == Operation::Login {
            self.invalidate_local_account(provider, self.local_providers.synthetic, cx);
        }
        let result = if self.local_providers.synthetic {
            pi_local::auth_fixture::start(provider, operation, vault)
        } else {
            AuthTask::start(provider, operation, vault)
        };
        self.local_providers.dismiss_dialog();
        match result {
            Ok(task) => {
                self.local_providers.revision = self.local_providers.revision.wrapping_add(1);
                self.local_providers.task = Some(task);
                self.local_providers.task_provider = Some(provider);
                self.local_providers.task_operation = Some(operation);
                self.local_providers.terminal_seen = false;
                self.local_providers.synthetic_copy_verified = false;
                self.local_providers.error = None;
            }
            Err(_) => {
                self.local_providers.error =
                    Some("Could not start sign-in; another operation may still be finishing".into())
            }
        }
        cx.notify();
    }
    fn copy_provider_sign_in_code(&mut self, cx: &mut Context<Self>) {
        if !self.local_providers.visible
            || !self.visible
            || !self.window_active
            || self.local_providers.task_operation != Some(Operation::Login)
        {
            return;
        }
        let Some(task) = &self.local_providers.task else {
            return;
        };
        if task.is_finished() || task.is_cancelled() {
            return;
        }
        // Read the current attempt, not a code captured by an old rendered callback.
        let state = task.snapshot();
        let Some(code) = sign_in_code(&state) else {
            return;
        };
        if gpui_android::activity::copy_provider_code(code) {
            if self.local_providers.synthetic {
                // Synthetic-only assertion: never publish clipboard text, even in tests.
                self.local_providers.synthetic_copy_verified = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref()
                    == Some(code);
            }
            self.notify_user("Sign-in code copied", cx);
        } else {
            self.notify_user("Could not copy the sign-in code", cx);
        }
    }
    fn provider_logout(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.local_providers.revision = self.local_providers.revision.wrapping_add(1);
        self.invalidate_local_account(provider, self.local_providers.synthetic, cx);
        if self.local_providers.task_provider == Some(provider) {
            if let Some(task) = &self.local_providers.task {
                task.cancel();
            }
            self.local_providers.dismiss_dialog();
        }
        if !self.local_providers.manage(Some(provider)) {
            self.local_providers.error =
                Some("Credential storage is busy; please try signing out again".into());
        }
        cx.notify();
    }
    pub(crate) fn tick_provider_accounts(&mut self, cx: &mut Context<Self>) {
        if !self.local_providers.opened {
            return;
        }
        if let Some(id) = self.local_providers.dialog.clone()
            && let Some(reply) = gpui_android::activity::take_provider_code(&id)
        {
            self.local_providers.dialog = None;
            if let Ok(value) = serde_json::from_str::<Value>(&reply)
                && let Some(code) = value["code"].as_str()
                && let Some(task) = &self.local_providers.task
                && task.respond(&id, code.to_owned()).is_err()
            {
                self.local_providers.error =
                    Some("Code could not be submitted. Try again or restart sign-in.".into());
            }
        }
        if self
            .local_providers
            .task
            .as_ref()
            .is_some_and(AuthTask::is_finished)
            && !self.local_providers.terminal_seen
        {
            self.local_providers.dismiss_dialog();
            self.local_providers.terminal_seen = self.local_providers.manage(None);
        }
        // No secret-bearing snapshots, URLs, codes or tokens are published. Even geometry
        // is only emitted in the explicitly synthetic mode, using a different vault namespace.
        let visible = self.local_providers.visible && self.visible && self.window_active;
        #[cfg(feature = "ui-test")]
        let path = self.data_dir.join("quickjs-provider-test");
        #[cfg(feature = "ui-test")]
        if self.local_providers.synthetic && !self.local_private {
            let geometry = cx.global::<crate::testing::State>();
            let phase = self
                .local_providers
                .task
                .as_ref()
                .map(AuthTask::snapshot)
                .unwrap_or(json!({"status":"idle"}));
            let ui = json!({"synthetic":true, "visible":visible, "pid":std::process::id(), "frame":self.local_providers.frame,
                "bounds":if visible {json!(geometry.bounds)} else {json!({})}, "scale":geometry.scale,
                "status":phase["status"], "provider":self.local_providers.task_provider.map(Provider::id),
                "dialog":self.local_providers.dialog.is_some(), "copied_code_verified":self.local_providers.synthetic_copy_verified, "accounts":*self.local_providers.saved.lock().unwrap_or_else(|e| e.into_inner())});
            publish(path, Some(ui));
        }
        if visible {
            cx.notify();
        }
    }
    pub(crate) fn provider_accounts_screen(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.local_providers.frame = self.local_providers.frame.wrapping_add(1);
        let colors = theme(cx);
        let stamp = (
            self.local_providers.generation,
            self.local_providers.revision,
        );
        let saved = self
            .local_providers
            .saved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let running = self.local_providers.running();
        let state = self
            .local_providers
            .task
            .as_ref()
            .map(AuthTask::snapshot)
            .unwrap_or(json!({"status":"idle"}));
        let synthetic = self.local_providers.synthetic;
        let busy = saved["busy"] == true;
        let available = self.local_providers.vault.is_some() && !busy;
        let cards = [Provider::Claude, Provider::Codex]
            .into_iter()
            .map(|provider| {
                let connected = saved[provider.id()]["saved"] == true;
                let message = if saved[provider.id()]["error"] == true {
                    "Storage unavailable. Sign out to reset this account."
                } else if connected {
                    "Account saved on this phone"
                } else {
                    "Not connected"
                };
                let operation = if connected {
                    Operation::Refresh
                } else {
                    Operation::Login
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .rounded(px(16.))
                    .bg(colors.panel)
                    .p(px(14.))
                    .child(ui::serif(provider.label(), 23.))
                    .child(ui::hint(message, &colors))
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(
                                ui::button(
                                    format!("provider-{}-login", provider.id()),
                                    Button::Primary,
                                    None,
                                    if connected {
                                        "Check / refresh"
                                    } else {
                                        "Sign in"
                                    },
                                    false,
                                    &colors,
                                )
                                .when(available && !running, |b| {
                                    b.on_click(cx.listener(move |this, _, _, cx| {
                                        if this.provider_action(stamp) {
                                            this.provider_login(provider, operation, cx);
                                        }
                                    }))
                                })
                                .opacity(if available && !running { 1. } else { 0.5 }),
                            )
                            .child(
                                ui::button(
                                    format!("provider-{}-logout", provider.id()),
                                    Button::Plain,
                                    None,
                                    "Sign out",
                                    false,
                                    &colors,
                                )
                                .when(available, |b| {
                                    b.on_click(cx.listener(move |this, _, _, cx| {
                                        if this.provider_action(stamp) {
                                            this.provider_logout(provider, cx);
                                        }
                                    }))
                                }),
                            ),
                    )
            })
            .collect::<Vec<_>>();
        let progress = match state["status"].as_str().unwrap_or("idle") {
            "connected" => "Account operation completed.",
            "failed" => {
                "Sign-in or refresh failed. Retry; sign out and sign in again if the saved account cannot be refreshed."
            }
            "cancelled" => "Sign-in cancelled.",
            "prompt" => {
                "Open Claude in your browser, then return here and enter the authorization code."
            }
            "running" if state.get("device_code").is_some() => {
                "Open Codex in your browser and enter this device code. Keep this attempt open while you sign in."
            }
            "running" => "Waiting for provider…",
            _ => {
                "Choose Chat on this phone for a signed-in account. Opening chat makes no request; Send contacts the selected provider."
            }
        };
        div().size_full().flex().flex_col()
            .child(ui::appbar(ui::tap("providers-back", "back", &colors).on_click(cx.listener(move |this, _, window, cx| { if this.provider_action(stamp) { this.back(window, cx); } })),
                "Local providers", Some(if synthetic {"Synthetic auth · offline"} else {"Claude + Codex · experimental"}.into()), &colors))
            .child(div().id("providers-content").flex_1().min_h_0().overflow_y_scroll().p(px(16.)).flex().flex_col().gap(px(12.))
                .child(ui::hint(if synthetic {"Test accounts only. Separate encrypted vault; no real provider is contacted. Claude test code: synthetic-code."}
                    else {"Credentials are encrypted with Android Keystore, outside model workspaces. This test APK is debuggable; authorized adb/debuggers can access its process."}, &colors))
                .children(cards)
                .child(ui::hint(progress, &colors))
                .children(sign_in_code(&state).filter(|_| running).map(|code|
                    div().flex().flex_wrap().items_center().gap(px(12.))
                        .child(ui::mono(code.to_owned(), 24.))
                        .child(ui::button("provider-copy-code", Button::Plain, Some("copy"), "Copy code", false, &colors)
                            .on_click(cx.listener(move |this, _, _, cx| { if this.provider_action(stamp) { this.copy_provider_sign_in_code(cx); } })))))
                .when(running && state.get("url").is_some(), |d| d.child(ui::button("provider-browser", Button::Plain, None, if synthetic {"Browser disabled in synthetic mode"} else {"Open browser"}, false, &colors)
                    .when(!synthetic, |b| b.on_click(cx.listener(move |this, _, _, cx| {
                        if !this.provider_action(stamp) { return; }
                        if let (Some(task), Some(provider)) = (&this.local_providers.task, this.local_providers.task_provider)
                            && !task.is_finished() && !task.is_cancelled()
                            && let Some(url) = task.snapshot()["url"].as_str()
                            && pi_local::auth::allowed_browser_url(provider, url) { cx.open_url(url); }
                    })))))
                .when(running && state["status"] == "prompt", |d| d.child(ui::button("provider-code", Button::Primary, None, "Enter authorization code", false, &colors)
                    .on_click(cx.listener(move |this, _, _, _| {
                        if !this.provider_action(stamp) { return; }
                        if let Some(task) = &this.local_providers.task && !task.is_finished() && !task.is_cancelled() && let Some(id) = task.snapshot()["prompt_id"].as_str() {
                            let id = id.to_owned(); gpui_android::activity::show_provider_code(&id); this.local_providers.dialog = Some(id);
                        }
                    }))))
                .when(running && self.local_providers.task_operation == Some(Operation::Login), |d| d.child(ui::button("provider-cancel", Button::Plain, None, "Cancel sign-in", false, &colors)
                    .on_click(cx.listener(move |this, _, _, cx| { if !this.provider_action(stamp) { return; }
                        if let Some(task) = &this.local_providers.task { task.cancel(); } this.local_providers.dismiss_dialog(); cx.notify(); }))))
                .children(self.local_providers.error.clone().map(|error| ui::hint(error, &colors)))
                .when(saved["error"] == true, |d| d.child(ui::hint("Could not remove account. Try signing out again.", &colors)))
                .child(ui::hint("Browser/background transitions keep sign-in running. Back cancels sign-in; refresh finishes independently to save rotated tokens. Sign out removes this phone's credentials, not the provider's remote authorization.", &colors)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_active_device_codes_can_be_copied() {
        assert_eq!(
            sign_in_code(&json!({"status":"running","device_code":"ABCD-EFGH"})),
            Some("ABCD-EFGH")
        );
        for status in ["idle", "prompt", "connected", "failed", "cancelled"] {
            assert!(sign_in_code(&json!({"status":status,"device_code":"ABCD-EFGH"})).is_none());
        }
        for code in [
            "",
            "   ",
            "ABCD\nEFGH",
            "https://example.com/code",
            "é",
            &"x".repeat(65),
        ] {
            assert!(sign_in_code(&json!({"status":"running","device_code":code})).is_none());
        }
        assert!(sign_in_code(&json!({"status":"running","access":"token","refresh":"token","url":"https://example.com/"})).is_none());
    }
    #[gpui::test]
    fn retained_provider_callbacks_cannot_cross_namespaces(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.visible = true;
                app.window_active = true;
                assert!(app.provider_accounts_url(
                    "pi://test/local-providers/synthetic",
                    window,
                    cx
                ));
                let stamp = (app.local_providers.generation, app.local_providers.revision);
                assert!(app.provider_action(stamp));
                app.close_provider_accounts();
                assert!(!app.provider_action(stamp));
                assert!(app.provider_accounts_url("pi://test/local-providers", window, cx));
                assert!(
                    !app.provider_action(stamp),
                    "synthetic callback cannot touch real accounts"
                );
                let stamp = (app.local_providers.generation, app.local_providers.revision);
                assert!(app.provider_action(stamp));
                app.local_providers.revision += 1;
                assert!(
                    !app.provider_action(stamp),
                    "old attempt callbacks must be rejected"
                );
                assert!(app.local_private);
            })
        });
    }
}
