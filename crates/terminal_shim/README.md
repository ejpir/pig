# terminal source shim

This package builds Zed's sibling `crates/terminal` source with two narrow patches
owned by this repository:

- an external, transport-neutral PTY byte-stream backend; and
- a bounded terminal-event queue for untrusted remote output.

`build.rs` copies the sibling source to Cargo's `OUT_DIR` and applies the checked
unified diffs there. It never modifies the Zed checkout. A source mismatch fails
the build and requires reviewing/regenerating the patches.

On Android, the shim substitutes a minimal terminal-settings global. This avoids
linking Zed's desktop settings/filesystem/import stack merely to render an
external terminal. Desktop builds continue to use Zed's normal settings types.
Set `PI_ZED_ROOT` if the sibling Zed checkout is not at the repository's default
`../zed` location.
