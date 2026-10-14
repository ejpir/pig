pub mod app_fixture;
pub mod fixture;
mod http;
mod workspace;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use http::HttpHost;
use rquickjs::{Context, Ctx, Exception, Function, Runtime};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use workspace::Workspace;

pub struct Options {
    pub workspace: PathBuf,
    pub base_url: String,
    pub model: String,
    pub prompt: String,
    pub api_key: String,
    /// Only the offline fixture should enable plaintext loopback HTTP.
    pub allow_loopback_http: bool,
    /// 4096 normally; fixtures use 1 to deterministically split UTF-8/SSE boundaries.
    pub http_chunk_bytes: usize,
    /// Upstream SDK timeout (distinct from the whole-run deadline).
    pub request_timeout_ms: u32,
    pub timeout: Duration,
}

/// A cloneable stop request, usable while another thread owns `Run::join`.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub struct Run {
    cancel: Cancellation,
    worker: Option<thread::JoinHandle<Result<Value>>>,
}
impl Run {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn join(mut self) -> Result<Value> {
        self.worker
            .take()
            .expect("worker owned until join")
            .join()
            .map_err(|_| anyhow!("QuickJS worker panicked"))?
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        // Never silently detach a paid, file-writing agent when a screen abandons it.
        if self.worker.is_some() {
            self.cancel();
        }
    }
}

/// Owns a fresh QuickJS VM on its own worker, never on an Android/GPUI UI thread.
/// `emit` runs on that worker; a UI should forward events to its own event queue.
pub fn spawn(options: Options, emit: impl FnMut(Value) + Send + 'static) -> Run {
    spawn_with_cancel(options, Cancellation::default(), emit)
}

pub fn spawn_with_cancel(
    options: Options,
    cancel: Cancellation,
    emit: impl FnMut(Value) + Send + 'static,
) -> Run {
    let worker_cancel = cancel.0.clone();
    let worker = thread::spawn(move || run(options, worker_cancel, emit));
    Run {
        cancel,
        worker: Some(worker),
    }
}

fn js<T>(ctx: &Ctx<'_>, result: rquickjs::Result<T>) -> Result<T> {
    result.map_err(|error| {
        if error.is_exception() {
            let caught = ctx.catch();
            let object = caught.as_object();
            let message = object
                .and_then(|o| o.get::<_, String>("message").ok())
                .unwrap_or_else(|| format!("{caught:?}"));
            let stack = object
                .and_then(|o| o.get::<_, String>("stack").ok())
                .unwrap_or_default();
            anyhow!("QuickJS: {message}\n{stack}")
        } else {
            anyhow!("QuickJS: {error}")
        }
    })
}

struct HttpJobs(Rc<RefCell<HashMap<u32, http::HttpJob>>>);
impl Drop for HttpJobs {
    fn drop(&mut self) {
        for job in self.0.borrow().values() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.0.borrow_mut().clear();
    }
}

