//! Isolated session operations. A fork uses a temporary RPC child, never repurposes
//! the process belonging to the original desktop tab, and never edits JSONL itself.
use crate::{
    protocol::{Command, SavedSession, SessionState},
    transport::{Backend, Launch, RpcClient, TransportEvent},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::PathBuf;

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
/// Runs one command in a temporary pi opened on a saved session, after checking pi
/// resumed that session. For sessions not open in a tab; an open one uses its own pi.
pub async fn on_session(
    cwd: PathBuf,
    path: String,
    expected_id: String,
    command: Command,
    backend: Backend,
) -> Result<Value> {
    let client = RpcClient::spawn(Launch::pi_with(cwd, Some(&path), &backend))?;
    let state: SessionState = serde_json::from_value(request(&client, Command::GetState).await?)?;
    if state.session_id.as_deref() != Some(&expected_id) {
        bail!("Pi did not resume the selected session; nothing was changed");
    }
    request(&client, command).await
}

/// Deletes a saved session file from a throwaway pi that saves no session itself:
/// pi refuses to delete the session a process has open.
pub async fn delete(cwd: PathBuf, path: String, backend: Backend) -> Result<Value> {
    let mut launch = Launch::pi_with(cwd, None, &backend);
    launch.args.push("--no-session".into());
    let client = RpcClient::spawn(launch)?;
    request(&client, Command::DeleteSession { session_path: path }).await
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
/// the entry; that needs pi-desktop-backend's `fork_cwd`.
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
            .context("This backend cannot fork into another folder")?;
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
