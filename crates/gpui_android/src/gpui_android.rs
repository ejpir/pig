//! A GPUI platform for Android.
//!
//! The app runs in `GpuiActivity`, a `NativeActivity` with a small Java layer
//! (`java/`) for the on-screen keyboard, insets, clipboard, links, display
//! modes and notifications. `android_main` owns the thread that GPUI treats as its main thread,
//! Vulkan draws through the shared wgpu renderer, and raw touches feed GPUI's
//! own gesture recognizers.
//!
//! The lifecycle, touch and dispatch logic is plain Rust and is tested on any
//! host; everything that talks to Android is compiled for Android only.

pub mod activity;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod dispatcher;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod ime;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod keys;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod lifecycle;
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod touch;

#[cfg(target_os = "android")]
mod display;
#[cfg(target_os = "android")]
mod fonts;
#[cfg(target_os = "android")]
mod frame_clock;
#[cfg(target_os = "android")]
mod java;
#[cfg(target_os = "android")]
mod platform;
#[cfg(target_os = "android")]
mod window;

#[cfg(target_os = "android")]
pub use android_activity::AndroidApp;
#[cfg(target_os = "android")]
pub use platform::{AndroidPlatform, init_logging};
