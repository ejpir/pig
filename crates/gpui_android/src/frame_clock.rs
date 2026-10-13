//! Vsync ticks on demand, from Android's Choreographer.
//!
//! The Choreographer lives on its own looper thread. Posting to it from the
//! `android_main` thread would make every frame end `ALooper_pollOnce` with
//! `ALOOPER_POLL_CALLBACK`, which android-activity logs as an error. Here a tick
//! sets a flag and wakes the main loop instead, and a callback is posted only
//! while GPUI asks for frames, so an idle window costs no wakeups.

use android_activity::AndroidAppWaker;
use std::{
    cell::{Cell, RefCell},
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
    tick: crate::frame_tick::FrameTick,
    stopping: AtomicBool,
}

/// What a frame callback needs, owned by the vsync thread until terminal activity shutdown.
struct Vsync {
    shared: Arc<Shared>,
    waker: AndroidAppWaker,
    posted: Cell<bool>,
}

struct Looper(*mut ndk_sys::ALooper);
// SAFETY: `ALooper_wake` may be called from any thread, and the looper is
// acquired for as long as the clock exists.
unsafe impl Send for Looper {}
unsafe impl Sync for Looper {}

impl Drop for Looper {
    fn drop(&mut self) {
        // SAFETY: exactly the extra acquired reference transferred by the worker.
        unsafe { ndk_sys::ALooper_release(self.0) };
    }
}
struct Worker {
    looper: Looper,
    thread: thread::JoinHandle<()>,
}
pub(crate) struct FrameClock {
    shared: Arc<Shared>,
    worker: RefCell<Option<Worker>>,
}

impl FrameClock {
    pub fn new(waker: AndroidAppWaker) -> anyhow::Result<Self> {
        let shared = Arc::new(Shared {
            wanted: AtomicBool::new(false),
            tick: crate::frame_tick::FrameTick::default(),
            stopping: AtomicBool::new(false),
        });
        let (looper_sender, looper) = mpsc::channel();
        let vsync = shared.clone();
        let worker = thread::Builder::new().name("Vsync".into()).spawn(move || {
            // Only this worker ever dispatches this looper/Choreographer.
            let raw = unsafe { ndk_sys::ALooper_prepare(0) };
            if raw.is_null() {
                looper_sender
                    .send(Err("Could not prepare Vsync looper"))
                    .ok();
                return;
            }
            unsafe { ndk_sys::ALooper_acquire(raw) };
            // Failed send drops/releases the extra reference and exits promptly.
            if looper_sender.send(Ok(Looper(raw))).is_err() {
                return;
            }
            run(vsync, waker);
        })?;
        let looper = looper.recv()?.map_err(anyhow::Error::msg)?;
        Ok(Self {
            shared,
            worker: RefCell::new(Some(Worker {
                looper,
                thread: worker,
            })),
        })
    }

    /// Terminal activity shutdown, including while a callback is pending. Wake,
    /// stop dispatch on its owner thread, join, then release the acquired looper.
    pub fn shutdown(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.borrow_mut().take() {
            let _trace = crate::trace::scope(c"pi.clock.stop");
            unsafe { ndk_sys::ALooper_wake(worker.looper.0) };
            if worker.thread.join().is_err() {
                log::error!("Vsync worker failed during shutdown");
            }
            drop(worker.looper);
        }
        self.shared.wanted.store(false, Ordering::SeqCst);
        self.shared.tick.take();
    }

    /// Asks for one tick at the next vsync.
    pub fn request(&self) {
        if self.shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        if !self.shared.wanted.swap(true, Ordering::SeqCst) {
            let _trace = crate::trace::scope(c"pi.clock.request.new");
            if let Some(worker) = self.worker.borrow().as_ref() {
                // SAFETY: held acquired reference until shutdown finishes joining.
                unsafe { ndk_sys::ALooper_wake(worker.looper.0) };
            }
        } else {
            let _trace = crate::trace::scope(c"pi.clock.request.coalesced");
        }
    }

