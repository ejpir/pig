//! Isolated session operations. A fork uses a temporary RPC child, never repurposes
//! the process belonging to the original desktop tab, and never edits JSONL itself.
use crate::{
    protocol::{Command, SavedSession, SessionState},
    transport::{Backend, Launch, RpcClient, TransportEvent},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::{BufRead as _, Read as _},
    path::{Path, PathBuf},
    time::Duration,
};

/// A fork as the desktop opens it.
#[derive(Clone, Debug, PartialEq)]
pub struct Forked {
    pub cwd: PathBuf,
    /// `None` for a fork with no reply yet, which pi saves only once it has one:
    /// the desktop starts a new session in `cwd` with the draft.
    pub saved: Option<SavedSession>,
    /// The forked user message, to edit and send again.
    pub draft: String,
}

async fn request(client: &RpcClient, command: Command) -> Result<Value> {
    let id = client.send(command)?;
    let events = client.events();
    while let Ok(event) = events.recv().await {
        match event {
            TransportEvent::Record(r) if r["type"] == "response" && r["id"] == id => {
                if r["success"] != true {
                    bail!("{}", r["error"].as_str().unwrap_or("RPC command failed"));
                }
                return Ok(r["data"].clone());
            }
            TransportEvent::Record(r)
                if r["type"] == "extension_ui_request"
                    && matches!(
                        r["method"].as_str(),
                        Some("select" | "confirm" | "input" | "editor")
                    ) =>
            {
                client.send_record(
                    json!({"type":"extension_ui_response","id":r["id"],"cancelled":true}),
                )?
            }
            TransportEvent::RequestFailed {
                id: failed, error, ..
            } if failed == id => bail!("{error}"),
            TransportEvent::ProtocolError(error) => bail!("{error}"),
            TransportEvent::Exited { description, .. } => bail!("{description}"),
            _ => {}
        }
    }
    bail!("Pi disconnected during fork")
}
/// A temporary pi opened on a saved session, once it has resumed that session.
async fn resumed(
    cwd: PathBuf,
    path: &str,
    expected_id: &str,
    backend: &Backend,
) -> Result<RpcClient> {
    let client = RpcClient::spawn(Launch::pi_with(cwd, Some(path), backend))?;
    let state: SessionState = serde_json::from_value(request(&client, Command::GetState).await?)?;
    if state.session_id.as_deref() != Some(expected_id) {
        bail!("Pi did not resume the selected session; nothing was changed");
    }
    Ok(client)
}

/// Runs one command in a temporary pi opened on a saved session, after checking pi
/// resumed that session. For sessions not open in a tab; an open one uses its own pi.
pub async fn on_session(
    cwd: PathBuf,
    path: String,
    expected_id: String,
    command: Command,
    backend: Backend,
) -> Result<Value> {
    let client = resumed(cwd, &path, &expected_id, &backend).await?;
    request(&client, command).await
}

/// Runs blocking work (processes, the trash) off the caller's executor.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _closed = sender.send_blocking(work());
    });
    receiver.recv().await.context("Background work stopped")?
}

/// A saved session file, checked by its header before anything touches it.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn session_file(path: &Path) -> Result<PathBuf> {
    anyhow::ensure!(path.is_absolute(), "The session path must be absolute");
    let file = path
        .canonicalize()
        .context("The session file no longer exists")?;
    anyhow::ensure!(file.is_file(), "Not a saved session file");
    let mut header = String::new();
    std::io::BufReader::new(std::fs::File::open(&file)?.take(64 * 1024)).read_line(&mut header)?;
    let header: Value = serde_json::from_str(header.trim_start_matches('\u{feff}'))
        .context("Not a saved session file")?;
    anyhow::ensure!(
        header["type"] == "session"
            && header["id"].is_string()
            && header["cwd"]
                .as_str()
                .is_some_and(|cwd| Path::new(cwd).is_absolute()),
        "Not a saved session file"
    );
    Ok(file)
}

/// Moves a saved session file to the system trash; there is no permanent-delete
/// fallback. The desktop refuses sessions open in a tab.
#[cfg(not(target_os = "android"))]
pub async fn delete(path: String) -> Result<Value> {
    blocking(move || {
        let file = session_file(Path::new(&path))?;
        trash::delete(&file)
            .context("Could not move the session to the trash; nothing was deleted")?;
        Ok(json!({ "method": "trash" }))
    })
    .await
}

/// Shares a saved session as pi's `/share` does: to Radius when pi has it set up,
/// otherwise as a private gist of pi's HTML export, made with the GitHub CLI.
pub async fn share(
    cwd: PathBuf,
    path: String,
    expected_id: String,
    backend: Backend,
) -> Result<Value> {
    let client = resumed(cwd, &path, &expected_id, &backend).await?;
    let radius = request(&client, Command::Share).await?;
    if !radius["destination"].is_null() {
        return Ok(radius);
    }
    let folder = tempfile::Builder::new()
        .prefix("pi-desktop-share-")
        .tempdir()?;
    let html = folder.path().join("session.html");
    request(
        &client,
        Command::ExportHtml {
            output_path: Some(html.to_string_lossy().into_owned()),
        },
    )
    .await?;
    blocking(move || {
        let gist = gist(&html);
        drop(folder);
        gist
    })
    .await
}

