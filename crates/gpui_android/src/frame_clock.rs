//! Vsync ticks on demand, from Android's Choreographer.
//!
//! The Choreographer lives on its own looper thread. Posting to it from the
//! `android_main` thread would make every frame end `ALooper_pollOnce` with
//! `ALOOPER_POLL_CALLBACK`, which android-activity logs as an error. Here a tick
//! sets a flag and wakes the main loop instead, and a callback is posted only
//! while GPUI asks for frames, so an idle window costs no wakeups.

use android_activity::AndroidAppWaker;
use std::{
    cell::Cell,
    ffi::c_void,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

struct Shared {
    wanted: AtomicBool,
    ticked: AtomicBool,
    waker: AndroidAppWaker,
}

/// What a frame callback needs, owned by the vsync thread for the process lifetime.
struct Vsync {
    shared: Arc<Shared>,
    posted: Cell<bool>,
}

struct Looper(*mut ndk_sys::ALooper);
// SAFETY: `ALooper_wake` may be called from any thread, and the looper is
// acquired for as long as the clock exists.
unsafe impl Send for Looper {}
unsafe impl Sync for Looper {}

pub(crate) struct FrameClock {
    shared: Arc<Shared>,
    looper: Looper,
}

impl FrameClock {
    pub fn new(waker: AndroidAppWaker) -> anyhow::Result<Self> {
        let shared = Arc::new(Shared {
            wanted: AtomicBool::new(false),
            ticked: AtomicBool::new(false),
            waker,
        });
        let (looper_sender, looper) = mpsc::channel();
        let vsync = shared.clone();
        thread::Builder::new().name("Vsync".into()).spawn(move || {
            // SAFETY: preparing a looper for this new thread, which keeps it.
            let looper = unsafe { ndk_sys::ALooper_prepare(0) };
            unsafe { ndk_sys::ALooper_acquire(looper) };
            looper_sender.send(Looper(looper)).ok();
            run(vsync);
        })?;
        let looper = looper.recv()?;
        Ok(Self { shared, looper })
    }

    /// Asks for one tick at the next vsync.
    pub fn request(&self) {
        if !self.shared.wanted.swap(true, Ordering::SeqCst) {
            // SAFETY: the looper was acquired by the vsync thread and never released.
            unsafe { ndk_sys::ALooper_wake(self.looper.0) };
        }
    }

    /// Whether a vsync arrived since the last call.
    pub fn take_tick(&self) -> bool {
        self.shared.ticked.swap(false, Ordering::SeqCst)
    }
}

fn run(shared: Arc<Shared>) {
    // SAFETY: this thread has a looper, which the Choreographer requires.
    let choreographer = unsafe { ndk_sys::AChoreographer_getInstance() };
    if choreographer.is_null() {
        log::warn!("No Choreographer on this device; pacing frames with a 60 Hz timer");
        return run_timer(shared);
    }
    // Callbacks may arrive until the process ends, so the state is never freed.
    let vsync: &'static Vsync = Box::leak(Box::new(Vsync {
        shared,
        posted: Cell::new(false),
    }));
    loop {
        if vsync.shared.wanted.load(Ordering::SeqCst) && !vsync.posted.replace(true) {
            // SAFETY: `vsync` lives forever and is only touched on this thread.
            unsafe {
                ndk_sys::AChoreographer_postFrameCallback64(
                    choreographer,
                    Some(on_vsync),
                    ptr::from_ref(vsync).cast_mut().cast::<c_void>(),
                )
            };
        }
        // Returns after a frame callback ran or `request` woke the looper.
        unsafe { ndk_sys::ALooper_pollOnce(-1, ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) };
    }
}

unsafe extern "C" fn on_vsync(_frame_time_nanos: i64, data: *mut c_void) {
    // SAFETY: `data` is the leaked `Vsync` passed when posting.
    let vsync = unsafe { &*data.cast::<Vsync>() };
    vsync.posted.set(false);
    if vsync.shared.wanted.swap(false, Ordering::SeqCst) {
        vsync.shared.ticked.store(true, Ordering::SeqCst);
        vsync.shared.waker.wake();
    }
}

fn run_timer(shared: Arc<Shared>) {
    loop {
        if shared.wanted.swap(false, Ordering::SeqCst) {
            thread::sleep(Duration::from_micros(16_667));
            shared.ticked.store(true, Ordering::SeqCst);
            shared.waker.wake();
        } else {
            unsafe {
                ndk_sys::ALooper_pollOnce(-1, ptr::null_mut(), ptr::null_mut(), ptr::null_mut())
            };
        }
    }
}
