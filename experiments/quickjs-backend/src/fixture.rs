//! Offline HTTP fixture: real SDK + upstream SSE parser + upstream agent/tool loop.
//! No provider, API key, SSH helper, Bun runtime or Node runtime is used.
use crate::{Options, spawn};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum Scenario {
    Normal,
    InvalidArguments,
    Traversal,
    Truncated,
    HttpError,
    BodyError,
    Stall,
    StallBody,
}

pub struct Fixture {
    pub workspace: tempfile::TempDir,
    pub base_url: String,
    request_seen: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<Result<()>>>,
}
impl Fixture {
    pub fn start(scenario: Scenario) -> Result<Self> {
        Self::with_workspace(scenario, tempfile::tempdir()?)
    }
    /// Android has no writable /tmp. Keep every fixture file in the caller's private cache.
    pub fn start_in(scenario: Scenario, parent: &std::path::Path) -> Result<Self> {
        Self::with_workspace(scenario, tempfile::tempdir_in(parent)?)
    }
    fn with_workspace(scenario: Scenario, workspace: tempfile::TempDir) -> Result<Self> {
        std::fs::write(workspace.path().join("seed.txt"), "hello 🐷\n")?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base_url = format!("http://{}/v1", listener.local_addr()?);
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = stop.clone();
        let request_seen = Arc::new(AtomicBool::new(false));
        let server_seen = request_seen.clone();
        let server = thread::spawn(move || serve(listener, scenario, server_stop, server_seen));
        Ok(Self {
            workspace,
            base_url,
            request_seen,
            stop,
            server: Some(server),
        })
    }
    pub fn options(&self) -> Options {
        Options {
            workspace: self.workspace.path().into(),
            base_url: self.base_url.clone(),
            model: "fixture-model".into(),
            prompt: "Read seed.txt, copy it to result.txt, read it back, and confirm.".into(),
            api_key: "fixture-secret-rust-only".into(),
            allow_loopback_http: true,
            http_chunk_bytes: 1,
            request_timeout_ms: 5000,
            timeout: Duration::from_secs(15),
        }
    }
    pub fn received_request(&self) -> bool {
        self.request_seen.load(Ordering::Relaxed)
    }
    pub fn verify_result(&self, result: &Value) -> Result<()> {
        ensure!(
            std::fs::read_to_string(self.workspace.path().join("result.txt"))? == "hello 🐷\n",
            "file round trip failed"
        );
        let last = result["messages"]
            .as_array()
            .context("missing messages")?
            .last()
            .context("empty messages")?;
        ensure!(
            last["content"][0]["text"] == "Saved 🐷",
            "wrong final message: {last}"
        );
        Ok(())
    }
    pub fn wait_for_request(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.request_seen.load(Ordering::Relaxed) {
            ensure!(Instant::now() < deadline, "fixture never received request");
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        self.server
            .take()
            .context("fixture already joined")?
            .join()
            .map_err(|_| anyhow::anyhow!("fixture server panicked"))?
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

pub fn demo(emit: impl FnMut(Value) + Send + 'static) -> Result<Value> {
    let mut fixture = Fixture::start(Scenario::Normal)?;
    let result = spawn(fixture.options(), emit).join()?;
    fixture.finish()?;
    fixture.verify_result(&result)?;
    Ok(result)
}

fn serve(
    listener: TcpListener,
    scenario: Scenario,
    stop: Arc<AtomicBool>,
    seen: Arc<AtomicBool>,
) -> Result<()> {
    let turns = match scenario {
        Scenario::Normal => 4,
        Scenario::HttpError | Scenario::BodyError | Scenario::Stall | Scenario::StallBody => 1,
        _ => 2,
    };
    for turn in 0..turns {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut socket = loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "fixture timed out awaiting request {turn}"
            );
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error.into()),
            }
        };
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        socket.set_write_timeout(Some(Duration::from_secs(3)))?;
        let request = read_request(&mut socket, &stop)?;
        seen.store(true, Ordering::Relaxed);
        ensure!(
            request["model"] == "fixture-model" && request["stream"] == true,
            "wrong model/stream request"
        );
        let declarations = request["tools"].as_array().context("tools missing")?;
        ensure!(
            declarations.iter().any(|t| t["function"]["name"] == "read"),
            "read declaration missing"
        );
        if matches!(scenario, Scenario::Stall | Scenario::StallBody) {
            if matches!(scenario, Scenario::StallBody) {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n: waiting\n\n")?;
            }
            socket.set_read_timeout(Some(Duration::from_millis(20)))?;
            let deadline = Instant::now() + Duration::from_secs(60);
            while !stop.load(Ordering::Relaxed) {
                ensure!(
                    Instant::now() < deadline,
                    "cancel did not close HTTP connection"
                );
                match socket.read(&mut [0u8; 1]) {
                    Ok(0) => return Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {
                        return Ok(());
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    other => anyhow::bail!("unexpected stalled connection read: {other:?}"),
                }
            }
            return Ok(());
        }
        if let Scenario::BodyError = scenario {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n: incomplete\n\n")?;
            return Ok(());
        }
        if let Scenario::HttpError = scenario {
            let body = r#"{"error":{"message":"fixture denied","type":"invalid_request_error"}}"#;
            write!(
                socket,
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
            return Ok(());
        }
        let messages = request["messages"].as_array().context("messages missing")?;
        if turn > 0 {
            let result = messages
                .iter()
                .rev()
                .find(|m| m["role"] == "tool")
                .context("tool result missing from follow-up")?;
            ensure!(
                result["tool_call_id"].as_str() == Some(format!("call_{}", turn - 1).as_str()),
                "tool ID mismatch"
            );
            let content = result["content"].as_str().context("tool content missing")?;
            match scenario {
                Scenario::Normal => ensure!(
                    content == if turn == 2 { "Written" } else { "hello 🐷\n" },
                    "unexpected tool result: {content}"
                ),
                Scenario::InvalidArguments => ensure!(
                    content.contains("content") && content.to_lowercase().contains("valid"),
                    "expected validation error, got {content}"
                ),
                Scenario::Traversal => ensure!(
                    content.contains("relative workspace"),
                    "expected path rejection, got {content}"
                ),
                Scenario::Truncated => ensure!(
                    content.to_lowercase().contains("truncat")
                        || content.to_lowercase().contains("token"),
                    "expected truncation rejection: {content}"
                ),
                _ => (),
            }
        }
        let tool = match (scenario, turn) {
            (Scenario::Normal, 0) => Some(("read", json!({"path": "seed.txt"}))),
            (Scenario::Normal, 1) => Some((
                "write",
                json!({"path": "result.txt", "content": "hello 🐷\n"}),
            )),
            (Scenario::Normal, 2) => Some(("read", json!({"path": "result.txt"}))),
            (Scenario::InvalidArguments, 0) => Some(("write", json!({"path": "result.txt"}))),
            (Scenario::Traversal, 0) => Some(("read", json!({"path": "../escape"}))),
            (Scenario::Truncated, 0) => Some((
                "write",
                json!({"path": "result.txt", "content": "must not execute"}),
            )),
            _ => None,
        };
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )?;
        if let Some((name, arguments)) = tool {
            let args = arguments.to_string();
            let split = args
                .char_indices()
                .nth(args.chars().count() / 2)
                .map(|(n, _)| n)
                .unwrap();
            sse(
                &mut socket,
                json!({"tool_calls": [{"index": 0, "id": format!("call_{turn}"), "type": "function", "function": {"name": name, "arguments": &args[..split]}}]}),
                None,
            )?;
            sse(
                &mut socket,
                json!({"tool_calls": [{"index": 0, "function": {"arguments": &args[split..]}}]}),
                None,
            )?;
            sse(
                &mut socket,
                json!({}),
                Some(if matches!(scenario, Scenario::Truncated) {
                    "length"
                } else {
                    "tool_calls"
                }),
            )?;
        } else {
            sse(
                &mut socket,
                json!({"role": "assistant", "content": "Saved "}),
                None,
            )?;
            sse(&mut socket, json!({"content": "🐷"}), None)?;
            sse(&mut socket, json!({}), Some("stop"))?;
        }
        socket.write_all(b"data: [DONE]\r\n\r\n")?;
    }
    Ok(())
}
fn sse(socket: &mut TcpStream, delta: Value, finish: Option<&str>) -> Result<()> {
    let value = json!({"id": "fixture", "object": "chat.completion.chunk", "created": 1, "model": "fixture-model", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
    let data = format!("data: {value}\r\n\r\n");
    // TCP may coalesce these writes; the host bridge ALSO splits into single bytes.
    for chunk in data.as_bytes().chunks(7) {
        socket.write_all(chunk)?;
    }
    socket.flush()?;
    thread::sleep(Duration::from_millis(5));
    Ok(())
}
/// Loopback is not UID-private on Android: bound even a hostile peer's partial request.
struct RequestReader<'a> {
    socket: &'a mut TcpStream,
    stop: &'a AtomicBool,
    deadline: Instant,
}
impl Read for RequestReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("fixture stopped"));
            }
            if Instant::now() >= self.deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "fixture request deadline",
                ));
            }
            match self.socket.read(bytes) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                result => return result,
            }
        }
    }
}
fn read_request(socket: &mut TcpStream, stop: &AtomicBool) -> Result<Value> {
    let (path, headers, body) = read_http_request(socket, stop)?;
    ensure!(
        path == "/v1/chat/completions"
            && headers["authorization"] == "Bearer fixture-secret-rust-only",
        "unexpected fixture request"
    );
    Ok(serde_json::from_slice(&body)?)
}