fn gist(html: &Path) -> Result<Value> {
    let gh = |args: &[&std::ffi::OsStr]| {
        let mut command = std::process::Command::new("gh");
        command
            .args(args)
            .envs(crate::transport::environment(|name| std::env::var_os(name)));
        crate::bounded_output(&mut command, Duration::from_secs(20))
    };
    let status = match gh(&["auth".as_ref(), "status".as_ref()]) {
        Ok(output) => output.status,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            bail!("GitHub CLI (gh) is not installed")
        }
        Err(error) => return Err(error),
    };
    if !status.success() {
        bail!("GitHub CLI is not logged in. Run gh auth login first.");
    }
    let created = gh(&[
        "gist".as_ref(),
        "create".as_ref(),
        "--public=false".as_ref(),
        html.as_os_str(),
    ])?;
    if !created.status.success() {
        bail!("Creating the private gist failed");
    }
    let gist_url = String::from_utf8_lossy(&created.stdout).trim().to_owned();
    let id = gist_url
        .strip_prefix("https://gist.github.com/")
        .and_then(|rest| rest.rsplit('/').next())
        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit()))
        .context("Invalid gist URL from GitHub CLI")?;
    // pi's session viewer, as pi's `/share` builds it.
    let viewer =
        std::env::var("PI_SHARE_VIEWER_URL").unwrap_or_else(|_| "https://pi.dev/session/".into());
    Ok(json!({ "destination": "gist", "gistUrl": gist_url, "url": format!("{viewer}#{id}") }))
}

/// Duplicates a saved session's active branch into a new session.
pub async fn clone(
    cwd: PathBuf,
    path: String,
    expected_id: String,
    backend: Backend,
) -> Result<SavedSession> {
    let client = RpcClient::spawn(Launch::pi_with(cwd.clone(), Some(&path), &backend))?;
    let state: SessionState = serde_json::from_value(request(&client, Command::GetState).await?)?;
    if state.session_id.as_deref() != Some(&expected_id) {
        bail!("Pi did not resume the selected session; clone was not attempted");
    }
    if request(&client, Command::Clone).await?["cancelled"] == true {
        bail!("Clone cancelled by Pi or an extension");
    }
    let state: SessionState = serde_json::from_value(request(&client, Command::GetState).await?)?;
    let id = state.session_id.context("Clone returned no session ID")?;
    let new_path = state
        .session_file
        .context("Clone returned no session path")?;
    if id == expected_id || new_path == path {
        bail!("Pi did not create a distinct clone");
    }
    Ok(SavedSession {
        id,
        path: new_path,
        cwd: cwd.to_string_lossy().into_owned(),
        name: state.session_name,
        ..Default::default()
    })
}

/// Forks a saved session before one of its user messages. With `into`, the fork
/// works in that folder, such as a jj workspace holding the files as they were at
/// the entry; the desktop extension writes that session file.
pub async fn fork(
    cwd: PathBuf,
    path: String,
    expected_id: String,
    entry_id: String,
    into: Option<PathBuf>,
    backend: Backend,
) -> Result<Forked> {
    let client = RpcClient::spawn(Launch::pi_with(cwd.clone(), Some(&path), &backend))?;
    let state: SessionState = serde_json::from_value(request(&client, Command::GetState).await?)?;
    if state.session_id.as_deref() != Some(&expected_id) {
        bail!("Pi did not resume the selected session; fork was not attempted");
    }
    let result = request(
        &client,
        Command::Fork {
            entry_id,
            cwd: into
                .as_ref()
                .map(|into| into.to_string_lossy().into_owned()),
        },
    )
    .await?;
    if result["cancelled"] == true {
        bail!("Fork cancelled by Pi or an extension");
    }
    let draft = result["text"].as_str().unwrap_or("").to_string();
    if let Some(into) = into {
        let new_path = result["sessionPath"]
            .as_str()
            .context("Pi did not report the forked session's file")?;
        let saved = std::path::Path::new(new_path)
            .is_file()
            .then(|| SavedSession {
                id: result["sessionId"].as_str().unwrap_or_default().to_owned(),
                path: new_path.to_owned(),
                cwd: into.to_string_lossy().into_owned(),
                first_message: draft.clone(),
                ..Default::default()
            });
        return Ok(Forked {
            cwd: into,
            saved,
            draft,
        });
    }
    let state: SessionState = serde_json::from_value(request(&client, Command::GetState).await?)?;
    let id = state.session_id.context("Fork returned no session ID")?;
    let new_path = state
        .session_file
        .context("Fork returned no session path")?;
    if id == expected_id || new_path == path {
        bail!("Pi did not create a distinct fork");
    }
    Ok(Forked {
        cwd: cwd.clone(),
        saved: Some(SavedSession {
            id,
            path: new_path,
            cwd: cwd.to_string_lossy().into_owned(),
            name: state.session_name,
            first_message: draft.clone(),
            ..Default::default()
        }),
        draft,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_saved_session_files_pass_the_check_before_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("session.jsonl");
        let cwd = dir.path().to_string_lossy().into_owned();
        std::fs::write(
            &session,
            format!(
                "{}\n{{\"type\":\"message\"}}\n",
                json!({"type": "session", "id": "s1", "cwd": cwd})
            ),
        )
        .unwrap();
        assert_eq!(
            session_file(&session).unwrap(),
            session.canonicalize().unwrap()
        );

        let other = dir.path().join("notes.jsonl");
        std::fs::write(&other, "{\"type\":\"message\"}\n").unwrap();
        assert!(session_file(&other).is_err());
        let relative = dir.path().join("relative.jsonl");
        std::fs::write(
            &relative,
            format!(
                "{}\n",
                json!({"type": "session", "id": "s2", "cwd": "project"})
            ),
        )
        .unwrap();
        assert!(
            session_file(&relative).is_err(),
            "the header's folder must be absolute"
        );
        assert!(
            session_file(Path::new("session.jsonl")).is_err(),
            "relative paths are refused"
        );
        assert!(session_file(&dir.path().join("missing.jsonl")).is_err());
        assert!(session_file(dir.path()).is_err(), "folders are refused");
    }
}
