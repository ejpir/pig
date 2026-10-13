//! Atomic urgency probe plus a synchronized tick/diagnostic-receipt pair.
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Default)]
pub(crate) struct FrameTick {
    pending: AtomicBool,
    receipt: Mutex<Option<Instant>>,
}
impl FrameTick {
    pub(crate) fn has_tick(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }
    pub(crate) fn publish(&self, receipt: Option<Instant>) {
        let mut saved = self.receipt.lock().unwrap();
        *saved = receipt;
        self.pending.store(true, Ordering::SeqCst);
    }
    pub(crate) fn take(&self) -> (bool, Option<Instant>) {
        let mut saved = self.receipt.lock().unwrap();
        let ticked = self.pending.swap(false, Ordering::SeqCst);
        // Clear even unprofiled consumption; never steal a newer callback's
        // receipt while leaving its independently-published tick pending.
        let receipt = saved.take();
        (ticked, receipt)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urgency_does_not_consume_and_take_clears_the_pair() {
        let tick = FrameTick::default();
        let now = Instant::now();
        assert!(!tick.has_tick());
        tick.publish(Some(now));
        assert!(tick.has_tick());
        assert!(tick.has_tick());
        assert_eq!(tick.take(), (true, Some(now)));
        assert!(!tick.has_tick());
        assert_eq!(tick.take(), (false, None));
    }
    #[test]
    fn coalesced_ticks_keep_the_latest_receipt() {
        let tick = FrameTick::default();
        let first = Instant::now();
        let last = Instant::now();
        tick.publish(Some(first));
        tick.publish(Some(last));
        assert_eq!(tick.take(), (true, Some(last)));
    }
    #[test]
    fn unprofiled_publish_and_consumption_do_not_retain_a_stale_receipt() {
        let tick = FrameTick::default();
        tick.publish(Some(Instant::now()));
        tick.publish(None);
        assert_eq!(tick.take(), (true, None));
        tick.publish(Some(Instant::now()));
        let _ = tick.take();
        tick.publish(None);
        assert_eq!(tick.take(), (true, None));
    }
    #[test]
    fn concurrent_callbacks_cannot_separate_receipts_from_profiled_ticks() {
        let tick = std::sync::Arc::new(FrameTick::default());
        let done = std::sync::Arc::new(AtomicBool::new(false));
        let writer = tick.clone();
        let finished = done.clone();
        let thread = std::thread::spawn(move || {
            for _ in 0..10000 {
                writer.publish(Some(Instant::now()));
                std::thread::yield_now();
            }
            finished.store(true, Ordering::SeqCst);
        });
        let mut count = 0;
        while !done.load(Ordering::SeqCst) || tick.has_tick() {
            let (pending, receipt) = tick.take();
            assert_eq!(pending, receipt.is_some());
            count += usize::from(pending);
        }
        thread.join().unwrap();
        assert!(count > 0);
    }
}
