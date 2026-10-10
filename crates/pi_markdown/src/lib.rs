//! Markdown, and the pictures and pages around it, shared by Pi Desktop and
//! Pi for Android: what a turn shows is found here once, and each app only
//! draws it. [`sample`] is one session with each kind, for both apps' tests.
//!
//! Desktop uses Zed's complete Markdown element. Android shares Zed's native
//! tree-sitter grammars, highlight queries, and Pi's semantic palette without
//! linking the desktop-only language/settings runtime.

mod document;
mod images;
mod pages;
mod portable;
pub mod sample;
mod svg;

pub use document::{Block, DiagramPalette, Media, Span, blocks, blocks_cached, mermaid_image};
pub use images::{DecodeError, Decoded, ToolImage, decode, decode_base64, tool_images};
pub use pages::{Page, Pages, is_page};
pub use portable::{SyntaxPalette, code_source, highlight};
pub use svg::embed_svg;

#[cfg(not(target_os = "android"))]
mod desktop;
#[cfg(not(target_os = "android"))]
pub use desktop::*;
