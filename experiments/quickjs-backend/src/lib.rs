pub mod app_fixture;
pub mod auth;
pub mod auth_fixture;
#[cfg(feature = "bun")]
pub mod bun;
#[cfg(feature = "bun")]
mod bun_vm;
pub mod chat;
pub mod chat_fixture;
mod dns;
pub mod fixture;
mod http;
mod sdk_platform;
mod vm;
mod workspace;

use anyhow::{Result, anyhow, ensure};
use http::HttpHost;
use rquickjs::{Ctx, Exception, Function};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
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
    let result = vm::execute(
        vm::Config {
            host,
            revoked: None,
            timeout: options.timeout,
            max_requests: 8,
            source: include_str!("../generated/agent.js"),
            entry: "startAgent",
            input: json!({"baseUrl": options.base_url, "model": options.model, "prompt": options.prompt, "requestTimeoutMs": options.request_timeout_ms}),
        },
        cancel,
        None,
        move |ctx| {
            let read_ws = workspace.clone();
            ctx.globals().set(
                "__read",
                Function::new(
                    ctx.clone(),
                    move |ctx: Ctx<'_>, path: String| -> rquickjs::Result<String> {
                        read_ws
                            .read(&path)
                            .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))
                    },
                )?,
            )?;
            ctx.globals().set(
                "__write",
                Function::new(
                    ctx.clone(),
                    move |ctx: Ctx<'_>, path: String, text: String| -> rquickjs::Result<()> {
                        workspace
                            .write(&path, &text)
                            .map_err(|e| Exception::throw_message(&ctx, &e.to_string()))
                    },
                )?,
            )?;
            Ok(())
        },
        emit,
    )?;
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
    Ok(result)
}
