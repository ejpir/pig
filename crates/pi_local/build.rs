//! Fail closed on missing/stale bundles: `cargo test` must not silently test yesterday's JS.
use sha2::{Digest, Sha256};
use std::fs;

fn main() {
    println!("cargo:rerun-if-changed=generated/manifest.json");
    let text = fs::read_to_string("generated/manifest.json")
        .expect("Run `npm ci --ignore-scripts && bun run build` in crates/pi_local first");
    let manifest: serde_json::Value = serde_json::from_str(&text).expect("invalid bundle manifest");
    let paths = [
        "build.ts",
        "package.json",
        "package-lock.json",
        "js/encoding.js",
        "js/platform.js",
        "js/auth.js",
        "js/no-node-http.js",
        "js/no-node-crypto.js",
        "generated/encoding.js",
        "generated/platform.js",
        "generated/auth.js",
        "generated/chat-models.json",
        "generated/LOCAL-AUTH-NPM-NOTICES.txt",
    ];
    for path in paths {
        println!("cargo:rerun-if-changed={path}");
        let bytes =
            fs::read(path).unwrap_or_else(|_| panic!("Missing {path}; run `bun run build`"));
        let hash = format!("{:x}", Sha256::digest(bytes));
        assert_eq!(
            manifest["files"][path].as_str(),
            Some(hash.as_str()),
            "Stale {path}; run `bun run build` before Cargo"
        );
    }
    // The production runtime must not silently compile yesterday's SDK bundle.
    println!("cargo:rerun-if-changed=generated/bun-manifest.json");
    let text = fs::read_to_string("generated/bun-manifest.json").expect(
        "Run `python3 experiments/bun-embedding/build_local.py` from the repository root first",
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&text).expect("invalid Bun bundle manifest");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    for (path, expected) in manifest["files"]
        .as_object()
        .expect("missing Bun bundle hashes")
    {
        let path = root.join(path);
        println!("cargo:rerun-if-changed={}", path.display());
        let hash = format!(
            "{:x}",
            Sha256::digest(fs::read(&path).expect("missing Bun bundle input"))
        );
        assert_eq!(
            expected.as_str(),
            Some(hash.as_str()),
            "Stale Bun bundle input {}; rebuild with build_local.py",
            path.display()
        );
    }
}
