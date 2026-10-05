//! Pi's helper on the computer, `pi-desktop-remote`, over SSH. Pi Desktop
//! installs it under `~/.pi/desktop/bin/<hash>/`; the phone uses the newest copy
//! that runs durable sessions, lets several apps watch one, and lists them.

use crate::ssh::{Connection, Pipe, quote};
use anyhow::{Result, bail};
use pi_core::ssh::{PROTOCOL_VERSION, RemoteBackend, SshTarget};
use serde::Deserialize;
use serde_json::Value;

/// Each installed helper with its version and capabilities, newest first, after
/// the account's home folder. No single quotes: it runs inside `sh -c '…'`.
const FIND: &str = r#"printf "home\t%s\n" "$HOME"; ls -t "$HOME"/.pi/desktop/bin/*/pi-desktop-remote 2>/dev/null | while IFS= read -r f; do printf "%s\t%s\t%s\n" "$f" "$("$f" --version 2>/dev/null)" "$("$f" --capabilities 2>/dev/null | tr -d "\n")"; done"#;

#[derive(Clone, Debug, PartialEq)]
pub struct Helper {
    pub path: String,
    /// The computer account's home, to show folders as `~/…`.
    pub home: String,
}

impl Helper {
    fn command(&self, arguments: &str) -> Result<String> {
        Ok(format!("{} {arguments}", quote(&self.path)?))
    }

    /// `~/repos/pi` for `/Users/nick/repos/pi`.
    pub fn short(&self, folder: &str) -> String {
        match folder.strip_prefix(&self.home) {
            Some(rest) if !self.home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
                format!("~{rest}")
            }
            _ => folder.to_owned(),
        }
    }
}

/// Picks a helper from what `FIND` printed.
pub fn choose(listing: &str) -> Result<Helper> {
    let mut home = String::new();
    let (mut any, mut durable) = (false, false);
    for line in listing.lines() {
        let mut fields = line.splitn(3, '\t');
        let (Some(path), Some(version)) = (fields.next(), fields.next()) else {
            continue;
        };
        if path == "home" {
            home = version.trim_end_matches('/').to_owned();
            continue;
        }
        any = true;
        let protocol = version.split_whitespace().nth(2);
        let capabilities: Value =
            serde_json::from_str(fields.next().unwrap_or("")).unwrap_or_default();
        if protocol != Some(&PROTOCOL_VERSION.to_string()) || capabilities["durable"] != true {
            continue;
        }
        durable = true;
        if capabilities["watchers"] == true && capabilities["sessions"] == true {
            return Ok(Helper {
                path: path.to_owned(),
                home,
            });
        }
    }
    match (any, durable) {
        (false, _) => bail!(
            "Pi Desktop's helper isn't on this computer yet. Install one built with durable sessions (see crates/pi_android/README.md)."
        ),
        (true, false) => bail!(
            "The helper on this computer can't run durable sessions. Install one built with bundled-durable."
        ),
        (true, true) => {
            bail!("The helper on this computer is older than the phone. Install the current one.")
        }
    }
}

pub async fn find(connection: &Connection) -> Result<Helper> {
    let output = connection.run(format!("sh -c '{FIND}'")).await?;
    choose(&output.stdout)
}

/// A session as the helper lists it, without attaching.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
    pub key: String,
    pub cwd: String,
    #[serde(default)]
    pub backend: RemoteBackend,
    pub title: Option<String>,
    #[serde(default)]
    pub busy: bool,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
    /// Seconds since 1970 when it last started or ended; 0 if never.
    #[serde(default)]
    pub updated: u64,
    /// Whether its daemon is up, so attaching does not start one.
    #[serde(default)]
    pub running: bool,
}

pub fn parse_sessions(output: &str) -> Result<Vec<Listed>> {
    let listing: Value = serde_json::from_str(output.trim())?;
    if listing["version"] != 1 {
        bail!("The helper listed sessions in a format the phone doesn't know");
    }
    Ok(serde_json::from_value(listing["sessions"].clone())?)
}

pub async fn sessions(connection: &Connection, helper: &Helper) -> Result<Vec<Listed>> {
    let output = connection.run(helper.command("sessions")?).await?;
    if output.status != Some(0) {
        bail!("Listing sessions failed: {}", output.stderr.trim());
    }
    parse_sessions(&output.stdout)
}

