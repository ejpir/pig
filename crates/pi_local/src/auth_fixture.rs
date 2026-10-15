//! Synthetic OAuth server. Only use with a separately namespaced synthetic vault.
//! Real auth destinations are validated before Rust remaps transport to this loopback server.
use crate::{
    auth::{AuthTask, Operation, Provider, Vault, VaultScope},
    http::HttpHost,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    io::Write,
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
pub(crate) const JWT: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF90ZXN0XzEyMyJ9fQ.";
#[cfg_attr(not(test), allow(dead_code))]
struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    requests: Arc<AtomicUsize>,
    pending: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
    pause_refresh: Arc<AtomicBool>,
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
    fn new() -> Result<Arc<Self>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let pending = Arc::new(AtomicBool::new(false));
        let waiting = pending.clone();
        let fail = Arc::new(AtomicBool::new(false));
        let failing = fail.clone();
        let pause_refresh = Arc::new(AtomicBool::new(false));
        let paused = pause_refresh.clone();
        let refreshing = Arc::new(AtomicBool::new(false));
        let refresh_started = refreshing.clone();
        let worker = thread::spawn(move || {
            let mut polls = 0;
            while !stopped.load(Ordering::Relaxed) {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                let _ = socket.set_write_timeout(Some(Duration::from_secs(1)));
                let result = (|| -> Result<()> {
                    let (path, headers, body) =
                        crate::fixture::read_http_request(&mut socket, &stopped)?;
                    ensure!(
                        headers.get("authorization").is_none(),
                        "auth must not inject a bearer token"
                    );
                    count.fetch_add(1, Ordering::Relaxed);
                    let data: Value = if path == "/oauth/token" {
                        ensure!(headers["content-type"].as_str().is_some_and(|v| v.starts_with("application/x-www-form-urlencoded")), "expected form body");
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
                    let is_refresh = data["grant_type"] == "refresh_token";
                    if is_refresh {
                        refresh_started.store(true, Ordering::Relaxed);
                        while paused.load(Ordering::Relaxed) && !stopped.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_millis(2));
                        }
                    }
                    let mut status = 200;
                    let response = match path.as_str() {
                        "/api/accounts/deviceauth/usercode" => {
                            ensure!(
                                data["client_id"] == "app_EMoamEEZ73f0CkXaXp7hrann",
                                "unexpected client"
                            );
                            json!({"device_auth_id":"synthetic-device", "user_code":"TEST-CODE", "interval":"0"})
                        }
                        "/api/accounts/deviceauth/token" => {
                            ensure!(
                                data["device_auth_id"] == "synthetic-device"
                                    && data["user_code"] == "TEST-CODE",
                                "unexpected polling data"
                            );
                            polls += 1;
                            // Give the APK's explicit Copy code control time to be exercised.
                            if polls <= 3 || waiting.load(Ordering::Relaxed) {
                                status = 403;
                                json!({"error":"authorization_pending"})
                            } else {
                                json!({"authorization_code":"synthetic-code", "code_verifier":"synthetic-verifier"})
                            }
                        }
                        "/oauth/token" => {
                            ensure!(
                                data["client_id"] == "app_EMoamEEZ73f0CkXaXp7hrann",
                                "unexpected client"
                            );
                            if !is_refresh {
                                ensure!(
                                    data["code"] == "synthetic-code"
                                        && data["code_verifier"] == "synthetic-verifier"
                                        && data["redirect_uri"]
                                            == "https://auth.openai.com/deviceauth/callback",
                                    "unexpected exchange data"
                                );
                            }
                            json!({"access_token": JWT, "refresh_token": if is_refresh {"synthetic-codex-refresh-2"} else {"synthetic-codex-refresh-1"}, "expires_in": if is_refresh {3600} else {1}})
                        }
                        "/v1/oauth/token" => {
                            ensure!(
                                data["client_id"] == "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
                                "unexpected client"
                            );
                            if !is_refresh {
                                ensure!(
                                    data["code"] == "synthetic-code"
                                        && data["state"] == data["code_verifier"],
                                    "unexpected code/state"
                                );
                            }
                            json!({"access_token": if is_refresh {"sk-ant-oat-synthetic-claude-access-2"} else {"sk-ant-oat-synthetic-claude-access-1"}, "refresh_token": if is_refresh {"synthetic-claude-refresh-2"} else {"synthetic-claude-refresh-1"}, "expires_in": if is_refresh {3600} else {1}})
                        }
                        _ => anyhow::bail!("unexpected fixture path"),
                    };
                    let body = if failing.load(Ordering::Relaxed) {
                        status = 500;
                        json!({"error":"synthetic-SECRET-must-not-leak"})
                    } else {
                        response
                    }
                    .to_string();
                    write!(
                        socket,
                        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )?;
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
            pending,
            fail,
            pause_refresh,
            refreshing,
        }))
    }
    fn host(self: &Arc<Self>, provider: Provider) -> Result<HttpHost> {
        HttpHost::auth_fixture(provider, &self.base, self.clone())
    }
}

