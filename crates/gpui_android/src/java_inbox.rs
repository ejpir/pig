//! Activity-scoped FIFO routing; generations never enter trace labels.
pub(crate) struct Inbox<E, W> {
    generation: u64,
    waker: Option<W>,
    events: Vec<E>,
}
impl<E, W> Inbox<E, W> {
    pub const fn new() -> Self {
        Self {
            generation: 0,
            waker: None,
            events: Vec::new(),
        }
    }
    pub fn attach(&mut self, waker: W) -> u64 {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("activity generation exhausted");
        self.events.clear();
        self.waker = Some(waker);
        self.generation
    }
    pub fn retire(&mut self, generation: u64) {
        if generation == self.generation {
            self.waker = None;
            self.events.clear();
        }
    }
    pub fn deliver(&mut self, generation: u64, event: E) -> Option<&W> {
        if generation != self.generation || self.waker.is_none() {
            return None;
        }
        self.events.push(event);
        self.waker.as_ref()
    }
    pub fn take(&mut self, generation: u64) -> Vec<E> {
        if generation != self.generation {
            return Vec::new();
        }
        std::mem::take(&mut self.events)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attach_preserves_new_fifo_and_rejects_old_picker_and_ime_events() {
        let mut inbox = Inbox::new();
        let old = inbox.attach("old");
        assert_eq!(inbox.deliver(old, "queued-old"), Some(&"old"));
        let new = inbox.attach("new");
        assert_eq!(inbox.deliver(old, "late-picker-request-1"), None);
        assert_eq!(inbox.deliver(old, "late-ime-higher-sequence"), None);
        assert_eq!(inbox.deliver(new, "startup-url"), Some(&"new"));
        inbox.deliver(new, "initial-insets");
        assert!(inbox.take(old).is_empty());
        inbox.retire(old); // Old Rust Drop / Java onDestroy cannot retire new.
        assert_eq!(inbox.take(new), vec!["startup-url", "initial-insets"]);
    }
    #[test]
    fn retire_purges_queued_events_and_rejects_late_callbacks() {
        let mut inbox = Inbox::new();
        assert!(inbox.deliver(0, 0).is_none());
        let generation = inbox.attach(());
        inbox.deliver(generation, 1);
        inbox.retire(generation);
        assert!(inbox.deliver(generation, 2).is_none());
        assert!(inbox.take(generation).is_empty());
        inbox.retire(generation);
        let next = inbox.attach(());
        assert_ne!(next, generation);
        inbox.deliver(next, 3);
        assert_eq!(inbox.take(next), vec![3]);
    }
}
