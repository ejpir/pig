//! Markdown behavior shared by Pi Desktop and Pi for Android.
//!
//! Desktop uses Zed's complete Markdown element. Android shares Zed's native
//! tree-sitter grammars, highlight queries, and Pi's semantic palette without
//! linking the desktop-only language/settings runtime.

mod document;
mod portable;

pub use document::{Block, DiagramPalette, Media, Span, blocks, mermaid_image};
pub use portable::{SyntaxPalette, code_source, highlight};

#[cfg(not(target_os = "android"))]
mod desktop;
#[cfg(not(target_os = "android"))]
pub use desktop::*;
