//! HTTP transport only. The unmodified upstream provider/SDK parses SSE and tool calls.
use crate::auth::Provider;
use anyhow::{Context, Result, ensure};
use reqwest::{Client, Url, redirect::Policy};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, TrySendError},
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Clone)]
pub struct HttpHost {
    auth_provider: Option<Provider>,
    inference: bool,
    pub(crate) cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    // Offline fixtures still validate the real destination before remapping to loopback.
    fixture: Option<Url>,
    fixture_owner: Option<Arc<dyn Send + Sync>>,
    chunk_bytes: usize,
}

impl HttpHost {
    pub(crate) fn auth(provider: Provider) -> Self {
        Self {
            auth_provider: Some(provider),
            inference: false,
            cancelled: None,
            fixture: None,
            fixture_owner: None,
            chunk_bytes: 4096,
        }
    }

    pub(crate) fn body_limit(&self) -> usize {
        if self.inference {
            10 * 1024 * 1024
        } else {
            1024 * 1024
        }
    }
    pub(crate) fn inference(provider: Provider) -> Self {
        let mut host = Self::auth(provider);
        host.inference = true;
        host
    }
    pub(crate) fn auth_fixture(
        provider: Provider,
        base: &str,
        owner: Arc<dyn Send + Sync>,
    ) -> Result<Self> {
        let url = Url::parse(base)?;
        ensure!(
            url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
            "fixture requires IPv4 loopback"
        );
        let mut host = Self::auth(provider);
        host.fixture = Some(url);
        host.fixture_owner = Some(owner);
        Ok(host)
    }

    fn destination(&self, request: &Value) -> Result<Url> {
        let requested = request["url"].as_str().context("missing URL")?;
        let allowed = if let Some(provider) = self.auth_provider {
            if self.inference {
                match provider {
                    Provider::Claude => {
                        requested == "https://api.anthropic.com/v1/messages?beta=true"
                    }
                    Provider::Codex => {
                        requested == "https://chatgpt.com/backend-api/codex/responses"
                    }
                }
            } else {
                match provider {
                    Provider::Claude => requested == "https://platform.claude.com/v1/oauth/token",
                    Provider::Codex => [
                        "https://auth.openai.com/api/accounts/deviceauth/usercode",
                        "https://auth.openai.com/api/accounts/deviceauth/token",
                        "https://auth.openai.com/oauth/token",
                    ]
                    .contains(&requested),
                }
            }
        } else {
            false
        };
        ensure!(allowed, "HTTP destination not allowed");
        ensure!(
            request["method"].as_str() == Some("POST"),
            "only POST supported"
        );
        let mut url = Url::parse(requested)?;
        if let Some(fixture) = &self.fixture {
            let path = url.path().to_owned();
            let query = url.query().map(str::to_owned);
            url = fixture.clone();
            url.set_path(&path);
            url.set_query(query.as_deref());
        }
        Ok(url)
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.as_ref().is_some_and(|check| check())
    }
    async fn fetch(&self, id: u32, request: &Value, tx: &SyncSender<Value>) -> Result<()> {
        ensure!(!self.is_cancelled(), "HTTP cancelled");
        let destination = self.destination(request)?;
        let body = if let Some(encoded) = request["bodyBase64"].as_str() {
            use base64::Engine;
            ensure!(
                self.inference && encoded.len() <= self.body_limit().div_ceil(3) * 4,
                "binary request exceeds transport limit"
            );
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .context("invalid binary body")?
        } else {
            request["body"]
                .as_str()
                .context("expected a text request body")?
                .as_bytes()
                .to_vec()
        };
        // Images come through the same bounded native attachment importer as SSH.
        // Keep the smaller limit for auth and the separate workspace PoC.
        ensure!(
            body.len() <= self.body_limit(),
            "request exceeds transport limit"
        );
        // Per-request runtime/client for this PoC; no connection pool survives cancellation.
        let client = Client::builder()
            .dns_resolver(Arc::new(crate::dns::OsResolver))
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(if self.inference { 180 } else { 30 }))
            .no_proxy()
            .build()?;
        let mut builder = client.post(destination);
        if let Some(headers) = request["headers"].as_object() {
            for (name, value) in headers {
                let name_lower = name.to_ascii_lowercase();
                if !matches!(
                    name_lower.as_str(),
                    "host" | "content-length" | "connection"
                ) {
                    builder = builder.header(name, value.as_str().context("invalid header")?);
                }
            }
        }
        ensure!(!self.is_cancelled(), "HTTP cancelled");
        let mut response = builder
            .body(body.to_owned())
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("HTTP transport failed (connect/TLS/timeout)"))?;
        ensure!(
            !response.status().is_redirection(),
            "redirects are disabled"
        );
        let headers: serde_json::Map<String, Value> = response
            .headers()
            .iter()
            .filter_map(|(key, value)| value.to_str().ok().map(|v| (key.to_string(), json!(v))))
            .collect();
        send(tx, json!({"id": id, "kind": "headers", "status": response.status().as_u16(), "headers": headers})).await?;
        let mut total = 0;
        while let Some(bytes) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("HTTP body read failed"))?
        {
            total += bytes.len();
            ensure!(total <= 4 * 1024 * 1024, "response exceeds 4 MiB");
            // Tests use ONE byte, independent of TCP coalescing, to split UTF-8/SSE boundaries.
            for chunk in bytes.chunks(self.chunk_bytes) {
                send(tx, json!({"id": id, "kind": "chunk", "bytes": chunk})).await?;
            }
        }
        send(tx, json!({"id": id, "kind": "done"})).await
    }
}

