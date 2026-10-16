//! Settings that stay on the phone, as JSON in the app's private storage.

use crate::theme::Appearance;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// The part of a remote slash command the phone needs after reconnecting.
/// Execution remains owned by the computer; this is only a cached picker label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCommand {
    pub name: String,
    pub description: Option<String>,
}

impl From<&pi_core::protocol::SlashCommand> for SavedCommand {
    fn from(command: &pi_core::protocol::SlashCommand) -> Self {
        Self {
            name: command.name.clone(),
            description: command.description.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub appearance: Appearance,
    pub notify_questions: bool,
    pub notify_finished: bool,
    pub notify_working: bool,
    /// Off: return adds a line and the send button sends.
    pub return_sends: bool,
    /// The computer the phone connected to last, as an SSH address.
    pub computer: Option<String>,
    /// Whether that was the sample sessions instead.
    pub sample: bool,
    /// Each computer's host key fingerprint, kept the first time the phone
    /// connects; a different one later is refused.
    pub host_keys: BTreeMap<String, String>,
    /// Last explicitly chosen project per computer, independent of list order.
    pub projects: BTreeMap<String, String>,
    /// Last command catalog reported by each computer and project. The remote
    /// session remains authoritative and refreshes this after it attaches.
    pub command_catalogs: BTreeMap<String, BTreeMap<String, Vec<SavedCommand>>>,
    pub model: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub thinking: String,
    /// Long lines of code and output wrap; off, they scroll sideways.
    pub wrap_lines: bool,
    /// What you have seen on each computer, by its address. Home lists what
    /// finished since as needing you.
    pub seen: BTreeMap<String, Seen>,
}

/// What you have seen on one computer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Seen {
    /// Seconds since 1970 when the phone started keeping track: sessions that
    /// ended before then count as seen.
    pub since: u64,
    /// Each session's `updated` when you last looked, by its key.
    pub sessions: BTreeMap<String, u64>,
}

impl Seen {
    /// The sessions you looked at most recently: a phone that follows a
    /// computer for months keeps a bounded record, and the rest count as
    /// seen up to when it dropped them.
    pub const KEPT: usize = 300;

    pub fn trim(&mut self) {
        if self.sessions.len() <= Self::KEPT {
            return;
        }
        let mut times: Vec<u64> = self.sessions.values().copied().collect();
        times.sort_unstable_by(|a, b| b.cmp(a));
        let cut = times[Self::KEPT - 1];
        self.since = self.since.max(cut);
        self.sessions.retain(|_, seen| *seen > cut);
    }
}

impl gpui::Global for Prefs {}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            notify_questions: true,
            notify_finished: true,
            // The ongoing notification is how a run reaches the lock screen.
            notify_working: true,
            return_sends: false,
            computer: None,
            sample: false,
            host_keys: BTreeMap::new(),
            projects: BTreeMap::new(),
            command_catalogs: BTreeMap::new(),
            model: "Opus 5.5".into(),
            model_provider: None,
            model_id: None,
            thinking: "High".into(),
            wrap_lines: true,
            seen: BTreeMap::new(),
        }
    }
}

impl Prefs {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        let _trace = gpui_android::trace::scope(c"pi.app.prefs.save");
        let result = serde_json::to_vec_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                let temporary = path.with_extension("tmp");
                std::fs::write(&temporary, bytes)?;
                std::fs::rename(&temporary, path)
            });
        if let Err(error) = result {
            log::warn!("Could not save settings to {}: {error}", path.display());
        }
    }
}

/// Where the settings live: the app's files folder on Android, none elsewhere.
pub fn path(data_dir: Option<PathBuf>) -> Option<PathBuf> {
    data_dir.map(|dir| dir.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_restart_and_tolerate_old_files() {
        let dir = std::env::temp_dir().join(format!("pi-android-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let prefs = Prefs {
            appearance: Appearance::Evening,
            return_sends: true,
            computer: Some("nick@studio-mac.local".into()),
            ..Prefs::default()
        };
        prefs.save(&path);
        assert_eq!(Prefs::load(&path), prefs);
        std::fs::write(&path, r#"{"return_sends": true}"#).unwrap();
        let loaded = Prefs::load(&path);
        assert!(loaded.return_sends && loaded.notify_questions);
        std::fs::remove_dir_all(dir).ok();
    }
}
