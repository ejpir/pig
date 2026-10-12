//! Native schedule destinations and review/confirmation flows, not web views.
use super::scroll_area;
use crate::{
    app::{PhoneApp, Route, Sheet},
    schedules::{Confirmation, status},
    text_area::TextArea,
    theme::{Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{AnyElement, Context, Div, Entity, FontWeight, Stateful, Window, div, prelude::*, px};
use pi_core::schedules::{MissedPolicy, Request, RunStatus};

fn action(
    id: &'static str,
    text: &str,
    primary: bool,
    enabled: bool,
    colors: &Theme,
) -> Stateful<Div> {
    ui::button(
        id,
        if primary {
            Button::Primary
        } else {
            Button::Plain
        },
        None,
        text.to_owned(),
        false,
        colors,
    )
    .debug_selector(move || id.into())
    .when(!enabled, |button| ui::disabled(button, colors))
}
fn field(id: &'static str, title: &str, area: &Entity<TextArea>, colors: &Theme) -> Div {
    div()
        .mb(px(14.))
        .child(ui::label(title.to_owned(), colors).mb(px(6.)))
        .child(
            ui::card(colors)
                .id(id)
                .relative()
                .child(crate::testing::probe(id))
                .debug_selector(move || id.into())
                .p(px(12.))
                .min_h(px(48.))
                .child(area.clone()),
        )
}
fn note(title: &str, text: impl Into<gpui::SharedString>, colors: &Theme) -> Div {
    ui::card(colors)
        .p(px(16.))
        .mb(px(14.))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_owned()),
        )
        .child(ui::hint(text, colors).mt(px(8.)))
}
fn quoted_helper_path(path: &str) -> String {
    if let Some(relative) = path.strip_prefix("~/") {
        format!("\"$HOME\"/'{}'", relative.replace('\'', "'\\''"))
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

pub(crate) fn date(ms: u64, zone: &str) -> String {
    let Some(utc) = chrono::DateTime::from_timestamp_millis(ms.min(i64::MAX as u64) as i64) else {
        return "Unknown time".into();
    };
    match zone.parse::<chrono_tz::Tz>() {
        Ok(zone) => utc
            .with_timezone(&zone)
            .format("%a %d %b · %H:%M %Z")
            .to_string(),
        Err(_) => utc.format("%d %b %Y · %H:%M UTC").to_string(),
    }
}

impl PhoneApp {
    pub(crate) fn schedules_screen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.route() {
            Route::ScheduleEdit => self.schedule_edit_screen(cx),
            Route::ScheduleConfirm => self.schedule_confirmation_screen(cx),
            Route::ScheduleDetail => self.schedule_detail_screen(cx),
            Route::ScheduleHistory => self.schedule_history_screen(cx),
            _ => self.schedule_list_screen(window, cx),
        }
    }
    fn schedule_top(&self, title: &str, colors: &Theme, cx: &Context<Self>) -> Div {
        ui::appbar(
            ui::tap("schedule-back", "back", colors)
                .debug_selector(|| "schedule-back".into())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.back(window, cx);
                })),
            title.to_owned(),
            Some(self.computer_name().into()),
            colors,
        )
        .child(
            ui::tap("schedule-refresh", "refresh", colors)
                .debug_selector(|| "schedule-refresh".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    let request = if this.route() == Route::ScheduleHistory {
                        this.schedules
                            .selected
                            .clone()
                            .map(|job_id| Request::History { job_id })
                            .unwrap_or(Request::List)
                    } else {
                        Request::List
                    };
                    this.schedule_request(request, cx);
                })),
        )
    }
    fn schedule_status_card(&self, colors: &Theme) -> Option<AnyElement> {
        let cache = self.schedule_cache();
        if !self.schedule_online() {
            Some(note("Computer unreachable · cached / unknown", format!("Last checked: {}. Unreachable is not paused. The host may still be running. Reconnect before any mutation; there is no phone retry queue.", cache.fetched_ms.map(|time| date(time, "UTC")).unwrap_or_else(|| "never".into())), colors)
                .id("schedule-offline").debug_selector(|| "schedule-offline".into()).into_any_element())
        } else if !self.schedule_supported() {
            let installed = self
                .store
                .as_ref()
                .and_then(|store| store.live.as_ref())
                .and_then(|live| live.helper.release.as_deref())
                .unwrap_or("unknown");
            Some(note("Helper update required", format!("pi-desktop-remote {installed} does not advertise schedules. Durable support alone is not enough. Update the helper on {} from https://github.com/ejpir/pig/releases/tag/v{} then reconnect. Existing sessions and drafts are kept.", self.computer_name(), crate::remote::APP_RELEASE), colors)
                .id("schedule-unsupported").debug_selector(|| "schedule-unsupported".into()).into_any_element())
        } else if let Some(health) = &cache.health
            && !health.running
        {
            let path = self
                .store
                .as_ref()
                .and_then(|store| store.live.as_ref())
                .map(|live| live.helper.path.as_str())
                .unwrap_or("~/.pi/desktop/bin/pi-desktop-remote");
            let quoted = quoted_helper_path(path);
            let command = if health.autostart_installed {
                format!("{quoted} scheduler start")
            } else {
                format!(
                    "{quoted} scheduler start\nOptional login autostart (does not start now):\n{quoted} scheduler install --confirm"
                )
            };
            Some(note(if health.autostart_installed { "Scheduler not running" } else { "Host setup required" },
                format!("{}\nProposed local command on {} (informational only):\n{}\nThe phone never installs or starts this service. User services normally start at login; the host must be awake. Linux pre-login operation requires explicit lingering setup.", health.detail, self.computer_name(), command), colors)
                .id("schedule-setup").debug_selector(|| "schedule-setup".into()).into_any_element())
        } else if !cache.trusted {
            Some(
                note(
                    "Schedule state unknown",
                    if self.schedules.busy {
                        "Reading from the computer…"
                    } else {
                        "Refresh from the computer before editing or submitting."
                    },
                    colors,
                )
                .into_any_element(),
            )
        } else {
            None
        }
    }
    fn schedule_sample_note(&self, colors: &Theme) -> Option<Div> {
        self.store.as_ref().is_some_and(|store| store.is_sample()).then(|| note("Sample schedules · no real effects", "Preview/test fixtures only. Creation, pause and history stay in memory. No SSH, provider calls or real execution; sample dates are illustrative.", colors))
    }
    fn schedule_list_screen(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Schedules);
        let cache = self.schedule_cache();
        let known = self.schedule_online() && self.schedule_supported() && cache.trusted;
        let can_mutate = self.schedule_can_mutate();
        let rows = cache
            .jobs
            .iter()
            .enumerate()
            .map(|(index, job)| {
                let job_id = job.id.clone();
                let state = if !known {
                    format!(
                        "Unknown · cached {}",
                        if job.spec.enabled {
                            "enabled"
                        } else {
                            "paused"
                        }
                    )
                } else if job.spec.enabled {
                    "Enabled".into()
                } else {
                    "Paused".into()
                };
                let next = if !known {
                    "Next occurrence unknown".into()
                } else if !job.spec.enabled {
                    "No future occurrences while paused".into()
                } else {
                    job.next_due_ms
                        .map(|ms| format!("Next {}", date(ms, &job.spec.timezone)))
                        .unwrap_or_else(|| "Next occurrence not reported".into())
                };
                ui::card(&colors)
                    .id(("schedule-row", index))
                    .relative()
                    .child(crate::testing::probe(format!("schedule-row-{index}")))
                    .debug_selector(move || format!("schedule-row-{index}"))
                    .p(px(16.))
                    .mb(px(12.))
                    .min_h(px(88.))
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .items_center()
                            .child(icon("clock", 20., colors.accent))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(job.spec.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(if known && job.spec.enabled {
                                        colors.green
                                    } else {
                                        colors.muted
                                    })
                                    .child(state),
                            ),
                    )
                    .child(ui::hint(next, &colors).mt(px(8.)))
                    .child(
                        ui::hint(
                            format!(
                                "{} · {} · {}",
                                job.spec.cwd, job.spec.cron, job.spec.timezone
                            ),
                            &colors,
                        )
                        .mt(px(4.)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.schedules.selected = Some(job_id.clone());
                        this.push(Route::ScheduleDetail, window, cx);
                    }))
            })
            .collect::<Vec<_>>();
        let content = div().px(px(20.)).py(px(12.))
            .children(self.schedule_sample_note(&colors)).children(self.schedule_status_card(&colors))
            .children(cache.error.as_ref().map(|error| note("Schedule request", error.clone(), &colors)))
            .child(self.schedule_uncertainty(&colors, cx))
            .when(cache.jobs.is_empty(), |body| body.child(note(if known { "No schedules" } else { "No cached schedules" }, if known { "Create a recurring instruction on this computer. Each occurrence is a fresh durable session." } else { "Schedule definitions on the computer are unknown. Reconnect and refresh; this does not mean there are no schedules." }, &colors)))
            .children(rows)
            .child(ui::hint("Runs on the computer, even with the phone disconnected. Fresh sessions; no same-job overlap. Different jobs share the host workspace unless you arrange isolation.", &colors));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.schedule_top("Schedules", &colors, cx))
            .child(scroll_area("schedules-list", &scroll).child(content))
            .child(
                div().p(px(12.)).child(
                    action("schedule-new", "New schedule", true, can_mutate, &colors)
                        .w_full()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.begin_schedule_draft(None, window, cx)
                        })),
                ),
            )
            .into_any_element()
    }
    fn schedule_edit_screen(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::ScheduleEdit);
        let Some(draft) = &self.schedules.draft else {
            return div().child("No schedule draft").into_any_element();
        };
        let valid_preview = draft.preview_matches(cx);
        let can_mutate = self.schedule_can_mutate();
        let editing = draft.original.is_some();
        let revision_changed = draft.original.as_ref().is_some_and(|old| {
            self.schedule_cache()
                .jobs
                .iter()
                .any(|job| job.id == old.id && job.revision != old.revision)
        });
        let latest_note = draft
            .original
            .as_ref()
            .and_then(|old| {
                self.schedule_cache()
                    .jobs
                    .into_iter()
                    .find(|job| job.id == old.id && job.revision != old.revision)
            })
            .map(|job| {
                note(
                    "Latest host definition · inspect before rebasing",
                    format!(
                        "Revision {} · {}\n{}\n{} / {}\n{} · {}\n{}",
                        job.revision,
                        if job.spec.enabled {
                            "enabled"
                        } else {
                            "paused"
                        },
                        job.spec.cwd,
                        job.spec.provider,
                        job.spec.model_id,
                        job.spec.cron,
                        job.spec.timezone,
                        job.spec.prompt
                    ),
                    &colors,
                )
            });
        let body = div().px(px(20.)).py(px(12.))
            .children(self.schedule_status_card(&colors))
            .child(field("schedule-name", "Name", &draft.name, &colors))
            .child(field("schedule-project", "Project on the computer", &draft.cwd, &colors))
            .child(action("schedule-project-picker", "Browse projects", false, can_mutate, &colors).mb(px(14.)).on_click(cx.listener(|this, _, _, cx| {
                if this.schedule_can_mutate() { this.open_sheet(Sheet::Project, cx); }
            })))
            .child(ui::label("Model", &colors).mb(px(6.)))
            .child(ui::card(&colors).id("schedule-model").relative().child(crate::testing::probe("schedule-model"))
                .debug_selector(|| "schedule-model".into()).min_h(px(48.)).p(px(12.)).mb(px(14.)).flex().items_center().gap(px(8.))
                .when(!can_mutate, |card| card.opacity(0.6))
                .child(div().flex_1().min_w_0().truncate().child(if draft.model_id.is_empty() { "Choose model".into() } else { format!("{} / {}", draft.provider, draft.model_id) }))
                .child(icon("chev_d", 18., colors.muted))
                .on_click(cx.listener(|this, _, _, cx| { if this.schedule_can_mutate() { this.open_sheet(Sheet::Model, cx); } })))
            .child(field("schedule-prompt", "Recurring instruction", &draft.prompt, &colors))
            .child(ui::label("When · numeric five-field cron", &colors).mb(px(6.)))
            .child(div().flex().gap(px(6.)).mb(px(12.))
                .child(action("schedule-daily", "Daily", false, can_mutate, &colors).flex_1().on_click(cx.listener(|this, _, _, cx| {
                    if this.schedule_can_mutate() && let Some(draft) = &mut this.schedules.draft { draft.cron.update(cx, |area, cx| area.set_text("0 9 * * *", cx)); draft.preview = None; cx.notify(); }
                })))
                .child(action("schedule-weekdays", "Weekdays", false, can_mutate, &colors).flex_1().on_click(cx.listener(|this, _, _, cx| {
                    if this.schedule_can_mutate() && let Some(draft) = &mut this.schedules.draft { draft.cron.update(cx, |area, cx| area.set_text("0 9 * * 1-5", cx)); draft.preview = None; cx.notify(); }
                }))))
            .child(field("schedule-cron", "Minute hour day month weekday", &draft.cron, &colors))
            .child(field("schedule-timezone", "Schedule timezone · IANA, not phone timezone", &draft.timezone, &colors))
            .child(field("schedule-timeout", "Timeout (minutes) · requests Stop, not a hard boundary", &draft.timeout, &colors))
            .child(action("schedule-missed-policy", if draft.missed_policy == MissedPolicy::Skip { "Missed times: skip" } else { "Missed times: catch up once" }, false, can_mutate, &colors)
                .max_w_full().mb(px(14.)).on_click(cx.listener(|this, _, _, cx| {
                    if this.schedule_can_mutate() && let Some(draft) = &mut this.schedules.draft { draft.missed_policy = if draft.missed_policy == MissedPolicy::Skip { MissedPolicy::Once } else { MissedPolicy::Skip }; }
                    cx.notify();
                })))
            .children(draft.error.as_ref().map(|error| note("Draft kept", error.clone(), &colors)))
            .children(latest_note)
            .when(revision_changed, |body| body.child(action("schedule-rebase", "Use latest revision; keep draft", false, can_mutate, &colors).w_full().mb(px(14.))
                .on_click(cx.listener(|this, _, _, cx| this.rebase_schedule_draft(cx)))))
            .child(action("schedule-preview", if self.schedules.busy { "Loading preview…" } else { "Preview next occurrences" }, false, can_mutate, &colors).w_full()
                .on_click(cx.listener(|this, _, _, cx| this.preview_schedule(cx))))
            .when(valid_preview, |body| body.child(note("Next occurrences · from host", draft.preview.as_ref().unwrap().2.iter().map(|ms| date(*ms, draft.timezone.read(cx).text())).collect::<Vec<_>>().join("\n"), &colors).mt(px(12.))))
            .child(ui::hint("DST: nonexistent local times are skipped; repeated times fire once at the first instant. Previews are calculated by the computer. Same-job occurrences never overlap.", &colors).my(px(12.)));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.schedule_top(
                if editing {
                    "Edit schedule"
                } else {
                    "New schedule"
                },
                &colors,
                cx,
            ))
            .child(scroll_area("schedule-edit-scroll", &scroll).child(body))
            .child(
                div().p(px(12.)).child(
                    action(
                        "schedule-review",
                        "Review schedule",
                        true,
                        can_mutate && valid_preview,
                        &colors,
                    )
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.review_schedule(window, cx))),
                ),
            )
            .into_any_element()
    }
    fn schedule_confirmation_screen(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::ScheduleConfirm);
        let Some(confirm) = self.schedules.confirmation.clone() else {
            return div()
                .child(self.schedule_top("Review", &colors, cx))
                .child("Confirmation expired. Return to the schedule; your draft is kept.")
                .into_any_element();
        };
        let (title, description, consent, submit) = match &confirm {
            Confirmation::Save { request, .. } => {
                let spec = match request {
                    Request::Create { spec, .. } | Request::Update { spec, .. } => spec,
                    _ => unreachable!(),
                };
                (
                    "Review schedule",
                    format!(
                        "{}\nProject: {}\nModel: {}/{}\n{} · {}\n{}\nTimeout: {} minutes · Missed: {}\n{}",
                        spec.name,
                        spec.cwd,
                        spec.provider,
                        spec.model_id,
                        spec.cron,
                        spec.timezone,
                        spec.prompt,
                        spec.timeout_minutes,
                        if spec.missed_policy == MissedPolicy::Skip {
                            "skip"
                        } else {
                            "run once when back"
                        },
                        if spec.enabled {
                            "Enabled for future occurrences"
                        } else {
                            "Stays paused; edits affect future occurrences only"
                        }
                    ),
                    "I trust this project and consent to unattended Pi runs on this computer.",
                    if spec.enabled {
                        "Enable schedule"
                    } else {
                        "Save paused"
                    },
                )
            }
            Confirmation::Run { job, request } => (
                "Run now?",
                format!(
                    "Start a fresh occurrence of {} on {} in {} using {}/{}.\nIt can run while paused. The next recurring time is unchanged. This confirms admission, not completion.\nManual request key: {}",
                    job.spec.name,
                    self.computer_name(),
                    job.spec.cwd,
                    job.spec.provider,
                    job.spec.model_id,
                    match request {
                        Request::RunNow { request_id, .. } => request_id,
                        _ => "",
                    }
                ),
                "I confirm this one fresh occurrence, with the host account's permissions.",
                "Run now",
            ),
            Confirmation::Delete { job } => (
                "Delete schedule?",
                format!(
                    "Remove future scheduling for {}. Existing sessions, history and project files are kept. Already admitted work is not stopped.",
                    job.spec.name
                ),
                "Delete only this schedule; keep sessions and project files.",
                "Delete schedule",
            ),
        };
        let recovering = match &confirm {
            Confirmation::Run { request, .. } => {
                self.schedule_cache().uncertain.as_ref() == Some(request)
            }
            _ => false,
        };
        let enabled = self.schedule_can_mutate()
            || matches!(&confirm, Confirmation::Run { request, .. } if self.schedule_can_recover(request));
        let body = div().px(px(20.)).py(px(12.))
            .children(self.schedule_sample_note(&colors)).children(self.schedule_status_card(&colors))
            .when(recovering, |body| body.child(note("Same manual operation · no new key", "This may recover the prior admission, or admit the originally confirmed operation once if absent. The exact original key and revision are retained. Inspect linked sessions and tool effects before continuing; deduplicated admission is not exactly-once external effects.", &colors)))
            .child(note(&format!("On {}", self.computer_name()), description, &colors))
            .children(match &confirm { Confirmation::Save { times, request } => {
                let zone = match request { Request::Create { spec, .. } | Request::Update { spec, .. } => spec.timezone.as_str(), _ => "UTC" };
                Some(note("Next occurrences", times.iter().map(|ms| date(*ms, zone)).collect::<Vec<_>>().join("\n"), &colors))
            }, _ => None })
            .child(note("Execution contract", "Every occurrence starts a fresh durable session. No same-job overlap; this is not isolation between different jobs. Pi runs with the host account's capabilities, not in a sandbox. Instructions such as ‘don't edit files’ are not enforced read-only access. Timeout requests Stop, not a hard execution/cost/security boundary. Accepted is not completed; completed is not verified task success or exactly-once external effects.", &colors))
            .children(self.schedule_cache().error.map(|error| note("Draft kept · inspect before resubmitting", error, &colors)))
            .child(
                div()
                    .id("schedule-consent")
                    .relative()
                    .child(crate::testing::probe("schedule-consent"))
                    .debug_selector(|| "schedule-consent".into())
                    .w_full()
                    .min_w_0()
                    .p(px(16.))
                    .min_h(px(64.))
                    .rounded(px(16.))
                    .bg(colors.panel)
                    .flex()
                    .gap(px(12.))
                    .items_center()
                    .child(icon(
                        if self.schedules.consent { "check" } else { "shield" },
                        24.,
                        colors.accent,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .line_height(px(22.))
                            .relative()
                            .debug_selector(|| "schedule-consent-label".into())
                            .child(crate::testing::probe("schedule-consent-label"))
                            .child(consent),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.schedules.busy {
                            this.schedules.consent = !this.schedules.consent;
                            cx.notify();
                        }
                    })),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.schedule_top(
                if recovering {
                    "Recover manual run?"
                } else {
                    title
                },
                &colors,
                cx,
            ))
            .child(scroll_area("schedule-confirm-scroll", &scroll).child(body))
            .child(
                div()
                    .p(px(12.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        action(
                            "schedule-cancel",
                            "Cancel",
                            false,
                            !self.schedules.busy,
                            &colors,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.schedules.busy {
                                this.back(window, cx);
                            }
                        })),
                    )
                    .child(
                        action(
                            "schedule-submit",
                            if recovering {
                                "Confirm same key"
                            } else {
                                submit
                            },
                            true,
                            enabled && self.schedules.consent,
                            &colors,
                        )
                        .flex_1()
                        .debug_selector(|| "schedule-confirm".into())
                        .relative()
                        .child(crate::testing::probe("schedule-confirm"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.submit_schedule_confirmation(cx)),
                        ),
                    ),
            )
            .into_any_element()
    }
    fn schedule_detail_screen(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::ScheduleDetail);
        let Some(job) = self.selected_schedule() else {
            return div()
                .child(self.schedule_top("Schedule", &colors, cx))
                .child("Schedule no longer listed. Refresh schedules; existing sessions are kept.")
                .into_any_element();
        };
        let cache = self.schedule_cache();
        let can_mutate = self.schedule_can_mutate();
        let known = self.schedule_online() && self.schedule_supported() && cache.trusted;
        let job_id = job.id.clone();
        let edit = job.clone();
        let body = div().px(px(20.)).py(px(12.))
            .children(self.schedule_sample_note(&colors)).children(self.schedule_status_card(&colors))
            .children(cache.error.map(|error| note("Schedule request", error, &colors)))
            .child(self.schedule_uncertainty(&colors, cx))
            .child(note(&job.spec.name, format!("{}\n{}\n{} / {}\n{} · {}\n{}",
                if !known { "Unknown · cached state" } else if job.spec.enabled { "Enabled" } else { "Paused" }, job.spec.cwd, job.spec.provider, job.spec.model_id, job.spec.cron, job.spec.timezone,
                if !known { "Next occurrence unknown".into() } else { job.next_due_ms.map(|ms| format!("Next {}", date(ms, &job.spec.timezone))).unwrap_or_else(|| "No next occurrence reported".into()) }), &colors))
            .child(action("schedule-toggle", if job.spec.enabled { "Pause schedule" } else { "Resume schedule" }, false, can_mutate, &colors).w_full().mb(px(14.))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_schedule(cx))))
            .child(note("Recurring instruction", job.spec.prompt.clone(), &colors))
            .child(note("Execution policy", format!("Fresh session each occurrence · no same-job overlap\nMissed: {} · timeout: {} minutes (requests Stop)\nPause stops future occurrences, not existing sessions. Edits affect future work only. The host account's permissions apply; project trust required. Host must be awake.", if job.spec.missed_policy == MissedPolicy::Skip { "skip" } else { "run once when back" }, job.spec.timeout_minutes), &colors))
            .child(action("schedule-history", "Occurrence history", false, true, &colors).w_full().mb(px(14.))
                .on_click(cx.listener(move |this, _, window, cx| { this.push(Route::ScheduleHistory, window, cx); this.schedule_request(Request::History { job_id: job_id.clone() }, cx); })))
            .child(action("schedule-delete", "Delete schedule…", false, can_mutate, &colors).w_full()
                .on_click(cx.listener(|this, _, window, cx| this.confirm_schedule_action("delete", window, cx))));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.schedule_top("Schedule", &colors, cx))
            .child(scroll_area("schedule-detail-scroll", &scroll).child(body))
            .child(
                div()
                    .p(px(12.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        action("schedule-edit", "Edit", false, can_mutate, &colors)
                            .flex_1()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.begin_schedule_draft(Some(edit.clone()), window, cx)
                            })),
                    )
                    .child(
                        action("schedule-run-now", "Run now…", true, can_mutate, &colors)
                            .flex_1()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_schedule_action("run", window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
    fn schedule_history_screen(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::ScheduleHistory);
        let cache = self.schedule_cache();
        let occurrences = self
            .schedules
            .selected
            .as_ref()
            .and_then(|id| cache.history.get(id))
            .cloned()
            .unwrap_or_default();
        let body = div().px(px(20.)).py(px(12.))
            .children(self.schedule_sample_note(&colors)).children(self.schedule_status_card(&colors))
            .children(cache.error.as_ref().map(|error| note("History request", error.clone(), &colors)))
            .child(note("Admission ≠ completion ≠ task success", "Accepted means committed admission. Completed means the agent settled, not verified success. Interrupted tools require inspection of effects, not blind retry. Skipped and missed are not executed work.", &colors))
            .when(occurrences.is_empty(), |body| body.child(ui::hint(if self.schedules.busy { "Reading occurrence history…" } else { "No occurrence history loaded. Refresh when connected." }, &colors)))
            .children(occurrences.into_iter().enumerate().map(|(index, occurrence)| {
                let (label, meaning) = status(occurrence.status);
                let hue = match occurrence.status { RunStatus::Completed => colors.green, RunStatus::Failed | RunStatus::Interrupted => colors.coral, RunStatus::Accepted | RunStatus::Running => colors.accent, _ => colors.muted };
                let link = occurrence.session_key.is_some() && self.schedule_online();
                let time = date(occurrence.scheduled_ms, &occurrence.spec.timezone);
                ui::card(&colors).p(px(16.)).mb(px(12.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_color(hue).child(if self.schedule_online() && self.schedule_supported() && cache.trusted { label.to_owned() } else { format!("Unknown · cached {label}") }))
                    .child(ui::hint(time, &colors).mt(px(4.)))
                    .child(ui::hint(meaning, &colors).mt(px(8.)))
                    .children(occurrence.error.as_ref().map(|error| ui::hint(error.clone(), &colors).mt(px(8.))))
                    .child(ui::hint(format!("Revision {} · {}", occurrence.job_revision, occurrence.request_id), &colors).mt(px(6.)))
                    .when(occurrence.session_key.is_some(), |card| card.child(ui::button(("schedule-session", index), Button::Plain, Some("chat"), "Open session", false, &colors)
                        .debug_selector(move || format!("schedule-session-{index}"))
                        .relative().child(crate::testing::probe(format!("schedule-session-{index}")))
                        .when(!link, |button| ui::disabled(button, &colors)).mt(px(12.))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_schedule_occurrence(occurrence.clone(), window, cx)))))
            }));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.schedule_top("Occurrence history", &colors, cx))
            .child(scroll_area("schedule-history-scroll", &scroll).child(body))
            .into_any_element()
    }
    fn schedule_uncertainty(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let cache = self.schedule_cache();
        if self.schedules.busy {
            return div();
        }
        let Some(request) = cache.uncertain.as_ref() else {
            return div();
        };
        let can_recover = self.schedule_can_recover(request) && cache.uncertain_job.is_some();
        let manual = matches!(request, Request::RunNow { .. });
        div().mb(px(14.)).child(note("Mutation outcome unknown", match &request {
            Request::RunNow { request_id, .. } => format!("Retained manual key: {request_id}. Inspect refreshed history, then explicitly review recovery of this same key. The phone never generates a new key or automatically repeats this run."),
            _ => "Refresh and inspect schedules on this computer before unlocking editing. A create may already have committed. There is no automatic resubmission.".into(),
        }, colors))
        .when(manual, |body| body.child(action("schedule-reconcile", "Review same manual key…", false, can_recover, colors).w_full().mb(px(12.)).on_click(cx.listener(|this, _, window, cx| this.review_manual_recovery(window, cx)))))
        .when(!manual, |body| body.child(action("schedule-uncertainty-ack", "I inspected refreshed host state", false, self.schedule_can_acknowledge_uncertainty(), colors).max_w_full()
            .on_click(cx.listener(|this, _, _, cx| this.acknowledge_schedule_uncertainty(cx)))))
    }
}

#[cfg(test)]
mod tests {
    use super::quoted_helper_path;

    #[test]
    fn schedule_setup_instructions_quote_paths_without_shell_expansion() {
        assert_eq!(
            quoted_helper_path("~/.pi/desktop/bin/pi desktop"),
            "\"$HOME\"/'.pi/desktop/bin/pi desktop'"
        );
        assert_eq!(
            quoted_helper_path("~/helper';$(echo not-executed)"),
            "\"$HOME\"/'helper'\\'';$(echo not-executed)'"
        );
        assert_eq!(
            quoted_helper_path("/opt/pi desktop/helper'one"),
            "'/opt/pi desktop/helper'\\''one'"
        );
    }
}
