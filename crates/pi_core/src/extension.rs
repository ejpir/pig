//! Pi Desktop's pi extension, `extension/pi-desktop.ts`. Every session's pi loads it
//! (`-e`): it answers the commands pi's RPC mode lacks over the desktop channel
//! ([`crate::channel`]) and sends the language-server and jj bridge requests.

use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};

use crate::protocol::SlashCommand;

pub const SOURCE: &str = include_str!("../extension/pi-desktop.ts");
const FILE: &str = "pi-desktop.ts";
/// The extension's own command, which pi lists with the user's commands.
const COMMAND: &str = "pi-desktop";
/// The pi release the extension is written against, and the one packaging embeds.
pub const PI_VERSION: &str = "1.0.0";

/// Where sessions load the extension from: the cache folder, else the temporary folder.
pub fn default_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("pi-desktop")
        .join("extension")
}

/// Writes the extension into `dir` when it is missing or differs, and returns its path.
/// It is written to a temporary file and renamed, so a pi starting at the same time
/// never loads half of it.
pub fn install(dir: &Path) -> Result<PathBuf> {
    let path = dir.join(FILE);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(SOURCE) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let partial = dir.join(format!(".{FILE}.{}", std::process::id()));
        std::fs::write(&partial, SOURCE)
            .with_context(|| format!("writing {}", partial.display()))?;
        if let Err(error) = std::fs::rename(&partial, &path) {
            let _ = std::fs::remove_file(&partial);
            return Err(error).with_context(|| format!("writing {}", path.display()));
        }
    }
    Ok(path)
}

/// The extension's own slash command, which is not one of the user's.
pub fn is_own(command: &SlashCommand) -> bool {
    command.source == "extension"
        && command.name == COMMAND
        && command
            .source_info
            .as_ref()
            .is_some_and(|info| Path::new(&info.path).file_name() == Some(FILE.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_once_and_repairs_a_changed_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = install(&dir.path().join("extension")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SOURCE);
        std::fs::write(&path, "stale").unwrap();
        assert_eq!(install(&dir.path().join("extension")).unwrap(), path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SOURCE);
        let names: Vec<_> = std::fs::read_dir(dir.path().join("extension"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [FILE], "no temporary file is left behind");
    }

    #[test]
    fn the_source_pins_the_same_pi_release() {
        assert!(SOURCE.contains(&format!("const EXTENSION_VERSION = \"{PI_VERSION}-")));
    }
}