async fn send(tx: &SyncSender<Value>, mut value: Value) -> Result<()> {
    loop {
        match tx.try_send(value) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Disconnected(_)) => anyhow::bail!("VM closed"),
            Err(TrySendError::Full(returned)) => value = returned,
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

pub struct HttpJob {
    pub cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for HttpJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // Completion/cancellation drops the async request and its runtime, including sockets.
        // The receiver is dropped before outstanding jobs during VM teardown.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn spawn_fetch(host: HttpHost, id: u32, request: Value, tx: SyncSender<Value>) -> HttpJob {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            ensure!(
                !worker_cancel.load(Ordering::Relaxed) && !host.is_cancelled(),
                "HTTP cancelled"
            );
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async {
                tokio::select! {
                    biased;
                    _ = async {
                        while !worker_cancel.load(Ordering::Relaxed) && !host.is_cancelled() {
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                    } => Ok(()),
                    result = host.fetch(id, &request, &tx) => result,
                }
            })
        })();
        if let Err(error) = result {
            let _ = tx.send(json!({"id": id, "kind": "error", "message": error.to_string()}));
        }
        // Sent after runtime/socket teardown, unlike the logical HTTP 'done' event.
        let _ = tx.send(json!({"id": id, "kind": "released"}));
    });
    HttpJob {
        cancel,
        worker: Some(worker),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oauth_destinations_are_exact_and_provider_scoped() {
        let claude = HttpHost::auth(Provider::Claude);
        let codex = HttpHost::auth(Provider::Codex);
        let token = "https://platform.claude.com/v1/oauth/token";
        assert!(
            claude
                .destination(&json!({"url":token,"method":"POST"}))
                .is_ok()
        );
        assert!(
            codex
                .destination(&json!({"url":token,"method":"POST"}))
                .is_err()
        );
        for url in [
            "http://platform.claude.com/v1/oauth/token",
            "https://platform.claude.com/v1/oauth/token?next=evil",
            "https://platform.claude.com.evil.invalid/v1/oauth/token",
            "https://user@platform.claude.com/v1/oauth/token",
            "file:///private",
            "https://api.anthropic.com/v1/messages",
            "https://auth.openai.com/oauth/token",
        ] {
            assert!(
                claude
                    .destination(&json!({"url":url,"method":"POST"}))
                    .is_err()
            );
        }
        for method in ["GET", "PUT", "DELETE"] {
            assert!(
                claude
                    .destination(&json!({"url":token,"method":method}))
                    .is_err()
            );
        }
        assert!(!crate::auth::allowed_browser_url(
            Provider::Codex,
            "https://auth.openai.com.evil.invalid/codex/device"
        ));
        assert!(!crate::auth::allowed_browser_url(
            Provider::Claude,
            "javascript:alert(1)"
        ));
        assert!(!crate::auth::allowed_browser_url(
            Provider::Claude,
            "https://user@claude.ai/oauth/authorize"
        ));
    }
    #[test]
    fn inference_policy_is_separate_from_auth_and_other_providers() {
        for (provider, allowed) in [
            (
                Provider::Claude,
                "https://api.anthropic.com/v1/messages?beta=true",
            ),
            (
                Provider::Codex,
                "https://chatgpt.com/backend-api/codex/responses",
            ),
        ] {
            let host = HttpHost::inference(provider);
            assert!(
                host.destination(&json!({"url":allowed,"method":"POST"}))
                    .is_ok()
            );
            for url in [
                "https://platform.claude.com/v1/oauth/token",
                "https://auth.openai.com/oauth/token",
                "https://api.anthropic.com/v1/messages",
                "https://api.anthropic.com/v1/messages?beta=true&x=1",
                "https://chatgpt.com/backend-api/codex/responses?x=1",
                "http://chatgpt.com/backend-api/codex/responses",
                if provider == Provider::Claude {
                    "https://chatgpt.com/backend-api/codex/responses"
                } else {
                    "https://api.anthropic.com/v1/messages?beta=true"
                },
            ] {
                assert!(
                    host.destination(&json!({"url":url,"method":"POST"}))
                        .is_err()
                );
            }
            assert!(
                host.destination(&json!({"url":allowed,"method":"GET"}))
                    .is_err()
            );
        }
    }
    #[test]
    fn cancelled_http_owner_never_connects() -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let mut host = HttpHost::auth_fixture(Provider::Claude, &base, Arc::new(()))?;
        host.cancelled = Some(Arc::new(|| true));
        let (tx, _rx) = std::sync::mpsc::sync_channel(16);
        let job = spawn_fetch(
            host,
            1,
            json!({"url":"https://platform.claude.com/v1/oauth/token","method":"POST","body":"{}"}),
            tx,
        );
        drop(job);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        Ok(())
    }
}
