//! Pi's helper on the computer, `pi-desktop-remote`, over SSH. Pi Desktop
//! installs it under `~/.pi/desktop/bin/<hash>/`; the phone uses the newest copy
//! that runs durable sessions, lets several apps watch one, and lists them.

use crate::ssh::{Connection, Pipe, quote};
use anyhow::{Context as _, Result, bail, ensure};
use pi_core::{
    remote_files::{FILE_PROTOCOL_VERSION, Request as FileRequest, Tree},
    ssh::{PROTOCOL_VERSION, RemoteBackend, SshTarget},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// Each installed helper with its version and capabilities, newest first, after
/// the account's home folder. No single quotes: it runs inside `sh -c '…'`.
const FIND: &str = r#"printf "home\t%s\n" "$HOME"; ls -t "$HOME"/.pi/desktop/bin/*/pi-desktop-remote 2>/dev/null | while IFS= read -r f; do printf "%s\t%s\t%s\n" "$f" "$("$f" --version 2>/dev/null)" "$("$f" --capabilities 2>/dev/null | tr -d "\n")"; done"#;

#[derive(Clone, Debug, PartialEq)]
pub struct Helper {
    pub path: String,
    /// The computer account's home, to show folders as `~/…`.
    pub home: String,
    pub images: bool,
    /// A paired phone runs through an authorized_keys forced-command gateway.
    pub gateway: bool,
}

impl Helper {
    fn command(&self, arguments: &str) -> Result<String> {
        Ok(format!(
            "{} {arguments}",
            if self.gateway {
                "pi-desktop-remote".to_owned()
            } else {
                quote(&self.path)?
            }
        ))
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
                images: capabilities["imagePrompts"] == true,
                gateway: false,
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
    // A paired key cannot run a shell to search the account. Its forced gateway
    // exposes only this fixed discovery command and the helper's allowlisted API.
    let discovered = connection.run("pi-desktop-remote discover".into()).await?;
    if discovered.status == Some(0)
        && let Ok(helper) = serde_json::from_str::<pi_core::pairing::Helper>(&discovered.stdout)
    {
        return Ok(Helper {
            path: helper.path,
            home: helper.home,
            images: helper.images,
            gateway: helper.gateway,
        });
    }
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

pub async fn models(
    connection: &Connection,
    helper: &Helper,
) -> Result<Vec<pi_core::protocol::Model>> {
    let output = connection.run(helper.command("models")?).await?;
    if output.status != Some(0) {
        if output.stderr.contains("Usage:") {
            bail!(
                "Update the computer's helper to choose a model before starting your first session."
            );
        }
        bail!("Could not load models: {}", output.stderr.trim());
    }
    let catalog: Value = serde_json::from_str(&output.stdout)?;
    if catalog["version"] != 1 {
        bail!("Unsupported model catalog format");
    }
    Ok(serde_json::from_value(catalog["models"].clone())?)
}

pub async fn commands(
    connection: &Connection,
    helper: &Helper,
) -> Result<Vec<pi_core::protocol::SlashCommand>> {
    let output = connection.run(helper.command("commands")?).await?;
    if output.status != Some(0) {
        if output.stderr.contains("Usage:") || output.stderr.contains("cannot run that command") {
            // The session-level get_commands request still discovers commands
            // after attaching to older helpers.
            return Ok(Vec::new());
        }
        bail!("Could not load commands: {}", output.stderr.trim());
    }
    let catalog: Value = serde_json::from_str(&output.stdout)?;
    if catalog["version"] != 1 {
        bail!("Unsupported command catalog format");
    }
    Ok(serde_json::from_value(catalog["commands"].clone())?)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JjHistory {
    pub version: u32,
    pub path: String,
    pub root: Option<String>,
    pub available: bool,
    pub reason: Option<String>,
    pub operations: Vec<JjOperation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JjOperation {
    pub id: String,
    pub time: i64,
    pub description: String,
    pub kind: String,
}

pub async fn jj_history(connection: &Connection, helper: &Helper, path: &str) -> Result<JjHistory> {
    let arguments = format!("jj-history --path {}", quote(path)?);
    let output = connection.run(helper.command(&arguments)?).await?;
    if output.status != Some(0) {
        if output.stderr.contains("Usage:") || output.stderr.contains("cannot run that command") {
            bail!("Update the computer's helper to browse jj file history.");
        }
        bail!("Could not load jj history: {}", output.stderr.trim());
    }
    let history: JjHistory = serde_json::from_str(&output.stdout)?;
    ensure!(history.version == 1, "Unsupported jj history format");
    Ok(history)
}

pub async fn restore_jj_operation(
    connection: &Connection,
    helper: &Helper,
    path: &str,
    operation: &str,
) -> Result<()> {
    let arguments = format!(
        "jj-restore --path {} --operation {}",
        quote(path)?,
        quote(operation)?
    );
    let output = connection.run(helper.command(&arguments)?).await?;
    if output.status != Some(0) {
        bail!("Could not restore jj history: {}", output.stderr.trim());
    }
    let response: Value = serde_json::from_str(&output.stdout)?;
    ensure!(
        response["version"] == 1 && response["restored"].is_string(),
        "Invalid jj restore response"
    );
    Ok(())
}

pub async fn enable_jj(connection: &Connection, helper: &Helper, path: &str) -> Result<JjHistory> {
    let arguments = format!("jj-enable --path {}", quote(path)?);
    let output = connection.run(helper.command(&arguments)?).await?;
    if output.status != Some(0) {
        bail!("Could not enable jj: {}", output.stderr.trim());
    }
    let history: JjHistory = serde_json::from_str(&output.stdout)?;
    ensure!(
        history.version == 1 && history.available,
        "Invalid jj enable response"
    );
    Ok(history)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Directory {
    pub version: u32,
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<Folder>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Folder {
    pub name: String,
    pub path: String,
    pub project: bool,
    /// What makes it a project: "git", "jj" or "package". Older helpers don't say.
    #[serde(default)]
    pub kind: Option<String>,
}

pub fn parse_directory(output: &str) -> Result<Directory> {
    let listing: Directory = serde_json::from_str(output.trim())?;
    if listing.version != 1 || !listing.path.starts_with('/') || listing.entries.len() > 512 {
        bail!("The helper returned an unsupported folder listing");
    }
    Ok(listing)
}

pub async fn directories(
    connection: &Connection,
    helper: &Helper,
    path: &str,
    show_hidden: bool,
) -> Result<Directory> {
    let command = helper.command(&format!(
        "directories --path {}{}",
        quote(path)?,
        if show_hidden { " --show-hidden" } else { "" }
    ))?;
    let output = connection.run(command).await?;
    if output.status != Some(0) {
        if output.stderr.contains("Usage:") {
            bail!(
                "Update Pi Desktop's remote helper to browse folders. Existing projects remain available."
            );
        }
        bail!("Could not open this folder: {}", output.stderr.trim());
    }
    parse_directory(&output.stdout)
}

async fn file_request(pipe: &Pipe, id: &str, request: FileRequest) -> Result<Value> {
    let mut record = serde_json::to_value(request)?;
    record
        .as_object_mut()
        .context("Invalid remote file request")?
        .insert("id".into(), json!(id));
    pipe.input
        .send(record)
        .await
        .map_err(|_| anyhow::anyhow!("The remote file channel closed"))?;
    while let Ok(record) = pipe.records.recv().await {
        if record["type"] == "response" && record["id"] == id {
            ensure!(
                record["success"] == true,
                "{}",
                record["error"]
                    .as_str()
                    .unwrap_or("The computer could not list project files")
            );
            return Ok(record["data"].clone());
        }
    }
    let ended = pipe
        .ended
        .recv()
        .await
        .unwrap_or_else(|_| "The remote file channel closed".into());
    bail!("{ended}")
}

/// Opens the helper's file channel on a project.
async fn files_channel(
    connection: &Connection,
    helper: &Helper,
    host: &str,
    cwd: &str,
) -> Result<Pipe> {
    let pipe = connection.pipe(helper.command("files --stdio")?).await?;
    let target = SshTarget::new(host.to_owned(), cwd.to_owned())?;
    file_request(
        &pipe,
        "phone-files-attach",
        FileRequest::FilesAttach {
            version: FILE_PROTOCOL_VERSION,
            target,
        },
    )
    .await?;
    Ok(pipe)
}

/// The bounded project tree, folders and files, relative to `cwd`.
pub async fn project_tree(
    connection: &Connection,
    helper: &Helper,
    host: &str,
    cwd: &str,
) -> Result<Tree> {
    let pipe = files_channel(connection, helper, host, cwd).await?;
    Ok(serde_json::from_value(
        file_request(&pipe, "phone-files-list", FileRequest::FilesList).await?,
    )?)
}

/// Lists the bounded project tree through the helper's existing file channel.
/// Paths stay relative to `cwd`, which is exactly what a prompt mention needs.
pub async fn project_files(
    connection: &Connection,
    helper: &Helper,
    host: &str,
    cwd: &str,
) -> Result<Vec<String>> {
    Ok(project_tree(connection, helper, host, cwd)
        .await?
        .entries
        .into_iter()
        .map(|entry| {
            if entry.directory {
                format!("{}/", entry.path.trim_end_matches('/'))
            } else {
                entry.path
            }
        })
        .collect())
}

/// A text file in a project, up to 1 MB, read-only.
pub async fn read_file(
    connection: &Connection,
    helper: &Helper,
    host: &str,
    cwd: &str,
    path: &str,
) -> Result<String> {
    let pipe = files_channel(connection, helper, host, cwd).await?;
    let document: pi_core::remote_files::Document = serde_json::from_value(
        file_request(
            &pipe,
            "phone-files-read",
            FileRequest::FilesRead { path: path.into() },
        )
        .await?,
    )?;
    Ok(document.text)
}

/// Attaches to a session, starting its daemon if it isn't up. Records from
/// the session arrive on the pipe; commands go into it.
pub async fn attach(connection: &Connection, helper: &Helper, target: &SshTarget) -> Result<Pipe> {
    let pipe = connection.pipe(helper.command("connect --stdio")?).await?;
    pipe.input.send(target.attach_record()).await?;
    Ok(pipe)
}

/// A separate short-lived attachment keeps deletion independent of the UI's
/// watch. The helper validates the exact key and refuses an active writer.
pub async fn delete(connection: &Connection, helper: &Helper, target: &SshTarget) -> Result<()> {
    target.validate()?;
    let pipe = attach(connection, helper, target).await?;
    let id = format!("phone-delete-{:016x}", rand::random::<u64>());
    let mut sent = false;
    while let Ok(record) = pipe.records.recv().await {
        if record["type"] == "remote_error" {
            bail!(
                "{}",
                record["error"]
                    .as_str()
                    .unwrap_or("The computer refused deletion")
            );
        }
        if record["type"] == "remote_snapshot" && !sent {
            pipe.input.send(serde_json::json!({"type":"remote_delete_session", "id":id, "confirmKey":target.key})).await?;
            sent = true;
        }
        if record["type"] == "response" && record["id"] == id {
            if record["success"] == true && record["data"]["key"] == target.key {
                return Ok(());
            }
            let error = record["error"]
                .as_str()
                .unwrap_or("The computer refused deletion");
            if error.contains("unknown variant") {
                bail!(
                    "Update Pi Desktop's remote helper on the computer to enable session deletion. Nothing was deleted."
                );
            }
            bail!("{error}");
        }
    }
    bail!(
        "The connection closed before deletion was confirmed. Refresh sessions before trying again."
    )
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

    #[test]
    fn folder_listings_are_versioned_and_preserve_unicode_and_spaces() {
        let listing = parse_directory(r#"{"version":1,"path":"/Users/me/my repos","parent":"/Users/me","entries":[{"name":"café's project","path":"/Users/me/my repos/café's project","project":true}],"truncated":false}"#).unwrap();
        assert_eq!(listing.entries[0].name, "café's project");
        assert!(
            parse_directory(
                r#"{"version":2,"path":"/","parent":null,"entries":[],"truncated":false}"#
            )
            .is_err()
        );
        assert!(
            parse_directory(
                r#"{"version":1,"path":"relative","parent":null,"entries":[],"truncated":false}"#
            )
            .is_err()
        );
    }

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