    /// Check urgency without consuming a tick or its diagnostic timestamp.
    pub fn has_tick(&self) -> bool {
        self.shared.tick.has_tick()
    }

    /// Whether a vsync arrived since the last call.
    pub fn take_tick(&self) -> bool {
        let (ticked, receipt) = self.shared.tick.take();
        let _trace = ticked.then(|| crate::trace::scope(c"pi.clock.consume"));
        if ticked
            && crate::frame_profile::enabled()
            && let Some(receipt) = receipt
        {
            crate::frame_profile::record(crate::frame_profile::Stage::WakeDelay, receipt.elapsed());
        }
        ticked
    }
}

impl Drop for FrameClock {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(shared: Arc<Shared>, waker: AndroidAppWaker) {
    // SAFETY: this thread has a looper, which the Choreographer requires.
    let choreographer = unsafe { ndk_sys::AChoreographer_getInstance() };
    if choreographer.is_null() {
        log::warn!("No Choreographer on this device; pacing frames with a 60 Hz timer");
        return run_timer(shared, waker);
    }
    // Stable allocation for all post/poll calls. Callbacks dispatch synchronously
    // on this worker's looper. After stopping, no code (including cleanup) may
    // poll this looper again. Pending callbacks are not cancelled; they become
    // uncallable when their sole dispatcher exits. Never export this looper for
    // dispatch elsewhere or introduce cleanup that dispatches pending callbacks.
    let vsync = Box::new(Vsync {
        shared,
        waker,
        posted: Cell::new(false),
    });
    while !vsync.shared.stopping.load(Ordering::SeqCst) {
        if vsync.shared.wanted.load(Ordering::SeqCst) && !vsync.posted.replace(true) {
            let _trace = crate::trace::scope(c"pi.clock.post");
            // SAFETY: stable Box remains live through all synchronous owner-thread dispatch.
            unsafe {
                ndk_sys::AChoreographer_postFrameCallback64(
                    choreographer,
                    Some(on_vsync),
                    ptr::from_ref(vsync.as_ref()).cast_mut().cast::<c_void>(),
                )
            };
        }
        // Returns after a frame callback ran or `request` woke the looper.
        unsafe { ndk_sys::ALooper_pollOnce(-1, ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) };
    }
}

unsafe extern "C" fn on_vsync(frame_time_nanos: i64, data: *mut c_void) {
    let _trace = crate::trace::scope(c"pi.vsync.callback");
    // SAFETY: `data` is the stable Vsync allocation; only its owner dispatches it.
    let vsync = unsafe { &*data.cast::<Vsync>() };
    vsync.posted.set(false);
    if vsync.shared.stopping.load(Ordering::SeqCst) {
        return;
    }
    if vsync.shared.wanted.swap(false, Ordering::SeqCst) {
        let _trace = crate::trace::scope(c"pi.clock.publish");
        let receipt = if crate::frame_profile::enabled() {
            crate::frame_profile::vsync(frame_time_nanos);
            Some(std::time::Instant::now())
        } else {
            None
        };
        vsync.shared.tick.publish(receipt);
        vsync.waker.wake();
    } else {
        let _trace = crate::trace::scope(c"pi.clock.unwanted");
    }
}

fn run_timer(shared: Arc<Shared>, waker: AndroidAppWaker) {
    while !shared.stopping.load(Ordering::SeqCst) {
        if shared.wanted.swap(false, Ordering::SeqCst) {
            // This is a fallback-timer wait, not a Choreographer callback.
            {
                let _trace = crate::trace::scope(c"pi.clock.timer.wait");
                thread::sleep(Duration::from_micros(16_667));
            }
            if shared.stopping.load(Ordering::SeqCst) {
                return;
            }
            let _trace = crate::trace::scope(c"pi.clock.timer.publish");
            shared.tick.publish(None);
            waker.wake();
        } else {
            unsafe {
                ndk_sys::ALooper_pollOnce(-1, ptr::null_mut(), ptr::null_mut(), ptr::null_mut())
            };
        }
    }
}
