// This crate compiles copied upstream sources that are intentionally kept
// byte-for-byte apart from the narrow Android patch. Lint Pi's adapter crates,
// but do not make a Rust toolchain's newest style lints rewrite that snapshot.
#![allow(clippy::all)]

// The build script copies Zed's terminal sources to OUT_DIR and applies the
// narrow, checked external-byte-stream patch stored in this repository. The
// sibling Zed checkout remains read-only.
include!(concat!(env!("OUT_DIR"), "/src/terminal.rs"));
