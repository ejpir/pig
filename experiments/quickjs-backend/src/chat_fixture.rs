//! Offline subscription wire fixtures. Real destination policy runs before loopback remapping.
use crate::{
    auth::{Vault, VaultScope},
    chat::{ChatTask, Request, SessionInput},
    http::HttpHost,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Reply,
    Stall,
    Reject,
    Truncated,
    #[cfg(test)]
    OversizeEnd,
    #[cfg(test)]
    OversizeSignature,
    #[cfg(test)]
    OverflowOnce,
    #[cfg(test)]
    OverflowAlways,
    #[cfg(test)]
    OverflowSummaryError,
    #[cfg(test)]
    SummaryError,
    #[cfg(test)]
    SummaryLength,
}
struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    #[cfg_attr(not(test), allow(dead_code))]
    requests: Arc<Mutex<Vec<Value>>>,
    #[cfg_attr(not(test), allow(dead_code))]
    closed: Arc<AtomicBool>,
    #[cfg_attr(not(test), allow(dead_code))]
    pause_refresh: Arc<AtomicBool>,
    #[cfg_attr(not(test), allow(dead_code))]
    refreshing: Arc<AtomicBool>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}
impl Server {
    fn new(mode: Mode) -> Result<Arc<Self>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let disconnected = closed.clone();
        let pause_refresh = Arc::new(AtomicBool::new(false));
        let paused = pause_refresh.clone();
        let refreshing = Arc::new(AtomicBool::new(false));
        let started_refresh = refreshing.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                let _ = socket.set_write_timeout(Some(Duration::from_secs(1)));
                let result = (|| -> Result<()> {
                    let (path, headers, body) = crate::fixture::read_http_request_limit(
                        &mut socket,
                        &stopping,
                        10 * 1024 * 1024,
                    )?;
                    if path == "/v1/oauth/token" || path == "/oauth/token" {
                        ensure!(
                            headers.get("authorization").is_none(),
                            "auth bearer injection"
                        );
                        let data: Value = if path == "/oauth/token" {
                            json!(
                                reqwest::Url::parse(&format!(
                                    "https://fixture.invalid/?{}",
                                    String::from_utf8(body)?
                                ))?
                                .query_pairs()
                                .into_owned()
                                .collect::<std::collections::HashMap<_, _>>()
                            )
                        } else {
                            serde_json::from_slice(&body)?
                        };
                        ensure!(data["grant_type"] == "refresh_token", "refresh expected");
                        started_refresh.store(true, Ordering::Relaxed);
                        while paused.load(Ordering::Relaxed) && !stopping.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_millis(2));
                        }
                        let token = if path == "/oauth/token" {
                            crate::auth_fixture::JWT
                        } else {
                            "sk-ant-oat-chat-fixture"
                        };
                        let text = json!({"access_token":token,"refresh_token":"synthetic-chat-rotated", "expires_in":3600}).to_string();
                        write!(
                            socket,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                            text.len()
                        )?;
                        return Ok(());
                    }
                    let data: Value = serde_json::from_slice(&body)?;
                    let claude = path == "/v1/messages?beta=true";
                    ensure!(
                        claude || path == "/backend-api/codex/responses",
                        "wrong inference path"
                    );
                    if claude {
                        ensure!(
                            headers["authorization"]
                                .as_str()
                                .is_some_and(|h| h.starts_with("Bearer sk-ant-oat-")),
                            "Claude must use OAuth branch"
                        );
                        ensure!(headers.get("x-api-key").is_none(), "no API key");
                        ensure!(
                            headers["anthropic-beta"]
                                .as_str()
                                .is_some_and(|h| h.contains("oauth-2025-04-20")),
                            "OAuth beta required"
                        );
                        ensure!(
                            data["system"].to_string().contains("You are Claude Code"),
                            "upstream OAuth system transform required"
                        );
                    } else {
                        ensure!(
                            headers["authorization"]
                                == format!("Bearer {}", crate::auth_fixture::JWT),
                            "OAuth JWT required"
                        );
                        ensure!(
                            headers["chatgpt-account-id"] == "acct_test_123",
                            "account header required"
                        );
                        ensure!(
                            data["store"] == false && data["stream"] == true,
                            "SSE with no remote storage required"
                        );
                    }
                    received.lock().unwrap().push(data.clone());
                    #[cfg(test)]
                    let summary = data.to_string().contains("context summarization assistant");
                    #[cfg(test)]
                    if (!summary
                        && (mode == Mode::OverflowAlways
                            || (matches!(mode, Mode::OverflowOnce | Mode::OverflowSummaryError)
                                && received.lock().unwrap().len() == 1)))
                        || (summary
                            && matches!(mode, Mode::SummaryError | Mode::OverflowSummaryError))
                    {
                        let text = if summary {
                            r#"{"error":{"message":"synthetic-SECRET-summary-error"}}"#
                        } else {
                            r#"{"error":{"message":"context length exceeded"}}"#
                        };
                        write!(
                            socket,
                            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                            text.len()
                        )?;
                        return Ok(());
                    }
                    if mode == Mode::Reject {
                        let text =
                            r#"{"error":{"message":"synthetic-SECRET-response-must-not-leak"}}"#;
                        write!(
                            socket,
                            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                            text.len()
                        )?;
                        return Ok(());
                    }
                    let text = "Hello from the phone 🌍";
                    let mut events = if claude {
                        vec![
                            json!({"type":"message_start","message":{"id":"msg_fixture","model":data["model"],"usage":{"input_tokens":1,"output_tokens":0}}}),
                            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
                            json!({"type":"content_block_stop","index":0}),
                            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
                            json!({"type":"message_stop"}),
                        ]
                    } else {
                        vec![
                            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_fixture","role":"assistant","content":[]}}),
                            json!({"type":"response.output_text.delta","output_index":0,"delta":text}),
                            json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","id":"msg_fixture","role":"assistant","content":[{"type":"output_text","text":text}]}}),
                            json!({"type":"response.completed","response":{"id":"resp_fixture","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}),
                        ]
                    };
                    #[cfg(test)]
                    if mode == Mode::SummaryLength && summary {
                        for event in &mut events {
                            if event["type"] == "message_delta" {
                                event["delta"]["stop_reason"] = json!("max_tokens");
                            }
                            if event["type"] == "response.completed" {
                                event["type"] = json!("response.incomplete");
                                event["response"]["status"] = json!("incomplete");
                                event["response"]["incomplete_details"] =
                                    json!({"reason":"max_output_tokens"});
                            }
                        }
                    }
                    let thinking = "Synthetic reasoning";
                    if claude && data["thinking"].is_object() {
                        for event in &mut events {
                            if event.get("index").is_some() {
                                event["index"] = json!(1);
                            }
                        }
                        events.splice(1..1,[
                            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
                            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":thinking}}),
                            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"synthetic-signature"}}),
                            json!({"type":"content_block_stop","index":0}),
                        ]);
                    } else if !claude
                        && data["reasoning"]["effort"]
                            .as_str()
                            .is_some_and(|e| e != "none")
                    {
                        for event in &mut events {
                            if event.get("output_index").is_some() {
                                event["output_index"] = json!(1);
                            }
                        }
                        events.splice(0..0,[
                            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"reason_fixture","summary":[]}}),
                            json!({"type":"response.reasoning_summary_text.delta","output_index":0,"delta":thinking}),
                            json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"reason_fixture","encrypted_content":"synthetic-thinking","summary":[{"type":"summary_text","text":thinking}]}}),
                        ]);
                    }
                    #[cfg(test)]
                    if mode == Mode::OversizeEnd {
                        events.retain(|e| e["type"] != "response.output_text.delta");
                        for e in &mut events {
                            if e["type"] == "response.output_item.done"
                                && e["item"]["type"] == "message"
                            {
                                e["item"]["content"][0]["text"] = json!("x".repeat(300 * 1024));
                            }
                        }
                    } else if mode == Mode::OversizeSignature {
                        for e in &mut events {
                            if e["type"] == "response.output_item.done"
                                && e["item"]["type"] == "reasoning"
                            {
                                e["item"]["encrypted_content"] = json!("x".repeat(600 * 1024));
                            }
                        }
                    }
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
                    )?;
                    let count = if mode != Mode::Stall && mode != Mode::Truncated {
                        events.len()
                    } else {
                        events.len() - 1
                    };
                    for event in events.into_iter().take(count) {
                        if claude {
                            writeln!(socket, "event: {}", event["type"].as_str().unwrap())?;
                        }
                        write!(socket, "data: {event}\n\n")?;
                    }
                    socket.flush()?;
                    if mode == Mode::Stall {
                        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
                        let mut byte = [0];
                        while !stopping.load(Ordering::Relaxed) {
                            match socket.read(&mut byte) {
                                Ok(0) => {
                                    disconnected.store(true, Ordering::Relaxed);
                                    break;
                                }
                                Err(e)
                                    if matches!(
                                        e.kind(),
                                        std::io::ErrorKind::TimedOut
                                            | std::io::ErrorKind::WouldBlock
                                    ) => {}
                                Err(_) => {
                                    disconnected.store(true, Ordering::Relaxed);
                                    break;
                                }
                                Ok(_) => (),
                            }
                        }
                    }
                    Ok(())
                })();
                if result.is_err() {
                    let _ = write!(
                        socket,
                        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                }
            }
        });
        Ok(Arc::new(Self {
            base,
            stop,
            worker: Mutex::new(Some(worker)),
            requests,
            closed,
            pause_refresh,
            refreshing,
        }))
    }
    fn start(self: &Arc<Self>, request: Request, vault: Arc<dyn Vault>) -> Result<ChatTask> {
        self.start_session(request, SessionInput::default(), vault)
    }
    fn start_session(
        self: &Arc<Self>,
        request: Request,
        session: SessionInput,
        vault: Arc<dyn Vault>,
    ) -> Result<ChatTask> {
        ensure!(
            vault.scope() == VaultScope::Synthetic,
            "fixture requires synthetic vault"
        );
        let provider = request.provider;
        ChatTask::start_session_with_hosts(
            request,
            session,
            vault,
            HttpHost::auth_fixture(provider, &self.base, self.clone())?,
            HttpHost::inference_fixture(provider, &self.base, self.clone())?,
        )
    }
}
pub fn start(request: Request, vault: Arc<dyn Vault>, mode: Mode) -> Result<ChatTask> {
    ensure!(
        vault.scope() == VaultScope::Synthetic,
        "fixture requires synthetic vault"
    );
    Server::new(mode)?.start(request, vault)
}

