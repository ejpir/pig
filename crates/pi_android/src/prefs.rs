//! Settings that stay on the phone, as JSON in the app's private storage.

use crate::theme::Appearance;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

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
    pub model: String,
    pub thinking: String,
}

impl gpui::Global for Prefs {}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            notify_questions: true,
            notify_finished: true,
            notify_working: false,
            return_sends: false,
            computer: None,
            sample: false,
            host_keys: BTreeMap::new(),
            model: "Opus 5.5".into(),
            thinking: "High".into(),
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
