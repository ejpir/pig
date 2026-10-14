//! HTTP transport only. The unmodified upstream provider/SDK parses SSE and tool calls.
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
    endpoint: Url,
    api_key: String,
    chunk_bytes: usize,
}

impl HttpHost {
    pub fn new(
        base_url: &str,
        api_key: String,
        allow_loopback_http: bool,
        chunk_bytes: usize,
    ) -> Result<Self> {
        let endpoint = Url::parse(&format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))?;
        ensure!(
            endpoint.username().is_empty()
                && endpoint.password().is_none()
                && endpoint.query().is_none()
                && endpoint.fragment().is_none(),
            "base URL must not contain credentials, a query, or a fragment"
        );
        let loopback = endpoint
            .host_str()
            .is_some_and(|host| host == "127.0.0.1" || host == "[::1]");
        ensure!(
            endpoint.scheme() == "https"
                || (allow_loopback_http && endpoint.scheme() == "http" && loopback),
            "HTTPS required (loopback HTTP is fixture-only)"
        );
        ensure!(
            (1..=4096).contains(&chunk_bytes),
            "HTTP bridge chunk size must be 1..4096"
        );
        Ok(Self {
            endpoint,
            api_key,
            chunk_bytes,
        })
    }

    async fn fetch(&self, id: u32, request: &Value, tx: &SyncSender<Value>) -> Result<()> {
        ensure!(
            request["url"].as_str() == Some(self.endpoint.as_str()),
            "HTTP destination not allowed"
        );
        ensure!(
            request["method"].as_str() == Some("POST"),
            "only POST supported"
        );
        let body = request["body"]
            .as_str()
            .context("expected a JSON request body")?;
        ensure!(body.len() <= 1024 * 1024, "request exceeds 1 MiB");
        // Per-request runtime/client for this PoC; no connection pool survives cancellation.
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .no_proxy()
            .build()?;
        let mut builder = client.post(self.endpoint.clone());
        if let Some(headers) = request["headers"].as_object() {
            for (name, value) in headers {
                // Credentials remain in Rust; the JS SDK sees only a placeholder.
                if !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "authorization" | "host" | "content-length" | "connection"
                ) {
                    builder = builder.header(name, value.as_str().context("invalid header")?);
                }
            }
        }
        let mut response = builder
            .bearer_auth(&self.api_key)
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
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async {
                tokio::select! {
                    result = host.fetch(id, &request, &tx) => result,
                    _ = async {
                        while !worker_cancel.load(Ordering::Relaxed) {
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                    } => Ok(()),
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
    fn endpoint_policy() {
        for base in [
            "http://example.com/v1",
            "https://user:pass@example.com/v1",
            "https://example.com/v1?key=bad",
            "https://example.com/v1#frag",
        ] {
            assert!(
                HttpHost::new(base, "test".into(), false, 4096).is_err(),
                "{base}"
            );
        }
        assert!(HttpHost::new("http://127.0.0.1:1234/v1", "test".into(), false, 4096).is_err());
        assert!(HttpHost::new("http://127.0.0.1:1234/v1", "test".into(), true, 4096).is_ok());
    }
}
