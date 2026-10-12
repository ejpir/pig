//! Host-owned scheduling protocol, independent of desktop/phone lifecycle.
//! One request and one response per `schedules --stdio` invocation (version 1).
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedPolicy {
    #[default]
    Skip,
    Once,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleSpec {
    pub name: String,
    pub cwd: String,
    pub prompt: String,
    pub provider: String,
    pub model_id: String,
    pub cron: String,
    pub timezone: String,
    pub enabled: bool,
    #[serde(default)]
    pub missed_policy: MissedPolicy,
    pub timeout_minutes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub revision: u64,
    pub spec: ScheduleSpec,
    pub next_due_ms: Option<u64>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Accepted,
    Running,
    Completed,
    Failed,
    Interrupted,
    Skipped,
    Missed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
    pub id: String,
    pub job_id: String,
    pub job_revision: u64,
    pub scheduled_ms: u64,
    pub session_key: Option<String>,
    pub request_id: String,
    pub spec: ScheduleSpec,
    pub status: RunStatus,
    pub error: Option<String>,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub running: bool,
    pub autostart_installed: bool,
    pub last_tick_ms: Option<u64>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    List,
    Preview {
        cron: String,
        timezone: String,
        count: u8,
    },
    Create {
        spec: ScheduleSpec,
        consent: bool,
    },
    Update {
        job_id: String,
        expected_revision: u64,
        spec: ScheduleSpec,
        consent: bool,
    },
    Delete {
        job_id: String,
        expected_revision: u64,
    },
    RunNow {
        job_id: String,
        expected_revision: u64,
        request_id: String,
        consent: bool,
    },
    History {
        job_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Data {
    List { jobs: Vec<Job>, health: Health },
    Preview { times_ms: Vec<u64> },
    Job { job: Job },
    Deleted,
    Run { occurrence: Occurrence },
    History { occurrences: Vec<Occurrence> },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub version: u32,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Data>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