pub fn start_session(
    request: Request,
    session: SessionInput,
    vault: Arc<dyn Vault>,
    mode: Mode,
) -> Result<ChatTask> {
    Server::new(mode)?.start_session(request, session, vault)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{self, Provider};
    use std::time::Instant;
    struct Memory(Mutex<Value>);
    impl Memory {
        fn new(provider: Provider) -> Arc<Self> {
            Arc::new(Self(Mutex::new(json!({"generation":"0","credential":{
                "type":"oauth", "access":if provider == Provider::Claude {"sk-ant-oat-old-fixture"} else {crate::auth_fixture::JWT},
                "refresh":"synthetic-chat-old", "expires":0, "accountId":"acct_test_123"
            }}))))
        }
    }
    impl Vault for Memory {
        fn scope(&self) -> VaultScope {
            VaultScope::Synthetic
        }
        fn read(&self, _: Provider) -> Result<Value> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn write(&self, _: Provider, expected: &str, value: &str) -> Result<Option<String>> {
            let mut record = self.0.lock().unwrap();
            if record["generation"] != expected {
                return Ok(None);
            }
            let next = (expected.parse::<u64>()? + 1).to_string();
            *record = json!({"generation":next,"credential":serde_json::from_str::<Value>(value)?});
            Ok(Some(next))
        }
        fn clear(&self, _: Provider) -> Result<()> {
            let mut record = self.0.lock().unwrap();
            let next = record["generation"].as_str().unwrap().parse::<u64>()? + 1;
            *record = json!({"generation":next.to_string(),"credential":null});
            Ok(())
        }
    }
    fn request(provider: Provider, messages: Vec<Value>) -> Request {
        Request {
            provider,
            model: crate::chat::models(provider)[0].0.clone(),
            thinking_level: "off".into(),
            messages,
        }
    }
    fn user(text: &str) -> Value {
        json!({"role":"user","content":text,"timestamp":1})
    }
    fn wait(task: &ChatTask, predicate: impl Fn(&ChatTask) -> bool) {
        let start = Instant::now();
        while !predicate(task) {
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "chat timed out: {}",
                task.snapshot().status
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn sdk_manual_compaction_restores_canonical_entries_and_context() -> Result<()> {
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            let server = Server::new(Mode::Reply)?;
            let vault = Memory::new(provider);
            let first = server.start(
                request(provider, vec![user(&"old history ".repeat(9000))]),
                vault.clone(),
            )?;
            wait(&first, ChatTask::is_finished);
            assert_eq!(first.snapshot().status, "done");
            let (checkpoint, stats, _) = first.take_session_result();
            let checkpoint = checkpoint.unwrap();
            assert!(stats.unwrap()["contextUsage"]["tokens"].as_u64().unwrap() > 0);
            let mut messages = checkpoint["messages"].as_array().unwrap().clone();
            messages.push(user(&"recent history ".repeat(9000)));
            let second = server.start_session(
                request(provider, messages),
                SessionInput {
                    session_id: Some(checkpoint["sessionId"].as_str().unwrap().into()),
                    checkpoint: Some(checkpoint),
                    ..Default::default()
                },
                vault.clone(),
            )?;
            wait(&second, ChatTask::is_finished);
            assert_eq!(second.snapshot().status, "done");
            let checkpoint = second.take_session_result().0.unwrap();
            let id = checkpoint["sessionId"].as_str().unwrap().to_owned();
            let entries = checkpoint["fileEntries"].as_array().unwrap().clone();
            let messages = checkpoint["messages"].as_array().unwrap().clone();
            let compact = server.start_session(
                request(provider, messages),
                SessionInput {
                    session_id: Some(id.clone()),
                    checkpoint: Some(checkpoint),
                    compact: true,
                    custom_instructions: Some("Keep important facts".into()),
                    ..Default::default()
                },
                vault.clone(),
            )?;
            wait(&compact, ChatTask::is_finished);
            assert_eq!(compact.snapshot().status, "done");
            let (checkpoint, stats, result) = compact.take_session_result();
            let checkpoint = checkpoint.unwrap();
            assert_eq!(checkpoint["sessionId"], id);
            assert_eq!(
                &checkpoint["fileEntries"].as_array().unwrap()[..entries.len()],
                entries.as_slice()
            );
            assert!(
                checkpoint["fileEntries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["type"] == "compaction")
            );
            assert!(stats.unwrap()["contextUsage"]["tokens"].is_null());
            assert!(
                result.unwrap()["summary"]
                    .as_str()
                    .unwrap()
                    .contains("Hello")
            );
            let mut context = checkpoint["messages"].as_array().unwrap().clone();
            context.push(user("After summary"));
            let next = server.start_session(
                request(provider, context),
                SessionInput {
                    session_id: Some(id),
                    checkpoint: Some(checkpoint),
                    ..Default::default()
                },
                vault,
            )?;
            wait(&next, ChatTask::is_finished);
            assert_eq!(next.snapshot().status, "done");
            assert_eq!(server.requests.lock().unwrap().len(), 4);
            assert!(
                server.requests.lock().unwrap()[3]
                    .to_string()
                    .contains("Hello")
            );
            assert!(next.take_session_result().1.unwrap()["contextUsage"]["tokens"].is_number());
        }
        Ok(())
    }
    fn seed_checkpoint(provider: Provider) -> Result<Value> {
        let server = Server::new(Mode::Reply)?;
        let vault = Memory::new(provider);
        let first = server.start(
            request(provider, vec![user(&"old facts ".repeat(10000))]),
            vault.clone(),
        )?;
        wait(&first, ChatTask::is_finished);
        assert_eq!(first.snapshot().status, "done");
        let cp = first.take_session_result().0.unwrap();
        let mut messages = cp["messages"].as_array().unwrap().clone();
        messages.push(user(&"recent facts ".repeat(10000)));
        let second = server.start_session(
            request(provider, messages),
            SessionInput {
                session_id: Some(cp["sessionId"].as_str().unwrap().into()),
                checkpoint: Some(cp),
                ..Default::default()
            },
            vault,
        )?;
        wait(&second, ChatTask::is_finished);
        assert_eq!(second.snapshot().status, "done");
        Ok(second.take_session_result().0.unwrap())
    }
    #[test]
    fn sdk_threshold_overflow_retry_and_failed_summary_preserve_graph() -> Result<()> {
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            let cp = seed_checkpoint(provider)?;
            for mode in [
                Mode::OverflowOnce,
                Mode::OverflowAlways,
                Mode::OverflowSummaryError,
            ] {
                let server = Server::new(mode)?;
                let mut messages = cp["messages"].as_array().unwrap().clone();
                messages.push(user("next turn"));
                let task = server.start_session(
                    request(provider, messages),
                    SessionInput {
                        session_id: Some(cp["sessionId"].as_str().unwrap().into()),
                        checkpoint: Some(cp.clone()),
                        ..Default::default()
                    },
                    Memory::new(provider),
                )?;
                wait(&task, ChatTask::is_finished);
                assert_eq!(
                    task.snapshot().status,
                    if mode == Mode::OverflowOnce {
                        "done"
                    } else if mode == Mode::OverflowSummaryError {
                        "compaction_failed"
                    } else {
                        "context_full"
                    },
                    "{mode:?}, requests={}",
                    server.requests.lock().unwrap().len()
                );
                let next = task.take_session_result().0.unwrap();
                let entries = next["fileEntries"].as_array().unwrap();
                assert_eq!(
                    &entries[..cp["fileEntries"].as_array().unwrap().len()],
                    cp["fileEntries"].as_array().unwrap()
                );
                assert!(
                    entries.iter().any(|e| e["type"] == "context_edit"),
                    "upstream recovery edits must survive failed summaries too"
                );
                assert_eq!(
                    entries.iter().filter(|e| e["type"] == "compaction").count(),
                    usize::from(mode != Mode::OverflowSummaryError)
                );
                assert_eq!(
                    server.requests.lock().unwrap().len(),
                    if mode == Mode::OverflowSummaryError {
                        2
                    } else {
                        3
                    },
                    "one compact/retry per turn"
                );
                if mode != Mode::OverflowOnce {
                    assert!(task.take_message().is_none());
                }
                assert!(!next.to_string().contains("SECRET"));
            }
            for mode in [Mode::SummaryError, Mode::SummaryLength, Mode::Stall] {
                let server = Server::new(mode)?;
                let task = server.start_session(
                    request(provider, cp["messages"].as_array().unwrap().clone()),
                    SessionInput {
                        session_id: Some(cp["sessionId"].as_str().unwrap().into()),
                        checkpoint: Some(cp.clone()),
                        compact: true,
                        custom_instructions: None,
                        ..Default::default()
                    },
                    Memory::new(provider),
                )?;
                if mode == Mode::Stall {
                    wait_for(|| !server.requests.lock().unwrap().is_empty());
                    thread::sleep(Duration::from_millis(50));
                    task.cancel();
                }
                wait(&task, ChatTask::is_finished);
                assert_ne!(task.snapshot().status, "done");
                assert!(task.take_message().is_none());
                let next = task.take_session_result().0.unwrap();
                assert!(
                    !next["fileEntries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|e| e["type"] == "compaction"),
                    "partial/failed summaries must not be committed"
                );
                assert_eq!(
                    next["messages"], cp["messages"],
                    "failed summary preserves context"
                );
                if mode == Mode::Stall {
                    assert_eq!(task.snapshot().status, "stopped");
                    assert!(server.closed.load(Ordering::Relaxed));
                }
            }
            let mut high = cp.clone();
            let window = crate::chat::catalog(provider)
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["id"] == crate::chat::models(provider)[0].0)
                .unwrap()["contextWindow"]
                .as_u64()
                .unwrap();
            let last = high["fileEntries"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .rev()
                .find(|e| e["type"] == "message" && e["message"]["role"] == "assistant")
                .unwrap();
            last["message"]["usage"]["input"] = json!(window - 16384);
            last["message"]["usage"]["totalTokens"] = json!(window - 16382); // + output crosses the strict threshold.
            let server = Server::new(Mode::Reply)?;
            let mut messages = cp["messages"].as_array().unwrap().clone();
            messages.push(user("threshold preflight"));
            let task = server.start_session(
                request(provider, messages),
                SessionInput {
                    session_id: Some(cp["sessionId"].as_str().unwrap().into()),
                    checkpoint: Some(high),
                    ..Default::default()
                },
                Memory::new(provider),
            )?;
            wait(&task, ChatTask::is_finished);
            assert_eq!(task.snapshot().status, "done");
            let (next, stats, _) = task.take_session_result();
            assert_eq!(
                server.requests.lock().unwrap().len(),
                2,
                "window={window}, stats={}",
                stats.unwrap()
            );
            let next = next.unwrap();
            assert!(
                next["fileEntries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["type"] == "compaction")
            );
            let mut high = cp.clone();
            let last = high["fileEntries"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .rev()
                .find(|e| e["type"] == "message" && e["message"]["role"] == "assistant")
                .unwrap();
            last["message"]["usage"]["input"] = json!(window - 16384);
            last["message"]["usage"]["totalTokens"] = json!(window - 16382);
            let server = Server::new(Mode::Reply)?;
            let mut messages = cp["messages"].as_array().unwrap().clone();
            messages.push(user("cancel at summary boundary"));
            let task = server.start_session(
                request(provider, messages),
                SessionInput {
                    session_id: Some(cp["sessionId"].as_str().unwrap().into()),
                    checkpoint: Some(high),
                    stop_after_compaction: true,
                    ..Default::default()
                },
                Memory::new(provider),
            )?;
            wait(&task, ChatTask::is_finished);
            assert_eq!(task.snapshot().status, "stopped");
            assert_eq!(
                server.requests.lock().unwrap().len(),
                1,
                "no request may begin after cancellation at compaction completion"
            );
            let next = task.take_session_result().0.unwrap();
            assert!(
                next["fileEntries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["type"] == "compaction")
            );
            assert!(
                !next["messages"]
                    .to_string()
                    .contains("cancel at summary boundary")
            );
        }
        Ok(())
    }
    #[test]
    fn oversized_pending_input_on_short_history_is_context_full_without_inference() -> Result<()> {
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let provider = Provider::Codex;
        let cp = seed_checkpoint(provider)?;
        let previous = cp["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "assistant")
            .unwrap()
            .clone();
        let server = Server::new(Mode::Reply)?;
        let mut request = request(
            provider,
            vec![user("short"), previous, user(&"x".repeat(1024 * 1024))],
        );
        request.model = "gpt-5.3-codex-spark".into();
        let task = server.start(request, Memory::new(provider))?;
        wait(&task, ChatTask::is_finished);
        assert_eq!(task.snapshot().status, "context_full");
        assert!(task.take_message().is_none());
        assert_eq!(server.requests.lock().unwrap().len(), 0);
        assert_eq!(
            task.take_session_result().0.unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["role"] == "user")
                .count(),
            1
        );
        Ok(())
    }
    #[test]
    fn large_native_images_cross_the_vm_envelope_and_full_blocks_are_bounded() -> Result<()> {
        use base64::Engine;
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            let data = base64::engine::general_purpose::STANDARD.encode(vec![1; 1024 * 1024]);
            let mut blocks = vec![json!({"type":"text","text":"four native-sized images"})];
            for _ in 0..4 {
                blocks.push(json!({"type":"image","mimeType":"image/png","data":data}));
            }
            let task = start(
                request(
                    provider,
                    vec![json!({"role":"user","content":blocks,"timestamp":1})],
                ),
                Memory::new(provider),
                Mode::Reply,
            )?;
            wait(&task, ChatTask::is_finished);
            assert_eq!(task.snapshot().status, "done");
        }
        for mode in [Mode::OversizeEnd, Mode::OversizeSignature] {
            let mut r = request(Provider::Codex, vec![user("bounded")]);
            r.thinking_level = "medium".into();
            let server = Server::new(mode)?;
            let mut host =
                HttpHost::inference_fixture(Provider::Codex, &server.base, server.clone())?;
            // Large limit probes need not exercise one-byte UTF-8 framing again.
            host.fixture_chunk_bytes(4096);
            let auth = HttpHost::auth_fixture(Provider::Codex, &server.base, server.clone())?;
            let task = ChatTask::start_with_hosts(r, Memory::new(Provider::Codex), auth, host)?;
            wait(&task, ChatTask::is_finished);
            assert_eq!(task.snapshot().status, "failed");
            assert!(task.take_message().is_none());
            assert!(serde_json::to_vec(&task.take_records())?.len() < 512 * 1024);
        }
        Ok(())
    }
    #[test]
    fn unchanged_agent_thinking_images_and_public_protocol_records() -> Result<()> {
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            let server = Server::new(Mode::Reply)?;
            let vault = Memory::new(provider);
            let mut r = request(
                provider,
                vec![json!({"role":"user","content":[
                {"type":"text","text":"Attached text: fixture-file-content"},
                {"type":"image","mimeType":"image/png","data":"aW1hZ2UtZml4dHVyZQ=="}
            ],"timestamp":1})],
            );
            r.thinking_level = "medium".into();
            let task = server.start(r, vault)?;
            wait(&task, ChatTask::is_finished);
            assert_eq!(task.snapshot().status, "done");
            let message = task.take_message().unwrap();
            assert_eq!(message["thinkingLevel"], "medium");
            assert!(
                message["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|b| b["type"] == "thinking")
            );
            let records = task.take_records();
            assert!(
                records
                    .iter()
                    .any(|r| r["assistantMessageEvent"]["type"] == "thinking_delta")
            );
            assert!(records.iter().any(|r| r["type"] == "message_end"));
            let requests = server.requests.lock().unwrap();
            let body = &requests[0];
            assert!(body.to_string().contains("fixture-file-content"));
            assert!(body.to_string().contains("aW1hZ2UtZml4dHVyZQ=="));
            if provider == Provider::Claude {
                assert_eq!(body["output_config"]["effort"], "medium");
                assert_eq!(
                    body["messages"][0]["content"][1]["source"]["type"],
                    "base64"
                );
            } else {
                assert_eq!(body["reasoning"]["effort"], "medium");
                assert!(body["input"].to_string().contains("input_image"));
            }
        }
        Ok(())
    }
    #[test]
    fn subscription_sse_context_refresh_errors_cancellation_and_revocation() -> Result<()> {
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            let vault = Memory::new(provider);
            let server = Server::new(Mode::Reply)?;
            let task = server.start(request(provider, vec![user("Hi")]), vault.clone())?;
            wait(&task, ChatTask::is_finished);
            assert_eq!(task.snapshot().status, "done");
            assert_eq!(task.snapshot().text, "Hello from the phone 🌍");
            assert_eq!(
                vault.read(provider)?["generation"],
                "1",
                "preflight must persist rotated token"
            );
            let message = task.take_message().expect("canonical assistant message");
            let next = server.start(
                request(provider, vec![user("Hi"), message, user("Again")]),
                vault.clone(),
            )?;
            wait(&next, ChatTask::is_finished);
            assert_eq!(next.snapshot().status, "done");
            let requests = server.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(
                requests[1].to_string().contains("Hello from the phone"),
                "follow-up must retain context"
            );
            drop(requests);
            for mode in [Mode::Reject, Mode::Truncated] {
                let task = start(
                    request(provider, vec![user("failure")]),
                    vault.clone(),
                    mode,
                )?;
                wait(&task, ChatTask::is_finished);
                assert_eq!(
                    task.snapshot().status,
                    if mode == Mode::Reject {
                        "auth_error"
                    } else {
                        "failed"
                    }
                );
                assert!(!task.snapshot().text.contains("SECRET"));
                assert!(task.take_message().is_none());
            }
            let stalled = Server::new(Mode::Stall)?;
            let task = stalled.start(request(provider, vec![user("Stop")]), vault.clone())?;
            wait(&task, |t| !t.snapshot().text.is_empty());
            assert!(
                stalled
                    .start(request(provider, vec![user("duplicate")]), vault.clone())
                    .is_err()
            );
            task.cancel();
            wait(&task, ChatTask::is_finished);
            assert_eq!(task.snapshot().status, "stopped");
            assert!(stalled.closed.load(Ordering::Relaxed));
            let stalled = Server::new(Mode::Stall)?;
            let task = stalled.start(request(provider, vec![user("Revoke")]), vault.clone())?;
            wait(&task, |t| !t.snapshot().text.is_empty());
            auth::logout(provider, vault.as_ref())?;
            wait(&task, ChatTask::is_finished);
            assert_eq!(task.snapshot().status, "account_changed");
            assert!(stalled.closed.load(Ordering::Relaxed));
            assert!(vault.read(provider)?["credential"].is_null());
            let missing =
                server.start(request(provider, vec![user("Signed out")]), vault.clone())?;
            wait(&missing, ChatTask::is_finished);
            assert_eq!(missing.snapshot().status, "failed");
            assert_eq!(
                server.requests.lock().unwrap().len(),
                2,
                "missing account must not infer"
            );
            assert!(
                ChatTask::start(request(provider, vec![user("No real calls")]), vault).is_err()
            );
        }
        Ok(())
    }
    fn wait_for(predicate: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(8);
        while !predicate() {
            assert!(Instant::now() < until, "fixture barrier timed out");
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn stop_drop_and_logout_during_preflight_never_start_inference() -> Result<()> {
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            for action in ["stop", "drop", "logout"] {
                let vault = Memory::new(provider);
                let server = Server::new(Mode::Reply)?;
                server.pause_refresh.store(true, Ordering::Relaxed);
                let mut task =
                    Some(server.start(request(provider, vec![user("preflight")]), vault.clone())?);
                wait_for(|| server.refreshing.load(Ordering::Relaxed));
                if action == "logout" {
                    auth::logout(provider, vault.as_ref())?;
                } else if action == "drop" {
                    drop(task.take());
                } else {
                    task.as_ref().unwrap().cancel();
                }
                assert!(
                    auth::AccountLease::reserve(provider, VaultScope::Synthetic).is_err(),
                    "owner must retain admission"
                );
                server.pause_refresh.store(false, Ordering::Relaxed);
                if let Some(task) = task {
                    wait(&task, ChatTask::is_finished);
                    assert_eq!(
                        task.snapshot().status,
                        if action == "logout" {
                            "account_changed"
                        } else {
                            "stopped"
                        }
                    );
                } else {
                    wait_for(|| {
                        auth::AccountLease::reserve(provider, VaultScope::Synthetic).is_ok()
                    });
                }
                assert!(
                    server.requests.lock().unwrap().is_empty(),
                    "cancelled preflight must not infer"
                );
                let credential = vault.read(provider)?["credential"].clone();
                if action == "logout" {
                    assert!(credential.is_null());
                } else {
                    assert_eq!(
                        credential["refresh"], "synthetic-chat-rotated",
                        "rotation must survive Stop/drop"
                    );
                }
            }
        }
        Ok(())
    }
    #[test]
    fn resolved_handoff_does_not_refresh_again_at_expiry_threshold() -> Result<()> {
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for provider in [Provider::Claude, Provider::Codex] {
            let vault = Memory::new(provider);
            let server = Server::new(Mode::Reply)?;
            let expiry = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64
                + 301000;
            vault.0.lock().unwrap()["credential"]["expires"] = json!(expiry);
            let lease = auth::AccountLease::reserve(provider, VaultScope::Synthetic)?;
            let prepared = lease.prepare(
                provider,
                vault.clone(),
                HttpHost::auth_fixture(provider, &server.base, server.clone())?,
            )?;
            assert_eq!(vault.read(provider)?["generation"], "0");
            assert!(
                !prepared.contains("refresh"),
                "handoff must not contain refresh tokens"
            );
            thread::sleep(Duration::from_millis(1100));
            assert!(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis() as u64
                    + 300000
                    >= expiry
            );
            let result = crate::vm::execute(
                crate::vm::Config {
                    host: HttpHost::inference_fixture(provider, &server.base, server.clone())?,
                    revoked: Some(lease.revoked()),
                    timeout: Duration::from_secs(5),
                    max_requests: 1,
                    source: include_str!("../generated/chat.js"),
                    entry: "startChat",
                    input: json!({"provider":provider.id(),"model":crate::chat::models(provider)[0].0,"messages":[user("handoff")]}),
                },
                Arc::new(AtomicBool::new(false)),
                None,
                move |ctx| {
                    ctx.globals().set(
                        "__resolvedAuth",
                        rquickjs::Function::new(ctx.clone(), move || prepared.clone())?,
                    )?;
                    Ok(())
                },
                |_| {},
            )?;
            assert_eq!(result["ok"], true);
            assert_eq!(vault.read(provider)?["generation"], "0");
            assert_eq!(server.requests.lock().unwrap().len(), 1);
        }
        Ok(())
    }
    struct SlowClear {
        inner: Arc<Memory>,
        entered: AtomicBool,
        release: AtomicBool,
    }
    impl Vault for SlowClear {
        fn scope(&self) -> VaultScope {
            VaultScope::Synthetic
        }
        fn read(&self, p: Provider) -> Result<Value> {
            self.inner.read(p)
        }
        fn write(&self, p: Provider, e: &str, c: &str) -> Result<Option<String>> {
            self.inner.write(p, e, c)
        }
        fn clear(&self, p: Provider) -> Result<()> {
            self.entered.store(true, Ordering::Relaxed);
            while !self.release.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(2));
            }
            self.inner.clear(p)
        }
    }
    #[test]
    fn admission_is_closed_until_logout_finishes_clearing() -> Result<()> {
        let _serial = auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let vault = Arc::new(SlowClear {
            inner: Memory::new(Provider::Claude),
            entered: AtomicBool::new(false),
            release: AtomicBool::new(false),
        });
        let clearing = vault.clone();
        let worker = thread::spawn(move || auth::logout(Provider::Claude, clearing.as_ref()));
        wait_for(|| vault.entered.load(Ordering::Relaxed));
        let admission = auth::AccountLease::reserve(Provider::Claude, VaultScope::Synthetic);
        vault.release.store(true, Ordering::Relaxed);
        worker.join().unwrap()?;
        assert!(
            admission.is_err(),
            "new epoch must not admit old credentials while clear is blocked"
        );
        assert!(vault.read(Provider::Claude)?["credential"].is_null());
        assert!(auth::AccountLease::reserve(Provider::Claude, VaultScope::Synthetic).is_ok());
        Ok(())
    }
}
