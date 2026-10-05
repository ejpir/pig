#[path = "src/executable.rs"]
mod executable;

fn main() {
    println!("cargo:rerun-if-env-changed=PI_DESKTOP_BACKEND_ARCHIVE");
    println!("cargo:rerun-if-env-changed=PI_DESKTOP_DURABLE_BINARY");
    println!("cargo:rerun-if-changed=src/executable.rs");
    if std::env::var_os("CARGO_FEATURE_BUNDLED_DURABLE").is_some() {
        use sha2::{Digest, Sha256};
        let binary = std::path::absolute(
            std::env::var_os("PI_DESKTOP_DURABLE_BINARY")
                .expect("bundled-durable needs PI_DESKTOP_DURABLE_BINARY from backend/durable"),
        )
        .expect("durable binary path");
        let os = std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo target OS");
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo target architecture");
        executable::validate(&binary, &os, &arch)
            .unwrap_or_else(|error| panic!("Cannot embed durable runner: {error}"));
        let notices = binary.parent().expect("durable parent").join("NOTICES.txt");
        assert!(
            notices.is_file(),
            "Bundled durable runner needs its generated NOTICES.txt"
        );
        println!("cargo:rerun-if-changed={}", binary.display());
        println!("cargo:rerun-if-changed={}", notices.display());
        println!(
            "cargo:rustc-env=PI_DESKTOP_DURABLE_BINARY={}",
            binary.display()
        );
        println!(
            "cargo:rustc-env=PI_DESKTOP_DURABLE_NOTICES={}",
            notices.display()
        );
        println!(
            "cargo:rustc-env=PI_DESKTOP_DURABLE_ID={:x}",
            Sha256::digest(std::fs::read(binary).expect("durable binary"))
        );
    }
    if std::env::var_os("CARGO_FEATURE_BUNDLED_BACKEND").is_some() {
        use std::hash::{Hash, Hasher};
        let archive = std::path::absolute(
            std::env::var_os("PI_DESKTOP_BACKEND_ARCHIVE")
                .expect("bundled-backend needs the archive from scripts/fetch_pi.py"),
        )
        .expect("archive path");
        println!("cargo:rerun-if-changed={}", archive.display());
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        std::fs::read(&archive)
            .expect("read Pi archive")
            .hash(&mut hash);
        println!(
            "cargo:rustc-env=PI_DESKTOP_BACKEND_ARCHIVE={}",
            archive.display()
        );
        println!(
            "cargo:rustc-env=PI_DESKTOP_BACKEND_ID={:016x}",
            hash.finish()
        );
        let notices = archive
            .parent()
            .expect("archive parent")
            .join("pi-notices.txt");
        let notices = if notices.is_file() {
            notices
        } else {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../licenses/PI-MIT.txt")
        };
        println!("cargo:rerun-if-changed={}", notices.display());
        println!(
            "cargo:rustc-env=PI_DESKTOP_BACKEND_NOTICES={}",
            notices.display()
        );
    }
}
