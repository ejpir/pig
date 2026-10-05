//! The activity's lifecycle, reduced to what GPUI reports.
//!
//! Android owns the lifecycle: the activity is started, resumed, paused and
//! stopped at the system's discretion, and its native window comes and goes
//! independently (rotation, backgrounding). GPUI hears about three things: the
//! app phase, whether the window takes input, and whether it is presented.

use gpui::{AppLifecyclePhase, WindowVisibility};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActivityEvent {
    Start,
    Resume,
    Pause,
    Stop,
    GainedFocus,
    LostFocus,
    WindowCreated,
    WindowDestroyed,
}

/// What changed after an event, so each callback fires only on a transition.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Transition {
    pub phase: Option<AppLifecyclePhase>,
    pub active: Option<bool>,
    pub visibility: Option<WindowVisibility>,
}

#[derive(Default)]
pub(crate) struct Lifecycle {
    started: bool,
    resumed: bool,
    focused: bool,
    window: bool,
    phase: Option<AppLifecyclePhase>,
}

impl Lifecycle {
    pub fn apply(&mut self, event: ActivityEvent) -> Transition {
        let active = self.is_active();
        let visibility = self.visibility();
        let phase = match event {
            ActivityEvent::Start => {
                self.started = true;
                Some(AppLifecyclePhase::Foreground)
            }
            ActivityEvent::Resume => {
                self.resumed = true;
                Some(AppLifecyclePhase::Active)
            }
            ActivityEvent::Pause => {
                self.resumed = false;
                Some(AppLifecyclePhase::Inactive)
            }
            ActivityEvent::Stop => {
                self.started = false;
                Some(AppLifecyclePhase::Background)
            }
            ActivityEvent::GainedFocus => {
                self.focused = true;
                None
            }
            ActivityEvent::LostFocus => {
                self.focused = false;
                None
            }
            ActivityEvent::WindowCreated => {
                self.window = true;
                None
            }
            ActivityEvent::WindowDestroyed => {
                self.window = false;
                None
            }
        };
        let phase = phase.filter(|phase| self.phase.replace(*phase) != Some(*phase));
        Transition {
            phase,
            active: Some(self.is_active()).filter(|now| *now != active),
            visibility: Some(self.visibility()).filter(|now| *now != visibility),
        }
    }

    /// Resumed with window focus: a system dialog or the notification shade
    /// on top takes focus while the activity stays resumed.
    pub fn is_active(&self) -> bool {
        self.resumed && self.focused
    }

    /// Frames are shown only while the activity is started and has a surface.
    pub fn visibility(&self) -> WindowVisibility {
        if self.started && self.window {
            WindowVisibility::Visible
        } else {
            WindowVisibility::Hidden
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ActivityEvent::*;

    fn run(lifecycle: &mut Lifecycle, events: &[ActivityEvent]) -> Vec<Transition> {
        events.iter().map(|e| lifecycle.apply(*e)).collect()
    }

    #[test]
    fn launch_reports_each_phase_then_input_and_presentation() {
        let mut lifecycle = Lifecycle::default();
        let steps = run(&mut lifecycle, &[Start, Resume, WindowCreated, GainedFocus]);
        let phases: Vec<_> = steps.iter().map(|t| t.phase).collect();
        assert_eq!(
            phases,
            [
                Some(AppLifecyclePhase::Foreground),
                Some(AppLifecyclePhase::Active),
                None,
                None
            ]
        );
        assert_eq!(steps[2].visibility, Some(WindowVisibility::Visible));
        assert_eq!(steps[3].active, Some(true));
        assert!(lifecycle.is_active());
    }

    #[test]
    fn backgrounding_hides_the_window_and_returning_restores_it() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[Start, Resume, WindowCreated, GainedFocus]);
        let away = run(&mut lifecycle, &[Pause, LostFocus, WindowDestroyed, Stop]);
        assert_eq!(away[0].active, Some(false), "pausing stops input");
        assert_eq!(away[1].active, None, "already inactive");
        assert_eq!(away[2].visibility, Some(WindowVisibility::Hidden));
        assert_eq!(away[3].phase, Some(AppLifecyclePhase::Background));

        let back = run(&mut lifecycle, &[Start, Resume, WindowCreated, GainedFocus]);
        assert_eq!(back[0].phase, Some(AppLifecyclePhase::Foreground));
        assert_eq!(back[2].visibility, Some(WindowVisibility::Visible));
        assert_eq!(back[3].active, Some(true));
    }

    #[test]
    fn a_window_without_a_started_activity_is_not_presented() {
        let mut lifecycle = Lifecycle::default();
        let created = lifecycle.apply(WindowCreated);
        assert_eq!(created.visibility, None);
        assert_eq!(lifecycle.visibility(), WindowVisibility::Hidden);
        assert_eq!(
            lifecycle.apply(Start).visibility,
            Some(WindowVisibility::Visible)
        );
    }

    #[test]
    fn a_dialog_on_top_takes_input_but_keeps_the_phase() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[Start, Resume, WindowCreated, GainedFocus]);
        let shade = lifecycle.apply(LostFocus);
        assert_eq!(
            shade,
            Transition {
                phase: None,
                active: Some(false),
                visibility: None
            }
        );
    }
}
