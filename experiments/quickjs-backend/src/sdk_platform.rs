//! Minimal framework adapter: one read-only embedded package asset, no host paths.
//! This is deliberately not a Node filesystem implementation or model tool authority.
use anyhow::Result;
use base64::Engine;
use rquickjs::{Ctx, Exception, Function};
use serde_json::{Value, json};

pub(crate) fn install(ctx: &Ctx<'_>) -> Result<()> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    // Android-like POSIX virtual paths; actual architecture, no ambient environment.
    ctx.globals().set(
        "__sdkPlatform",
        rquickjs::Object::from_value(
            ctx.json_parse(json!({"platform":"android","arch":arch}).to_string())?,
        )?,
    )?;
    ctx.globals().set(
        "__nodeFs",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'_>, input: String| -> rquickjs::Result<String> {
                if input.len() > 4096 {
                    return Err(Exception::throw_message(&ctx, "Asset request limit"));
                }
                let request: Value = serde_json::from_str(&input)
                    .map_err(|_| Exception::throw_message(&ctx, "Invalid asset request"))?;
                let result = asset_request(&request);
                Ok(result.to_string())
            },
        )?,
    )?;
    Ok(())
}
fn asset_request(request: &Value) -> Value {
    let path = request["path"].as_str().unwrap_or("");
    let op = request["op"].as_str().unwrap_or("");
    let package = include_bytes!("../generated/sdk-package.json");
    let file = path == "/pi-coding-agent/package.json";
    let directory = path == "/pi-coding-agent";
    if !matches!(op, "read" | "access" | "stat" | "realpath" | "readdir") {
        return json!({"error":{"code":"EACCES"}});
    }
    if !file && !directory {
        return json!({"error":{"code":"ENOENT"}});
    }
    let data = match op {
        "access" if request["mode"].as_u64().unwrap_or(0) & 3 == 0 => Value::Null,
        "read" if file => json!(base64::engine::general_purpose::STANDARD.encode(package)),
        "stat" => {
            json!({"kind":if file {"file"}else{"directory"},"size":if file {package.len()}else{0},"mtimeMs":0})
        }
        "realpath" => json!(path),
        "readdir" if directory => json!([{"name":"package.json","kind":"file"}]),
        _ => return json!({"error":{"code":"EACCES"}}),
    };
    json!({"data":data})
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{http::HttpHost, vm};
    use std::{
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };
    #[test]
    fn upstream_public_agent_session_constructs_in_actual_quickjs() -> Result<()> {
        let result = vm::execute(
            vm::Config {
                host: HttpHost::auth(crate::auth::Provider::Claude),
                revoked: None,
                timeout: Duration::from_secs(15),
                max_requests: 0,
                source: include_str!("../generated/sdk-probe.js"),
                entry: "runSdkProbe",
                input: json!({}),
            },
            Arc::new(AtomicBool::new(false)),
            None,
            install,
            |_| {},
        )?;
        assert_eq!(result["className"], "AgentSession");
        assert_eq!(result["stableId"], true);
        assert_eq!(result["tools"], json!([]));
        assert_eq!(result["messages"], 1);
        for cp in result["roundTrips"].as_array().unwrap() {
            crate::chat::validate_checkpoint(cp, result["sessionId"].as_str())?;
        }
        assert!(
            result["roundTrips"][2]["messages"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(result["systemPromptLength"].as_u64().unwrap() > 1000);
        Ok(())
    }
    #[test]
    fn asset_mount_has_no_native_files_or_write_authority() {
        for path in [
            "/pi/agent/auth.json",
            "/data/user/0/app/no_backup/accounts.json",
            "/workspace/seed.txt",
            "/pi-coding-agent/../auth.json",
        ] {
            assert!(asset_request(&json!({"op":"read","path":path}))["error"].is_object());
        }
        assert!(
            asset_request(&json!({"op":"write","path":"/pi-coding-agent/package.json","data":""}))
                ["error"]
                .is_object()
        );
    }
}
