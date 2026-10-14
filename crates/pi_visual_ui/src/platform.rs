#[cfg(feature = "android")]
pub use gpui_android::{raw_touch, trace};
#[cfg(not(feature = "android"))]
pub mod trace {
    pub fn enabled() -> bool {
        false
    }
    pub struct Scope;
    pub fn scope(_: &'static std::ffi::CStr) -> Scope {
        Scope
    }
}
#[cfg(not(feature = "android"))]
pub mod raw_touch {
    pub fn current() -> Option<gpui::TouchEvent> {
        None
    }
    pub fn claim() {}
}