pub(crate) fn read_http_request(
    socket: &mut TcpStream,
    stop: &AtomicBool,
) -> Result<(String, Value, Vec<u8>)> {
    read_http_request_limit(socket, stop, 1024 * 1024)
}
pub(crate) fn read_http_request_limit(
    socket: &mut TcpStream,
    stop: &AtomicBool,
    limit: usize,
) -> Result<(String, Value, Vec<u8>)> {
    socket.set_read_timeout(Some(Duration::from_millis(20)))?;
    let mut reader = BufReader::new(RequestReader {
        socket,
        stop,
        deadline: Instant::now() + Duration::from_secs(2),
    });
    let mut line = String::new();
    reader.by_ref().take(4097).read_line(&mut line)?;
    ensure!(
        line.len() <= 4096 && line.ends_with("\r\n"),
        "fixture request line too long or incomplete"
    );
    let parts: Vec<_> = line.split_whitespace().collect();
    ensure!(
        parts.len() == 3 && parts[0] == "POST" && parts[2] == "HTTP/1.1",
        "unexpected request line"
    );
    let path = parts[1].to_owned();
    let mut length = 0;
    let mut headers = json!({});
    let mut header_bytes = line.len();
    loop {
        line.clear();
        reader.by_ref().take(8193).read_line(&mut line)?;
        header_bytes += line.len();
        ensure!(header_bytes <= 16 * 1024, "fixture headers exceed 16 KiB");
        ensure!(
            line.len() <= 8192 && !line.is_empty(),
            "bad fixture request header"
        );
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').context("invalid header")?;
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse::<usize>()?;
        }
        ensure!(
            headers.get(name.to_ascii_lowercase()).is_none(),
            "duplicate fixture header"
        );
        headers[name.to_ascii_lowercase()] = json!(value.trim());
    }
    ensure!(
        headers.get("transfer-encoding").is_none() && length > 0 && length <= limit,
        "bad fixture request length"
    );
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok((path, headers, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn hostile_loopback_requests_are_bounded_and_cancellable() -> Result<()> {
        for mode in ["long-line", "many-headers", "cancel", "deadline"] {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let mut peer = TcpStream::connect(listener.local_addr()?)?;
            let (mut socket, _) = listener.accept()?;
            let stop = Arc::new(AtomicBool::new(false));
            let server_stop = stop.clone();
            let reader = thread::spawn(move || read_request(&mut socket, &server_stop));
            let started = Instant::now();
            match mode {
                "long-line" => {
                    let _ = peer.write_all(&vec![b'x'; 5000]);
                }
                "many-headers" => {
                    let _ = peer.write_all(
                        format!(
                            "POST /v1/chat/completions HTTP/1.1\r\n{}",
                            "x: y\r\n".repeat(4000)
                        )
                        .as_bytes(),
                    );
                }
                "cancel" => {
                    peer.write_all(b"POST /partial")?;
                    stop.store(true, Ordering::Relaxed);
                }
                "deadline" => {
                    peer.write_all(b"POST /partial")?;
                }
                _ => unreachable!(),
            }
            assert!(reader.join().unwrap().is_err(), "{mode}");
            assert!(started.elapsed() < Duration::from_secs(3), "{mode}");
        }
        Ok(())
    }
    #[test]
    fn upstream_streaming_agent_reads_writes_and_reads_back() -> Result<()> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let output = events.clone();
        demo(move |event| output.lock().unwrap().push(event))?;
        let events = events.lock().unwrap();
        assert_eq!(
            events.iter().filter(|e| e["type"] == "agent_end").count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e["type"] == "tool_execution_end")
                .count(),
            3
        );
        assert!(
            events.iter().any(|e| e["type"] == "message_update"
                && e["assistantMessageEvent"]["type"] == "text_delta")
        );
        assert!(!serde_json::to_string(&*events)?.contains("fixture-secret-rust-only"));
        Ok(())
    }
    #[test]
    fn upstream_validation_and_truncation_and_host_path_policy() -> Result<()> {
        for scenario in [
            Scenario::InvalidArguments,
            Scenario::Traversal,
            Scenario::Truncated,
        ] {
            let mut fixture = Fixture::start(scenario)?;
            spawn(fixture.options(), |_| {})
                .join()
                .with_context(|| format!("{scenario:?}"))?;
            fixture.finish()?;
            assert!(!fixture.workspace.path().join("result.txt").exists());
        }
        Ok(())
    }
    #[test]
    fn provider_http_error_settles() -> Result<()> {
        let mut fixture = Fixture::start(Scenario::HttpError)?;
        let error = spawn(fixture.options(), |_| {}).join().unwrap_err();
        assert!(error.to_string().contains("fixture denied"), "{error:#}");
        fixture.finish()?;
        Ok(())
    }
    #[test]
    fn cancellation_drop_and_deadline_close_http_connections() -> Result<()> {
        for scenario in [Scenario::Stall, Scenario::StallBody] {
            for mode in ["cancel", "drop", "deadline"] {
                let mut fixture = Fixture::start(scenario)?;
                let mut options = fixture.options();
                if mode == "deadline" {
                    options.timeout = Duration::from_secs(1);
                }
                let run = spawn(options, |_| {});
                fixture.wait_for_request()?;
                let started = Instant::now();
                if mode == "drop" {
                    drop(run);
                } else {
                    if mode == "cancel" {
                        run.cancel();
                    }
                    assert!(run.join().is_err());
                }
                fixture.finish()?; // Asserts actual socket EOF/reset, not just VM completion.
                assert!(started.elapsed() < Duration::from_secs(2));
            }
        }
        Ok(())
    }
    #[test]
    fn upstream_sdk_timeout_uses_js_timer_and_abort_bridge() -> Result<()> {
        let mut fixture = Fixture::start(Scenario::Stall)?;
        let mut options = fixture.options();
        options.request_timeout_ms = 100;
        let error = spawn(options, |_| {}).join().unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error:#}");
        fixture.finish()?; // SDK AbortSignal must propagate to Rust and close the socket.
        Ok(())
    }
    #[test]
    fn post_header_transport_error_settles() -> Result<()> {
        let mut fixture = Fixture::start(Scenario::BodyError)?;
        let error = spawn(fixture.options(), |_| {}).join().unwrap_err();
        assert!(
            error.to_string().contains("HTTP body read failed"),
            "{error:#}"
        );
        fixture.finish()?;
        Ok(())
    }
}
