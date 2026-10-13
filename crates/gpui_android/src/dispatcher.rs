//! Task dispatch for the `android_main` thread, a worker pool and timers.
//!
//! The main thread blocks in Android's looper, so main-thread work is queued
//! here and the looper is woken to run it.

use gpui::{PlatformDispatcher, Priority, RunnableVariant, profiler};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
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
    closed: bool,
}

impl<T> Default for Tasks<T> {
    fn default() -> Self {
        Self {
            levels: Default::default(),
            turn: 0,
            closed: false,
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

    fn push(&self, priority: Priority, runnable: RunnableVariant) -> Result<(), RunnableVariant> {
        let mut tasks = self.tasks();
        if tasks.closed {
            return Err(runnable);
        }
        tasks.push(priority, runnable);
        drop(tasks);
        self.ready.notify_one();
        Ok(())
    }
    fn close(&self) -> [VecDeque<RunnableVariant>; 3] {
        let pending = {
            let mut tasks = self.tasks();
            tasks.closed = true;
            std::mem::take(&mut tasks.levels)
        };
        self.ready.notify_all();
        pending
    }
    fn wait(&self) -> Option<RunnableVariant> {
        let mut tasks = self.tasks();
        loop {
            if tasks.closed {
                return None;
            }
            if let Some(runnable) = tasks.pop() {
                return Some(runnable);
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

#[derive(Default)]
struct Timers {
    state: Mutex<TimerState>,
    ready: Condvar,
}
#[derive(Default)]
struct TimerState {
    closed: bool,
    sequence: u64,
    pending: BinaryHeap<Reverse<Timer>>,
}
pub(crate) struct AndroidDispatcher {
    main: Arc<Queue>,
    wake: Mutex<Option<Wake>>,
    background: Arc<Queue>,
    timers: Arc<Timers>,
    retirement: Mutex<VecDeque<RunnableVariant>>,
    local_retirement: Mutex<VecDeque<RunnableVariant>>,
    stopped: AtomicBool,
    main_thread: thread::ThreadId,
}

/// The receiving end of main-thread work, drained by the platform's run loop.
pub(crate) struct MainQueue {
    queue: Arc<Queue>,
    yielded_last_turn: bool,
    dispatcher: Arc<AndroidDispatcher>,
    _owner: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl MainQueue {
    /// Cooperative time slice: a single runnable can exceed `budget`.
    /// Returns whether work remains. Zero budget still allows one runnable.
    #[cfg(test)]
    pub fn run(&mut self, budget: Duration) -> bool {
        self.run_until(budget, || false)
    }

    /// Yield before taking another runnable when input/frame work is urgent.
    /// Never removes/cancels the runnable that should wait for the next turn.
    pub fn run_until(&mut self, budget: Duration, mut should_yield: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + budget;
        // If rendering remains continuously overdue, permit one task after a
        // yielded turn. Otherwise foreground completions could starve forever.
        let mut may_yield = !std::mem::take(&mut self.yielded_last_turn);
        loop {
            if should_yield() && may_yield {
                let pending = !self.queue.tasks().is_empty();
                self.yielded_last_turn = pending;
                return pending;
            }
            // Released before running: the task may queue more work.
            let next = self.queue.tasks().pop();
            let Some(runnable) = next else {
                return false;
            };
            run(runnable);
            may_yield = true;
            if Instant::now() >= deadline {
                return !self.queue.tasks().is_empty();
            }
        }
    }
}

impl Drop for MainQueue {
    fn drop(&mut self) {
        self.dispatcher.shutdown();
        drop(self.queue.close());
    }
}

fn run(runnable: RunnableVariant) {
    let _trace = crate::trace::scope(c"pi.task.run");
    let metadata = runnable.metadata();
    profiler::update_running_task(metadata.spawned, metadata.location);
    {
        // Distinguish the actual synchronous poll from dispatcher/profiler
        // overhead without recording task names, source locations or data.
        let _poll_trace = crate::trace::scope(c"pi.task.poll");
        runnable.run();
    }
    profiler::save_task_timing();
}

impl Drop for AndroidDispatcher {
    fn drop(&mut self) {
        // The last Arc may be released by a successful weak-scheduler upgrade
        // inside an old producer's notification callback, AFTER owner finish.
        // Never implicitly cancel residual futures in this destructor: their
        // listener Drop may reenter that producer's lock, even for Send work.
        for level in self.main.close() {
            for runnable in level {
                std::mem::forget(runnable);
            }
        }
        for level in self.background.close() {
            for runnable in level {
                std::mem::forget(runnable);
            }
        }
        let timers = {
            let mut state = self
                .timers
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.closed = true;
            std::mem::take(&mut state.pending)
        };
        self.timers.ready.notify_all();
        for timer in timers {
            std::mem::forget(timer.0.runnable);
        }
        for runnable in self
            .retirement
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            std::mem::forget(runnable);
        }
        for runnable in self
            .local_retirement
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            std::mem::forget(runnable);
        }
    }
}

// Constructor-only guard: no runnables have been admitted yet. A failed later
// spawn must close/wake already-created workers and timer infrastructure.
struct WorkersStartup(Option<(Arc<Queue>, Arc<Timers>)>);
impl Drop for WorkersStartup {
    fn drop(&mut self) {
        if let Some((background, timers)) = self.0.take() {
            drop(background.close());
            timers
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .closed = true;
            timers.ready.notify_all();
        }
    }
}

impl AndroidDispatcher {
    /// Must be called on the thread that will drain the [`MainQueue`]; `wake`
    /// interrupts that thread's wait for events.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> (Arc<Self>, MainQueue) {
        let workers = thread::available_parallelism().map_or(2, |n| n.get().max(2));
        Self::with_workers(wake, workers)
    }

    fn with_workers(
        wake: impl Fn() + Send + Sync + 'static,
        workers: usize,
    ) -> (Arc<Self>, MainQueue) {
        let main = Arc::new(Queue::default());
        let background = Arc::new(Queue::default());
        let timers = Arc::new(Timers::default());
        let mut startup = WorkersStartup(Some((background.clone(), timers.clone())));
        for i in 0..workers {
            let background = background.clone();
            thread::Builder::new()
                .name(format!("Worker-{i}"))
                .spawn(move || {
                    while let Some(runnable) = background.wait() {
                        run(runnable);
                    }
                })
                .expect("spawn a worker thread");
        }
        let timer_worker = timers.clone();
        thread::Builder::new()
            .name("Timer".into())
            .spawn(move || run_timers(timer_worker))
            .expect("spawn the timer thread");
        let dispatcher = Arc::new(Self {
            main: main.clone(),
            wake: Mutex::new(Some(Box::new(wake))),
            background,
            timers,
            retirement: Mutex::new(VecDeque::new()),
            local_retirement: Mutex::new(VecDeque::new()),
            stopped: AtomicBool::new(false),
            main_thread: thread::current().id(),
        });
        startup.0.take();
        let main_queue = MainQueue {
            queue: main,
            yielded_last_turn: false,
            dispatcher: dispatcher.clone(),
            _owner: std::marker::PhantomData,
        };
        (dispatcher, main_queue)
    }

    /// Terminal admission shutdown on the native/main owner. Never joins an
    /// arbitrary runnable or runs/cancels work while queue/timer locks are held.
    pub fn shutdown(&self) {
        assert!(
            self.is_main_thread(),
            "dispatcher shutdown must run on its owner"
        );
        if self.stopped.swap(true, Ordering::SeqCst) {
            self.finish_retirement();
            return;
        }
        let _trace = crate::trace::scope(c"pi.dispatcher.stop");
        let main = self.main.close();
        let background = self.background.close();
        let timers = {
            let mut state = self
                .timers
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.closed = true;
            std::mem::take(&mut state.pending)
        };
        self.timers.ready.notify_all();
        let wake = self
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // Publish all Send cancellations before retiring a local parent: its
        // Scope::drop can synchronously wait for these children. The scheduler
        // cooperatively drains this queue while such a scoped wait is parked.
        {
            let mut retirement = self
                .retirement
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for level in background {
                retirement.extend(level);
            }
            retirement.extend(timers.into_iter().map(|timer| timer.0.runnable));
        }
        // All admissions closed before any destructor/waker can reenter.
        drop(wake);
        {
            let mut retirement = self
                .local_retirement
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for level in main {
                retirement.extend(level);
            }
        }
        self.finish_retirement();
    }

    fn finish_retirement(&self) {
        // A wake can occur inside a channel/event-listener lock. Destruction
        // must wait until its scheduling callback has returned, even on owner.
        loop {
            let runnable = self
                .local_retirement
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            if let Some(runnable) = runnable {
                drop(runnable);
                continue;
            }
            if !self.poll_background_cleanup() {
                break;
            }
        }
    }
}

/// Runs each timer's runnable on this thread once it is due, as on Linux.
fn run_timers(timers: Arc<Timers>) {
    loop {
        let runnable = {
            let mut state = timers.state.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if state.closed {
                    return;
                }
                let now = Instant::now();
                if let Some(Reverse(timer)) = state.pending.peek() {
                    if timer.due <= now {
                        break state.pending.pop().expect("peeked timer").0.runnable;
                    }
                    let wait = timer.due - now;
                    state = timers
                        .ready
                        .wait_timeout(state, wait)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                } else {
                    state = timers
                        .ready
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
        };
        run(runnable);
    }
}

impl PlatformDispatcher for AndroidDispatcher {
    fn poll_background_cleanup(&self) -> bool {
        let runnable = self
            .retirement
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        if let Some(runnable) = runnable {
            drop(runnable);
            true
        } else {
            false
        }
    }

    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        // Background futures are Send; cancellation occurs outside the lock.
        if let Err(rejected) = self.background.push(priority, runnable) {
            // Scheduling may occur under a channel's notification lock. Never
            // synchronously cancel the waking future from that callback.
            self.retirement
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(rejected);
        }
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        if let Err(rejected) = self.main.push(priority, runnable) {
            if self.is_main_thread() {
                self.local_retirement
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_back(rejected);
            } else {
                // A local future cannot be destroyed on this posting thread,
                // and its owner has stopped. Preserve an inert terminal residue
                // rather than poll/drop it on another Activity/thread.
                std::mem::forget(rejected);
            }
            return;
        }
        let wake = self.wake.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(wake) = wake.as_ref() {
            wake();
        }
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        let rejected = {
            let mut state = self
                .timers
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if state.closed {
                Some(runnable)
            } else {
                state.sequence += 1;
                let seq = state.sequence;
                state.pending.push(Reverse(Timer {
                    due: Instant::now() + duration,
                    seq,
                    runnable,
                }));
                None
            }
        };
        self.timers.ready.notify_one();
        if let Some(rejected) = rejected {
            self.retirement
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(rejected);
        }
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        // Linearize realtime admission with the same terminal wake lock.
        let active = self.wake.lock().unwrap_or_else(PoisonError::into_inner);
        if active.is_some() {
            thread::spawn(f);
        } else {
            drop(active);
            // No execution owner remains; arbitrary closure destruction may
            // reenter its producer's notification lock. Inert terminal residue.
            std::mem::forget(f);
        }
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
    fn pending_frames_yield_before_a_task_without_cancelling_it() {
        let (dispatcher, mut main) = AndroidDispatcher::new(|| {});
        let foreground = ForegroundExecutor::new(dispatcher);
        let ran = Arc::new(AtomicUsize::new(0));
        for _ in 0..2 {
            let ran = ran.clone();
            foreground
                .spawn(async move {
                    ran.fetch_add(1, Ordering::SeqCst);
                })
                .detach();
        }
        assert!(main.run_until(Duration::from_secs(1), || true));
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        assert!(main.run_until(Duration::from_secs(1), || ran.load(Ordering::SeqCst) >= 1));
        assert_eq!(ran.load(Ordering::SeqCst), 1);
        assert!(!main.run(Duration::from_secs(1)));
        assert_eq!(ran.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn continuously_overdue_frames_do_not_starve_foreground_work() {
        let (dispatcher, mut main) = AndroidDispatcher::new(|| {});
        let foreground = ForegroundExecutor::new(dispatcher);
        let ran = Arc::new(AtomicUsize::new(0));
        for _ in 0..3 {
            let ran = ran.clone();
            foreground
                .spawn(async move {
                    ran.fetch_add(1, Ordering::SeqCst);
                })
                .detach();
        }
        assert!(main.run_until(Duration::from_secs(1), || true));
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        for expected in 1..=3 {
            assert_eq!(
                main.run_until(Duration::from_secs(1), || true),
                expected < 3
            );
            assert_eq!(ran.load(Ordering::SeqCst), expected);
        }
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
        let background = BackgroundExecutor::new(dispatcher);
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
    #[test]
    fn shutdown_retires_main_work_on_owner_and_releases_waker() {
        struct Dropped(Arc<AtomicUsize>, thread::ThreadId);
        impl Drop for Dropped {
            fn drop(&mut self) {
                assert_eq!(thread::current().id(), self.1);
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake_owner = Dropped(drops.clone(), thread::current().id());
        let (dispatcher, mut main) = AndroidDispatcher::new({
            let wakes = wakes.clone();
            move || {
                let _ = &wake_owner;
                wakes.fetch_add(1, Ordering::SeqCst);
            }
        });
        let foreground = ForegroundExecutor::new(dispatcher.clone());
        let body = Arc::new(AtomicUsize::new(0));
        let future_owner = Dropped(drops.clone(), thread::current().id());
        foreground
            .spawn({
                let body = body.clone();
                async move {
                    let _ = &future_owner;
                    body.fetch_add(1, Ordering::SeqCst);
                }
            })
            .detach();
        dispatcher.shutdown();
        dispatcher.shutdown();
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert!(!main.run(Duration::from_secs(1)));
        assert_eq!(body.load(Ordering::SeqCst), 0);
        let prior = wakes.load(Ordering::SeqCst);
        let late_owner = Dropped(drops.clone(), thread::current().id());
        foreground
            .spawn(async move {
                let _ = &late_owner;
            })
            .detach();
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        dispatcher.shutdown(); // Safe owner boundary after scheduling returned.
        assert_eq!(drops.load(Ordering::SeqCst), 3);
        assert_eq!(wakes.load(Ordering::SeqCst), prior);
        assert!(dispatcher.background.wait().is_none());
        assert!(dispatcher.timers.state.lock().unwrap().closed);
    }

    #[test]
    fn late_off_thread_foreground_wakeup_never_runs_or_drops_local_future_there() {
        let (dispatcher, mut main) = AndroidDispatcher::new(|| {});
        let foreground = ForegroundExecutor::new(dispatcher.clone());
        let signal = Arc::new((AtomicBool::new(false), Mutex::new(None::<std::task::Waker>)));
        let body = Arc::new(AtomicUsize::new(0));
        foreground
            .spawn({
                let body = body.clone();
                let signal = signal.clone();
                async move {
                    std::future::poll_fn(|cx| {
                        if signal.0.load(Ordering::SeqCst) {
                            return std::task::Poll::Ready(());
                        }
                        *signal.1.lock().unwrap() = Some(cx.waker().clone());
                        std::task::Poll::Pending
                    })
                    .await;
                    body.fetch_add(1, Ordering::SeqCst);
                }
            })
            .detach();
        assert!(!main.run(Duration::from_secs(1)));
        dispatcher.shutdown();
        // On Android also exercise the weak-scheduler lifetime gap (guarded in
        // GPUI's PlatformScheduler). Host dispatcher-only test retains executor.
        #[cfg(target_os = "android")]
        drop(foreground);
        thread::spawn(move || {
            signal.0.store(true, Ordering::SeqCst);
            let waker = signal.1.lock().unwrap().take();
            if let Some(waker) = waker {
                waker.wake();
            }
        })
        .join()
        .unwrap();
        assert!(!main.run(Duration::from_secs(1)));
        assert_eq!(body.load(Ordering::SeqCst), 0);
        // Inert terminal residue, not complete reclamation of !Send futures.
    }
    #[test]
    fn shutdown_progresses_scoped_children_before_local_parent_destruction() {
        let (done, received) = std::sync::mpsc::channel();
        thread::spawn(move || {
            use std::future::Future;
            let (dispatcher, mut main) = AndroidDispatcher::with_workers(|| {}, 0);
            let foreground = ForegroundExecutor::new(dispatcher.clone());
            let background = BackgroundExecutor::new(dispatcher.clone());
            let wake = Arc::new(Mutex::new(None::<std::task::Waker>));
            foreground
                .spawn({
                    let wake = wake.clone();
                    async move {
                        let borrowed = String::from("scoped borrowed data");
                        let scope = background.scoped(|scope| {
                            scope.spawn(async {
                                let _ = &borrowed;
                                std::future::pending::<()>().await;
                            });
                        });
                        let mut scope = std::pin::pin!(scope);
                        std::future::poll_fn(|cx| {
                            *wake.lock().unwrap() = Some(cx.waker().clone());
                            scope.as_mut().poll(cx)
                        })
                        .await;
                    }
                })
                .detach();
            assert!(!main.run(Duration::from_secs(1)));
            assert!(!dispatcher.background.tasks().is_empty());
            wake.lock().unwrap().take().unwrap().wake();
            dispatcher.shutdown();
            assert!(dispatcher.retirement.lock().unwrap().is_empty());
            done.send(()).unwrap();
        });
        received
            .recv_timeout(Duration::from_secs(5))
            .expect("scoped cancellation must make progress");
    }
    #[test]
    fn owner_queue_keeps_shutdown_available_after_every_executor_handle_drops() {
        let (dispatcher, main) = AndroidDispatcher::new(|| {});
        let background = Arc::downgrade(&dispatcher.background);
        let timers = Arc::downgrade(&dispatcher.timers);
        drop(dispatcher);
        drop(main);
        let deadline = Instant::now() + Duration::from_secs(2);
        while background.upgrade().is_some() || timers.upgrade().is_some() {
            assert!(
                Instant::now() < deadline,
                "retired idle infrastructure must exit"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn terminal_channel_notification_does_not_cancel_listener_inside_its_lock() {
        let (done, received) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let (dispatcher, mut main) = AndroidDispatcher::new(|| {});
            let foreground = ForegroundExecutor::new(dispatcher.clone());
            let (sender, receiver) = async_channel::unbounded::<()>();
            foreground
                .spawn(async move {
                    receiver.recv().await.ok();
                })
                .detach();
            main.run(Duration::from_secs(1));
            dispatcher.shutdown();
            // Exact event_listener path from the isolated ANR stack:
            // sender Drop -> notify under lock -> schedule terminal local task.
            drop(sender);
            dispatcher.shutdown();
            assert!(dispatcher.local_retirement.lock().unwrap().is_empty());
            done.send(()).unwrap();
        });
        received
            .recv_timeout(Duration::from_secs(5))
            .expect("notification must return before cancellation");
    }
    #[test]
    fn last_arc_after_successful_weak_upgrade_never_cancels_under_notification_lock() {
        struct Gate {
            inner: Arc<AndroidDispatcher>,
            paused: Arc<AtomicBool>,
            entered: Arc<std::sync::Barrier>,
            release: Arc<std::sync::Barrier>,
        }
        impl PlatformDispatcher for Gate {
            fn is_main_thread(&self) -> bool {
                self.inner.is_main_thread()
            }
            fn dispatch(&self, r: RunnableVariant, p: Priority) {
                if self.paused.load(Ordering::SeqCst) {
                    self.entered.wait();
                    self.release.wait();
                }
                self.inner.dispatch(r, p);
            }
            fn dispatch_on_main_thread(&self, r: RunnableVariant, p: Priority) {
                self.inner.dispatch_on_main_thread(r, p)
            }
            fn dispatch_after(&self, d: Duration, r: RunnableVariant) {
                self.inner.dispatch_after(d, r)
            }
            fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
                self.inner.spawn_realtime(f)
            }
        }
        let (done, received) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let (dispatcher, main) = AndroidDispatcher::with_workers(|| {}, 0);
            let paused = Arc::new(AtomicBool::new(false));
            let entered = Arc::new(std::sync::Barrier::new(2));
            let release = Arc::new(std::sync::Barrier::new(2));
            let executor = BackgroundExecutor::new(Arc::new(Gate {
                inner: dispatcher.clone(),
                paused: paused.clone(),
                entered: entered.clone(),
                release: release.clone(),
            }));
            let (sender, receiver) = async_channel::unbounded::<()>();
            executor
                .spawn(async move {
                    receiver.recv().await.ok();
                })
                .detach();
            let runnable = dispatcher.background.tasks().pop().unwrap();
            runnable.run(); // Register the real event_listener under its channel.
            paused.store(true, Ordering::SeqCst);
            let notifying = thread::spawn(move || drop(sender));
            entered.wait(); // Scheduler weak upgrade succeeded under notify lock.
            drop(executor);
            drop(dispatcher);
            drop(main); // Owner retirement completes before rejected enqueue.
            release.wait();
            notifying.join().unwrap(); // Final dispatcher Arc drops in notify.
            done.send(()).unwrap();
        });
        received
            .recv_timeout(Duration::from_secs(5))
            .expect("last Arc release must not recursively cancel listener");
    }
}
