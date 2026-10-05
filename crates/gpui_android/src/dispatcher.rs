//! Task dispatch for the `android_main` thread, a worker pool and timers.
//!
//! The main thread blocks in Android's looper, so main-thread work is queued
//! here and the looper is woken to run it.

use gpui::{PlatformDispatcher, Priority, RunnableVariant, profiler};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
    sync::{Arc, Condvar, Mutex, PoisonError, mpsc},
    thread,
    time::{Duration, Instant},
};

type Wake = Box<dyn Fn() + Send + Sync>;

/// Tasks waiting to run, by priority. GPUI's own queue is not built for Android.
#[derive(Default)]
struct Queue {
    tasks: Mutex<Tasks<RunnableVariant>>,
    ready: Condvar,
}

struct Tasks<T> {
    /// High, medium and low priority.
    levels: [VecDeque<T>; 3],
    turn: u32,
}

impl<T> Default for Tasks<T> {
    fn default() -> Self {
        Self {
            levels: Default::default(),
            turn: 0,
        }
    }
}

impl<T> Tasks<T> {
    fn push(&mut self, priority: Priority, runnable: T) {
        let level = match priority {
            Priority::RealtimeAudio | Priority::High => 0,
            Priority::Medium => 1,
            Priority::Low => 2,
        };
        self.levels[level].push_back(runnable);
    }

    /// Mostly the highest priority, in proportion to GPUI's weights (60/30/10),
    /// so lower priorities still make progress under load.
    fn pop(&mut self) -> Option<T> {
        self.turn = (self.turn + 1) % 100;
        let preferred = match self.turn {
            0..60 => 0,
            60..90 => 1,
            _ => 2,
        };
        std::iter::once(preferred)
            .chain(0..3)
            .find_map(|level| self.levels[level].pop_front())
    }

    fn is_empty(&self) -> bool {
        self.levels.iter().all(VecDeque::is_empty)
    }
}

impl Queue {
    fn tasks(&self) -> std::sync::MutexGuard<'_, Tasks<RunnableVariant>> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, priority: Priority, runnable: RunnableVariant) {
        self.tasks().push(priority, runnable);
        self.ready.notify_one();
    }

    /// Blocks until a task is available.
    fn wait(&self) -> RunnableVariant {
        let mut tasks = self.tasks();
        loop {
            if let Some(runnable) = tasks.pop() {
                return runnable;
            }
            tasks = self
                .ready
                .wait(tasks)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

struct Timer {
    due: Instant,
    seq: u64,
    runnable: RunnableVariant,
}

impl PartialEq for Timer {
    fn eq(&self, other: &Self) -> bool {
        (self.due, self.seq) == (other.due, other.seq)
    }
}
impl Eq for Timer {}
impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Timer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.due, self.seq).cmp(&(other.due, other.seq))
    }
}

pub(crate) struct AndroidDispatcher {
    main: Arc<Queue>,
    wake: Wake,
    background: Arc<Queue>,
    timer_sender: mpsc::Sender<(Duration, RunnableVariant)>,
    main_thread: thread::ThreadId,
}

/// The receiving end of main-thread work, drained by the platform's run loop.
pub(crate) struct MainQueue(Arc<Queue>);

impl MainQueue {
    /// Runs queued main-thread work for at most `budget`, so input and frames
    /// are not starved. Returns whether work remains.
    pub fn run(&mut self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        loop {
            // Released before running: the task may queue more work.
            let next = self.0.tasks().pop();
            let Some(runnable) = next else {
                return false;
            };
            run(runnable);
            if Instant::now() >= deadline {
                return !self.0.tasks().is_empty();
            }
        }
    }
}

fn run(runnable: RunnableVariant) {
    let metadata = runnable.metadata();
    profiler::update_running_task(metadata.spawned, metadata.location);
    runnable.run();
    profiler::save_task_timing();
}

