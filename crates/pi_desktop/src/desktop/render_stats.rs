//! Opt-in rendering diagnostics, the desktop twin of Android's overlay: frame
//! pacing, whole-window layout/paint CPU (the shared Visualize profile's root
//! wrapper), how often the transcript rebuilds, and the process's CPU use.
//! Nothing is measured, counted or scheduled while it is hidden.
use gpui::{SharedString, Window, div, prelude::*, px};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static TRANSCRIPT_RENDERS: AtomicU64 = AtomicU64::new(0);
static ROWS_BUILT: AtomicU64 = AtomicU64::new(0);

/// A transcript view re-rendered: it rebuilds every visible row.
pub(super) fn transcript_render() {
    if ENABLED.load(Ordering::Relaxed) {
        TRANSCRIPT_RENDERS.fetch_add(1, Ordering::Relaxed);
    }
}
pub(super) fn row_built() {
    if ENABLED.load(Ordering::Relaxed) {
        ROWS_BUILT.fetch_add(1, Ordering::Relaxed);
    }
}

const LIMIT: usize = 120;
/// Gaps longer than this are idle time, not slow frames.
const IDLE: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct RenderStats {
    pub visible: bool,
    last_frame: Option<Instant>,
    intervals: VecDeque<Duration>,
    /// Frames drawn since the last one-second sample, idle or not.
    draws: u64,
    second: Option<Second>,
    last_sample: Option<(Instant, Duration)>,
    last_label: Option<Instant>,
    cached_label: SharedString,
}

/// Rates over the last whole second.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Second {
    cpu: Option<f64>,
    draws: u64,
    transcript: u64,
    rows: u64,
}

impl RenderStats {
    pub fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        *self = Self {
            visible,
            ..Default::default()
        };
        ENABLED.store(visible, Ordering::Relaxed);
        TRANSCRIPT_RENDERS.store(0, Ordering::Relaxed);
        ROWS_BUILT.store(0, Ordering::Relaxed);
        if visible {
            self.last_sample = cpu_time().map(|cpu| (Instant::now(), cpu));
        }
    }

    pub fn observe(&mut self, now: Instant) {
        self.draws += 1;
        let Some(last) = self.last_frame.replace(now) else {
            return;
        };
        let interval = now.duration_since(last);
        if interval > IDLE {
            self.intervals.clear();
            return;
        }
        if self.intervals.len() == LIMIT {
            self.intervals.pop_front();
        }
        self.intervals.push_back(interval);
    }

    /// Called once a second while visible, whether or not frames are drawn.
    pub fn sample(&mut self, now: Instant) {
        let cpu = cpu_time().and_then(|cpu| {
            let (then, before) = self.last_sample.replace((now, cpu))?;
            let wall = now.duration_since(then).as_secs_f64();
            (wall > 0.).then(|| cpu.saturating_sub(before).as_secs_f64() / wall * 100.)
        });
        self.second = Some(Second {
            cpu,
            draws: std::mem::take(&mut self.draws),
            transcript: TRANSCRIPT_RENDERS.swap(0, Ordering::Relaxed),
            rows: ROWS_BUILT.swap(0, Ordering::Relaxed),
        });
        self.last_label = None;
    }

    fn label(&mut self, now: Instant) -> SharedString {
        // 10 Hz text is readable without formatting on every measured frame.
        if self
            .last_label
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
        {
            return self.cached_label.clone();
        }
        self.last_label = Some(now);
        let seconds: f64 = self.intervals.iter().map(Duration::as_secs_f64).sum();
        let fps = if seconds > 0. {
            self.intervals.len() as f64 / seconds
        } else {
            0.
        };
        let ms = |d: Option<&Duration>| d.map_or(0., |d| d.as_secs_f64() * 1000.);
        let mut sorted: Vec<_> = self.intervals.iter().copied().collect();
        sorted.sort_unstable();
        let p95 = ms(sorted.get(sorted.len().saturating_sub(1) * 95 / 100));
        // Count frames that clearly missed a 60 Hz refresh.
        let late = sorted
            .iter()
            .filter(|gap| **gap > Duration::from_micros(20_833))
            .count();
        let second = self.second.map_or_else(
            || "cpu … · draws …/s\ntranscript …/s · rows …/s".to_owned(),
            |s| {
                format!(
                    "cpu {} · draws {}/s\ntranscript {}/s · rows {}/s",
                    s.cpu.map_or("?".to_owned(), |cpu| format!("{cpu:.0}%")),
                    s.draws,
                    s.transcript,
                    s.rows,
                )
            },
        );
        self.cached_label = format!(
            "active {fps:.0} FPS · gap {:.1} ms\ngap95 {p95:.1} · late {late}/{}\n{second}",
            ms(self.intervals.back()),
            self.intervals.len(),
        )
        .into();
        self.cached_label.clone()
    }

    pub fn render(&mut self, cx: &gpui::App) -> impl IntoElement + use<> {
        let label: SharedString = format!(
            "{}{}",
            self.label(Instant::now()),
            pi_visual_ui::visual_profile::label(cx)
        )
        .into();
        div()
            .id("rendering-stats")
            .debug_selector(|| "rendering-stats".into())
            .absolute()
            .bottom(px(32.))
            .right(px(16.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(8.))
            .bg(gpui::rgba(0x05070ddd))
            .text_color(gpui::rgba(0xe8edf2ff))
            .text_size(px(10.))
            .line_height(px(14.))
            .font_family(crate::theme::MONO)
            .child(label)
    }
}

