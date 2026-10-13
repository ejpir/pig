//! Fixed-name, opt-in Android system trace sections; host builds are no-ops.
//! No guest source/state, coordinates, IDs or task locations are recorded.
use std::{ffi::CStr, marker::PhantomData, rc::Rc};

/// Android trace slices are thread-local. This guard must not cross threads.
#[must_use]
pub struct Scope {
    active: bool,
    _thread: PhantomData<Rc<()>>,
}
#[inline]
pub fn enabled() -> bool {
    #[cfg(target_os = "android")]
    // SAFETY: Android's process-global trace flag query requires no resources.
    unsafe {
        ndk_sys::ATrace_isEnabled()
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}
#[inline]
pub fn scope(name: &'static CStr) -> Scope {
    let active = enabled();
    if active {
        section(name, true);
    }
    Scope {
        active,
        _thread: PhantomData,
    }
}
impl Drop for Scope {
    #[inline]
    fn drop(&mut self) {
        if self.active {
            section(c"", false);
        }
    }
}
/// Used by typed renderer hooks whose own RAII guard balances the boundaries.
/// Callers pass only fixed static names and begin/end on the same thread.
#[inline]
pub(crate) fn section(name: &'static CStr, begin: bool) {
    #[cfg(target_os = "android")]
    // SAFETY: the name is static and NUL-terminated. Android ignores calls when
    // tracing is disabled. All callers balance sections with same-thread RAII.
    unsafe {
        if begin {
            ndk_sys::ATrace_beginSection(name.as_ptr());
        } else {
            ndk_sys::ATrace_endSection();
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = (name, begin);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_trace_is_disabled_and_sections_are_noops() {
        assert!(!enabled());
        let outer = scope(c"pi.test.outer");
        let inner = scope(c"pi.test.inner");
        assert!(!outer.active && !inner.active);
        drop(inner);
        drop(outer);
    }
}
