use crate::http::{self, HttpHost};
use anyhow::{Context as _, Result, anyhow, bail, ensure};
use base64::{
    Engine,
    alphabet::STANDARD,
    engine::{GeneralPurpose, GeneralPurposeConfig},
};
use rquickjs::{Context, Ctx, Exception, Function, Runtime};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
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

pub(crate) struct Config {
    pub host: HttpHost,
    pub revoked: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub timeout: Duration,
    pub max_requests: u32,
    pub source: &'static str,
    pub entry: &'static str,
    pub input: Value,
}

pub(crate) fn execute(
    config: Config,
    cancel: Arc<AtomicBool>,
    input: Option<mpsc::Receiver<Value>>,
    install: impl FnOnce(&Ctx<'_>) -> Result<()>,
    emit: impl FnMut(Value) + 'static,
) -> Result<Value> {
    let mut host = config.host;
    let http_cancel = cancel.clone();
    let http_revoked = config.revoked.clone();
    let cancelled = Arc::new(move || {
        http_cancel.load(Ordering::Relaxed) || http_revoked.as_ref().is_some_and(|check| check())
    });
    host.cancelled = Some(cancelled.clone());
    // The body is itself JSON inside this envelope; escaping can double it.
    let envelope_limit = host.body_limit() * 2 + 64 * 1024;
    let started = Instant::now();
    let deadline = started + config.timeout;
    let rt = Runtime::new()?;
    rt.set_memory_limit(96 * 1024 * 1024);
    rt.set_max_stack_size(1024 * 1024);
    let interrupt_cancel = cancel.clone();
    let revoked = config.revoked.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        Instant::now() >= deadline
            || interrupt_cancel.load(Ordering::Relaxed)
            || revoked.as_ref().is_some_and(|check| check())
    })));
    let ctx = Context::full(&rt)?;
    let timers = Rc::new(RefCell::new(HashMap::<u32, Instant>::new()));
    let jobs = HttpJobs(Rc::new(RefCell::new(HashMap::new())));
    let finished = Rc::new(RefCell::new(None::<Value>));
    let (tx, rx) = mpsc::sync_channel::<Value>(16);
    ctx.with(|ctx| -> Result<()> {
        let globals = ctx.globals();
        globals.set(
            "__now",
            Function::new(ctx.clone(), move || {
                started.elapsed().as_secs_f64() * 1000.0
            })?,
        )?;
        globals.set(
            "__random",
            Function::new(
                ctx.clone(),
                |ctx: Ctx<'_>, len: usize| -> rquickjs::Result<Vec<u8>> {
                    if len > 65536 {
                        return Err(Exception::throw_message(&ctx, "random byte limit"));
                    }
                    let mut bytes = vec![0; len];
                    getrandom::fill(&mut bytes)
                        .map_err(|_| Exception::throw_message(&ctx, "OS randomness unavailable"))?;
                    Ok(bytes)
                },
            )?,
        )?;
        globals.set(
            "__sha256",
            Function::new(
                ctx.clone(),
                |ctx: Ctx<'_>, bytes: Vec<u8>| -> rquickjs::Result<Vec<u8>> {
                    if bytes.len() > 1024 * 1024 {
                        return Err(Exception::throw_message(&ctx, "digest input limit"));
                    }
                    Ok(Sha256::digest(bytes).to_vec())
                },
            )?,
        )?;
        globals.set(
            "__btoa",
            Function::new(
                ctx.clone(),
                |ctx: Ctx<'_>, bytes: Vec<u8>| -> rquickjs::Result<String> {
                    if bytes.len() > 1024 * 1024 {
                        return Err(Exception::throw_message(&ctx, "base64 input limit"));
                    }
                    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
                },
            )?,
        )?;
        globals.set(
            "__atob",
            Function::new(
                ctx.clone(),
                |ctx: Ctx<'_>, text: String| -> rquickjs::Result<Vec<u8>> {
                    if text.len() > 1024 * 1024 {
                        return Err(Exception::throw_message(&ctx, "base64 input limit"));
                    }
                    let engine = GeneralPurpose::new(
                        &STANDARD,
                        GeneralPurposeConfig::new().with_decode_padding_mode(
                            base64::engine::DecodePaddingMode::Indifferent,
                        ),
                    );
                    engine
                        .decode(text)
                        .map_err(|_| Exception::throw_message(&ctx, "invalid base64"))
                },
            )?,
        )?;
        install(&ctx)?;
        let start_timers = timers.clone();
        globals.set(
            "__timer",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, id: u32, ms: f64| -> rquickjs::Result<()> {
                    if start_timers.borrow().len() >= 128 {
                        return Err(Exception::throw_message(&ctx, "timer limit"));
                    }
                    start_timers.borrow_mut().insert(
                        id,
                        Instant::now() + Duration::from_millis(ms.clamp(0.0, 2147483647.0) as u64),
                    );
                    Ok(())
                },
            )?,
        )?;
        let clear_timers = timers.clone();
        globals.set(
            "__clearTimer",
            Function::new(ctx.clone(), move |id: u32| {
                clear_timers.borrow_mut().remove(&id);
            })?,
        )?;
        let http_jobs = jobs.0.clone();
        let count = std::cell::Cell::new(0);
        globals.set(
            "__http",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, id: u32, input: String| -> rquickjs::Result<()> {
                    if cancelled() {
                        return Err(Exception::throw_message(&ctx, "HTTP cancelled"));
                    }
                    if input.len() > envelope_limit
                        || count.get() >= config.max_requests
                        || http_jobs.borrow().len() >= 2
                    {
                        return Err(Exception::throw_message(&ctx, "HTTP request limit"));
                    }
                    let request = serde_json::from_str(&input)
                        .map_err(|_| Exception::throw_message(&ctx, "invalid HTTP request"))?;
                    count.set(count.get() + 1);
                    let job = http::spawn_fetch(host.clone(), id, request, tx.clone());
                    http_jobs.borrow_mut().insert(id, job);
                    Ok(())
                },
            )?,
        )?;
        let cancel_jobs = jobs.0.clone();
        globals.set(
            "__cancelHttp",
            Function::new(ctx.clone(), move |id: u32| {
                if let Some(job) = cancel_jobs.borrow().get(&id) {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            })?,
        )?;
        let emit = RefCell::new(emit);
        globals.set(
            "__emit",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, event: String| -> rquickjs::Result<()> {
                    let value = serde_json::from_str(&event)
                        .map_err(|_| Exception::throw_message(&ctx, "invalid event JSON"))?;
                    (emit.borrow_mut())(value);
                    Ok(())
                },
            )?,
        )?;
        let result = finished.clone();
        globals.set(
            "__finish",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, data: String| -> rquickjs::Result<()> {
                    *result.borrow_mut() = Some(
                        serde_json::from_str(&data)
                            .map_err(|_| Exception::throw_message(&ctx, "invalid result JSON"))?,
                    );
                    Ok(())
                },
            )?,
        )?;
        for source in [
            include_str!("../generated/encoding.js"),
            include_str!("../generated/platform.js"),
            config.source,
        ] {
            js(&ctx, ctx.eval::<(), _>(source))?;
        }
        let start: Function = globals.get(config.entry)?;
        js(&ctx, start.call::<_, ()>((config.input.to_string(),)))?;
        Ok(())
    })?;
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "cancelled");
        ensure!(
            !config.revoked.as_ref().is_some_and(|check| check()),
            "account revoked"
        );
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
            return Ok(result);
        }
        if let Some(input) = &input {
            while let Ok(event) = input.try_recv() {
                dispatch(&ctx, event)?;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mobile_web_crypto_encoding_and_abort_contracts() -> Result<()> {
        let value = execute(
            Config {
                host: HttpHost::auth(crate::auth::Provider::Claude),
                revoked: None,
                timeout: Duration::from_secs(2),
                max_requests: 0,
                entry: "checkPlatform",
                input: json!({}),
                source: r#"
            globalThis.checkPlatform = async () => {
                try {
                    const bytes = new TextEncoder().encode('xabcx').subarray(1,4);
                    const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), x => x.toString(16).padStart(2,'0')).join('');
                    if (digest !== 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad') throw Error('digest');
                    if (btoa('ÿ') !== '/w==' || atob('YQ') !== 'a' || atob(' Y Q ==\n') !== 'a') throw Error('base64');
                    let rejected = false; try { btoa('Ā'); } catch { rejected = true; } if (!rejected) throw Error('Latin-1');
                    const a = new AbortController(), b = new AbortController();
                    const signal = AbortSignal.any([a.signal,b.signal]); const reason = Error('expected'); a.abort(reason);
                    if (!signal.aborted || signal.reason !== reason) throw Error('reason');
                    try { signal.throwIfAborted(); throw Error('did not throw'); } catch (error) { if (error !== reason) throw error; }
                    const timeout = AbortSignal.timeout(1);
                    await new Promise(resolve => timeout.addEventListener('abort', resolve, {once:true}));
                    if (timeout.reason.name !== 'TimeoutError') throw Error('timeout');
                    if (typeof __read !== 'undefined' || typeof __write !== 'undefined') throw Error('workspace capability leaked');
                    __finish(JSON.stringify({ok:true}));
                } catch (error) { __finish(JSON.stringify({error:error.message})); }
            };
            "#,
            },
            Arc::new(AtomicBool::new(false)),
            None,
            |_| Ok(()),
            |_| {},
        )?;
        ensure!(value["ok"] == true, "platform contract failed");
        Ok(())
    }
}