/// Attaches to a session, starting its daemon if it isn't up. Records from
/// the session arrive on the pipe; commands go into it.
pub async fn attach(connection: &Connection, helper: &Helper, target: &SshTarget) -> Result<Pipe> {
    let pipe = connection.pipe(helper.command("connect --stdio")?).await?;
    pipe.input.send(target.attach_record()).await?;
    Ok(pipe)
}

/// The identity a new durable session is started with.
pub fn new_target(host: &str, folder: &str) -> Result<SshTarget> {
    let mut target = SshTarget::new(host.to_owned(), folder.to_owned())?;
    target.backend = RemoteBackend::Durable;
    Ok(target)
}

/// The identity of a listed session, to attach to it.
pub fn listed_target(host: &str, listed: &Listed) -> Result<SshTarget> {
    let target = SshTarget {
        host: host.to_owned(),
        cwd: listed.cwd.clone(),
        key: listed.key.clone(),
        backend: listed.backend,
        session_file: None,
    };
    target.validate()?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DURABLE: &str =
        r#"{"pi":true,"durable":true,"durableExperimental":true,"watchers":true,"sessions":true}"#;

    #[test]
    fn the_newest_helper_that_can_do_everything_is_chosen() {
        let listing = format!(
            "home\t/Users/nick\n\
             /Users/nick/.pi/desktop/bin/new/pi-desktop-remote\tpi-desktop-remote 0.2.0 1 macos-arm64\t{DURABLE}\n\
             /Users/nick/.pi/desktop/bin/old/pi-desktop-remote\tpi-desktop-remote 0.1.0 1 macos-arm64\t{DURABLE}\n"
        );
        let helper = choose(&listing).unwrap();
        assert_eq!(
            helper.path,
            "/Users/nick/.pi/desktop/bin/new/pi-desktop-remote"
        );
        assert_eq!(helper.short("/Users/nick/repos/pi"), "~/repos/pi");
        assert_eq!(helper.short("/Users/nickel/x"), "/Users/nickel/x");
        assert_eq!(helper.short("/opt/work"), "/opt/work");
    }

    #[test]
    fn each_missing_piece_is_named() {
        let error = |listing: &str| choose(listing).unwrap_err().to_string();
        assert!(error("home\t/Users/nick\n").contains("isn't on this computer"));
        let stock = "home\t/h\n/h/a\tpi-desktop-remote 0.1.0 1 macos-arm64\t{\"pi\":true,\"durable\":false}\n";
        assert!(error(stock).contains("can't run durable"));
        let old = "home\t/h\n/h/a\tpi-desktop-remote 0.1.0 1 macos-arm64\t{\"pi\":true,\"durable\":true}\n";
        assert!(error(old).contains("older than the phone"));
        let other_protocol =
            format!("home\t/h\n/h/a\tpi-desktop-remote 0.1.0 2 linux-amd64\t{DURABLE}\n");
        assert!(error(&other_protocol).contains("can't run durable"));
    }

    #[test]
    fn the_finder_runs_in_single_quotes() {
        assert!(!FIND.contains('\''));
    }

    #[test]
    fn listed_sessions_become_targets() {
        let sessions = parse_sessions(
            r#"{"version":1,"sessions":[{"key":"0123456789abcdef0123456789abcdef","cwd":"/Users/nick/repos/pi","backend":"durable","title":"Fix the flaky test","busy":true,"stopReason":null,"error":null,"updated":1759600000,"running":true}]}"#,
        )
        .unwrap();
        assert_eq!(sessions[0].title.as_deref(), Some("Fix the flaky test"));
        assert!(sessions[0].busy && sessions[0].running);
        let target = listed_target("nick@studio-mac.local", &sessions[0]).unwrap();
        assert_eq!(target.backend, RemoteBackend::Durable);
        assert_eq!(target.cwd, "/Users/nick/repos/pi");
        assert!(parse_sessions(r#"{"version":2,"sessions":[]}"#).is_err());
        let fresh = new_target("nick@studio-mac.local", "/Users/nick/repos/pi").unwrap();
        assert_eq!(fresh.backend, RemoteBackend::Durable);
        assert_eq!(fresh.key.len(), 32);
    }
}
