//! Typed, independent SSH file channel. All methods block: use a background executor.
use crate::{
    ssh::{self, SshTarget},
    transport::{RpcClient, TransportEvent},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Mutex;

pub const FILE_PROTOCOL_VERSION: u32 = 1;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_TREE_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub directory: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tree {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub path: String,
    pub text: String,
    /// SHA-256 of the exact remote bytes, used for optimistic saves.
    pub revision: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    FilesAttach {
        version: u32,
        target: SshTarget,
    },
    FilesList,
    FilesRead {
        path: String,
    },
    FilesSave {
        path: String,
        text: String,
        revision: String,
    },
}
impl Request {
    pub fn name(&self) -> &'static str {
        match self {
            Self::FilesAttach { .. } => "files_attach",
            Self::FilesList => "files_list",
            Self::FilesRead { .. } => "files_read",
            Self::FilesSave { .. } => "files_save",
        }
    }
}

pub struct Client {
    // Serialise requests and response consumption. Never replay a failed/uncertain save.
    rpc: Mutex<RpcClient>,
}
impl Client {
    pub fn connect(target: &SshTarget) -> Result<Self> {
        Self::from_launch(target, ssh::install_files(target)?)
    }
    /// Alternate transport for deterministic helper tests; installs no Pi extension.
    pub fn from_launch(target: &SshTarget, launch: crate::transport::Launch) -> Result<Self> {
        ensure!(
            launch.extension.is_none(),
            "File transport cannot install a Pi extension"
        );
        let client = Self {
            rpc: Mutex::new(RpcClient::spawn(launch)?),
        };
        let hello = client.request(Request::FilesAttach {
            version: FILE_PROTOCOL_VERSION,
            target: target.clone(),
        })?;
        ensure!(
            hello["version"] == FILE_PROTOCOL_VERSION
                && hello["target"] == serde_json::to_value(target)?,
            "Remote file channel identity mismatch"
        );
        Ok(client)
    }
    pub fn request(&self, request: Request) -> Result<Value> {
        let rpc = self
            .rpc
            .lock()
            .map_err(|_| anyhow::anyhow!("Remote file channel failed"))?;
        let id = rpc.send_custom(request.name(), serde_json::to_value(&request)?)?;
        loop {
            match rpc
                .events()
                .recv_blocking()
                .context("Remote file channel closed")?
            {
                TransportEvent::Record(record)
                    if record["type"] == "response" && record["id"] == id =>
                {
                    ensure!(
                        record["success"] == true,
                        "{}",
                        record["error"]
                            .as_str()
                            .unwrap_or("Remote file operation failed")
                    );
                    return Ok(record["data"].clone());
                }
                TransportEvent::RequestFailed { error, .. }
                | TransportEvent::ProtocolError(error) => bail!("{error}"),
                TransportEvent::Exited {
                    description,
                    stderr,
                } => bail!("{description}: {stderr}"),
                _ => {}
            }
        }
    }
    pub fn list(&self) -> Result<Tree> {
        Ok(serde_json::from_value(self.request(Request::FilesList)?)?)
    }
    pub fn read(&self, path: String) -> Result<Document> {
        Ok(serde_json::from_value(
            self.request(Request::FilesRead { path })?,
        )?)
    }
    pub fn save(&self, path: String, text: String, revision: String) -> Result<Document> {
        Ok(serde_json::from_value(self.request(
            Request::FilesSave {
                path,
                text,
                revision,
            },
        )?)?)
    }
}

pub fn response(id: &str, name: &str, result: Result<Value>) -> Value {
    match result {
        Ok(data) => {
            json!({"type":"response", "id":id, "command":name, "success":true, "data":data})
        }
        Err(error) => {
            json!({"type":"response", "id":id, "command":name, "success":false, "error":format!("{error:#}")})
        }
    }
}
