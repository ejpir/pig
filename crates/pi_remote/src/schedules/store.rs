//! Separate FULL-synchronous SQLite registry/ledger. All claims and mutations
//! use BEGIN IMMEDIATE; no transaction stays open over a worker/network call.
use super::cron::Calendar;
use anyhow::{Context, Result, ensure};
use pi_core::schedules::{Data, Job, MissedPolicy, Occurrence, Request, RunStatus, ScheduleSpec};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

pub const CATCHUP_MS: u64 = 24 * 60 * 60 * 1000;
const GRACE_MS: u64 = 60_000;
fn id() -> String {
    format!("{:032x}", rand::random::<u128>())
}
fn active(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Pending | RunStatus::Accepted | RunStatus::Running | RunStatus::Unknown
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ledger {
    pub occurrence: Occurrence,
    pub payload: Value,
    /// `prepared` has never dispatched; `dispatching` might have. Never resend
    /// a dispatching row, even when a subsequent lookup returns no receipt.
    pub phase: String,
    pub receipt: Option<Value>,
    pub stop_requested_ms: Option<u64>,
    /// Terminal outcomes survive cleanup failures. Retrying guarded retirement
    /// must never redispatch or change the already-correlated outcome.
    #[serde(default)]
    pub retirement_pending: bool,
}
pub struct Store {
    pub connection: Connection,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        super::service::private_dir(root)?;
        let path = root.join("schedules.sqlite3");
        // Never open/close this inode outside SQLite: POSIX fcntl locks are
        // process-scoped and closing ANY descriptor could release another
        // connection's locks. The private directory protects a newly created
        // empty database until chmod, before any prompts or WAL are written.
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Scheduler database must be a regular file, not a symlink"
            );
        }
        let mut connection = Connection::open_with_flags(
            &path,
            OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        connection.busy_timeout(Duration::from_secs(10))?;
        let layout: u32 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(layout <= 1, "Unsupported scheduler database version");
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let has_metadata: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='metadata')",
            [],
            |r| r.get(0),
        )?;
        if has_metadata {
            let version: Option<String> = tx
                .query_row("SELECT value FROM metadata WHERE key='schema'", [], |r| {
                    r.get(0)
                })
                .optional()?;
            ensure!(
                version.as_deref().is_none_or(|v| v == "1"),
                "Unsupported scheduler database version"
            );
        }
        tx.execute_batch("CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, data TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS occurrences (
                id TEXT PRIMARY KEY, job_id TEXT NOT NULL, scheduled_ms INTEGER NOT NULL,
                manual_key TEXT, active INTEGER NOT NULL, data TEXT NOT NULL);
            CREATE UNIQUE INDEX IF NOT EXISTS recurring_identity ON occurrences(job_id, scheduled_ms) WHERE manual_key IS NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS manual_identity ON occurrences(job_id, manual_key) WHERE manual_key IS NOT NULL;
            CREATE INDEX IF NOT EXISTS outstanding ON occurrences(active);")?;
        tx.execute_batch(
            "INSERT OR IGNORE INTO metadata VALUES ('schema','1'); PRAGMA user_version=1;",
        )?;
        tx.commit()?;
        Ok(Self { connection })
    }
    pub fn list(&self) -> Result<Vec<Job>> {
        let mut query = self
            .connection
            .prepare("SELECT data FROM jobs WHERE deleted=0 ORDER BY id")?;
        query
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub fn history(&self, job_id: &str) -> Result<Vec<Occurrence>> {
        let mut query = self.connection.prepare("SELECT data FROM occurrences WHERE job_id=? ORDER BY scheduled_ms DESC, id DESC LIMIT 500")?;
        query
            .query_map([job_id], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str::<Ledger>(&r?)?.occurrence))
            .collect()
    }
    pub fn outstanding(&self) -> Result<Vec<Ledger>> {
        let mut query = self
            .connection
            .prepare("SELECT data FROM occurrences WHERE active=1 OR json_extract(data,'$.retirement_pending')=1 ORDER BY scheduled_ms,id")?;
        query
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub fn save(&self, ledger: &Ledger) -> Result<()> {
        self.connection.execute(
            "UPDATE occurrences SET data=?,active=? WHERE id=?",
            params![
                serde_json::to_string(ledger)?,
                active(ledger.occurrence.status),
                ledger.occurrence.id
            ],
        )?;
        Ok(())
    }
    /// Recheck the frozen occurrence's policy before any attachment/dispatch.
    /// Manual consent is an explicit persisted identity, not a guessed timestamp.
    /// Intent/receipt rows are uncertain execution and must remain lookup-only.
    pub fn expire_prepared(&self, ledger: &mut Ledger, now: u64) -> Result<bool> {
        if ledger.phase != "prepared"
            || ledger.receipt.is_some()
            || ledger.occurrence.started_ms.is_some()
            || !active(ledger.occurrence.status)
        {
            return Ok(false);
        }
        let manual: bool = self.connection.query_row(
            "SELECT manual_key IS NOT NULL FROM occurrences WHERE id=?",
            [&ledger.occurrence.id],
            |r| r.get(0),
        )?;
        if manual {
            return Ok(false);
        }
        let age = now.saturating_sub(ledger.occurrence.scheduled_ms);
        let expired = match ledger.occurrence.spec.missed_policy {
            MissedPolicy::Skip => age >= GRACE_MS,
            MissedPolicy::Once => age > CATCHUP_MS,
        };
        if expired {
            ledger.phase = "expired".into();
            ledger.retirement_pending = true; // existing-only cleanup, never dispatch
            ledger.occurrence.status = RunStatus::Missed;
            ledger.occurrence.finished_ms = Some(now);
            ledger.occurrence.error = Some(
                "Never dispatched: recurring occurrence expired under its missed policy".into(),
            );
            self.save(ledger)?;
        }
        Ok(expired)
    }
    pub fn tick_stamp(&self, now: u64) -> Result<()> {
        self.connection.execute("INSERT INTO metadata VALUES ('last_tick',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [now.to_string()])?;
        Ok(())
    }
    pub fn last_tick(&self) -> Result<Option<u64>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='last_tick'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        value.map(|v| Ok(v.parse()?)).transpose()
    }
    pub fn mutate(&mut self, request: Request, now: u64) -> Result<Data> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let data = match request {
            Request::Create { mut spec, consent } => {
                validate(&mut spec)?;
                ensure!(
                    !spec.enabled || consent,
                    "Enabling requires explicit consent to trust this project and run with the host account's permissions"
                );
                let job = Job {
                    id: id(),
                    revision: 1,
                    next_due_ms: next(&spec, now)?,
                    spec,
                    created_ms: now,
                    updated_ms: now,
                };
                tx.execute(
                    "INSERT INTO jobs (id,data) VALUES (?,?)",
                    params![job.id, serde_json::to_string(&job)?],
                )?;
                Data::Job { job }
            }
            Request::Update {
                job_id,
                expected_revision,
                mut spec,
                consent,
            } => {
                let mut job = get_job(&tx, &job_id, expected_revision)?;
                let mut just_paused = job.spec.clone();
                just_paused.enabled = false;
                // Disabling an unchanged, already-validated schedule must work
                // even if its project has since been removed or renamed.
                if spec != just_paused {
                    validate(&mut spec)?;
                }
                ensure!(
                    consent || spec == job.spec || spec == just_paused,
                    "Enabling or changing configuration requires explicit consent"
                );
                job.next_due_ms = if spec == job.spec {
                    job.next_due_ms
                } else {
                    next(&spec, now)?
                };
                job.spec = spec;
                job.revision = job.revision.checked_add(1).context("Revision exhausted")?;
                job.updated_ms = now;
                put_job(&tx, &job)?;
                Data::Job { job }
            }
            Request::Delete {
                job_id,
                expected_revision,
            } => {
                get_job(&tx, &job_id, expected_revision)?;
                tx.execute("UPDATE jobs SET deleted=1 WHERE id=?", [job_id])?;
                Data::Deleted
            }
            Request::RunNow {
                job_id,
                expected_revision,
                request_id,
                consent,
            } => {
                ensure!(consent, "Run now requires explicit consent");
                ensure!(
                    !request_id.is_empty()
                        && request_id.len() <= 128
                        && request_id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                    "Invalid manual request_id"
                );
                // A confirmed operation remains recoverable after an edit/delete,
                // but cannot reuse the operation key with a different revision.
                let existing: Option<String> = tx
                    .query_row(
                        "SELECT data FROM occurrences WHERE job_id=? AND manual_key=?",
                        params![job_id, request_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let occurrence = if let Some(existing) = existing {
                    let row: Ledger = serde_json::from_str(&existing)?;
                    ensure!(
                        row.occurrence.job_revision == expected_revision,
                        "Manual request key belongs to another revision"
                    );
                    row.occurrence
                } else {
                    let job = get_job(&tx, &job_id, expected_revision)?;
                    insert(
                        &tx,
                        &job,
                        now,
                        now,
                        Some(&request_id),
                        RunStatus::Pending,
                        None,
                    )?
                    .occurrence
                };
                Data::Run { occurrence }
            }
            _ => anyhow::bail!("Not a mutation"),
        };
        tx.commit()?;
        Ok(data)
    }
    /// At most two rows per overdue job, not a replay queue. Times outside the
    /// 24-hour catchup window are summarized by a Missed row. Within the window
    /// Once selects ONE actual cron instant. Missed summaries name the interval.
    pub fn claim_due(&mut self, now: u64) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let jobs: Vec<String> = {
            let mut query = tx.prepare("SELECT data FROM jobs WHERE deleted=0")?;
            query
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for data in jobs {
            let mut job: Job = serde_json::from_str(&data)?;
            let Some(due) = job
                .next_due_ms
                .filter(|due| *due <= now && job.spec.enabled)
            else {
                continue;
            };
            let calendar = Calendar::parse(&job.spec.cron, &job.spec.timezone)?;
            if now.saturating_sub(due) < GRACE_MS {
                insert(&tx, &job, due, now, None, RunStatus::Pending, None)?;
            } else if job.spec.missed_policy == MissedPolicy::Once {
                let candidate = if now.saturating_sub(due) <= CATCHUP_MS {
                    due
                } else {
                    calendar.next(now.saturating_sub(CATCHUP_MS))?
                };
                if candidate != due {
                    insert(
                        &tx,
                        &job,
                        due,
                        now,
                        None,
                        RunStatus::Missed,
                        Some(format!(
                            "Missed instants before the 24-hour catchup window; backlog coalesced through {now}"
                        )),
                    )?;
                }
                if candidate <= now {
                    insert(
                        &tx,
                        &job,
                        candidate,
                        now,
                        None,
                        RunStatus::Pending,
                        Some(
                            "One coalesced catchup; remaining missed instants not replayed".into(),
                        ),
                    )?;
                }
            } else {
                insert(
                    &tx,
                    &job,
                    due,
                    now,
                    None,
                    RunStatus::Missed,
                    Some(format!(
                        "Missed instants coalesced from {due} through {now}; missed policy skip"
                    )),
                )?;
            }
            job.next_due_ms = Some(calendar.next(now)?);
            put_job(&tx, &job)?;
        }
        tx.commit()?;
        Ok(())
    }
}
fn next(spec: &ScheduleSpec, now: u64) -> Result<Option<u64>> {
    if spec.enabled {
        Ok(Some(
            Calendar::parse(&spec.cron, &spec.timezone)?.next(now)?,
        ))
    } else {
        Ok(None)
    }
}
fn validate(spec: &mut ScheduleSpec) -> Result<()> {
    ensure!(
        !spec.name.trim().is_empty()
            && spec.name.len() <= 200
            && !spec.name.chars().any(char::is_control),
        "Choose a schedule name (up to 200 bytes)"
    );
    ensure!(
        !spec.prompt.trim().is_empty() && spec.prompt.len() <= 256 * 1024,
        "Instruction must be nonempty and at most 256 KiB"
    );
    ensure!(
        !spec.provider.is_empty()
            && spec.provider.len() <= 128
            && !spec.model_id.is_empty()
            && spec.model_id.len() <= 256,
        "Choose a provider and model"
    );
    ensure!(
        (1..=10080).contains(&spec.timeout_minutes),
        "Timeout must be 1 through 10080 minutes; Stop is best-effort, not a hard boundary"
    );
    Calendar::parse(&spec.cron, &spec.timezone)?.next(0)?;
    ensure!(
        Path::new(&spec.cwd).is_absolute()
            && !spec.cwd.contains(['\0', '\n', '\r'])
            && spec.cwd.len() <= 4096,
        "Project must be an absolute host path"
    );
    let path = Path::new(&spec.cwd)
        .canonicalize()
        .context("Project directory must exist on this host")?;
    ensure!(path.is_dir(), "Project must be a directory");
    spec.cwd = path
        .to_str()
        .context("Project path must be UTF-8")?
        .to_owned();
    Ok(())
}
fn get_job(tx: &Transaction<'_>, id: &str, revision: u64) -> Result<Job> {
    let data: String = tx
        .query_row(
            "SELECT data FROM jobs WHERE id=? AND deleted=0",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .context("Schedule not found")?;
    let job: Job = serde_json::from_str(&data)?;
    ensure!(
        job.revision == revision,
        "Schedule revision conflict; refresh before changing it"
    );
    Ok(job)
}
fn put_job(tx: &Transaction<'_>, job: &Job) -> Result<()> {
    tx.execute(
        "UPDATE jobs SET data=? WHERE id=?",
        params![serde_json::to_string(job)?, job.id],
    )?;
    Ok(())
}
fn insert(
    tx: &Transaction<'_>,
    job: &Job,
    scheduled_ms: u64,
    now: u64,
    manual: Option<&str>,
    mut status: RunStatus,
    mut error: Option<String>,
) -> Result<Ledger> {
    // Recurring identities do not include revision. Manual identities are a
    // separate namespace and don't advance or collide with the recurring clock.
    if manual.is_none() {
        let existing: Option<String> = tx.query_row("SELECT data FROM occurrences WHERE job_id=? AND scheduled_ms=? AND manual_key IS NULL", params![job.id, scheduled_ms], |r| r.get(0)).optional()?;
        if let Some(data) = existing {
            return Ok(serde_json::from_str(&data)?);
        }
    }
    let overlap: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM occurrences WHERE job_id=? AND active=1)",
        [&job.id],
        |r| r.get(0),
    )?;
    if active(status) && overlap {
        status = RunStatus::Skipped;
        error =
            Some("Previous occurrence is active or unresolved; same-job overlap skipped".into());
    }
    let occurrence_id = id();
    let request_id = format!("schedule_{occurrence_id}");
    let occurrence = Occurrence {
        id: occurrence_id,
        job_id: job.id.clone(),
        job_revision: job.revision,
        scheduled_ms,
        session_key: active(status).then(id),
        request_id: request_id.clone(),
        spec: job.spec.clone(),
        status,
        error,
        started_ms: None,
        finished_ms: (!active(status)).then_some(now),
    };
    let ledger = Ledger {
        payload: json!({"type":"prompt", "message":job.spec.prompt,"requestId":request_id}),
        occurrence,
        phase: "prepared".into(),
        receipt: None,
        stop_requested_ms: None,
        retirement_pending: false,
    };
    tx.execute(
        "INSERT INTO occurrences VALUES (?,?,?,?,?,?)",
        params![
            ledger.occurrence.id,
            job.id,
            scheduled_ms,
            manual,
            active(status),
            serde_json::to_string(&ledger)?
        ],
    )?;
    Ok(ledger)
}
