//! Host-owned scheduler. The single-line API only edits a durable registry;
//! a separately and explicitly started process claims and executes occurrences.
mod cron;
mod service;
mod store;
#[cfg(test)]
mod tests;
mod worker;
use anyhow::{Result, ensure};
use pi_core::schedules::{Data, Request, Response, VERSION};
pub(crate) use service::command;
use std::{
    collections::HashSet,
    io::{BufRead, Write},
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |v| u64::try_from(v.as_millis()).unwrap_or(u64::MAX))
}
fn handle(root: &Path, request: Request, now: u64) -> Result<Data> {
    let mut store = store::Store::open(root)?;
    match request {
        Request::List => Ok(Data::List {
            jobs: store.list()?,
            health: service::health(root, store.last_tick()?)?,
        }),
        Request::Preview {
            cron,
            timezone,
            count,
        } => Ok(Data::Preview {
            times_ms: cron::Calendar::parse(&cron, &timezone)?.preview(now, count)?,
        }),
        Request::History { job_id } => Ok(Data::History {
            occurrences: store.history(&job_id)?,
        }),
        mutation => {
            ensure!(
                matches!(std::env::consts::OS, "linux" | "macos"),
                "Windows/this platform: durable scheduling explicitly unsupported"
            );
            store.mutate(mutation, now)
        }
    }
}
fn response(result: Result<Data>) -> Response {
    match result {
        Ok(data) => Response {
            version: VERSION,
            success: true,
            data: Some(data),
            error: None,
        },
        Err(error) => Response {
            version: VERSION,
            success: false,
            data: None,
            error: Some(format!("{error:#}")),
        },
    }
}
fn known_fields(value: &serde_json::Value, canonical: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => canonical.as_object().is_some_and(|expected| {
            map.iter().all(|(key, value)| {
                expected
                    .get(key)
                    .is_some_and(|field| known_fields(value, field))
            })
        }),
        serde_json::Value::Array(items) => canonical.as_array().is_some_and(|expected| {
            items.len() == expected.len()
                && items
                    .iter()
                    .zip(expected)
                    .all(|(value, field)| known_fields(value, field))
        }),
        _ => true,
    }
}
fn serve_at(root: &Path, input: impl BufRead, mut output: impl Write, now: u64) -> Result<()> {
    let result = (|| {
        let mut bytes = Vec::new();
        input.take(512 * 1024 + 1).read_until(b'\n', &mut bytes)?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= 512 * 1024 && bytes.last() == Some(&b'\n'),
            "Expected one JSON request line, at most 512 KiB"
        );
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let request: Request = serde_json::from_value(value.clone())?;
        // Reject misspelled/extra mutation keys, including correlation envelope
        // id. The shared contract uses job_id and no request envelope/version.
        let canonical = serde_json::to_value(&request)?;
        ensure!(
            known_fields(&value, &canonical),
            "Unexpected schedule request field"
        );
        handle(root, request, now)
    })();
    serde_json::to_writer(&mut output, &response(result))?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
pub fn serve() -> Result<()> {
    let root = match service::root() {
        Ok(root) => root,
        Err(error) => {
            println!("{}", serde_json::to_string(&response(Err(error)))?);
            return Ok(());
        }
    };
    serve_at(
        &root,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        now_ms(),
    )
}

/// Cron instants use wall time; scan cadence must not. The first scan is
/// immediate, including when wall time is before the Unix epoch.
#[derive(Default)]
struct ClaimCadence {
    last: Option<Instant>,
}
impl ClaimCadence {
    fn due(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(5))
        {
            self.last = Some(now);
            true
        } else {
            false
        }
    }
}

fn run(root: &Path) -> Result<()> {
    service::private_dir(root)?;
    let lock = service::lock_file(&root.join("scheduler.lock"))?;
    lock.try_lock().map_err(|e| {
        anyhow::anyhow!("One per-user scheduler is already running or cannot lock: {e}")
    })?;
    // Only the owner may clear a stale cooperative stop request.
    match std::fs::remove_file(root.join("stop.request")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut store = store::Store::open(root)?;
    let (done, finished) = std::sync::mpsc::channel();
    let mut in_flight = HashSet::new();
    let mut cadence = ClaimCadence::default();
    let mut scan_cursor: usize = 0;
    loop {
        if root.join("stop.request").exists() {
            return Ok(());
        }
        while let Ok(id) = finished.try_recv() {
            in_flight.remove(&id);
        }
        let now = now_ms();
        store.tick_stamp(now)?;
        if cadence.due(Instant::now()) {
            store.claim_due(now)?;
        }
        // Bound host work and don't let one unavailable session stall cron
        // claims/heartbeat for every other job. Writer ownership remains in the
        // helper, and each outstanding session gets lookup/reopen, never replay.
        let mut outstanding = store.outstanding()?;
        if !outstanding.is_empty() {
            let count = outstanding.len();
            outstanding.rotate_left(scan_cursor % count);
        }
        for mut ledger in outstanding {
            if in_flight.len() >= 8 {
                break;
            }
            scan_cursor = scan_cursor.wrapping_add(1);
            if !in_flight.insert(ledger.occurrence.id.clone()) {
                continue;
            }
            let root = root.to_owned();
            let done = done.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<()> {
                    let store = store::Store::open(&root)?;
                    store.expire_prepared(&mut ledger, now_ms())?;
                    if ledger.retirement_pending {
                        if worker::DurableWorker::retire_existing(&ledger)? {
                            ledger.retirement_pending = false;
                            store.save(&ledger)?;
                        }
                        return Ok(());
                    }
                    let mut worker = worker::DurableWorker::open(&ledger)?;
                    worker::progress_with_clock(&store, &mut ledger, &mut worker, now_ms)
                })();
                if let Err(error) = result {
                    if !ledger.retirement_pending {
                        ledger.occurrence.error = Some(
                            format!("Worker unavailable/unresolved: {error:#}")
                                .chars()
                                .take(4096)
                                .collect(),
                        );
                    } // cleanup failures cannot overwrite a proven outcome/reason
                    if let Ok(store) = store::Store::open(&root) {
                        let _ = store.save(&ledger);
                    }
                    eprintln!("Schedule occurrence {}: {error:#}", ledger.occurrence.id);
                }
                let _ = done.send(ledger.occurrence.id);
            });
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