impl AndroidDispatcher {
    /// Must be called on the thread that will drain the [`MainQueue`]; `wake`
    /// interrupts that thread's wait for events.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> (Self, MainQueue) {
        let main = Arc::new(Queue::default());
        let background = Arc::new(Queue::default());
        let workers = thread::available_parallelism().map_or(2, |n| n.get().max(2));
        for i in 0..workers {
            let background = background.clone();
            thread::Builder::new()
                .name(format!("Worker-{i}"))
                .spawn(move || {
                    loop {
                        run(background.wait());
                    }
                })
                .expect("spawn a worker thread");
        }
        let (timer_sender, timers) = mpsc::channel();
        thread::Builder::new()
            .name("Timer".into())
            .spawn(move || run_timers(timers))
            .expect("spawn the timer thread");
        let dispatcher = Self {
            main: main.clone(),
            wake: Box::new(wake),
            background,
            timer_sender,
            main_thread: thread::current().id(),
        };
        (dispatcher, MainQueue(main))
    }
}

/// Runs each timer's runnable on this thread once it is due, as on Linux.
fn run_timers(timers: mpsc::Receiver<(Duration, RunnableVariant)>) {
    let mut pending = BinaryHeap::<Reverse<Timer>>::new();
    let mut seq = 0;
    loop {
        let now = Instant::now();
        while pending
            .peek()
            .is_some_and(|Reverse(timer)| timer.due <= now)
        {
            let Reverse(timer) = pending.pop().expect("peeked");
            run(timer.runnable);
        }
        let next = match pending.peek() {
            Some(Reverse(timer)) => timers.recv_timeout(timer.due - now),
            None => timers
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match next {
            Ok((duration, runnable)) => {
                seq += 1;
                pending.push(Reverse(Timer {
                    due: Instant::now() + duration,
                    seq,
                    runnable,
                }));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Shutting down: dropping a pending runnable would cancel its task
                // and panic its awaiter, so leak them instead.
                pending.into_iter().for_each(std::mem::forget);
                return;
            }
        }
    }
}

impl PlatformDispatcher for AndroidDispatcher {
    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        self.background.push(priority, runnable);
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        self.main.push(priority, runnable);
        (self.wake)();
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        if let Err(error) = self.timer_sender.send((duration, runnable)) {
            std::mem::forget(error);
        }
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        thread::spawn(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{BackgroundExecutor, ForegroundExecutor};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn main_thread_work_waits_for_the_loop_and_wakes_it() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let (dispatcher, mut main) = AndroidDispatcher::new({
            let wakes = wakes.clone();
            move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            }
        });
        let dispatcher = Arc::new(dispatcher);
        let foreground = ForegroundExecutor::new(dispatcher.clone());

        let ran = Arc::new(AtomicUsize::new(0));
        foreground
            .spawn({
                let ran = ran.clone();
                async move {
                    ran.fetch_add(1, Ordering::SeqCst);
                }
            })
            .detach();
        assert_eq!(ran.load(Ordering::SeqCst), 0, "not until the loop runs it");
        assert!(wakes.load(Ordering::SeqCst) >= 1, "the looper was woken");
        assert!(dispatcher.is_main_thread());
        assert!(!main.run(Duration::from_millis(50)));
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn priorities_share_turns_by_weight() {
        let mut tasks = Tasks::default();
        for (priority, name) in [
            (Priority::Low, "low"),
            (Priority::Medium, "medium"),
            (Priority::High, "high"),
        ] {
            for _ in 0..100 {
                tasks.push(priority, name);
            }
        }
        let first: Vec<_> = (0..100).map(|_| tasks.pop().unwrap()).collect();
        let count = |name| first.iter().filter(|n| **n == name).count();
        assert_eq!((count("high"), count("medium"), count("low")), (60, 30, 10));
        let mut only_low = Tasks::default();
        only_low.push(Priority::Low, 1);
        assert_eq!(
            only_low.pop(),
            Some(1),
            "an empty level never blocks another"
        );
        assert!(only_low.is_empty());
    }

    #[test]
    fn timers_fire_in_due_order() {
        let (dispatcher, _main) = AndroidDispatcher::new(|| {});
        let background = BackgroundExecutor::new(Arc::new(dispatcher));
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tasks: Vec<_> = [(30, "late"), (5, "early")]
            .into_iter()
            .map(|(ms, name)| {
                let background = background.clone();
                let order = order.clone();
                background.clone().spawn(async move {
                    background.timer(Duration::from_millis(ms)).await;
                    order.lock().unwrap().push(name);
                })
            })
            .collect();
        for task in tasks {
            gpui::block_on(task);
        }
        assert_eq!(*order.lock().unwrap(), ["early", "late"]);
    }
}
