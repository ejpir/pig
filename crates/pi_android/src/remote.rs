//! Pi's helper on the computer, `pi-desktop-remote`, over SSH. Pi Desktop
//! installs immutable content objects and activates one stable helper path. The
//! phone uses that explicit selection rather than directory mtimes.

use crate::ssh::{Connection, Pipe, quote};
use anyhow::{Context as _, Result, bail, ensure};
use pi_core::{
    pairing::HelperCapabilities,
    remote_files::{FILE_PROTOCOL_VERSION, Request as FileRequest, Tree},
    ssh::{PROTOCOL_VERSION, RemoteBackend, SshTarget},
};
use semver::Version;
use serde::Deserialize;
use serde_json::{Value, json};

/// The explicitly activated helper, after the account's home folder. No single
/// quotes: this runs inside `sh -c '…'`.
const FIND: &str = r#"printf "home\t%s\n" "$HOME"; f="$HOME/.pi/desktop/bin/pi-desktop-remote"; if test -x "$f"; then printf "%s\t%s\t%s\n" "$f" "$("$f" --version 2>/dev/null)" "$("$f" --capabilities 2>/dev/null | tr -d "\n")"; fi"#;

pub const APP_RELEASE: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/earendil-works/pi/releases";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperUpdateState {
    UpdateRequired,
    UpdateRecommended,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelperUpdate {
    pub state: HelperUpdateState,
    pub installed_release: Option<String>,
    pub app_release: String,
    pub platform: Option<String>,
    pub reasons: Vec<String>,
}

impl HelperUpdate {
    pub fn title(&self, computer: &str) -> String {
        match self.state {
            HelperUpdateState::UpdateRequired => {
                format!("Helper update required on {computer}")
            }
            HelperUpdateState::UpdateRecommended => {
                format!("Helper update recommended on {computer}")
            }
        }
    }

    pub fn details(&self, computer: &str) -> String {
        let installed = self.installed_release.as_deref().unwrap_or("unknown");
        let platform = self
            .platform
            .as_deref()
            .map(|platform| format!(" for {platform}"))
            .unwrap_or_default();
        let comparison = match self.state {
            HelperUpdateState::UpdateRequired => format!(
                "Pi Android {} cannot use all requested features: {}.",
                self.app_release,
                self.reasons.join(", ")
            ),
            HelperUpdateState::UpdateRecommended => format!(
                "It is compatible, but older than the current Pi Android {} release.",
                self.app_release
            ),
        };
        let artifact = self
            .platform
            .as_deref()
            .map(|platform| format!(" (pi-desktop-remote-{platform})"))
            .unwrap_or_default();
        format!(
            "{computer} has pi-desktop-remote {installed}{platform}. {comparison} Manually update `pi-desktop-remote` from the matching GitHub release{artifact}: {RELEASES}/tag/v{}, then reconnect.",
            self.app_release
        )
    }
}

struct HelperMetadata {
    release: Option<String>,
    protocol: Option<u32>,
    platform: Option<String>,
    capabilities: HelperCapabilities,
    legacy_images: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Helper {
    pub path: String,
    /// The computer account's home, to show folders as `~/…`.
    pub home: String,
    pub images: bool,
    /// A paired phone runs through an authorized_keys forced-command gateway.
    pub gateway: bool,
    pub release: Option<String>,
    pub protocol: Option<u32>,
    pub platform: Option<String>,
    pub capabilities: HelperCapabilities,
    pub update: Option<HelperUpdate>,
}

impl Helper {
    fn new(path: String, home: String, gateway: bool, metadata: HelperMetadata) -> Self {
        let HelperMetadata {
            release,
            protocol,
            platform,
            mut capabilities,
            legacy_images,
        } = metadata;
        capabilities.image_prompts |= legacy_images;
        let images = capabilities.image_prompts;
        let update = evaluate_update(
            release.as_deref(),
            protocol,
            platform.as_deref(),
            &capabilities,
        );
        Self {
            path,
            home,
            images,
            gateway,
            release,
            protocol,
            platform,
            capabilities,
            update,
        }
    }

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

