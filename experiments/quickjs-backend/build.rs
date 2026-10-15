//! Fail closed on missing/stale bundles: `cargo test` must not silently test yesterday's JS.
use sha2::{Digest, Sha256};
use std::fs;

fn main() {
    println!("cargo:rerun-if-changed=generated/manifest.json");
    let text = fs::read_to_string("generated/manifest.json").expect(
        "Run `npm ci --ignore-scripts && bun run build` in experiments/quickjs-backend first",
    );
    let manifest: serde_json::Value = serde_json::from_str(&text).expect("invalid bundle manifest");
    let mut paths: Vec<String> = fs::read_dir("js/node")
        .expect("Missing SDK adapters")
        .map(|entry| format!("js/node/{}", entry.unwrap().file_name().to_str().unwrap()))
        .collect();
    paths.sort();
    paths.extend(
        [
            "build.ts",
            "package.json",
            "package-lock.json",
            "js/encoding.js",
            "js/platform.js",
            "js/agent.js",
            "js/auth.js",
            "js/chat.js",
            "js/no-node-http.js",
            "js/no-node-crypto.js",
            "js/sdk-platform.js",
            "js/sdk-probe.js",
            "js/session.js",
            "generated/sdk-platform.js",
            "generated/sdk-probe.js",
            "generated/session.js",
            "generated/sdk-package.json",
            "generated/encoding.js",
            "generated/platform.js",
            "generated/agent.js",
            "generated/auth.js",
            "generated/chat.js",
            "generated/chat-models.json",
            "generated/QUICKJS-BACKEND-NPM-NOTICES.txt",
        ]
        .map(str::to_owned),
    );
    println!("cargo:rerun-if-changed=js/node");
    for path in paths {
        println!("cargo:rerun-if-changed={path}");
        let bytes =
            fs::read(&path).unwrap_or_else(|_| panic!("Missing {path}; run `bun run build`"));
        let hash = format!("{:x}", Sha256::digest(bytes));
        assert_eq!(
            manifest["files"][&path].as_str(),
            Some(hash.as_str()),
            "Stale {path}; run `bun run build` before Cargo"
        );
    }
}
