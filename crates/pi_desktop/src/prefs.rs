//! pi-desktop's own preferences (design study 06), which no agent reads: the app's
//! `settings.json` in its config folder, per-project overrides in the project's
//! `.pi/pi-desktop.json`, and `state.json`, what the app remembers between
//! launches (open sessions, "Not now" answers).
//!
//! A value resolves from the project's file, for the settings a project may
//! override, then the user's file, then the default. A value of the wrong type
//! counts as unset. Without the [`Prefs`] global, as in most tests, every value is
//! its default and nothing is saved.
mod schema;

use anyhow::Result;
use gpui::{App, BorrowAppContext as _, Global, Task, TaskExt as _, WindowAppearance};
use pi_core::protocol::SavedSession;
use pi_settings::{Scope, Setting, SettingsFile};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

pub use schema::DESKTOP;

pub fn setting(key: &str) -> Option<&'static Setting> {
    DESKTOP.iter().find(|setting| setting.key == key)
}

/// Categories in schema order.
pub fn categories() -> Vec<&'static str> {
    let mut categories: Vec<&'static str> = Vec::new();
    for setting in DESKTOP {
        if !categories.contains(&setting.category) {
            categories.push(setting.category);
        }
    }
    categories
}

/// A choice as the Settings view names it: `alt` is "While ⌥ is held".
pub fn choice_label(key: &str, value: &Value) -> String {
    let value = value.as_str().unwrap_or_default();
    match (key, value) {
        ("jj.turnBars", "alt") if cfg!(target_os = "macos") => "While ⌥ is held".into(),
        ("jj.turnBars", "alt") => "While Alt is held".into(),
        ("jj.parallelSessions", "workspace") => "Own workspace".into(),
        ("jj.parallelSessions", "same") => "Same folder".into(),
        _ => {
            let mut chars = value.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// `PI_DESKTOP_CONFIG_DIR`, else `pi-desktop` in the platform's config folder:
/// `~/Library/Application Support` on macOS, `~/.config` on Linux, `%APPDATA%` on Windows.
pub fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("PI_DESKTOP_CONFIG_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(dirs::config_dir()?.join("pi-desktop")),
    }
}

/// The user file in use: in [`config_dir`] once loaded, `None` without the global.
pub fn user_path(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Prefs>()
        .map(|prefs| prefs.user.path.clone())
        .filter(|path| !path.as_os_str().is_empty())
}

/// Beside pi's `.pi/settings.json`, so neither program overwrites the other's file.
pub fn project_path(root: &Path) -> PathBuf {
    root.join(".pi").join("pi-desktop.json")
}

/// A session open when the app quit.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenSession {
    pub cwd: PathBuf,
    /// `None` for a session that had nothing saved yet.
    pub saved: Option<SavedSession>,
}

pub struct Prefs {
    user: SettingsFile,
    projects: HashMap<PathBuf, SettingsFile>,
    state: SettingsFile,
    /// `--light`: Moonstone for this run, whatever the setting says.
    pub force_light: bool,
    /// The backend built into this executable, once unpacked: what sessions run
    /// when `general.backend` is Automatic.
    pub bundled_backend: Option<PathBuf>,
    writers: HashMap<PathBuf, Writer>,
}
impl Global for Prefs {}

impl Prefs {
    /// Blocking, once at launch. Without a config folder nothing is read or saved.
    pub fn load() -> Self {
        Self::load_from(config_dir().as_deref())
    }

    pub(crate) fn load_from(dir: Option<&Path>) -> Self {
        let file = |name: &str| {
            dir.map(|dir| SettingsFile::load(dir.join(name)))
                .unwrap_or_default()
        };
        Self {
            user: file("settings.json"),
            projects: HashMap::new(),
            state: file("state.json"),
            force_light: false,
            bundled_backend: None,
            writers: HashMap::new(),
        }
    }

    pub fn value(&self, key: &str, project: Option<&Path>) -> Value {
        let Some(setting) = setting(key) else {
            debug_assert!(false, "unknown desktop setting {key}");
            return Value::Null;
        };
        let overrides = project
            .filter(|_| setting.scope == Scope::Any)
            .and_then(|root| self.projects.get(root))
            .and_then(|file| file.get(key))
            .filter(|value| setting.accepts(value));
        overrides
            .or_else(|| self.user.get(key).filter(|value| setting.accepts(value)))
            .cloned()
            .or_else(|| setting.default_value())
            .unwrap_or(Value::Null)
    }

    /// The sessions to reopen and which of them was active.
    pub fn open_sessions(&self) -> (Vec<OpenSession>, usize) {
        let sessions = self
            .state
            .get("openSessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|session| {
                let cwd = PathBuf::from(session["cwd"].as_str()?);
                let saved = session["path"].as_str().map(|path| SavedSession {
                    id: session["id"].as_str().unwrap_or_default().to_owned(),
                    path: path.to_owned(),
                    cwd: cwd.to_string_lossy().into_owned(),
                    name: session["name"].as_str().map(str::to_owned),
                    first_message: session["title"].as_str().unwrap_or_default().to_owned(),
                    ..Default::default()
                });
                Some(OpenSession { cwd, saved })
            })
            .collect::<Vec<_>>();
        let active = self
            .state
            .get("activeSession")
            .and_then(Value::as_u64)
            .map_or(0, |active| active as usize);
        (sessions, active)
    }

    fn declined_jj(&self) -> Vec<&str> {
        self.state
            .get("declinedJj")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    /// Replaces the copy of the file at `file.path`: the user's or a project's.
    fn replace(&mut self, file: SettingsFile) {
        if file.path == self.user.path {
            self.user = file;
        } else if let Some(root) = file.path.parent().and_then(Path::parent) {
            self.projects.insert(root.to_owned(), file);
        }
    }

    fn save(&mut self, file: SettingsFile, cx: &App) -> Task<Result<()>> {
        if file.path.as_os_str().is_empty() {
            return Task::ready(Ok(()));
        }
        self.writers
            .entry(file.path.clone())
            .or_default()
            .save(file, cx)
    }

    fn remember(&mut self, key: &str, value: Value, cx: &App) {
        self.state.set(key, value);
        let state = self.state.clone();
        self.save(state, cx).detach_and_log_err(cx);
    }
}

/// Saves one file off the UI thread. Saves never overlap, and an older snapshot
/// never lands after a newer one: each save writes the newest pending snapshot.
#[derive(Clone, Default)]
struct Writer {
    pending: Arc<Mutex<Option<SettingsFile>>>,
    io: Arc<Mutex<()>>,
}

impl Writer {
    fn save(&self, file: SettingsFile, cx: &App) -> Task<Result<()>> {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(file);
        let (pending, io) = (self.pending.clone(), self.io.clone());
        cx.background_executor().spawn(async move {
            let _io = io.lock().unwrap_or_else(PoisonError::into_inner);
            let file = pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            file.map_or(Ok(()), |file| file.save())
        })
    }
}

pub fn get(cx: &App, key: &str, project: Option<&Path>) -> Value {
    match cx.try_global::<Prefs>() {
        Some(prefs) => prefs.value(key, project),
        None => setting(key)
            .and_then(Setting::default_value)
            .unwrap_or(Value::Null),
    }
}

pub fn flag(cx: &App, key: &str, project: Option<&Path>) -> bool {
    get(cx, key, project).as_bool().unwrap_or(false)
}

/// A text setting, `None` when it is Automatic (unset or blank).
pub fn text(cx: &App, key: &str) -> Option<String> {
    get(cx, key, None)
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// What runs sessions and pi's short-lived helpers: `general.backend` and
/// `general.node`. Environment variables still win (`Launch::pi_with`).
pub fn backend(cx: &App) -> pi_core::transport::Backend {
    pi_core::transport::Backend {
        program: text(cx, "general.backend").map(PathBuf::from).or_else(|| {
            cx.try_global::<Prefs>()
                .and_then(|prefs| prefs.bundled_backend.clone())
        }),
        node: text(cx, "general.node").map(PathBuf::from),
    }
}

pub fn choice(cx: &App, key: &str, project: Option<&Path>) -> String {
    get(cx, key, project)
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Whether windows use the light theme: `--light`, else `appearance.theme`.
pub fn light(cx: &App) -> bool {
    if cx
        .try_global::<Prefs>()
        .is_some_and(|prefs| prefs.force_light)
    {
        return true;
    }
    match choice(cx, "appearance.theme", None).as_str() {
        "evening" => false,
        "moonstone" => true,
        _ => matches!(
            cx.window_appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    }
}

/// Reads a project's overrides once. A small blocking read, as jj's detection is
/// when a session starts.
pub fn load_project(cx: &mut App, root: &Path) {
    if !cx.has_global::<Prefs>() || cx.global::<Prefs>().projects.contains_key(root) {
        return;
    }
    let file = SettingsFile::load(project_path(root));
    cx.update_global::<Prefs, _>(|prefs, _| prefs.replace(file));
}

/// A file the Settings view changed: it applies at once and is saved.
pub fn write(cx: &mut App, file: SettingsFile) -> Task<Result<()>> {
    if !cx.has_global::<Prefs>() {
        return Task::ready(Ok(()));
    }
    cx.update_global::<Prefs, _>(|prefs, cx| {
        prefs.replace(file.clone());
        prefs.save(file, cx)
    })
}

/// Sets one of a project's overrides in its `.pi/pi-desktop.json`, as
/// "Remember for this project" does.
pub fn set_project(cx: &mut App, root: &Path, key: &str, value: Value) -> Task<Result<()>> {
    if !cx.has_global::<Prefs>() {
        return Task::ready(Ok(()));
    }
    load_project(cx, root);
    let Some(mut file) = cx.global::<Prefs>().projects.get(root).cloned() else {
        return Task::ready(Ok(()));
    };
    if let Some(error) = &file.error {
        return Task::ready(Err(anyhow::anyhow!("{error}; fix the file by hand first")));
    }
    file.set(key, value);
    write(cx, file)
}

/// A file the Settings view read again, so hand edits apply.
pub fn reloaded(cx: &mut App, file: SettingsFile) {
    if cx.has_global::<Prefs>() {
        cx.update_global::<Prefs, _>(|prefs, _| prefs.replace(file));
    }
}

/// Saves the sessions open now, for `general.reopenSessions`.
pub fn remember_open_sessions(cx: &mut App, sessions: &[OpenSession], active: usize) {
    if !cx.has_global::<Prefs>() {
        return;
    }
    let list = sessions
        .iter()
        .map(|session| {
            let mut entry = json!({ "cwd": session.cwd.to_string_lossy() });
            if let Some(saved) = &session.saved {
                entry["path"] = json!(saved.path);
                entry["id"] = json!(saved.id);
                entry["name"] = json!(saved.name);
                entry["title"] = json!(saved.first_message);
            }
            entry
        })
        .collect::<Vec<_>>();
    let prefs = cx.global::<Prefs>();
    if prefs.state.get("openSessions") == Some(&json!(list))
        && prefs.state.get("activeSession") == Some(&json!(active))
    {
        return;
    }
    cx.update_global::<Prefs, _>(|prefs, cx| {
        prefs.state.set("openSessions", json!(list));
        prefs.remember("activeSession", json!(active), cx);
    });
}

/// Whether "Not now" was answered to jj for this project in an earlier launch.
pub fn jj_declined(cx: &App, root: &Path) -> bool {
    flag(cx, "general.rememberDismissed", None)
        && cx.try_global::<Prefs>().is_some_and(|prefs| {
            prefs
                .declined_jj()
                .contains(&root.to_string_lossy().as_ref())
        })
}

pub fn decline_jj(cx: &mut App, root: &Path) {
    if !cx.has_global::<Prefs>() || !flag(cx, "general.rememberDismissed", None) {
        return;
    }
    let root = root.to_string_lossy().into_owned();
    let mut declined: Vec<String> = cx
        .global::<Prefs>()
        .declined_jj()
        .into_iter()
        .map(str::to_owned)
        .collect();
    if declined.contains(&root) {
        return;
    }
    declined.push(root);
    cx.update_global::<Prefs, _>(|prefs, cx| prefs.remember("declinedJj", json!(declined), cx));
}

/// Projects whose "Not now" is remembered.
pub fn dismissed(cx: &App) -> usize {
    cx.try_global::<Prefs>()
        .map_or(0, |prefs| prefs.declined_jj().len())
}

pub fn forget_dismissed(cx: &mut App) {
    if dismissed(cx) > 0 {
        cx.update_global::<Prefs, _>(|prefs, cx| prefs.remember("declinedJj", json!([]), cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn projects_override_only_their_settings_and_bad_values_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"jj": {"tools": true, "offer": "sometimes"}, "editor": {"fontSize": "big"}, "general": {"reopenSessions": false}}"#,
        )
        .unwrap();
        let mut prefs = Prefs::load_from(Some(dir.path()));
        let root = dir.path().join("project");
        let mut project = SettingsFile::load(project_path(&root));
        project.set("jj.tools", json!(false));
        project.set("general.reopenSessions", json!(true));
        prefs.replace(project);

        assert_eq!(prefs.value("jj.tools", None), json!(true), "the user's");
        assert_eq!(
            prefs.value("jj.tools", Some(&root)),
            json!(false),
            "overridden"
        );
        assert_eq!(
            prefs.value("general.reopenSessions", Some(&root)),
            json!(false),
            "a project may not override a user-only setting"
        );
        assert_eq!(prefs.value("jj.offer", None), json!("ask"), "not a choice");
        assert_eq!(
            prefs.value("editor.fontSize", None),
            json!(11.5),
            "not a number"
        );
        assert_eq!(prefs.value("appearance.theme", None), json!("system"));
        assert_eq!(
            choice_label("jj.parallelSessions", &json!("same")),
            "Same folder"
        );
        assert_eq!(
            choice_label("appearance.theme", &json!("moonstone")),
            "Moonstone"
        );
        assert_eq!(categories()[0], "General");
        for setting in DESKTOP {
            if let Some(default) = setting.default_value() {
                assert!(setting.accepts(&default), "{}", setting.key);
            }
        }
    }

    #[gpui::test]
    async fn state_remembers_open_sessions_and_declined_offers(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| cx.set_global(Prefs::load_from(Some(dir.path()))));
        let root = Path::new("/repos/pi");
        let saved = SavedSession {
            id: "abc".into(),
            path: "/sessions/abc.jsonl".into(),
            cwd: "/repos/pi".into(),
            name: None,
            first_message: "Fix the tests".into(),
            ..Default::default()
        };
        let open = vec![
            OpenSession {
                cwd: root.into(),
                saved: Some(saved),
            },
            OpenSession {
                cwd: "/repos/other".into(),
                saved: None,
            },
        ];
        cx.update(|cx| {
            remember_open_sessions(cx, &open, 1);
            assert!(!jj_declined(cx, root));
            decline_jj(cx, root);
            assert!(jj_declined(cx, root));
        });
        cx.run_until_parked();

        let reloaded = Prefs::load_from(Some(dir.path()));
        assert_eq!(reloaded.open_sessions(), (open, 1));
        assert_eq!(reloaded.declined_jj(), ["/repos/pi"]);
        cx.update(|cx| {
            forget_dismissed(cx);
            assert!(!jj_declined(cx, root));
        });
    }
}