    fn reevaluate(&mut self) {
        self.capabilities.image_prompts |= self.images;
        self.images = self.capabilities.image_prompts;
        self.update = evaluate_update(
            self.release.as_deref(),
            self.protocol,
            self.platform.as_deref(),
            &self.capabilities,
        );
    }

    pub fn can_list_sessions(&self) -> bool {
        self.protocol == Some(PROTOCOL_VERSION)
            && self.capabilities.durable
            && self.capabilities.sessions
    }

    pub fn can_attach(&self) -> bool {
        self.protocol == Some(PROTOCOL_VERSION)
            && self.capabilities.durable
            && self.capabilities.watchers
    }

    pub fn require_attach(&self, action: &str) -> Result<()> {
        if self.can_attach() {
            return Ok(());
        }
        bail!("{}", self.action_update_message(action))
    }

    fn require_capability(&self, supported: bool, action: &str) -> Result<()> {
        if supported {
            return Ok(());
        }
        bail!("{}", self.action_update_message(action))
    }

    fn action_update_message(&self, action: &str) -> String {
        format!(
            "Update `pi-desktop-remote` from {RELEASES}/tag/v{APP_RELEASE} and reconnect before {action}. Existing sessions and drafts are kept."
        )
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

fn evaluate_update(
    release: Option<&str>,
    protocol: Option<u32>,
    platform: Option<&str>,
    capabilities: &HelperCapabilities,
) -> Option<HelperUpdate> {
    let mut reasons = Vec::new();
    if protocol != Some(PROTOCOL_VERSION) {
        reasons.push(match protocol {
            Some(protocol) => format!("helper protocol {protocol}, required {PROTOCOL_VERSION}"),
            None => format!("unknown helper protocol, required {PROTOCOL_VERSION}"),
        });
    }
    for (supported, name) in [
        (capabilities.durable, "durable sessions"),
        (capabilities.watchers, "multi-client session watching"),
        (capabilities.sessions, "session discovery"),
    ] {
        if !supported {
            reasons.push(format!("missing {name}"));
        }
    }
    if !reasons.is_empty() {
        return Some(HelperUpdate {
            state: HelperUpdateState::UpdateRequired,
            installed_release: release.map(str::to_owned),
            app_release: APP_RELEASE.into(),
            platform: platform.map(str::to_owned),
            reasons,
        });
    }

    let installed =
        release.and_then(|release| Version::parse(release.trim_start_matches('v')).ok());
    let current = Version::parse(APP_RELEASE).ok();
    matches!((installed, current), (Some(installed), Some(current)) if installed < current).then(
        || HelperUpdate {
            state: HelperUpdateState::UpdateRecommended,
            installed_release: release.map(str::to_owned),
            app_release: APP_RELEASE.into(),
            platform: platform.map(str::to_owned),
            reasons: Vec::new(),
        },
    )
}

fn version_metadata(version: &str) -> (Option<String>, Option<u32>, Option<String>) {
    let mut fields = version.split_whitespace();
    if fields.next() != Some("pi-desktop-remote") {
        return (None, None, None);
    }
    let release = fields.next().map(str::to_owned);
    let protocol = fields.next().and_then(|protocol| protocol.parse().ok());
    let platform = fields.next().map(str::to_owned);
    if fields.next().is_some() {
        return (None, None, None);
    }
    (release, protocol, platform)
}

fn capabilities(output: &str) -> HelperCapabilities {
    serde_json::from_str(output.trim()).unwrap_or_default()
}

fn from_discovery(helper: pi_core::pairing::Helper) -> Helper {
    Helper::new(
        helper.path,
        helper.home,
        helper.gateway,
        HelperMetadata {
            release: helper.release,
            protocol: helper.protocol,
            platform: helper.platform,
            capabilities: helper.capabilities,
            legacy_images: helper.images,
        },
    )
}

async fn probe(connection: &Connection, mut helper: Helper) -> Helper {
    if let Ok(command) = helper.command("--version")
        && let Ok(output) = connection.run(command).await
        && output.status == Some(0)
    {
        let (release, protocol, platform) = version_metadata(output.stdout.trim());
        helper.release = release.or(helper.release);
        helper.protocol = protocol.or(helper.protocol);
        helper.platform = platform.or(helper.platform);
    }
    if let Ok(command) = helper.command("--capabilities")
        && let Ok(output) = connection.run(command).await
        && output.status == Some(0)
    {
        helper.capabilities = capabilities(&output.stdout);
    }
    helper.reevaluate();
    helper
}

/// Picks the stable helper from what `FIND` printed, retaining incompatible
/// metadata so Android can explain a required manual update.
pub fn choose(listing: &str) -> Result<Helper> {
    let mut home = String::new();
    for line in listing.lines() {
        let mut fields = line.splitn(3, '\t');
        let (Some(path), Some(version)) = (fields.next(), fields.next()) else {
            continue;
        };
        if path == "home" {
            home = version.trim_end_matches('/').to_owned();
            continue;
        }
        let (release, protocol, platform) = version_metadata(version);
        return Ok(Helper::new(
            path.to_owned(),
            home,
            false,
            HelperMetadata {
                release,
                protocol,
                platform,
                capabilities: capabilities(fields.next().unwrap_or("")),
                legacy_images: false,
            },
        ));
    }
    bail!(
        "Pi Desktop's helper isn't on this computer yet. Install `pi-desktop-remote` from the matching GitHub release."
    )
}

pub async fn find(connection: &Connection) -> Result<Helper> {
    // A paired key cannot run a shell to search the account. Its forced gateway
    // exposes only discover, --version, --capabilities and the helper API.
    let discovered = connection.run("pi-desktop-remote discover".into()).await?;
    if discovered.status == Some(0)
        && let Ok(helper) = serde_json::from_str::<pi_core::pairing::Helper>(&discovered.stdout)
    {
        return Ok(probe(connection, from_discovery(helper)).await);
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
    helper.require_capability(helper.can_list_sessions(), "listing sessions")?;
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
    helper.require_capability(helper.capabilities.durable, "loading models")?;
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
    if !helper.capabilities.commands {
        // Session-level get_commands remains available after attaching.
        return Ok(Vec::new());
    }
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
    helper.require_capability(helper.capabilities.jj_history, "browsing jj file history")?;
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
    helper.require_capability(helper.capabilities.jj_history, "restoring jj file history")?;
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
    helper.require_capability(helper.capabilities.jj_history, "enabling jj file history")?;
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
    helper.require_capability(helper.capabilities.directories, "browsing folders")?;
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
    helper.require_attach("opening sessions")?;
    let pipe = connection.pipe(helper.command("connect --stdio")?).await?;
    pipe.input.send(target.attach_record()).await?;
    Ok(pipe)
}

/// A separate short-lived attachment keeps deletion independent of the UI's
/// watch. The helper validates the exact key and refuses an active writer.
pub async fn delete(connection: &Connection, helper: &Helper, target: &SshTarget) -> Result<()> {
    helper.require_capability(helper.capabilities.delete_sessions, "deleting sessions")?;
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
    fn the_explicitly_activated_helper_is_chosen() {
        let listing = format!(
            "home\t/Users/nick\n\
             /Users/nick/.pi/desktop/bin/pi-desktop-remote\tpi-desktop-remote 0.2.0 {PROTOCOL_VERSION} macos-arm64\t{DURABLE}\n"
        );
        let helper = choose(&listing).unwrap();
        assert_eq!(helper.path, "/Users/nick/.pi/desktop/bin/pi-desktop-remote");
        assert_eq!(helper.release.as_deref(), Some("0.2.0"));
        assert_eq!(helper.protocol, Some(PROTOCOL_VERSION));
        assert_eq!(helper.platform.as_deref(), Some("macos-arm64"));
        assert!(helper.capabilities.durable && helper.capabilities.watchers);
        assert_eq!(helper.short("/Users/nick/repos/pi"), "~/repos/pi");
        assert_eq!(helper.short("/Users/nickel/x"), "/Users/nickel/x");
        assert_eq!(helper.short("/opt/work"), "/opt/work");
    }

    #[test]
    fn incompatible_helpers_are_retained_for_a_required_update() {
        assert!(
            choose("home\t/Users/nick\n")
                .unwrap_err()
                .to_string()
                .contains("isn't on this computer")
        );
        let stock = format!(
            "home\t/h\n/h/a\tpi-desktop-remote 0.0.4 {PROTOCOL_VERSION} macos-arm64\t{{\"pi\":true,\"durable\":false}}\n"
        );
        let helper = choose(&stock).unwrap();
        let update = helper.update.unwrap();
        assert_eq!(update.state, HelperUpdateState::UpdateRequired);
        assert!(update.reasons.contains(&"missing durable sessions".into()));
        assert!(update.reasons.contains(&"missing session discovery".into()));

        let old_protocol = PROTOCOL_VERSION - 1;
        let other_protocol = format!(
            "home\t/h\n/h/a\tpi-desktop-remote 0.0.4 {old_protocol} linux-amd64\t{DURABLE}\n"
        );
        let update = choose(&other_protocol).unwrap().update.unwrap();
        assert_eq!(update.state, HelperUpdateState::UpdateRequired);
        assert!(
            update.reasons[0].contains(&format!("protocol {old_protocol}")),
            "{:?}",
            update.reasons
        );
    }

    #[test]
    fn releases_compare_semantically_without_downgrade_notices() {
        let compatible = capabilities(DURABLE);
        let recommended = evaluate_update(
            Some("0.0.4"),
            Some(PROTOCOL_VERSION),
            Some("linux-arm64"),
            &compatible,
        )
        .unwrap();
        assert_eq!(recommended.state, HelperUpdateState::UpdateRecommended);

        assert_eq!(
            evaluate_update(
                Some(APP_RELEASE),
                Some(PROTOCOL_VERSION),
                Some("linux-arm64"),
                &compatible,
            ),
            None
        );
        assert_eq!(
            evaluate_update(
                Some("0.0.10"),
                Some(PROTOCOL_VERSION),
                Some("linux-arm64"),
                &compatible,
            ),
            None,
            "a newer compatible helper must not suggest a downgrade"
        );

        let mut reconnected = Helper::new(
            "/helper".into(),
            "/home/me".into(),
            true,
            HelperMetadata {
                release: Some("0.0.4".into()),
                protocol: Some(0),
                platform: Some("linux-arm64".into()),
                capabilities: HelperCapabilities::default(),
                legacy_images: false,
            },
        );
        assert_eq!(
            reconnected.update.as_ref().map(|update| update.state),
            Some(HelperUpdateState::UpdateRequired)
        );
        reconnected.release = Some("0.0.10".into());
        reconnected.protocol = Some(PROTOCOL_VERSION);
        reconnected.capabilities = compatible;
        reconnected.reevaluate();
        assert_eq!(reconnected.update, None, "reconnect clears the warning");
    }

    #[test]
    fn paired_metadata_probes_use_only_allowlisted_commands() {
        let helper = Helper::new(
            "/ignored/by/gateway".into(),
            "/home/me".into(),
            true,
            HelperMetadata {
                release: None,
                protocol: None,
                platform: None,
                capabilities: HelperCapabilities::default(),
                legacy_images: false,
            },
        );
        assert_eq!(
            helper.command("--version").unwrap(),
            "pi-desktop-remote --version"
        );
        assert_eq!(
            helper.command("--capabilities").unwrap(),
            "pi-desktop-remote --capabilities"
        );
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
