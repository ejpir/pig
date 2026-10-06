//! Explicit, one-shot jj history operations for the phone. Reading history is
//! harmless; restoring requires a separate command and Android confirmation.

use anyhow::{Context, Result, bail};
use pi_jj::{ObjectId as _, OperationId, OperationKind, Project, Turn, Vcs};
use serde::Serialize;
use std::path::PathBuf;

const VERSION: u32 = 1;
const LIMIT: usize = 40;

/// Records remote agent runs with the same built-in jj implementation as the
/// desktop app. Git-only projects stay untouched until the user explicitly
/// enables jj in Pi Desktop.
pub struct Recorder {
    path: PathBuf,
    project: Option<Project>,
    turn: Option<Turn>,
}

impl Recorder {
    pub fn open(path: &std::path::Path) -> Self {
        let project = match pi_jj::detect(path) {
            Vcs::Jj { root } => Project::open(&root)
                .inspect_err(|error| eprintln!("Could not open jj history: {error:#}"))
                .ok(),
            Vcs::Git { .. } | Vcs::None => None,
        };
        Self {
            path: path.to_owned(),
            project,
            turn: None,
        }
    }

    pub fn begin(&mut self, description: &str) {
        if self.turn.is_some() {
            return;
        }
        if self.project.is_none()
            && let Vcs::Jj { root } = pi_jj::detect(&self.path)
        {
            self.project = Project::open(&root)
                .inspect_err(|error| eprintln!("Could not open jj history: {error:#}"))
                .ok();
        }
        let Some(project) = &mut self.project else {
            return;
        };
        match project.begin_turn(description) {
            Ok(turn) => self.turn = Some(turn),
            Err(error) => eprintln!("Could not begin jj turn: {error:#}"),
        }
    }

    pub fn cancel(&mut self) {
        self.turn = None;
    }

    pub fn finish(&mut self) {
        let (Some(project), Some(turn)) = (&mut self.project, self.turn.take()) else {
            return;
        };
        if let Err(error) = project.end_turn(&turn) {
            eprintln!("Could not record jj turn: {error:#}");
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    version: u32,
    path: String,
    root: Option<String>,
    available: bool,
    reason: Option<String>,
    operations: Vec<Operation>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Operation {
    id: String,
    time: i64,
    description: String,
    kind: &'static str,
}

fn canonical_directory(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path)
        .canonicalize()
        .context("The project folder does not exist")?;
    anyhow::ensure!(path.is_dir(), "The project path is not a directory");
    Ok(path)
}

fn kind(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Pi => "pi",
        OperationKind::Snapshot => "snapshot",
        OperationKind::Git => "git",
        OperationKind::Other => "other",
    }
}

fn listing(path: &str) -> Result<Listing> {
    let path = canonical_directory(path)?;
    let shown = path.to_string_lossy().into_owned();
    let root = match pi_jj::detect(&path) {
        Vcs::Jj { root } => root,
        Vcs::Git { root } => {
            return Ok(Listing {
                version: VERSION,
                path: shown,
                root: Some(root.to_string_lossy().into_owned()),
                available: false,
                reason: Some("This project uses Git, but jj is not enabled.".into()),
                operations: Vec::new(),
            });
        }
        Vcs::None => {
            return Ok(Listing {
                version: VERSION,
                path: shown,
                root: None,
                available: false,
                reason: Some("No jj workspace was found for this project.".into()),
                operations: Vec::new(),
            });
        }
    };
    let mut project = Project::open(&root).context("Could not open this jj workspace")?;
    let operations = project
        .operations(LIMIT)?
        .into_iter()
        .map(|operation| Operation {
            id: operation.id.hex(),
            time: operation.time,
            description: operation.description,
            kind: kind(operation.kind),
        })
        .collect();
    Ok(Listing {
        version: VERSION,
        path: shown,
        root: Some(root.to_string_lossy().into_owned()),
        available: true,
        reason: None,
        operations,
    })
}

pub fn print(path: &str) -> Result<()> {
    println!("{}", serde_json::to_string(&listing(path)?)?);
    Ok(())
}

pub fn restore(path: &str, operation: &str) -> Result<()> {
    let path = canonical_directory(path)?;
    let Vcs::Jj { root } = pi_jj::detect(&path) else {
        bail!("This project does not have jj history");
    };
    let operation = OperationId::try_from_hex(operation).context("Invalid jj operation")?;
    let mut project = Project::open(&root).context("Could not open this jj workspace")?;
    let restored = project
        .restore_operation(&operation)
        .context("Could not restore this jj operation")?;
    println!(
        "{}",
        serde_json::json!({"version":VERSION,"restored":restored.hex()})
    );
    Ok(())
}

pub fn enable(path: &str) -> Result<()> {
    let path = canonical_directory(path)?;
    let root = match pi_jj::detect(&path) {
        Vcs::Jj { root } => root,
        Vcs::Git { root } => {
            Project::init(&root).context("Could not enable jj for this Git project")?;
            root
        }
        Vcs::None => bail!("This folder is not a Git project"),
    };
    print(root.to_string_lossy().as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_jj_projects_report_unavailable_without_mutating_them() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join(".git")).unwrap();
        let before = std::fs::read_dir(directory.path()).unwrap().count();
        let listing = listing(directory.path().to_str().unwrap()).unwrap();
        assert!(!listing.available);
        assert!(listing.reason.unwrap().contains("jj is not enabled"));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), before);
    }

    #[test]
    fn recorder_creates_a_pi_operation_for_a_remote_turn() {
        let directory = tempfile::tempdir().unwrap();
        Project::init(directory.path()).unwrap();
        let mut recorder = Recorder::open(directory.path());
        recorder.begin("make the phone screen compact");
        std::fs::write(directory.path().join("screen.txt"), "compact\n").unwrap();
        recorder.finish();
        let history = listing(directory.path().to_str().unwrap()).unwrap();
        assert!(history.available);
        assert_eq!(history.operations[0].kind, "pi");
        assert!(
            history.operations[0]
                .description
                .starts_with("pi: record turn")
        );
    }
}
