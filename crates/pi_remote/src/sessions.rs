//! The sessions on this host, for an app that wants a list without attaching to
//! each one. Every daemon keeps a small summary beside its endpoint; `sessions`
//! prints them with the identity each was started with.
use anyhow::Result;
use pi_core::{session::Session, ssh::SshTarget};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// What a list shows of one session, without its transcript.
fn summary(model: &Session, target: &SshTarget) -> Value {
    let stop_reason = model
        .messages
        .iter()
        .rev()
        .find(|message| message["role"] == "assistant")
        .and_then(|message| message["stopReason"].as_str());
    json!({
        "key": target.key,
        "cwd": target.cwd,
        "backend": target.backend,
        "title": model.title(),
        "busy": model.busy(),
        "stopReason": stop_reason,
        "error": model.error,
    })
}

/// Rewrites `path` when the summary changed, stamped with when it did.
pub(crate) fn record(
    path: &Path,
    model: &Session,
    target: &SshTarget,
    last: &mut Option<Value>,
) -> Result<()> {
    let current = summary(model, target);
    if last.as_ref() == Some(&current) {
        return Ok(());
    }
    let mut stamped = current.clone();
    stamped["updated"] = json!(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())
    );
    let partial = path.with_extension("partial");
    let mut file = crate::server::private_file(&partial)?;
    file.write_all(&serde_json::to_vec(&stamped)?)?;
    file.sync_all()?;
    fs::rename(partial, path)?;
    *last = Some(current);
    Ok(())
}

fn running(endpoint: &Path) -> bool {
    let Ok(endpoint) = fs::read(endpoint) else {
        return false;
    };
    let pid = serde_json::from_slice::<Value>(&endpoint)
        .ok()
        .and_then(|endpoint| endpoint["pid"].as_u64())
        .and_then(|pid| i32::try_from(pid).ok())
        .filter(|pid| *pid > 0);
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        pid.is_some_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
    }
    #[cfg(not(unix))]
    {
        pid.is_some()
    }
}

/// Every session that ever had a daemon here, newest first.
pub(crate) fn list(root: &Path) -> Result<Value> {
    let mut sessions = Vec::new();
    for entry in fs::read_dir(root)?.flatten() {
        let name = entry.file_name();
        let Some(key) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".identity.json"))
        else {
            continue;
        };
        let Ok(identity) = fs::read(entry.path())
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(serde_json::from_slice::<Value>(&bytes)?))
        else {
            continue;
        };
        let summary = fs::read(root.join(format!("{key}.summary.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|summary| summary["key"] == key)
            .unwrap_or_else(|| json!({}));
        sessions.push(json!({
            "key": key,
            "cwd": identity["cwd"],
            "backend": identity.get("backend").cloned().unwrap_or_else(|| json!("pi")),
            "title": summary["title"],
            "busy": summary["busy"].as_bool().unwrap_or(false),
            "stopReason": summary["stopReason"],
            "error": summary["error"],
            "updated": summary["updated"].as_u64().unwrap_or(0),
            "running": running(&root.join(format!("{key}.json"))),
        }));
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session["updated"].as_u64()));
    Ok(json!({ "version": 1, "sessions": sessions }))
}

pub fn print() -> Result<()> {
    println!("{}", list(&crate::server::root()?)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_is_kept_beside_the_identity_and_listed() {
        let root = tempfile::tempdir().unwrap();
        let target = SshTarget::new("phone".into(), "/work/pi".into()).unwrap();
        fs::write(
            root.path().join(format!("{}.identity.json", target.key)),
            json!({"cwd":"/work/pi","backend":"durable"}).to_string(),
        )
        .unwrap();
        let path = root.path().join(format!("{}.summary.json", target.key));
        let mut model = Session::new("/work/pi".into());
        model
            .apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
                {"role":"user","content":"Fix the flaky test"},
                {"role":"assistant","content":[{"type":"text","text":"Done"}],"stopReason":"stop"}
            ]}}))
            .unwrap();
        let mut last = None;
        record(&path, &model, &target, &mut last).unwrap();
        let written = fs::metadata(&path).unwrap().modified().unwrap();
        record(&path, &model, &target, &mut last).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            written,
            "an unchanged summary is not rewritten"
        );
        let listed = list(root.path()).unwrap();
        let session = &listed["sessions"][0];
        assert_eq!(session["key"], target.key.as_str());
        assert_eq!(session["cwd"], "/work/pi");
        assert_eq!(session["backend"], "durable");
        assert_eq!(session["title"], "Fix the flaky test");
        assert_eq!(session["busy"], false);
        assert_eq!(session["stopReason"], "stop");
        assert_eq!(session["running"], false, "no endpoint, no daemon");
        assert!(session["updated"].as_u64().unwrap() > 0);
    }
}
