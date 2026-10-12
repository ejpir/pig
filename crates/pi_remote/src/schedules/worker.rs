//! Admission is not completion. Only committed helper projections and receipt
//! lookups can settle a row. An uncertain dispatch is never submitted again.
use super::store::{Ledger, Store};
use anyhow::{Context, Result, ensure};
use pi_core::{
    schedules::RunStatus,
    session::Session,
    ssh::{RemoteBackend, SshTarget},
};
use serde_json::{Value, json};

#[derive(Default)]
pub struct Observation {
    pub submission: Option<Value>,
    pub busy: bool,
    pub has_messages: bool,
    pub settled: Option<(RunStatus, Option<String>)>,
}
pub trait Worker {
    fn inspect(&mut self, ledger: &Ledger) -> Result<Observation>;
    fn configure(&mut self, ledger: &Ledger) -> Result<()>;
    fn submit(&mut self, ledger: &Ledger) -> Result<Value>;
    fn stop(&mut self, ledger: &Ledger) -> Result<()>;
    /// Normal helper ownership teardown, guarded atomically against live work
    /// and other attached clients. False keeps cleanup pending, not execution.
    fn retire(&mut self, ledger: &Ledger) -> Result<bool>;
}
pub struct DurableWorker {
    attachment: crate::server::SchedulerAttachment,
}
impl DurableWorker {
    fn target(ledger: &Ledger) -> Result<SshTarget> {
        let occurrence = &ledger.occurrence;
        Ok(SshTarget {
            host: "localhost".into(),
            cwd: occurrence.spec.cwd.clone(),
            key: occurrence
                .session_key
                .clone()
                .context("Missing saved session key")?,
            backend: RemoteBackend::Durable,
            session_file: None,
        })
    }
    pub fn open(ledger: &Ledger) -> Result<Self> {
        Ok(Self {
            attachment: crate::server::SchedulerAttachment::open(&Self::target(ledger)?)?,
        })
    }
    pub fn retire_existing(ledger: &Ledger) -> Result<bool> {
        let Some(attachment) =
            crate::server::SchedulerAttachment::open_existing(&Self::target(ledger)?)?
        else {
            return Ok(true);
        };
        Self { attachment }.retire(ledger)
    }
}
impl Worker for DurableWorker {
    fn inspect(&mut self, ledger: &Ledger) -> Result<Observation> {
        let (lookup, _) = self
            .attachment
            .call(json!({"type":"get_submission","requestId":ledger.occurrence.request_id}))?;
        let (_, snapshot) = self.attachment.call(json!({"type":"get_messages"}))?;
        let model: Session =
            serde_json::from_value(snapshot.context("Missing committed helper snapshot")?)?;
        Ok(observe(
            lookup.get("submission").filter(|s| !s.is_null()).cloned(),
            lookup.get("correlated").filter(|s| !s.is_null()),
            &model,
        ))
    }
    fn configure(&mut self, ledger: &Ledger) -> Result<()> {
        self.attachment.call(json!({"type":"set_model","provider":ledger.occurrence.spec.provider,"modelId":ledger.occurrence.spec.model_id,"persist":false}))?;
        Ok(())
    }
    fn submit(&mut self, ledger: &Ledger) -> Result<Value> {
        Ok(self.attachment.call(ledger.payload.clone())?.0)
    }
    fn stop(&mut self, ledger: &Ledger) -> Result<()> {
        self.attachment.call(cancel_request(ledger)?)?;
        Ok(())
    }
    fn retire(&mut self, _: &Ledger) -> Result<bool> {
        let (reply, _) = self.attachment.call(json!({"type":"remote_retire"}))?;
        // Older helpers reject this custom ownership operation rather than
        // interpreting it as unguarded session-global shutdown.
        Ok(reply["retired"] == true)
    }
}
pub(super) fn cancel_request(ledger: &Ledger) -> Result<Value> {
    let receipt = ledger
        .receipt
        .as_ref()
        .context("No exact submission to cancel")?;
    let id = submission_id(receipt).context("No stable submission ID to cancel")?;
    Ok(
        json!({"type":"cancel_submission","submissionId":id,"requestId":ledger.occurrence.request_id}),
    )
}
fn submission_id(receipt: &Value) -> Option<&str> {
    receipt["id"]
        .as_str()
        .or_else(|| receipt["submissionId"].as_str())
}
/// Walk only A's receipt-bounded answer and its exact background report
/// continuations. Shared spans (a report steering A's live turn) are allowed;
/// reused tool IDs, missing spans/receipts, and backwards/cyclic links are not.
fn settled_descendants(scoped: &Value, model: &Session) -> Option<(bool, Option<String>)> {
    let mut pending = vec![(scoped["entry"].as_u64()?, scoped["messages"].as_array()?)];
    let mut reports_seen = std::collections::HashSet::new();
    let mut call_counts = std::collections::HashMap::new();
    for id in model
        .messages
        .iter()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "toolCall")
        .filter_map(|b| b["id"].as_str())
    {
        *call_counts.entry(id).or_insert(0usize) += 1;
    }
    let mut interrupted = false;
    let mut report_failure = None;
    while let Some((start, messages)) = pending.pop() {
        for block in messages
            .iter()
            .flat_map(|m| m["content"].as_array().into_iter().flatten())
            .filter(|b| b["type"] == "toolCall")
        {
            let id = block["id"].as_str()?;
            // The flattened tool projection is keyed by ID, so a collision
            // anywhere in the session makes this particular call ambiguous.
            if call_counts.get(id) != Some(&1) {
                return None;
            }
            let tool = model.tools.iter().find(|tool| tool.id == id)?;
            if !tool.finished {
                return None;
            }
            interrupted |= tool.is_error && tool.output.to_ascii_lowercase().contains("interrupt");
            if tool.name != "subagent" {
                continue;
            }
            let handoff = crate::server::scheduler_handoff(tool)?;
            if handoff
                .subagents
                .iter()
                .any(|agent| !agent.status.finished())
            {
                return None;
            }
            interrupted |= handoff.subagents.iter().any(|agent| {
                !agent.interrupted.is_empty() || agent.status == pi_core::subagent::Status::Stopped
            });
            if !handoff.background {
                continue;
            }
            let report = scoped["reports"].get(id)?;
            let receipt = &report["receipt"];
            let entry = receipt["entry"].as_u64()?;
            let end = receipt["answer"].as_u64()?;
            if receipt["type"] != "input"
                || receipt["status"] != "done"
                || !receipt["requestId"]
                    .as_str()?
                    .starts_with("subagent-report-")
                || entry <= start
                || end < entry
            {
                return None;
            }
            let continuation = report["messages"].as_array()?;
            let input = continuation.first().filter(|m| m["role"] == "user")?;
            if !input["content"]
                .as_str()?
                .starts_with(&format!("<subagent_report call=\"{id}\">\n"))
            {
                return None;
            }
            let answer = continuation.last().filter(|m| m["role"] == "assistant")?;
            if answer != &report["answer"] {
                return None;
            }
            match answer["stopReason"].as_str()? {
                "aborted" => interrupted = true,
                "error" => {
                    report_failure = Some(
                        answer["errorMessage"]
                            .as_str()
                            .unwrap_or("Background report failed")
                            .to_owned(),
                    )
                }
                "stop" | "length" => {}
                _ => return None,
            }
            if reports_seen.insert(id) {
                pending.push((entry, continuation));
            }
        }
    }
    Some((interrupted, report_failure))
}
pub(super) fn observe(
    submission: Option<Value>,
    correlated: Option<&Value>,
    model: &Session,
) -> Observation {
    let busy = !crate::server::scheduler_idle(model);
    let settled = submission.as_ref().and_then(|receipt| {
        // An unanswered input/report has no authoritative end boundary. Its
        // reason alone cannot prove which tools or descendants ran before it
        // ended; absence from a session-wide tool list is not that proof.
        if receipt["type"] != "input" || receipt["status"] != "done" {
            return None;
        }
        let scoped = correlated?;
        if scoped["entry"] != receipt["entry"] || scoped["answer"] != receipt["answer"] {
            return None;
        }
        let messages = scoped["messages"].as_array()?;
        let answer = messages.last().filter(|m| m["role"] == "assistant")?;
        let (interrupted, report_failure) = settled_descendants(scoped, model)?;
        if let Some(error) = report_failure {
            return Some((RunStatus::Failed, Some(error)));
        }
        match answer["stopReason"].as_str()? {
            "aborted" => Some((RunStatus::Interrupted, Some("Agent stopped/interrupted".into()))),
            "error" => Some((RunStatus::Failed, answer["errorMessage"].as_str().map(str::to_owned))),
            "stop" | "length" => Some(if interrupted {
                (RunStatus::Interrupted, Some("An interrupted tool was not blindly repeated; inspect the durable session".into()))
            } else {
                (RunStatus::Completed, None)
            }),
            _ => None,
        }
    });
    Observation {
        submission,
        busy,
        has_messages: !model.messages.is_empty(),
        settled,
    }
}
fn cleanup(store: &Store, ledger: &mut Ledger, worker: &mut impl Worker) -> Result<()> {
    if worker.retire(ledger)? {
        ledger.retirement_pending = false;
        store.save(ledger)?;
    }
    Ok(())
}
fn diagnostic(ledger: &mut Ledger, message: &str) {
    ledger.occurrence.error = Some(message.chars().take(4096).collect());
}
/// Called with one scheduler owner and at most one in-flight task for this row.
/// Crash points are persisted BEFORE dispatch and Stop. Receipt absence after
/// uncertain dispatch is not authorization to retry, even with the same key.
#[cfg(test)]
pub fn progress(
    store: &Store,
    ledger: &mut Ledger,
    worker: &mut impl Worker,
    now: u64,
) -> Result<()> {
    progress_with_clock(store, ledger, worker, || now)
}
pub fn progress_with_clock(
    store: &Store,
    ledger: &mut Ledger,
    worker: &mut impl Worker,
    mut clock: impl FnMut() -> u64,
) -> Result<()> {
    if ledger.retirement_pending {
        if worker.retire(ledger)? {
            ledger.retirement_pending = false;
            store.save(ledger)?;
        }
        return Ok(());
    }
    if store.expire_prepared(ledger, clock())? {
        return cleanup(store, ledger, worker);
    }
    let observation = worker.inspect(ledger)?;
    let now = clock();
    if let Some(submission) = observation.submission {
        ensure!(
            submission["requestId"] == ledger.occurrence.request_id
                && submission_id(&submission).is_some(),
            "Submission request identity mismatch"
        );
        if let Some(previous) = &ledger.receipt {
            ensure!(
                submission_id(previous) == submission_id(&submission),
                "Submission ID changed"
            );
        }
        ledger.receipt = Some(submission.clone());
        if ledger.occurrence.started_ms.is_none() {
            ledger.occurrence.started_ms = Some(now);
        }
        // Session-wide busy says nothing about whether THIS input is running.
        ledger.occurrence.status = match submission["status"].as_str() {
            Some("placed") => RunStatus::Running,
            Some("queued") => RunStatus::Accepted,
            _ => RunStatus::Unknown,
        };
        if let Some((status, error)) = observation.settled {
            ledger.occurrence.status = status;
            ledger.occurrence.finished_ms = Some(now);
            ledger.occurrence.error = error;
            ledger.retirement_pending = true;
            store.save(ledger)?; // cleanup cannot lose or change a proven outcome
            if worker.retire(ledger)? {
                ledger.retirement_pending = false;
                store.save(ledger)?;
            }
            return Ok(());
        }
        if matches!(submission["status"].as_str(), Some("queued" | "placed"))
            && ledger.occurrence.started_ms.is_some_and(|start| {
                now.saturating_sub(start)
                    >= u64::from(ledger.occurrence.spec.timeout_minutes) * 60_000
            })
            && ledger.stop_requested_ms.is_none()
        {
            ledger.stop_requested_ms = Some(now);
            diagnostic(
                ledger,
                "Timeout: exact submission cancellation requested best-effort; not a hard execution/cost boundary. Background work remains unresolved until committed state settles.",
            );
            store.save(ledger)?;
            worker.stop(ledger)?;
        } else if ledger.occurrence.status == RunStatus::Unknown {
            diagnostic(
                ledger,
                "Receipt found but correlated terminal state/background work remains unresolved; no later session work is attributed to this occurrence",
            );
        }
        store.save(ledger)?;
    } else if ledger.phase == "prepared"
        && ledger.receipt.is_none()
        && ledger.occurrence.started_ms.is_none()
    {
        if store.expire_prepared(ledger, now)? {
            return cleanup(store, ledger, worker);
        }
        if observation.busy || observation.has_messages {
            ledger.phase = "blocked".into();
            ledger.occurrence.status = RunStatus::Unknown;
            diagnostic(
                ledger,
                "Saved session contains unrelated work; refusing to change model or displace its controller",
            );
            store.save(ledger)?;
            return Ok(());
        }
        worker.configure(ledger)?;
        let now = clock();
        // Model configuration/attachment can themselves cross the grace/window.
        if store.expire_prepared(ledger, now)? {
            return cleanup(store, ledger, worker);
        }
        ledger.phase = "dispatching".into();
        ledger.occurrence.started_ms = Some(now);
        store.save(ledger)?;
        match worker.submit(ledger) {
            Ok(receipt) => {
                ensure!(
                    receipt["requestId"] == ledger.occurrence.request_id
                        && receipt["submissionId"].is_string(),
                    "Invalid durable admission receipt"
                );
                ledger.receipt = Some(receipt);
                ledger.occurrence.status = RunStatus::Accepted;
                ledger.occurrence.error = None;
            }
            Err(error) => {
                ledger.occurrence.status = RunStatus::Unknown;
                diagnostic(
                    ledger,
                    &format!(
                        "Uncertain admission: {error:#}; lookup only, never blind resubmission"
                    ),
                );
            }
        }
        store.save(ledger)?;
    } else {
        ledger.occurrence.status = RunStatus::Unknown;
        diagnostic(
            ledger,
            "Admission unresolved: no committed receipt found after dispatch intent. Original request will not be resubmitted; no exact task identity exists to cancel safely.",
        );
        store.save(ledger)?;
        // Even a busy dedicated session is not proof of task identity. Without
        // a receipt, NEVER substitute session-global abort for exact cancellation.
    }
    Ok(())
}