fn run(
    options: Options,
    cancel: Arc<AtomicBool>,
    emit: impl FnMut(Value) + 'static,
) -> Result<Value> {
    ensure!(options.prompt.len() <= 256 * 1024, "prompt exceeds 256 KiB");
    let workspace = Rc::new(Workspace::open(&options.workspace)?);
    let host = HttpHost::new(
        &options.base_url,
        options.api_key,
        options.allow_loopback_http,
        options.http_chunk_bytes,
    )?;
    let started = Instant::now();
    let deadline = started + options.timeout;
    let rt = Runtime::new()?;
    rt.set_memory_limit(96 * 1024 * 1024);
    rt.set_max_stack_size(1024 * 1024);
    let interrupt_cancel = cancel.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        Instant::now() >= deadline || interrupt_cancel.load(Ordering::Relaxed)
    })));
    let ctx = Context::full(&rt)?;
    let timers = Rc::new(RefCell::new(HashMap::<u32, Instant>::new()));
    let jobs = HttpJobs(Rc::new(RefCell::new(HashMap::new())));
    let finished = Rc::new(RefCell::new(None::<Value>));
    let (tx, rx) = mpsc::sync_channel::<Value>(16);
    ctx.with(|ctx| -> Result<()> {
        let globals = ctx.globals();
        globals.set("__now", Function::new(ctx.clone(), move || started.elapsed().as_secs_f64() * 1000.0)?)?;
        globals.set("__random", Function::new(ctx.clone(), |ctx: Ctx<'_>, len: usize| -> rquickjs::Result<Vec<u8>> {
            if len > 65536 { return Err(Exception::throw_message(&ctx, "random byte limit")); }
            let mut bytes = vec![0; len];
            getrandom::fill(&mut bytes).map_err(|_| Exception::throw_message(&ctx, "OS randomness unavailable"))?;
            Ok(bytes)
        })?)?;
        let read_ws = workspace.clone();
        globals.set("__read", Function::new(ctx.clone(), move |ctx: Ctx<'_>, path: String| -> rquickjs::Result<String> {
            read_ws.read(&path).map_err(|e| Exception::throw_message(&ctx, &e.to_string()))
        })?)?;
        globals.set("__write", Function::new(ctx.clone(), move |ctx: Ctx<'_>, path: String, text: String| -> rquickjs::Result<()> {
            workspace.write(&path, &text).map_err(|e| Exception::throw_message(&ctx, &e.to_string()))
        })?)?;
        let start_timers = timers.clone();
        globals.set("__timer", Function::new(ctx.clone(), move |ctx: Ctx<'_>, id: u32, ms: f64| -> rquickjs::Result<()> {
            if start_timers.borrow().len() >= 128 { return Err(Exception::throw_message(&ctx, "timer limit")); }
            start_timers.borrow_mut().insert(id, Instant::now() + Duration::from_millis(ms.clamp(0.0, 2147483647.0) as u64));
            Ok(())
        })?)?;
        let clear_timers = timers.clone();
        globals.set("__clearTimer", Function::new(ctx.clone(), move |id: u32| { clear_timers.borrow_mut().remove(&id); })?)?;
        let http_jobs = jobs.0.clone();
        let count = std::cell::Cell::new(0);
        globals.set("__http", Function::new(ctx.clone(), move |ctx: Ctx<'_>, id: u32, input: String| -> rquickjs::Result<()> {
            if input.len() > 2 * 1024 * 1024 || count.get() >= 8 || http_jobs.borrow().len() >= 2 {
                return Err(Exception::throw_message(&ctx, "HTTP request limit"));
            }
            let request = serde_json::from_str(&input).map_err(|_| Exception::throw_message(&ctx, "invalid HTTP request"))?;
            count.set(count.get() + 1);
            let job = http::spawn_fetch(host.clone(), id, request, tx.clone());
            http_jobs.borrow_mut().insert(id, job);
            Ok(())
        })?)?;
        let cancel_jobs = jobs.0.clone();
        globals.set("__cancelHttp", Function::new(ctx.clone(), move |id: u32| {
            if let Some(job) = cancel_jobs.borrow().get(&id) { job.cancel.store(true, Ordering::Relaxed); }
        })?)?;
        let emit = RefCell::new(emit);
        globals.set("__emit", Function::new(ctx.clone(), move |ctx: Ctx<'_>, event: String| -> rquickjs::Result<()> {
            let value = serde_json::from_str(&event).map_err(|_| Exception::throw_message(&ctx, "invalid event JSON"))?;
            (emit.borrow_mut())(value);
            Ok(())
        })?)?;
        let result = finished.clone();
        globals.set("__finish", Function::new(ctx.clone(), move |ctx: Ctx<'_>, data: String| -> rquickjs::Result<()> {
            *result.borrow_mut() = Some(serde_json::from_str(&data).map_err(|_| Exception::throw_message(&ctx, "invalid result JSON"))?);
            Ok(())
        })?)?;
        for source in [include_str!("../generated/encoding.js"), include_str!("../generated/platform.js"), include_str!("../generated/agent.js")] {
            js(&ctx, ctx.eval::<(), _>(source))?;
        }
        let input = json!({ "baseUrl": options.base_url, "model": options.model, "prompt": options.prompt,
            "requestTimeoutMs": options.request_timeout_ms });
        let start: Function = globals.get("startAgent")?;
        js(&ctx, start.call::<_, ()>((input.to_string(),)))?;
        Ok(())
    })?;
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "cancelled");
        ensure!(Instant::now() < deadline, "agent deadline exceeded");
        // Bound each pump so timers, I/O and cancellation aren't starved by microtasks.
        for _ in 0..128 {
            match rt.execute_pending_job() {
                Ok(true) => (),
                Ok(false) => break,
                Err(error) => {
                    return error
                        .0
                        .with(|ctx| js::<Value>(&ctx, Err(rquickjs::Error::Exception)));
                }
            }
        }
        if let Some(result) = finished.borrow_mut().take() {
            if let Some(error) = result.get("error") {
                bail!("agent failed: {error}\n{}", result["stack"]);
            }
            // Provider failures are returned as assistant messages by upstream, not rejected promises.
            let failed = result["messages"]
                .as_array()
                .and_then(|messages| messages.iter().rev().find(|m| m["role"] == "assistant"));
            if let Some(message) = failed {
                ensure!(
                    message["stopReason"] != "error" && message["stopReason"] != "aborted",
                    "provider failed: {}",
                    message["errorMessage"]
                );
            }
            return Ok(result);
        }
        let due: Vec<_> = timers
            .borrow()
            .iter()
            .filter(|(_, when)| **when <= Instant::now())
            .map(|(&id, _)| id)
            .collect();
        for id in due {
            timers.borrow_mut().remove(&id);
            dispatch(&ctx, json!({"id": id, "kind": "timer"}))?;
        }
        let wait = if rt.is_job_pending() {
            Duration::ZERO
        } else {
            Duration::from_millis(5)
        };
        match rx.recv_timeout(wait) {
            Ok(event) => {
                if event["kind"] == "released" {
                    jobs.0
                        .borrow_mut()
                        .remove(&(event["id"].as_u64().context("bad request ID")? as u32));
                }
                dispatch(&ctx, event)?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("host bridge disconnected"),
        }
    }
}

fn dispatch(ctx: &Context, event: Value) -> Result<()> {
    ctx.with(|ctx| {
        let function: Function = ctx.globals().get("hostDispatch")?;
        js(&ctx, function.call::<_, ()>((event.to_string(),)))
    })
}
