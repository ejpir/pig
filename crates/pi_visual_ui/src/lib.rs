//! One native Visualize implementation shared by the desktop and Android hosts.
mod apps;
pub mod hooks;
mod platform;
pub mod theme;
mod ui;
pub mod visual_controls;
pub mod visual_input;
mod visual_mesh;
pub mod visual_painter;
pub mod visual_profile;
mod visual_tessellation;
pub use apps::{Host, HostExt, VisualApps, identity};

mod visibility;