/// Keeps the per-second figures current while the window is idle: one cheap
/// redraw a second, counted in `draws`.
pub(super) fn sampler(window: &Window, cx: &mut gpui::Context<super::Desktop>) {
    cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let running = this.update(cx, |this, cx| {
                if this.render_stats.visible {
                    this.render_stats.sample(Instant::now());
                    cx.notify();
                }
                this.render_stats.visible
            });
            if !matches!(running, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

/// `PI_DESKTOP_SCROLL_BENCH=<px>` scrolls every transcript that many pixels a
/// frame, bouncing at either end, so scrolling can be measured without a hand
/// on the trackpad.
pub(super) fn scroll_bench(
    list: &gpui::ListState,
    window: &mut Window,
    cx: &mut gpui::Context<super::transcript::TranscriptView>,
) {
    static STEP: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    let Some(step) = *STEP.get_or_init(|| {
        std::env::var("PI_DESKTOP_SCROLL_BENCH")
            .ok()
            .and_then(|v| v.parse().ok())
    }) else {
        return;
    };
    thread_local!(static DOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(true) });
    let list = list.clone();
    let view = cx.entity().downgrade();
    window.on_next_frame(move |_, cx| {
        let before = list.scroll_px_offset_for_scrollbar();
        let down = DOWN.get();
        list.scroll_by(px(if down { step } else { -step }));
        if list.scroll_px_offset_for_scrollbar() == before {
            DOWN.set(!down);
        }
        let _ = view.update(cx, |_, cx| cx.notify());
    });
}

/// User plus system CPU time of the whole process, every thread.
#[cfg(unix)]
fn cpu_time() -> Option<Duration> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage fills the struct it is given and reports failure.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: initialised by the successful call above.
    let usage = unsafe { usage.assume_init() };
    let time = |t: libc::timeval| {
        Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64)
    };
    Some(time(usage.ru_utime) + time(usage.ru_stime))
}
#[cfg(not(unix))]
fn cpu_time() -> Option<Duration> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_gaps_reset_the_window_and_draws_still_count() {
        let mut stats = RenderStats::default();
        let start = Instant::now();
        stats.observe(start);
        stats.observe(start + Duration::from_millis(16));
        stats.observe(start + Duration::from_millis(32));
        assert_eq!(stats.intervals.len(), 2);
        stats.observe(start + Duration::from_secs(2));
        assert!(stats.intervals.is_empty());
        assert_eq!(stats.draws, 4);
    }

    #[test]
    fn label_reports_rate_late_frames_and_the_last_second() {
        let mut stats = RenderStats::default();
        let start = Instant::now();
        for i in 0..5 {
            stats.observe(start + Duration::from_millis(i * 16));
        }
        stats.observe(start + Duration::from_millis(64 + 40));
        stats.sample(start + Duration::from_secs(1));
        let label = stats.label(start + Duration::from_secs(1));
        assert!(label.contains("late 1/5"), "{label}");
        assert!(label.contains("draws 6/s"), "{label}");
    }

    #[test]
    fn hidden_stats_count_nothing() {
        let mut stats = RenderStats::default();
        stats.set_visible(false);
        transcript_render();
        row_built();
        assert_eq!(TRANSCRIPT_RENDERS.load(Ordering::Relaxed), 0);
        assert_eq!(ROWS_BUILT.load(Ordering::Relaxed), 0);
    }
}