/// An explicit offline demo, never a production credential store. No real URLs are fetched.
pub fn start(
    provider: Provider,
    operation: Operation,
    synthetic_vault: Arc<dyn Vault>,
) -> Result<AuthTask> {
    ensure!(
        synthetic_vault.scope() == VaultScope::Synthetic,
        "synthetic auth requires a synthetic vault"
    );
    let server = Server::new()?;
    AuthTask::start_with_host(provider, operation, synthetic_vault, server.host(provider)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use std::time::Instant;
    #[derive(Default)]
    struct MemoryVault(Mutex<std::collections::HashMap<&'static str, Value>>);
    impl Vault for MemoryVault {
        fn scope(&self) -> VaultScope {
            VaultScope::Synthetic
        }
        fn read(&self, p: Provider) -> Result<Value> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(p.id())
                .cloned()
                .unwrap_or(json!({"generation":"", "credential":null})))
        }
        fn write(&self, p: Provider, expected: &str, credential: &str) -> Result<Option<String>> {
            let mut values = self.0.lock().unwrap();
            let record = values
                .entry(p.id())
                .or_insert(json!({"generation":"", "credential":null}));
            if record["generation"] != expected {
                return Ok(None);
            }
            let next = format!("{expected}x");
            *record =
                json!({"generation":next, "credential":serde_json::from_str::<Value>(credential)?});
            Ok(Some(next))
        }
        fn clear(&self, p: Provider) -> Result<()> {
            let mut values = self.0.lock().unwrap();
            let generation = format!(
                "{}-deleted",
                values
                    .get(p.id())
                    .map(|v| v["generation"].as_str().unwrap())
                    .unwrap_or("")
            );
            values.insert(p.id(), json!({"generation":generation,"credential":null}));
            Ok(())
        }
    }
    struct DeferredVault {
        inner: Arc<MemoryVault>,
        entered: AtomicBool,
        release: AtomicBool,
    }
    impl Vault for DeferredVault {
        fn scope(&self) -> VaultScope {
            self.inner.scope()
        }
        fn read(&self, p: Provider) -> Result<Value> {
            if !self.entered.swap(true, Ordering::Relaxed) {
                while !self.release.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(2));
                }
            }
            self.inner.read(p)
        }
        fn write(&self, p: Provider, expected: &str, credential: &str) -> Result<Option<String>> {
            self.inner.write(p, expected, credential)
        }
        fn clear(&self, p: Provider) -> Result<()> {
            self.inner.clear(p)
        }
    }
    struct AccountScopeVault(Arc<MemoryVault>);
    impl Vault for AccountScopeVault {
        fn scope(&self) -> VaultScope {
            VaultScope::Accounts
        }
        fn read(&self, p: Provider) -> Result<Value> {
            self.0.read(p)
        }
        fn write(&self, p: Provider, expected: &str, credential: &str) -> Result<Option<String>> {
            self.0.write(p, expected, credential)
        }
        fn clear(&self, p: Provider) -> Result<()> {
            self.0.clear(p)
        }
    }
    fn wait(task: &AuthTask, predicate: impl Fn(&Value) -> bool) -> Value {
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            let state = task.snapshot();
            if predicate(&state) {
                return state;
            }
            assert!(
                Instant::now() < until,
                "auth fixture did not reach expected state: {state}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn done(task: AuthTask, expected: &str) {
        let state = wait(&task, |_| task.is_finished());
        assert_eq!(state["status"], expected, "{state}");
        task.join();
    }
    #[test]
    fn upstream_mobile_login_refresh_cancellation_and_logout() -> Result<()> {
        let _serial = crate::auth::TEST_ACCOUNT_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let vault = Arc::new(MemoryVault::default());
        for provider in [Provider::Claude, Provider::Codex] {
            let server = Server::new()?;
            let task = AuthTask::start_with_host(
                provider,
                Operation::Login,
                vault.clone(),
                server.host(provider)?,
            )?;
            assert!(AuthTask::start(provider, Operation::Login, vault.clone()).is_err());
            if provider == Provider::Claude {
                let state = wait(&task, |s| {
                    s["status"] == "prompt" || s["status"] == "failed"
                });
                assert_eq!(state["status"], "prompt");
                let url = reqwest::Url::parse(state["url"].as_str().unwrap())?;
                let query: std::collections::HashMap<_, _> =
                    url.query_pairs().into_owned().collect();
                let verifier = &query["state"];
                assert_eq!(verifier.len(), 43);
                assert_eq!(
                    query["code_challenge"],
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .encode(Sha256::digest(verifier.as_bytes()))
                );
                task.respond(
                    state["prompt_id"].as_str().unwrap(),
                    format!("synthetic-code#{verifier}"),
                )?;
                assert!(task.respond("stale", "must-not-be-used".into()).is_err());
            } else {
                let state = wait(&task, |s| {
                    s.get("device_code").is_some() || s["status"] == "failed"
                });
                assert_eq!(state["device_code"], "TEST-CODE");
            }
            done(task, "connected");
            let saved = vault.read(provider)?;
            assert_eq!(saved["credential"]["type"], "oauth");
            if provider == Provider::Codex {
                assert_eq!(saved["credential"]["accountId"], "acct_test_123");
            }
            let task = AuthTask::start_with_host(
                provider,
                Operation::Refresh,
                vault.clone(),
                server.host(provider)?,
            )?;
            task.cancel(); // caller cancellation must not interrupt rotated-token persistence
            done(task, "connected");
            assert!(
                vault.read(provider)?["credential"]["refresh"]
                    .as_str()
                    .unwrap()
                    .ends_with("-2")
            );
            let requests = server.requests.load(Ordering::Relaxed);
            done(
                AuthTask::start_with_host(
                    provider,
                    Operation::Refresh,
                    vault.clone(),
                    server.host(provider)?,
                )?,
                "connected",
            );
            assert_eq!(
                server.requests.load(Ordering::Relaxed),
                requests,
                "valid credential should not refresh"
            );
            vault.clear(provider)?;
            done(
                AuthTask::start_with_host(
                    provider,
                    Operation::Refresh,
                    vault.clone(),
                    server.host(provider)?,
                )?,
                "failed",
            );
        }
        // Logout after admission but before the initial vault read also revokes login.
        let delayed = Arc::new(DeferredVault {
            inner: vault.clone(),
            entered: AtomicBool::new(false),
            release: AtomicBool::new(false),
        });
        let delayed_server = Server::new()?;
        let task = AuthTask::start_with_host(
            Provider::Claude,
            Operation::Login,
            delayed.clone(),
            delayed_server.host(Provider::Claude)?,
        )?;
        wait(&task, |_| delayed.entered.load(Ordering::Relaxed));
        crate::auth::logout(Provider::Claude, delayed.as_ref())?;
        delayed.release.store(true, Ordering::Relaxed);
        done(task, "failed");
        assert_eq!(delayed_server.requests.load(Ordering::Relaxed), 0);
        // Synthetic logout must never revoke an in-flight account-namespace refresh.
        // Both stores here contain only test data; both transports still use loopback.
        let account = Arc::new(AccountScopeVault(Arc::new(MemoryVault::default())));
        account.write(Provider::Claude, "", &json!({"type":"oauth","access":"synthetic-access","refresh":"synthetic-refresh","expires":1}).to_string())?;
        let isolated_server = Server::new()?;
        isolated_server.pause_refresh.store(true, Ordering::Relaxed);
        let task = AuthTask::start_with_host(
            Provider::Claude,
            Operation::Refresh,
            account.clone(),
            isolated_server.host(Provider::Claude)?,
        )?;
        wait(&task, |_| {
            isolated_server.refreshing.load(Ordering::Relaxed)
        });
        crate::auth::logout(Provider::Claude, vault.as_ref())?;
        isolated_server
            .pause_refresh
            .store(false, Ordering::Relaxed);
        done(task, "connected");
        assert_eq!(
            account.read(Provider::Claude)?["credential"]["refresh"],
            "synthetic-claude-refresh-2"
        );
        assert!(start(Provider::Claude, Operation::Login, account).is_err());
        // Wrong Claude state is rejected by upstream before any token request.
        let server = Server::new()?;
        let task = AuthTask::start_with_host(
            Provider::Claude,
            Operation::Login,
            vault.clone(),
            server.host(Provider::Claude)?,
        )?;
        let state = wait(&task, |s| s["status"] == "prompt");
        task.respond(
            state["prompt_id"].as_str().unwrap(),
            "synthetic-code#wrong-state".into(),
        )?;
        done(task, "failed");
        assert_eq!(server.requests.load(Ordering::Relaxed), 0);
        // Explicit cancel while polling leaves no account and releases admission.
        server.pending.store(true, Ordering::Relaxed);
        let task = AuthTask::start_with_host(
            Provider::Codex,
            Operation::Login,
            vault.clone(),
            server.host(Provider::Codex)?,
        )?;
        wait(&task, |s| s.get("device_code").is_some());
        task.cancel();
        done(task, "cancelled");
        assert!(vault.read(Provider::Codex)?["credential"].is_null());
        // Logout before manual-code exchange cannot resurrect the account (even if flow continues).
        let task = AuthTask::start_with_host(
            Provider::Claude,
            Operation::Login,
            vault.clone(),
            server.host(Provider::Claude)?,
        )?;
        let state = wait(&task, |s| s["status"] == "prompt");
        vault.clear(Provider::Claude)?;
        task.respond(
            state["prompt_id"].as_str().unwrap(),
            "synthetic-code".into(),
        )?;
        done(task, "failed");
        assert!(vault.read(Provider::Claude)?["credential"].is_null());
        // Refresh failure preserves old credential and does not leak provider error bodies.
        let old = json!({"type":"oauth","access":"synthetic-access","refresh":"synthetic-refresh","expires":1});
        let generation = vault.read(Provider::Claude)?["generation"]
            .as_str()
            .unwrap()
            .to_owned();
        vault.write(Provider::Claude, &generation, &old.to_string())?;
        server.fail.store(true, Ordering::Relaxed);
        let task = AuthTask::start_with_host(
            Provider::Claude,
            Operation::Refresh,
            vault.clone(),
            server.host(Provider::Claude)?,
        )?;
        let state = wait(&task, |_| task.is_finished());
        assert!(!state.to_string().contains("SECRET"));
        done(task, "failed");
        assert_eq!(vault.read(Provider::Claude)?["credential"], old);
        // Logout racing an already-started refresh invalidates its eventual write.
        server.fail.store(false, Ordering::Relaxed);
        server.refreshing.store(false, Ordering::Relaxed);
        server.pause_refresh.store(true, Ordering::Relaxed);
        let task = AuthTask::start_with_host(
            Provider::Claude,
            Operation::Refresh,
            vault.clone(),
            server.host(Provider::Claude)?,
        )?;
        wait(&task, |_| server.refreshing.load(Ordering::Relaxed));
        vault.clear(Provider::Claude)?;
        server.pause_refresh.store(false, Ordering::Relaxed);
        done(task, "failed");
        assert!(vault.read(Provider::Claude)?["credential"].is_null());
        Ok(())
    }
}
