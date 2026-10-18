//! Host-owned schedule state. No phone timer, execution or automatic mutation retry.
use crate::{
    app::{PhoneApp, Route},
    ssh,
    text_area::TextArea,
};
use gpui::{App, AppContext, Context, Entity, TextInputConfiguration, Window};
use pi_core::schedules::{
    Data, Health, Job, MissedPolicy, Occurrence, Request, RunStatus, ScheduleSpec,
};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub(crate) struct Cache {
    pub jobs: Vec<Job>,
    pub health: Option<Health>,
    pub history: HashMap<String, Vec<Occurrence>>,
    pub trusted: bool,
    pub fetched_ms: Option<u64>,
    pub error: Option<String>,
    /// Frozen operation; especially the original manual admission key.
    pub uncertain: Option<Request>,
    /// The exact job revision shown when a manual operation was confirmed.
    pub uncertain_job: Option<Job>,
}
impl Cache {
    fn apply(&mut self, request: &Request, data: Data) {
        match data {
            Data::List { jobs, health } => {
                self.jobs = jobs;
                self.health = Some(health);
                self.trusted = true;
                self.fetched_ms = Some(now_ms());
            }
            Data::Job { job } => {
                self.jobs.retain(|old| old.id != job.id);
                self.jobs.push(job);
            }
            Data::Deleted => {
                if let Request::Delete { job_id, .. } = request {
                    self.jobs.retain(|job| job.id != *job_id);
                    // Keep the occurrence ledger and links. Never delete sessions.
                }
            }
            Data::Run { occurrence } => {
                let history = self.history.entry(occurrence.job_id.clone()).or_default();
                history.retain(|old| old.id != occurrence.id);
                history.insert(0, occurrence);
            }
            Data::History { occurrences } => {
                if let Request::History { job_id } = request {
                    // Occurrence::request_id is the durable prompt admission key,
                    // not necessarily the phone's manual operation key. Recover an
                    // uncertain RunNow by explicitly repeating its identical key.
                    self.history.insert(job_id.clone(), occurrences);
                }
            }
            Data::Preview { .. } => {}
        }
    }
}

pub(crate) struct Draft {
    pub host: String,
    pub original: Option<Job>,
    pub name: Entity<TextArea>,
    pub cwd: Entity<TextArea>,
    pub prompt: Entity<TextArea>,
    pub cron: Entity<TextArea>,
    pub timezone: Entity<TextArea>,
    pub timeout: Entity<TextArea>,
    pub provider: String,
    pub model_id: String,
    pub missed_policy: MissedPolicy,
    /// A preview authorizes review only for exactly these cron/zone inputs.
    pub preview: Option<(String, String, Vec<u64>)>,
    pub error: Option<String>,
}
impl Draft {
    pub fn spec(&self, cx: &App) -> Result<ScheduleSpec, String> {
        let spec = ScheduleSpec {
            name: self.name.read(cx).text().trim().into(),
            cwd: self.cwd.read(cx).text().trim().into(),
            prompt: self.prompt.read(cx).text().into(),
            provider: self.provider.clone(),
            model_id: self.model_id.clone(),
            cron: self.cron.read(cx).text().trim().into(),
            timezone: self.timezone.read(cx).text().trim().into(),
            enabled: self.original.as_ref().is_none_or(|job| job.spec.enabled),
            missed_policy: self.missed_policy,
            timeout_minutes: self
                .timeout
                .read(cx)
                .text()
                .trim()
                .parse()
                .map_err(|_| "Timeout must be a number of minutes".to_owned())?,
        };
        validate(&spec)?;
        Ok(spec)
    }
    pub fn preview_matches(&self, cx: &App) -> bool {
        self.preview.as_ref().is_some_and(|(cron, zone, times)| {
            cron == self.cron.read(cx).text().trim()
                && zone == self.timezone.read(cx).text().trim()
                && !times.is_empty()
        })
    }
}

#[derive(Clone)]
pub(crate) enum Confirmation {
    Save { request: Request, times: Vec<u64> },
    Run { request: Request, job: Box<Job> },
    Delete { job: Box<Job> },
}
#[derive(Default)]
pub(crate) struct Schedules {
    pub caches: HashMap<String, Cache>,
    pub draft: Option<Draft>,
    pub selected: Option<String>,
    pub confirmation: Option<Confirmation>,
    pub consent: bool,
    pub busy: bool,
    pub generation: u64,
    pub sample_mode: Option<&'static str>,
}
impl Schedules {
    /// Retire obsolete replies, including leaving and returning to the same host.
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.confirmation = None;
        self.consent = false;
        self.selected = None;
        self.sample_mode = None;
        for cache in self.caches.values_mut() {
            cache.trusted = false;
        }
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn validate(spec: &ScheduleSpec) -> Result<(), String> {
    if spec.name.is_empty()
        || spec.cwd.is_empty()
        || spec.prompt.trim().is_empty()
        || spec.provider.is_empty()
        || spec.model_id.is_empty()
    {
        return Err("Choose a project and model, and enter a name and instruction.".into());
    }
    if !spec.cwd.starts_with('/') {
        return Err("Project must be an absolute path on the computer.".into());
    }
    if spec.cron.split_whitespace().count() != 5 {
        return Err("Use a numeric five-field cron (minute hour day month weekday).".into());
    }
    if spec.timezone.is_empty() {
        return Err("Choose an explicit IANA timezone, such as Europe/Amsterdam.".into());
    }
    if !(1..=1440).contains(&spec.timeout_minutes) {
        return Err("Timeout must be 1–1440 minutes.".into());
    }
    Ok(())
}

pub(crate) fn status(status: RunStatus) -> (&'static str, &'static str) {
    match status {
        RunStatus::Pending => ("Pending", "Awaiting admission; not accepted yet"),
        RunStatus::Accepted => ("Accepted", "Admission committed; not completion"),
        RunStatus::Running => ("Running", "The agent is working on the computer"),
        RunStatus::Completed => ("Completed", "Agent settled; task success is not verified"),
        RunStatus::Failed => (
            "Failed",
            "Execution or admission failed; inspect the session",
        ),
        RunStatus::Interrupted => ("Interrupted", "Inspect tool effects before any retry"),
        RunStatus::Skipped => ("Skipped", "Not executed (for example, same-job overlap)"),
        RunStatus::Missed => ("Missed", "Due time passed without execution"),
        RunStatus::Unknown => ("Unknown", "No authoritative execution outcome available"),
    }
}

impl PhoneApp {
    pub(crate) fn schedule_host(&self) -> Option<String> {
        if !self.backend_supports("schedules") {
            return None;
        }
        self.store.as_ref().map(|store| {
            format!(
                "{}:{}",
                if store.is_sample() { "sample" } else { "ssh" },
                store.computer.address
            )
        })
    }
    pub(crate) fn schedule_cache(&self) -> Cache {
        self.schedule_host()
            .and_then(|host| self.schedules.caches.get(&host).cloned())
            .unwrap_or_default()
    }
    pub(crate) fn schedule_online(&self) -> bool {
        self.store.as_ref().is_some_and(|store| {
            store.computer.connected
                && store
                    .live
                    .as_ref()
                    .is_none_or(|live| !live.connection.is_closed())
        }) && self.schedules.sample_mode != Some("offline")
    }
    pub(crate) fn schedule_supported(&self) -> bool {
        if !self.backend_supports("schedules") {
            return false;
        }
        self.store.as_ref().is_some_and(|store| {
            store
                .live
                .as_ref()
                .is_none_or(|live| live.helper.capabilities.schedules)
        }) && self.schedules.sample_mode != Some("unsupported")
    }
    pub(crate) fn schedule_available(&self) -> bool {
        let cache = self.schedule_cache();
        self.schedule_online()
            && self.schedule_supported()
            && cache.trusted
            && cache.health.is_some_and(|health| health.running)
            && !self.schedules.busy
    }
    pub(crate) fn schedule_can_mutate(&self) -> bool {
        self.schedule_available() && self.schedule_cache().uncertain.is_none()
    }
    pub(crate) fn schedule_can_recover(&self, request: &Request) -> bool {
        self.schedule_available()
            && matches!(request, Request::RunNow { .. })
            && self.schedule_cache().uncertain.as_ref() == Some(request)
    }
    pub(crate) fn schedule_can_acknowledge_uncertainty(&self) -> bool {
        let cache = self.schedule_cache();
        self.schedule_online()
            && self.schedule_supported()
            && cache.trusted
            && !self.schedules.busy
            && cache
                .uncertain
                .as_ref()
                .is_some_and(|request| !matches!(request, Request::RunNow { .. }))
    }
    pub(crate) fn acknowledge_schedule_uncertainty(&mut self, cx: &mut Context<Self>) {
        if !self.schedule_can_acknowledge_uncertainty() {
            return;
        }
        let Some(host) = self.schedule_host() else {
            return;
        };
        if let Some(cache) = self.schedules.caches.get_mut(&host) {
            cache.uncertain = None;
            cache.uncertain_job = None;
            cache.error = None;
        }
        self.schedules.confirmation = None;
        self.schedules.consent = false;
        if self
            .schedules
            .draft
            .as_ref()
            .is_some_and(|draft| draft.host == host)
        {
            self.routes = vec![Route::Sessions, Route::Schedules, Route::ScheduleEdit];
        }
        cx.notify();
    }
    pub(crate) fn review_manual_recovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cache = self.schedule_cache();
        let (Some(request @ Request::RunNow { .. }), Some(job)) =
            (cache.uncertain, cache.uncertain_job)
        else {
            return;
        };
        if !self.schedule_can_recover(&request) {
            return;
        }
        self.schedules.confirmation = Some(Confirmation::Run {
            request,
            job: Box::new(job),
        });
        self.schedules.consent = false;
        self.push(Route::ScheduleConfirm, window, cx);
    }
    pub(crate) fn open_schedules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.as_ref().is_some_and(|store| store.is_sample()) {
            self.ensure_sample_schedules();
        }
        self.push(Route::Schedules, window, cx);
        self.schedule_request(Request::List, cx);
    }
    pub(crate) fn begin_schedule_draft(
        &mut self,
        job: Option<Job>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.schedule_can_mutate() {
            return;
        }
        let Some(host) = self.schedule_host() else {
            return;
        };
        let cwd = self
            .store
            .as_ref()
            .and_then(|store| store.projects.get(self.project))
            .map(|project| project.path.clone())
            .unwrap_or_default();
        let prefs = self.prefs(cx);
        let spec = job
            .as_ref()
            .map(|job| job.spec.clone())
            .unwrap_or_else(|| ScheduleSpec {
                name: String::new(),
                cwd,
                prompt: String::new(),
                provider: if self.store.as_ref().is_some_and(|s| s.is_sample()) {
                    "sample".into()
                } else {
                    prefs.model_provider.clone().unwrap_or_default()
                },
                model_id: if self.store.as_ref().is_some_and(|s| s.is_sample()) {
                    "Sample model".into()
                } else {
                    prefs.model_id.clone().unwrap_or_default()
                },
                cron: "0 9 * * 1-5".into(),
                timezone: "UTC".into(),
                enabled: true,
                missed_policy: MissedPolicy::Skip,
                timeout_minutes: 30,
            });
        let field = |text: &str, hint: &str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut area =
                    TextArea::single_line(hint.to_owned(), TextInputConfiguration::default(), cx);
                area.set_text(text, cx);
                area
            })
        };
        let prompt = cx.new(|cx| {
            let mut area = TextArea::multiline("Recurring instruction…", 5, cx);
            area.set_text(&spec.prompt, cx);
            area
        });
        self.schedules.draft = Some(Draft {
            host,
            original: job,
            name: field(&spec.name, "Morning project check", cx),
            cwd: field(&spec.cwd, "Absolute project path on computer", cx),
            prompt,
            cron: field(&spec.cron, "0 9 * * 1-5", cx),
            timezone: field(&spec.timezone, "Europe/Amsterdam", cx),
            timeout: field(&spec.timeout_minutes.to_string(), "30", cx),
            provider: spec.provider,
            model_id: spec.model_id,
            missed_policy: spec.missed_policy,
            preview: None,
            error: None,
        });
        self.push(Route::ScheduleEdit, window, cx);
    }
    pub(crate) fn rebase_schedule_draft(&mut self, cx: &mut Context<Self>) {
        if !self.schedule_can_mutate() {
            return;
        }
        let latest = self
            .schedules
            .draft
            .as_ref()
            .and_then(|draft| draft.original.as_ref())
            .and_then(|job| {
                self.schedule_cache()
                    .jobs
                    .into_iter()
                    .find(|latest| latest.id == job.id)
            });
        if let (Some(draft), Some(latest)) = (&mut self.schedules.draft, latest) {
            draft.original = Some(latest);
            draft.error = None;
            self.schedules.confirmation = None;
            self.schedules.consent = false;
            cx.notify();
        }
    }

    pub(crate) fn preview_schedule(&mut self, cx: &mut Context<Self>) {
        if !self.schedule_can_mutate() {
            return;
        }
        let Some(draft) = &mut self.schedules.draft else {
            return;
        };
        draft.error = None;
        draft.preview = None;
        let request = Request::Preview {
            cron: draft.cron.read(cx).text().trim().into(),
            timezone: draft.timezone.read(cx).text().trim().into(),
            count: 3,
        };
        self.schedule_request(request, cx);
    }
    pub(crate) fn review_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.schedule_can_mutate() {
            return;
        }
        let host = self.schedule_host().unwrap_or_default();
        let latest = self
            .schedules
            .draft
            .as_ref()
            .and_then(|draft| draft.original.as_ref())
            .and_then(|job| {
                self.schedule_cache()
                    .jobs
                    .into_iter()
                    .find(|latest| latest.id == job.id)
            });
        let Some(draft) = &mut self.schedules.draft else {
            return;
        };
        if draft.host != host {
            return;
        }
        if let Some(original) = &draft.original
            && latest
                .as_ref()
                .is_none_or(|job| job.revision != original.revision)
        {
            draft.error = Some("Schedule changed on the computer. Inspect the latest definition, then use its revision without replacing your draft.".into());
            cx.notify();
            return;
        }
        let spec = match draft.spec(cx) {
            Ok(spec) if draft.preview_matches(cx) => spec,
            Ok(_) => {
                draft.error =
                    Some("Preview next occurrences from the computer before review.".into());
                cx.notify();
                return;
            }
            Err(error) => {
                draft.error = Some(error);
                cx.notify();
                return;
            }
        };
        let request = match &draft.original {
            Some(job) => Request::Update {
                job_id: job.id.clone(),
                expected_revision: job.revision,
                spec,
                consent: true,
            },
            None => Request::Create {
                spec,
                consent: true,
            },
        };
        self.schedules.confirmation = Some(Confirmation::Save {
            request,
            times: draft.preview.as_ref().unwrap().2.clone(),
        });
        self.schedules.consent = false;
        self.push(Route::ScheduleConfirm, window, cx);
    }
    pub(crate) fn confirm_schedule_action(
        &mut self,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.schedule_can_mutate() {
            return;
        }
        let Some(job) = self.selected_schedule() else {
            return;
        };
        self.schedules.confirmation = Some(if action == "delete" {
            Confirmation::Delete { job: Box::new(job) }
        } else {
            Confirmation::Run {
                request: Request::RunNow {
                    job_id: job.id.clone(),
                    expected_revision: job.revision,
                    request_id: format!("phone-schedule-{:032x}", rand::random::<u128>()),
                    consent: true,
                },
                job: Box::new(job),
            }
        });
        self.schedules.consent = false;
        self.push(Route::ScheduleConfirm, window, cx);
    }
    pub(crate) fn selected_schedule(&self) -> Option<Job> {
        let selected = self.schedules.selected.as_ref()?;
        self.schedule_cache()
            .jobs
            .into_iter()
            .find(|job| job.id == *selected)
    }
    pub(crate) fn submit_schedule_confirmation(&mut self, cx: &mut Context<Self>) {
        if self.route() != Route::ScheduleConfirm || !self.schedules.consent {
            return;
        }
        let Some(confirmation) = self.schedules.confirmation.clone() else {
            return;
        };
        let request = match confirmation {
            Confirmation::Save { request, .. } | Confirmation::Run { request, .. } => request,
            Confirmation::Delete { job } => Request::Delete {
                job_id: job.id,
                expected_revision: job.revision,
            },
        };
        if self.schedule_can_mutate() || self.schedule_can_recover(&request) {
            self.schedule_request(request, cx);
        }
    }
    pub(crate) fn toggle_schedule(&mut self, cx: &mut Context<Self>) {
        if !self.schedule_can_mutate() {
            return;
        }
        let Some(mut job) = self.selected_schedule() else {
            return;
        };
        job.spec.enabled = !job.spec.enabled;
        // Original creation/edit review has already granted unattended consent.
        self.schedule_request(
            Request::Update {
                job_id: job.id,
                expected_revision: job.revision,
                spec: job.spec,
                consent: true,
            },
            cx,
        );
    }

    pub(crate) fn schedule_request(&mut self, request: Request, cx: &mut Context<Self>) {
        if self.schedules.busy || !self.schedule_online() || !self.schedule_supported() {
            return;
        }
        let mutation = is_mutation(&request);
        if mutation && !self.schedule_can_mutate() && !self.schedule_can_recover(&request) {
            return;
        }
        let Some(host) = self.schedule_host() else {
            return;
        };
        // Freeze the operation before dispatch. Switching hosts retires replies,
        // but must not erase an in-flight manual key or authorize a duplicate.
        if mutation {
            let original_job = if let Request::RunNow {
                job_id,
                expected_revision,
                ..
            } = &request
            {
                self.schedule_cache()
                    .jobs
                    .into_iter()
                    .find(|job| job.id == *job_id && job.revision == *expected_revision)
            } else {
                None
            };
            let cache = self.schedules.caches.entry(host.clone()).or_default();
            if cache.uncertain != Some(request.clone()) {
                cache.uncertain_job = original_job;
            }
            cache.uncertain = Some(request.clone());
        }
        self.schedules.busy = true;
        let generation = self.schedules.generation;
        if let Request::List = request {
            self.schedules
                .caches
                .entry(host.clone())
                .or_default()
                .trusted = false;
        }
        if self.store.as_ref().is_some_and(|store| store.is_sample()) {
            let result = self
                .sample_schedule_request(&request)
                .unwrap_or_else(|error| pi_core::schedules::Response {
                    version: pi_core::schedules::VERSION,
                    success: false,
                    data: None,
                    error: Some(error),
                });
            self.finish_schedule_request(host, generation, request, Ok(result), cx);
            return;
        }
        let Some(live) = self.store.as_ref().and_then(|store| store.live.as_ref()) else {
            self.schedules.busy = false;
            return;
        };
        let (connection, helper) = (live.connection.clone(), live.helper.clone());
        let (send, receive) = async_channel::bounded(1);
        let remote_request = request.clone();
        ssh::spawn(async move {
            let result = crate::remote::schedules(&connection, &helper, &remote_request)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = send.send(result).await;
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = receive.recv().await else {
                return;
            };
            this.update(cx, |this, cx| {
                this.finish_schedule_request(host, generation, request, result, cx)
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    fn finish_schedule_request(
        &mut self,
        host: String,
        generation: u64,
        request: Request,
        result: Result<pi_core::schedules::Response, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.schedules.generation || self.schedule_host().as_ref() != Some(&host) {
            return;
        }
        self.schedules.busy = false;
        let cache = self.schedules.caches.entry(host.clone()).or_default();
        match result {
            Err(error) => {
                cache.trusted = false;
                cache.error = Some(if is_mutation(&request) {
                    if cache.uncertain_job.is_none()
                        && let Request::RunNow {
                            job_id,
                            expected_revision,
                            ..
                        } = &request
                    {
                        cache.uncertain_job = cache
                            .jobs
                            .iter()
                            .find(|job| job.id == *job_id && job.revision == *expected_revision)
                            .cloned();
                    }
                    cache.uncertain = Some(request.clone());
                    // A refreshed host must not reactivate the old checked
                    // confirmation. Same-key recovery requires renewed consent.
                    self.schedules.consent = false;
                    format!(
                        "Outcome unknown. Do not resubmit blindly. Refresh schedules and history, and inspect the host/session. {error}"
                    )
                } else {
                    error
                });
            }
            Ok(response) if !response.success => {
                if is_mutation(&request) {
                    cache.uncertain = None;
                    cache.uncertain_job = None;
                }
                let error = response
                    .error
                    .unwrap_or_else(|| "The computer rejected the request".into());
                cache.error = Some(error.clone());
                if let Some(draft) = &mut self.schedules.draft {
                    draft.error = Some(error);
                }
                if is_mutation(&request) {
                    cache.trusted = false;
                    // Refresh optimistic conflicts without changing draft/original revision.
                    // User explicitly reopens/rebases after inspecting current host state.
                    self.schedules.confirmation = None;
                    self.schedules.consent = false;
                    if self.route() == Route::ScheduleConfirm {
                        self.routes = vec![
                            Route::Sessions,
                            Route::Schedules,
                            if matches!(request, Request::Create { .. } | Request::Update { .. }) {
                                Route::ScheduleEdit
                            } else {
                                Route::ScheduleDetail
                            },
                        ];
                    }
                    self.schedule_request(Request::List, cx);
                }
            }
            Ok(response) => {
                let Some(data) = response.data else { return };
                cache.error = None;
                if is_mutation(&request) {
                    cache.uncertain = None;
                    cache.uncertain_job = None;
                }
                if let Data::Preview { times_ms } = &data
                    && let (Some(draft), Request::Preview { cron, timezone, .. }) =
                        (&mut self.schedules.draft, &request)
                    && draft.host == host
                    && draft.cron.read(cx).text().trim() == cron
                    && draft.timezone.read(cx).text().trim() == timezone
                {
                    draft.preview = Some((cron.clone(), timezone.clone(), times_ms.clone()));
                }
                let saved = match &data {
                    Data::Job { job } => Some(job.id.clone()),
                    _ => None,
                };
                cache.apply(&request, data);
                if is_mutation(&request) {
                    self.schedules.confirmation = None;
                    self.schedules.consent = false;
                    if matches!(request, Request::Create { .. } | Request::Update { .. }) {
                        if matches!(self.route(), Route::ScheduleConfirm) {
                            self.schedules.draft = None;
                        }
                        self.schedules.selected = saved;
                        self.routes =
                            vec![Route::Sessions, Route::Schedules, Route::ScheduleDetail];
                    } else if matches!(request, Request::Delete { .. }) {
                        self.schedules.selected = None;
                        self.routes = vec![Route::Sessions, Route::Schedules];
                    } else if let Request::RunNow { job_id, .. } = &request {
                        self.routes = vec![
                            Route::Sessions,
                            Route::Schedules,
                            Route::ScheduleDetail,
                            Route::ScheduleHistory,
                        ];
                        self.schedules.selected = Some(job_id.clone());
                        // Show committed admission honestly, never turn it into completion.
                    }
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn open_schedule_occurrence(
        &mut self,
        occurrence: Occurrence,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = occurrence.session_key.as_ref() else {
            return;
        };
        if !self.schedule_online() {
            return;
        }
        let Some(store) = &mut self.store else { return };
        let id = if let Some(live) = &mut store.live {
            match live.open_scheduled(key, &occurrence.spec.cwd) {
                Ok(id) => id,
                Err(error) => {
                    self.notify_user(error, cx);
                    return;
                }
            }
        } else if store.is_sample() {
            crate::model::SessionId(1)
        } else {
            return;
        };
        // Unlike show_session(), this retains history/detail for normal Back.
        // Watching, scroll setup and live display still use ordinary session entry.
        self.push_session(id, window, cx);
    }
}

fn is_mutation(request: &Request) -> bool {
    matches!(
        request,
        Request::Create { .. }
            | Request::Update { .. }
            | Request::Delete { .. }
            | Request::RunNow { .. }
    )
}

pub(crate) const SAMPLE_NOW: u64 = 1_791_787_320_000;
pub(crate) fn sample_spec() -> ScheduleSpec {
    ScheduleSpec {
        name: "Morning project check".into(), cwd: "/Users/nick/repos/pi".into(),
        prompt: "Review local changes and failing tests. Summarize findings; ask before risky changes. This instruction is not an enforced read-only boundary.".into(),
        provider: "sample".into(), model_id: "Sample model".into(), cron: "0 9 * * 1-5".into(), timezone: "Europe/Amsterdam".into(),
        enabled: true, missed_policy: MissedPolicy::Skip, timeout_minutes: 30,
    }
}
impl PhoneApp {
    pub(crate) fn ensure_sample_schedules(&mut self) {
        let Some(host) = self.schedule_host() else {
            return;
        };
        self.schedules.caches.entry(host).or_insert_with(|| {
            let first = Job { id: "sample-morning".into(), revision: 1, spec: sample_spec(), next_due_ms: Some(SAMPLE_NOW + 87_480_000), created_ms: SAMPLE_NOW, updated_ms: SAMPLE_NOW };
            let mut second = first.clone(); second.id = "sample-weekly".into(); second.spec.name = "Weekly notes".into(); second.spec.enabled = false; second.spec.cron = "0 17 * * 5".into(); second.next_due_ms = None;
            let statuses = [RunStatus::Accepted, RunStatus::Running, RunStatus::Completed, RunStatus::Failed, RunStatus::Interrupted, RunStatus::Skipped, RunStatus::Missed, RunStatus::Pending, RunStatus::Unknown];
            let occurrences = statuses.into_iter().enumerate().map(|(index, status)| Occurrence {
                id: format!("sample-run-{index}"), job_id: first.id.clone(), job_revision: 1,
                scheduled_ms: SAMPLE_NOW - index as u64 * 86_400_000,
                session_key: (index < 5).then(|| "sample-session".into()), request_id: format!("sample-request-{index}"),
                spec: first.spec.clone(), status, error: (status == RunStatus::Interrupted).then(|| "Host restarted during tool execution. Inspect effects before deciding to continue.".into()),
                started_ms: None, finished_ms: None,
            }).collect();
            Cache { jobs: vec![first.clone(), second], health: Some(Health { running: true, autostart_installed: true, last_tick_ms: Some(SAMPLE_NOW), detail: "Sample scheduler — no real effects".into() }),
                history: HashMap::from([(first.id, occurrences)]), trusted: true, fetched_ms: Some(SAMPLE_NOW), ..Default::default() }
        });
    }
    fn sample_schedule_request(
        &self,
        request: &Request,
    ) -> Result<pi_core::schedules::Response, String> {
        let cache = self.schedule_cache();
        let data = match request {
            Request::List => Data::List {
                jobs: cache.jobs,
                health: cache.health.unwrap_or_default(),
            },
            Request::Preview {
                cron,
                timezone,
                count,
            } => {
                // Deterministic fixture only. Production validation and DST belong to host.
                if cron.split_whitespace().count() != 5
                    || !cron
                        .chars()
                        .all(|c| c.is_ascii_digit() || " *,/-".contains(c))
                    || timezone.parse::<chrono_tz::Tz>().is_err()
                {
                    return Ok(pi_core::schedules::Response {
                        version: 1,
                        success: false,
                        data: None,
                        error: Some("Invalid cron or IANA timezone (sample validation)".into()),
                    });
                }
                Data::Preview {
                    times_ms: (1..=(*count).min(5))
                        .map(|i| SAMPLE_NOW + i as u64 * 86_400_000)
                        .collect(),
                }
            }
            Request::Create { spec, .. } => Data::Job {
                job: Job {
                    id: format!("sample-{:032x}", rand::random::<u128>()),
                    revision: 1,
                    spec: spec.clone(),
                    next_due_ms: Some(SAMPLE_NOW + 87_480_000),
                    created_ms: SAMPLE_NOW,
                    updated_ms: SAMPLE_NOW,
                },
            },
            Request::Update {
                job_id,
                expected_revision,
                spec,
                ..
            } => {
                let mut job = cache
                    .jobs
                    .into_iter()
                    .find(|job| job.id == *job_id && job.revision == *expected_revision)
                    .ok_or("Sample revision conflict; refresh before editing")?;
                job.spec = spec.clone();
                job.revision += 1;
                job.next_due_ms = spec.enabled.then_some(SAMPLE_NOW + 87_480_000);
                Data::Job { job }
            }
            Request::Delete { .. } => Data::Deleted,
            Request::RunNow {
                job_id,
                request_id,
                expected_revision,
                ..
            } => {
                if let Some(existing) = cache.history.get(job_id).and_then(|runs| {
                    runs.iter()
                        .find(|run| run.id == format!("sample-{request_id}"))
                }) {
                    if existing.job_revision != *expected_revision {
                        return Err("Sample manual key belongs to another revision".into());
                    }
                    return Ok(pi_core::schedules::Response {
                        version: 1,
                        success: true,
                        data: Some(Data::Run {
                            occurrence: existing.clone(),
                        }),
                        error: None,
                    });
                }
                let job = cache
                    .jobs
                    .into_iter()
                    .find(|job| job.id == *job_id && job.revision == *expected_revision)
                    .ok_or("Sample revision conflict")?;
                Data::Run {
                    occurrence: Occurrence {
                        id: format!("sample-{request_id}"),
                        job_id: job.id,
                        job_revision: job.revision,
                        scheduled_ms: SAMPLE_NOW,
                        session_key: None,
                        request_id: request_id.clone(),
                        spec: job.spec,
                        status: RunStatus::Accepted,
                        error: Some(
                            "Sample admission only. No provider or session started.".into(),
                        ),
                        started_ms: None,
                        finished_ms: None,
                    },
                }
            }
            Request::History { job_id } => Data::History {
                occurrences: cache.history.get(job_id).cloned().unwrap_or_default(),
            },
        };
        Ok(pi_core::schedules::Response {
            version: 1,
            success: true,
            data: Some(data),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_requires_host_path_model_prompt_and_timeout() {
        let mut spec = sample_spec();
        assert!(validate(&spec).is_ok());
        spec.cwd = "~/repos/pi".into();
        assert!(validate(&spec).is_err());
        spec.cwd = "/repo".into();
        spec.timeout_minutes = 0;
        assert!(validate(&spec).is_err());
        spec.timeout_minutes = 30;
        spec.prompt.clear();
        assert!(validate(&spec).is_err());
    }
    #[test]
    fn statuses_never_conflate_admission_and_success() {
        assert!(status(RunStatus::Accepted).1.contains("not completion"));
        assert!(status(RunStatus::Completed).1.contains("not verified"));
        assert!(status(RunStatus::Interrupted).1.contains("effects"));
        let statuses = [
            RunStatus::Pending,
            RunStatus::Accepted,
            RunStatus::Running,
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Interrupted,
            RunStatus::Skipped,
            RunStatus::Missed,
            RunStatus::Unknown,
        ];
        assert_eq!(
            statuses
                .into_iter()
                .map(|s| status(s).0)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            9
        );
    }
    #[test]
    fn deleting_keeps_history_and_manual_uncertainty_survives_list() {
        let mut cache = Cache::default();
        let request = Request::RunNow {
            job_id: "a".into(),
            expected_revision: 3,
            request_id: "stable-key".into(),
            consent: true,
        };
        cache.uncertain = Some(request.clone());
        cache.history.insert("a".into(), vec![]);
        cache.apply(
            &Request::List,
            Data::List {
                jobs: vec![],
                health: Health::default(),
            },
        );
        assert_eq!(cache.uncertain, Some(request));
        cache.apply(
            &Request::Delete {
                job_id: "a".into(),
                expected_revision: 3,
            },
            Data::Deleted,
        );
        assert!(cache.history.contains_key("a"));
    }
    #[test]
    fn invalidation_keeps_host_scoped_cache_but_retires_replies_and_consent() {
        let mut state = Schedules::default();
        state.caches.insert(
            "ssh:a".into(),
            Cache {
                trusted: true,
                ..Default::default()
            },
        );
        state.caches.insert("ssh:b".into(), Cache::default());
        state.busy = true;
        state.consent = true;
        state.invalidate();
        assert_eq!(state.generation, 1);
        assert!(!state.busy && !state.consent);
        assert!(!state.caches["ssh:a"].trusted);
        assert_eq!(state.caches.len(), 2);
    }
}

#[cfg(feature = "ui-test")]
impl PhoneApp {
    /// Fixture-only read telemetry. Never exposes a real host's jobs or prompts.
    pub(crate) fn schedule_test_state(&self, cx: &App) -> serde_json::Value {
        if !self.paused || !self.store.as_ref().is_some_and(|store| store.is_sample()) {
            return serde_json::Value::Null;
        }
        let cache = self.schedule_cache();
        let definitions: Vec<_> = cache
            .jobs
            .iter()
            .map(|job| {
                serde_json::json!({
                    "id": job.id, "revision": job.revision, "name": job.spec.name,
                    "enabled": job.spec.enabled, "next_due": job.next_due_ms,
                    "cron": job.spec.cron, "timezone": job.spec.timezone, "cwd": job.spec.cwd,
                    "provider": job.spec.provider, "model_id": job.spec.model_id,
                })
            })
            .collect();
        let mut keys = cache.history.keys().collect::<Vec<_>>();
        keys.sort();
        let occurrences: Vec<_> = keys
            .into_iter()
            .flat_map(|key| &cache.history[key])
            .map(|run| {
                serde_json::json!({
                    "id": run.id, "job_id": run.job_id, "job_revision": run.job_revision,
                    "status": run.status, "session_id": run.session_key.as_ref().map(|_| 1),
                    "scheduled_ms": run.scheduled_ms, "request_id": run.request_id,
                })
            })
            .collect();
        let preview = self
            .schedules
            .draft
            .as_ref()
            .filter(|draft| draft.preview_matches(cx))
            .and_then(|draft| draft.preview.as_ref())
            .map(|(_, _, times)| times.clone())
            .unwrap_or_default();
        serde_json::json!({
            "mode": "sample", "status": if !self.schedule_online() { "offline" } else if !self.schedule_supported() { "unsupported" } else if cache.health.as_ref().is_some_and(|h| !h.running) { "setup_required" } else { "online" },
            "appearance": if self.prefs(cx).appearance == crate::theme::Appearance::Evening { "dark" } else { "light" },
            "selected_id": self.schedules.selected, "definitions": definitions,
            "draft": self.schedules.draft.as_ref().map(|draft| serde_json::json!({ "name_chars": draft.name.read(cx).text().chars().count(), "prompt_chars": draft.prompt.read(cx).text().chars().count(), "revision": draft.original.as_ref().map(|job| job.revision) })),
            "preview": preview, "preview_error": self.schedules.draft.as_ref().and_then(|draft| draft.error.clone()),
            "confirmation": self.schedules.confirmation.as_ref().map(|c| match c { Confirmation::Save { .. } => "save", Confirmation::Run { .. } => "run", Confirmation::Delete { .. } => "delete" }),
            "consent": self.schedules.consent, "occurrences": occurrences,
        })
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use gpui::{TestAppContext, px, size};

    #[gpui::test]
    fn schedules_open_from_the_computers_menu_and_back_returns_home(cx: &mut TestAppContext) {
        use crate::app::Sheet;
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for (width, height) in [(320., 640.), (412., 915.)] {
            cx.simulate_resize(size(px(width), px(height)));
            for fixture in [
                "schedules",
                "schedules-evening",
                "schedule-offline",
                "schedule-unsupported",
            ] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.preview(fixture, window, cx);
                        app.back(window, cx);
                    })
                });
                cx.run_until_parked();
                let (host, sessions) = app.read_with(cx, |app, _| {
                    assert_eq!(app.routes, [Route::Sessions]);
                    (
                        app.schedule_host(),
                        app.store.as_ref().unwrap().sessions.clone(),
                    )
                });
                assert!(cx.debug_bounds("home-sessions").is_none());
                assert!(cx.debug_bounds("home-schedules").is_none());
                let computers = cx.debug_bounds("computers").unwrap();
                cx.simulate_click(computers.center(), gpui::Modifiers::none());
                cx.run_until_parked();
                app.read_with(cx, |app, _| assert_eq!(app.sheet, Some(Sheet::Computers)));
                let schedules = cx
                    .debug_bounds("open-schedules")
                    .expect("Schedules is in the Computers menu");
                assert!(schedules.left() >= px(0.) && schedules.right() <= px(width));
                assert!(schedules.top() >= px(0.) && schedules.bottom() <= px(height));
                assert!(schedules.size.height >= px(48.));
                cx.simulate_click(schedules.center(), gpui::Modifiers::none());
                cx.run_until_parked();
                app.read_with(cx, |app, _| {
                    assert_eq!(app.routes, [Route::Sessions, Route::Schedules]);
                    assert_eq!(app.sheet, None);
                    assert_eq!(app.schedule_host(), host);
                    assert_eq!(app.store.as_ref().unwrap().sessions, sessions);
                });
                assert!(cx.debug_bounds("home-sessions").is_none());
                assert!(cx.debug_bounds("home-schedules").is_none());
                let back = cx.debug_bounds("schedule-back").unwrap();
                cx.simulate_click(back.center(), gpui::Modifiers::none());
                cx.run_until_parked();
                app.read_with(cx, |app, _| assert_eq!(app.routes, [Route::Sessions]));
            }
        }
    }

    #[gpui::test]
    fn schedule_create_requires_review_and_consent_and_preserves_sessions(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(size(px(360.), px(640.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.preview("schedule-review", window, cx))
        });
        let (jobs, sessions) = app.read_with(cx, |app, _| {
            (
                app.schedule_cache().jobs.len(),
                app.store.as_ref().unwrap().sessions.len(),
            )
        });
        cx.update(|_, cx| app.update(cx, |app, cx| app.submit_schedule_confirmation(cx)));
        app.read_with(cx, |app, _| {
            assert_eq!(app.schedule_cache().jobs.len(), jobs)
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.schedules.consent = true;
                app.submit_schedule_confirmation(cx);
            })
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.schedule_cache().jobs.len(), jobs + 1);
            assert_eq!(app.store.as_ref().unwrap().sessions.len(), sessions);
            assert!(app.selected_schedule().unwrap().spec.enabled);
            assert_eq!(app.route(), Route::ScheduleDetail);
            assert!(!app.schedules.consent);
        });
        // A duplicated click on an obsolete confirmation cannot create twice.
        cx.update(|_, cx| app.update(cx, |app, cx| app.submit_schedule_confirmation(cx)));
        app.read_with(cx, |app, _| {
            assert_eq!(app.schedule_cache().jobs.len(), jobs + 1)
        });
    }

    #[gpui::test]
    fn schedule_paused_edits_stay_paused_run_now_is_only_admission_delete_keeps_history(
        cx: &mut TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.preview("schedule-detail", window, cx))
        });
        cx.update(|_, cx| app.update(cx, |app, cx| app.toggle_schedule(cx)));
        let job = app.read_with(cx, |app, _| app.selected_schedule().unwrap());
        assert!(!job.spec.enabled);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_schedule_draft(Some(job.clone()), window, cx);
                app.schedules
                    .draft
                    .as_mut()
                    .unwrap()
                    .name
                    .update(cx, |area, cx| area.set_text("Edited paused schedule", cx));
                app.preview_schedule(cx);
                app.review_schedule(window, cx);
                app.schedules.consent = true;
                app.submit_schedule_confirmation(cx);
            })
        });
        let edited = app.read_with(cx, |app, _| app.selected_schedule().unwrap());
        assert!(!edited.spec.enabled);
        assert_eq!(edited.revision, job.revision + 1);
        let count = app.read_with(cx, |app, _| app.schedule_cache().history[&job.id].len());
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.confirm_schedule_action("run", window, cx);
                app.back(window, cx);
            })
        });
        app.read_with(cx, |app, _| {
            assert!(app.schedules.confirmation.is_none());
            assert_eq!(app.schedule_cache().history[&job.id].len(), count);
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.confirm_schedule_action("run", window, cx);
                app.schedules.consent = true;
                app.submit_schedule_confirmation(cx);
            })
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.selected_schedule().unwrap(), edited);
            let cache = app.schedule_cache();
            assert_eq!(cache.history[&job.id].len(), count + 1);
            assert_eq!(cache.history[&job.id][0].status, RunStatus::Accepted);
            assert!(cache.history[&job.id][0].session_key.is_none());
        });
        let sessions = app.read_with(cx, |app, _| app.store.as_ref().unwrap().sessions.len());
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.back(window, cx);
                app.confirm_schedule_action("delete", window, cx);
                app.schedules.consent = true;
                app.submit_schedule_confirmation(cx);
            })
        });
        app.read_with(cx, |app, _| {
            assert!(!app.schedule_cache().jobs.iter().any(|j| j.id == job.id));
            assert_eq!(app.schedule_cache().history[&job.id].len(), count + 1);
            assert_eq!(app.store.as_ref().unwrap().sessions.len(), sessions);
        });
    }

    #[gpui::test]
    fn schedule_preview_is_invalidated_by_changed_cron_and_project_model_are_draft_only(
        cx: &mut TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("schedule-new", window, cx)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.schedules.draft.as_ref().unwrap().preview_matches(cx));
                app.schedules
                    .draft
                    .as_mut()
                    .unwrap()
                    .cron
                    .update(cx, |area, cx| area.set_text("invalid", cx));
                app.review_schedule(window, cx);
                assert!(app.schedules.confirmation.is_none());
                app.preview_schedule(cx);
                assert!(app.schedules.draft.as_ref().unwrap().error.is_some());
                let prefs_model = app.prefs(cx).model_id.clone();
                app.choose_model("Chosen".into(), "test".into(), "model".into(), cx);
                assert_eq!(app.prefs(cx).model_id, prefs_model);
                app.use_folder("/Users/nick/another", cx);
                assert_eq!(app.route(), Route::ScheduleEdit);
                assert_eq!(
                    app.schedules.draft.as_ref().unwrap().cwd.read(cx).text(),
                    "/Users/nick/another"
                );
                assert_eq!(app.schedules.draft.as_ref().unwrap().model_id, "model");
            })
        });
    }

    #[gpui::test]
    fn schedule_unavailable_states_have_no_mutations_and_obsolete_replies_cannot_switch_hosts(
        cx: &mut TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for fixture in [
            "schedule-offline",
            "schedule-setup",
            "schedule-stopped",
            "schedule-unsupported",
        ] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.preview(fixture, window, cx);
                    let before = app.schedule_cache().jobs;
                    app.toggle_schedule(cx);
                    app.begin_schedule_draft(None, window, cx);
                    app.confirm_schedule_action("run", window, cx);
                    assert!(!app.schedule_can_mutate());
                    assert_eq!(app.schedule_cache().jobs, before);
                    assert!(app.schedules.confirmation.is_none());
                })
            });
        }
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("schedules", window, cx);
                let host = app.schedule_host().unwrap();
                let generation = app.schedules.generation;
                app.open_store("other@computer", cx);
                app.ensure_sample_schedules();
                let before = app.schedule_cache().jobs;
                app.finish_schedule_request(
                    host,
                    generation,
                    Request::List,
                    Ok(pi_core::schedules::Response {
                        version: 1,
                        success: true,
                        data: Some(Data::List {
                            jobs: vec![],
                            health: Health::default(),
                        }),
                        error: None,
                    }),
                    cx,
                );
                assert_eq!(app.schedule_cache().jobs, before);
            })
        });
    }

    #[gpui::test]
    fn schedule_revision_conflict_refresh_preserves_draft_then_explicitly_rebases(
        cx: &mut TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("schedule-detail", window, cx);
                let original = app.selected_schedule().unwrap();
                app.begin_schedule_draft(Some(original.clone()), window, cx);
                app.schedules
                    .draft
                    .as_mut()
                    .unwrap()
                    .prompt
                    .update(cx, |area, cx| area.set_text("Keep my local draft", cx));
                app.preview_schedule(cx);
                app.review_schedule(window, cx);
                let request = match app.schedules.confirmation.as_ref().unwrap() {
                    Confirmation::Save { request, .. } => request.clone(),
                    _ => panic!(),
                };
                let host = app.schedule_host().unwrap();
                let cache = app.schedules.caches.get_mut(&host).unwrap();
                let current = cache
                    .jobs
                    .iter_mut()
                    .find(|job| job.id == original.id)
                    .unwrap();
                current.revision += 1;
                current.spec.prompt = "Someone else's remote edit".into();
                current.spec.enabled = false;
                app.finish_schedule_request(
                    host,
                    app.schedules.generation,
                    request,
                    Ok(pi_core::schedules::Response {
                        version: 1,
                        success: false,
                        data: None,
                        error: Some("Revision conflict".into()),
                    }),
                    cx,
                );
                assert_eq!(app.route(), Route::ScheduleEdit);
                let draft = app.schedules.draft.as_ref().unwrap();
                assert_eq!(draft.prompt.read(cx).text(), "Keep my local draft");
                assert_eq!(draft.original.as_ref().unwrap().revision, original.revision);
                app.review_schedule(window, cx);
                assert!(app.schedules.confirmation.is_none());
                app.rebase_schedule_draft(cx);
                assert_eq!(
                    app.schedules
                        .draft
                        .as_ref()
                        .unwrap()
                        .original
                        .as_ref()
                        .unwrap()
                        .revision,
                    original.revision + 1
                );
                assert_eq!(
                    app.schedules.draft.as_ref().unwrap().prompt.read(cx).text(),
                    "Keep my local draft"
                );
                app.review_schedule(window, cx);
                let Confirmation::Save {
                    request:
                        Request::Update {
                            spec,
                            expected_revision,
                            ..
                        },
                    ..
                } = app.schedules.confirmation.as_ref().unwrap()
                else {
                    panic!()
                };
                assert_eq!(*expected_revision, original.revision + 1);
                assert!(!spec.enabled);
            })
        });
    }

    #[gpui::test]
    fn schedule_manual_uncertainty_retains_key_and_blocks_new_runs(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("schedule-run-confirm", window, cx);
                let request = match app.schedules.confirmation.as_ref().unwrap() {
                    Confirmation::Run { request, .. } => request.clone(),
                    _ => panic!(),
                };
                let host = app.schedule_host().unwrap();
                app.schedules.consent = true;
                app.finish_schedule_request(
                    host.clone(),
                    app.schedules.generation,
                    request.clone(),
                    Err("lost after send".into()),
                    cx,
                );
                assert_eq!(app.schedules.caches[&host].uncertain, Some(request.clone()));
                assert!(
                    !app.schedules.consent,
                    "Lost responses retire prior consent"
                );
                app.schedule_request(Request::List, cx);
                app.schedule_request(
                    Request::History {
                        job_id: "sample-morning".into(),
                    },
                    cx,
                );
                app.submit_schedule_confirmation(cx);
                assert_eq!(app.schedules.caches[&host].uncertain, Some(request.clone()));
                assert!(!app.schedule_can_mutate());
                app.back(window, cx);
                app.confirm_schedule_action("run", window, cx);
                assert!(app.schedules.confirmation.is_none());
                assert_eq!(app.schedules.caches[&host].uncertain, Some(request));
            })
        });
    }

    #[gpui::test]
    fn schedule_explicit_recovery_reuses_original_key_and_revision_even_after_edit_or_delete(
        cx: &mut TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for deleted in [false, true] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.preview("schedule-run-confirm", window, cx);
                    let Confirmation::Run { request, job } =
                        app.schedules.confirmation.as_ref().unwrap().clone()
                    else {
                        panic!()
                    };
                    let host = app.schedule_host().unwrap();
                    let response = app.sample_schedule_request(&request).unwrap();
                    // The inert sample host committed, but its response was lost.
                    app.schedules
                        .caches
                        .get_mut(&host)
                        .unwrap()
                        .apply(&request, response.data.unwrap());
                    app.finish_schedule_request(
                        host.clone(),
                        app.schedules.generation,
                        request.clone(),
                        Err("lost response".into()),
                        cx,
                    );
                    app.back(window, cx);
                    // The original key/revision owns admission even after another
                    // client edits or deletes the recurring definition.
                    if deleted {
                        app.schedules.caches.get_mut(&host).unwrap().jobs.clear();
                    } else {
                        app.schedules.caches.get_mut(&host).unwrap().jobs[0].revision += 1;
                    }
                    app.schedule_request(Request::List, cx);
                    let count = app.schedule_cache().history[&job.id].len();
                    app.review_manual_recovery(window, cx);
                    let Confirmation::Run {
                        request: recovered,
                        job: snapshot,
                    } = app.schedules.confirmation.as_ref().unwrap()
                    else {
                        panic!()
                    };
                    assert_eq!(recovered, &request);
                    assert_eq!(snapshot, &job);
                    app.submit_schedule_confirmation(cx);
                    assert!(
                        app.schedule_cache().uncertain.is_some(),
                        "Recovery also requires new explicit consent"
                    );
                    app.schedules.consent = true;
                    app.submit_schedule_confirmation(cx);
                    assert!(app.schedule_cache().uncertain.is_none());
                    assert_eq!(
                        app.schedule_cache().history[&job.id].len(),
                        count,
                        "Same key cannot create another admission"
                    );
                    assert_eq!(
                        app.schedule_cache().history[&job.id][0].job_revision,
                        job.revision
                    );
                })
            });
        }
    }

    #[gpui::test]
    fn schedule_history_links_return_to_history_without_new_prompt(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("schedule-history", window, cx);
                let occurrence = app.schedule_cache().history["sample-morning"][2].clone();
                let sessions = app.store.as_ref().unwrap().sessions.clone();
                let history = app.schedule_cache().history;
                let stack = app.routes.clone();
                let id = crate::model::SessionId(1);
                app.open_schedule_occurrence(occurrence.clone(), window, cx);
                assert_eq!(app.route(), Route::Thread(id));
                assert_eq!(&app.routes[..app.routes.len() - 1], stack.as_slice());
                app.back(window, cx);
                assert_eq!(app.route(), Route::ScheduleHistory);
                // A cached reopen must not reset the reader's scroll position.
                app.scroll(Route::Thread(id))
                    .set_offset(gpui::point(px(0.), px(-12.)));
                app.open_schedule_occurrence(occurrence, window, cx);
                assert_eq!(app.scroll(Route::Thread(id)).offset().y, px(-12.));
                app.back(window, cx);
                app.back(window, cx);
                assert_eq!(app.route(), Route::ScheduleDetail);
                assert_eq!(app.schedules.selected.as_deref(), Some("sample-morning"));
                app.back(window, cx);
                assert_eq!(app.route(), Route::Schedules);
                app.back(window, cx);
                assert_eq!(app.route(), Route::Sessions);
                assert_eq!(app.store.as_ref().unwrap().sessions, sessions);
                assert_eq!(app.schedule_cache().history, history);
                // Ordinary list/notification entry still has Sessions behind it.
                app.show_session(id, window, cx);
                assert_eq!(app.routes, vec![Route::Sessions, Route::Thread(id)]);
            })
        });
    }

    #[gpui::test]
    fn schedule_native_history_click_and_back_preserve_detail(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for (width, height) in [(360., 640.), (412., 915.)] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.preview("schedule-history", window, cx))
            });
            cx.run_until_parked();
            let target = cx.debug_bounds("schedule-session-2").unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let scroll = app.scroll(Route::ScheduleHistory);
                    scroll.set_offset(gpui::point(
                        px(0.),
                        scroll.offset().y - (target.top() - scroll.bounds().top() - px(12.)),
                    ));
                    cx.notify();
                })
            });
            cx.run_until_parked();
            let target = cx.debug_bounds("schedule-session-2").unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let viewport = app.scroll(Route::ScheduleHistory).bounds();
                    assert!(target.top() >= viewport.top() && target.bottom() <= viewport.bottom());
                });
            });
            cx.simulate_click(target.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            app.read_with(cx, |app, _| {
                assert_eq!(app.route(), Route::Thread(crate::model::SessionId(1)))
            });
            // The same GoBack action used by Android's hardware Back/gesture.
            for expected in [
                Route::ScheduleHistory,
                Route::ScheduleDetail,
                Route::Schedules,
            ] {
                cx.update(|window, cx| window.dispatch_action(Box::new(crate::GoBack), cx));
                cx.run_until_parked();
                app.read_with(cx, |app, _| {
                    assert_eq!(app.route(), expected);
                    assert_eq!(app.schedules.selected.as_deref(), Some("sample-morning"));
                });
            }
        }
    }

    #[gpui::test]
    fn schedule_native_unavailable_detail_controls_cannot_queue_work(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(size(px(360.), px(640.)));
        for fixture in [
            "schedule-offline",
            "schedule-setup",
            "schedule-stopped",
            "schedule-unsupported",
        ] {
            cx.update(|window, cx| app.update(cx, |app, cx| app.preview(fixture, window, cx)));
            cx.run_until_parked();
            let row = cx.debug_bounds("schedule-row-0").unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let scroll = app.scroll(Route::Schedules);
                    scroll.set_offset(gpui::point(
                        px(0.),
                        scroll.offset().y - (row.top() - scroll.bounds().top() - px(12.)),
                    ));
                    cx.notify();
                })
            });
            cx.run_until_parked();
            let row = cx.debug_bounds("schedule-row-0").unwrap().center();
            cx.simulate_click(row, gpui::Modifiers::none());
            cx.run_until_parked();
            let before = app.read_with(cx, |app, _| app.schedule_cache());
            for selector in [
                "schedule-edit",
                "schedule-run-now",
                "schedule-toggle",
                "schedule-delete",
            ] {
                let target = cx.debug_bounds(selector).unwrap();
                if matches!(selector, "schedule-toggle" | "schedule-delete") {
                    cx.update(|_, cx| {
                        app.update(cx, |app, cx| {
                            let scroll = app.scroll(Route::ScheduleDetail);
                            scroll.set_offset(gpui::point(
                                px(0.),
                                scroll.offset().y
                                    - (target.top() - scroll.bounds().top() - px(12.)),
                            ));
                            cx.notify();
                        })
                    });
                    cx.run_until_parked();
                }
                let target = cx.debug_bounds(selector).unwrap();
                assert!(target.top() >= px(0.) && target.bottom() <= px(640.));
                cx.simulate_click(target.center(), gpui::Modifiers::none());
                cx.run_until_parked();
                app.read_with(cx, |app, _| {
                    assert_eq!(app.route(), Route::ScheduleDetail, "{fixture} {selector}");
                    let cache = app.schedule_cache();
                    assert_eq!(cache.jobs, before.jobs);
                    assert_eq!(cache.history, before.history);
                    assert!(cache.uncertain.is_none());
                    assert!(!app.schedules.busy);
                    assert!(app.schedules.confirmation.is_none());
                });
            }
            if fixture == "schedule-offline" {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        let occurrence = app.schedule_cache().history["sample-morning"][2].clone();
                        app.open_schedule_occurrence(occurrence, window, cx);
                        assert_eq!(app.route(), Route::ScheduleDetail);
                    })
                });
            }
        }
    }

    #[gpui::test]
    fn schedule_run_delete_revision_conflicts_retire_review_and_consent(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for fixture in ["schedule-run-confirm", "schedule-delete-confirm"] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.preview(fixture, window, cx);
                    let confirmation = app.schedules.confirmation.as_ref().unwrap().clone();
                    let request = match confirmation {
                        Confirmation::Run { request, .. } => request,
                        Confirmation::Delete { job } => Request::Delete {
                            job_id: job.id,
                            expected_revision: job.revision,
                        },
                        _ => panic!(),
                    };
                    let host = app.schedule_host().unwrap();
                    app.schedules.caches.get_mut(&host).unwrap().jobs[0].revision += 1;
                    let before = app.schedule_cache();
                    app.schedules.consent = true;
                    app.finish_schedule_request(
                        host,
                        app.schedules.generation,
                        request,
                        Ok(pi_core::schedules::Response {
                            version: 1,
                            success: false,
                            data: None,
                            error: Some("Revision conflict".into()),
                        }),
                        cx,
                    );
                    assert_eq!(app.route(), Route::ScheduleDetail);
                    assert!(app.schedules.confirmation.is_none());
                    assert!(!app.schedules.consent);
                    assert!(
                        app.schedule_can_mutate(),
                        "Conflict refresh permits a new explicit review"
                    );
                    app.submit_schedule_confirmation(cx);
                    assert_eq!(app.schedule_cache().jobs, before.jobs);
                    assert_eq!(app.schedule_cache().history, before.history);
                    app.confirm_schedule_action(
                        if fixture == "schedule-run-confirm" {
                            "run"
                        } else {
                            "delete"
                        },
                        window,
                        cx,
                    );
                    match app.schedules.confirmation.as_ref().unwrap() {
                        Confirmation::Run { job, .. } | Confirmation::Delete { job } => {
                            assert_eq!(job.revision, 2)
                        }
                        _ => panic!(),
                    }
                    assert!(!app.schedules.consent);
                })
            });
        }
    }

    #[gpui::test]
    fn schedule_unknown_save_unlock_requires_online_refreshed_inspection(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("schedule-review", window, cx);
                let Confirmation::Save { request, .. } =
                    app.schedules.confirmation.as_ref().unwrap().clone()
                else {
                    panic!()
                };
                let host = app.schedule_host().unwrap();
                let before = app.schedule_cache().jobs;
                app.schedules.consent = true;
                app.finish_schedule_request(
                    host,
                    app.schedules.generation,
                    request.clone(),
                    Err("Lost save response".into()),
                    cx,
                );
                app.acknowledge_schedule_uncertainty(cx);
                assert_eq!(app.schedule_cache().uncertain, Some(request.clone()));
                app.schedule_request(Request::List, cx);
                for mode in [Some("offline"), Some("unsupported")] {
                    app.schedules.sample_mode = mode;
                    app.acknowledge_schedule_uncertainty(cx);
                    assert_eq!(app.schedule_cache().uncertain, Some(request.clone()));
                    app.submit_schedule_confirmation(cx);
                    assert_eq!(app.schedule_cache().jobs, before);
                }
                app.schedules.sample_mode = None;
                app.schedules.busy = true;
                app.acknowledge_schedule_uncertainty(cx);
                assert_eq!(app.schedule_cache().uncertain, Some(request));
                app.schedules.busy = false;
                app.acknowledge_schedule_uncertainty(cx);
                assert!(app.schedule_can_mutate());
                assert_eq!(app.route(), Route::ScheduleEdit);
                assert!(app.schedules.draft.is_some());
                assert!(!app.schedules.consent);
                assert!(app.schedules.confirmation.is_none());
                assert_eq!(app.schedule_cache().jobs, before);
            })
        });
    }

    #[gpui::test]
    fn schedule_consent_wraps_and_the_last_line_remains_tappable(cx: &mut TestAppContext) {
        use crate::theme::Appearance;
        use gpui::{Modifiers, point};
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for width in [320., 360., 412.] {
            cx.simulate_resize(size(px(width), px(640.)));
            for fixture in [
                "schedule-review",
                "schedule-run-confirm",
                "schedule-delete-confirm",
            ] {
                for appearance in [Appearance::Moonstone, Appearance::Evening] {
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| {
                            app.preview(fixture, window, cx);
                            app.update_prefs(cx, |prefs| prefs.appearance = appearance);
                            app.apply_theme(window, cx);
                            app.scroll(Route::ScheduleConfirm).scroll_to_bottom();
                        })
                    });
                    cx.run_until_parked();
                    let before = app.read_with(cx, |app, _| {
                        assert!(!app.schedules.consent);
                        (
                            app.schedule_cache().jobs.clone(),
                            app.store.as_ref().unwrap().sessions.clone(),
                        )
                    });
                    let row = cx.debug_bounds("schedule-consent").unwrap();
                    let text = cx.debug_bounds("schedule-consent-label").unwrap();
                    let confirm = cx.debug_bounds("schedule-confirm").unwrap();
                    assert!(row.left() >= px(20.) && row.right() <= px(width - 20.));
                    assert!(text.left() >= row.left() + px(16. + 24. + 12.));
                    assert!(
                        text.right() <= row.right() - px(16.),
                        "{fixture} at {width}: consent text overflows its row: {text:?}"
                    );
                    assert!(
                        text.size.height >= px(44.),
                        "the complete consent must wrap"
                    );
                    assert!(
                        text.top() >= row.top() + px(16.)
                            && text.bottom() <= row.bottom() - px(16.)
                    );
                    assert!(
                        row.top() >= px(56.) && row.bottom() < confirm.top(),
                        "consent must be fully visible above the action dock"
                    );
                    // Tap the last wrapped line, not just the icon or first line.
                    cx.simulate_click(
                        point(text.center().x, text.bottom() - px(11.)),
                        Modifiers::none(),
                    );
                    cx.run_until_parked();
                    app.read_with(cx, |app, _| assert!(app.schedules.consent));
                    cx.simulate_click(
                        point(row.left() + px(28.), row.center().y),
                        Modifiers::none(),
                    );
                    cx.run_until_parked();
                    app.read_with(cx, |app, _| {
                        assert!(!app.schedules.consent);
                        assert_eq!(app.schedule_cache().jobs, before.0);
                        assert_eq!(app.store.as_ref().unwrap().sessions, before.1);
                    });
                }
            }
        }
    }

    #[gpui::test]
    fn schedule_native_confirmation_touch_requires_visible_consent(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(size(px(360.), px(640.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.preview("schedule-review", window, cx))
        });
        cx.run_until_parked();
        let count = app.read_with(cx, |app, _| app.schedule_cache().jobs.len());
        let confirm = cx.debug_bounds("schedule-confirm").unwrap().center();
        cx.simulate_click(confirm, gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.schedule_cache().jobs.len(), count)
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.scroll(Route::ScheduleConfirm)
                    .set_offset(gpui::point(px(0.), px(-10000.)));
                cx.notify();
            })
        });
        cx.run_until_parked();
        let consent = cx.debug_bounds("schedule-consent").unwrap();
        assert!(
            consent.top() >= px(56.) && consent.bottom() < px(570.),
            "Consent must be a visible native target: {consent:?}"
        );
        cx.simulate_click(consent.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| assert!(app.schedules.consent));
        let confirm = cx.debug_bounds("schedule-confirm").unwrap().center();
        cx.simulate_click(confirm, gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.schedule_cache().jobs.len(), count + 1);
            assert_eq!(app.route(), Route::ScheduleDetail);
        });
    }

    #[cfg(feature = "ui-test")]
    #[gpui::test]
    fn schedule_native_hook_contract_routes_selectors_and_fresh_bounds(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| {
            cx.set_global(crate::testing::State::default());
            PhoneApp::new(None, window, cx)
        });
        cx.simulate_resize(size(px(360.), px(640.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_url("pi://preview/schedule-new", window, cx)
            })
        });
        cx.run_until_parked();
        for (fixture, status, appearance) in [
            ("schedules", "online", "light"),
            ("schedules-evening", "online", "dark"),
            ("schedule-offline", "offline", "light"),
            ("schedule-setup", "setup_required", "light"),
            ("schedule-stopped", "setup_required", "light"),
            ("schedule-unsupported", "unsupported", "light"),
        ] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.open_url(&format!("pi://preview/{fixture}"), window, cx)
                })
            });
            cx.run_until_parked();
            app.read_with(cx, |app, cx| {
                assert_eq!(app.route(), Route::Schedules);
                assert_eq!(cx.global::<crate::testing::State>().fixture, fixture);
                let state = app.schedule_test_state(cx);
                assert_eq!(state["mode"], "sample");
                assert_eq!(state["status"], status);
                assert_eq!(state["appearance"], appearance);
                assert_eq!(state["selected_id"], "sample-morning");
                assert_eq!(state["definitions"][0]["id"], "sample-morning");
                assert_eq!(state["definitions"][0]["revision"], 1);
                assert!(state["definitions"][0]["next_due"].is_number());
                assert_eq!(state["occurrences"][2]["status"], "completed");
                assert_eq!(state["occurrences"][2]["session_id"], 1);
                assert!(state["draft"].is_null() && state["confirmation"].is_null());
                assert_eq!(state["consent"], false);
            });
            cx.update(|_, cx| {
                let bounds = &cx.global::<crate::testing::State>().bounds;
                for selector in [
                    "schedule-back",
                    "schedule-refresh",
                    "schedule-new",
                    "schedule-row-0",
                ] {
                    assert_eq!(
                        bounds
                            .keys()
                            .filter(
                                |key| *key == selector || key.contains(&format!("\"{selector}\""))
                            )
                            .count(),
                        1,
                        "{fixture}: {selector}"
                    );
                }
                assert_eq!(
                    bounds
                        .keys()
                        .filter(|key| key.starts_with("scroll:") && key.contains("schedule"))
                        .count(),
                    1
                );
            });
        }
        // Follow one fixture across destinations: stale scrolls/targets must
        // disappear without a fresh deep link clearing the telemetry for us.
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_url("pi://preview/schedules", window, cx)
            })
        });
        cx.run_until_parked();
        let row = cx.debug_bounds("schedule-row-0").unwrap().center();
        cx.simulate_click(row, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| {
            let bounds = &cx.global::<crate::testing::State>().bounds;
            assert!(
                !bounds
                    .keys()
                    .any(|key| key.contains("schedule-row-0") || key.contains("schedule-new"))
            );
            assert_eq!(
                bounds
                    .keys()
                    .filter(|key| key.starts_with("scroll:") && key.contains("schedule"))
                    .count(),
                1
            );
            assert!(bounds.contains_key("scroll:Name(\"schedule-detail-scroll\")"));
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.begin_schedule_draft(None, window, cx))
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let bounds = &cx.global::<crate::testing::State>().bounds;
            for selector in [
                "schedule-name",
                "schedule-project",
                "schedule-model",
                "schedule-prompt",
                "schedule-cron",
                "schedule-timezone",
                "schedule-timeout",
                "schedule-preview",
                "schedule-review",
            ] {
                assert_eq!(
                    bounds
                        .keys()
                        .filter(|key| *key == selector || key.contains(&format!("\"{selector}\"")))
                        .count(),
                    1,
                    "edit: {selector}"
                );
            }
            assert!(!bounds.keys().any(|key| key.contains("schedule-toggle")));
            assert_eq!(
                bounds
                    .keys()
                    .filter(|key| key.starts_with("scroll:") && key.contains("schedule"))
                    .count(),
                1
            );
        });
    }

    #[gpui::test]
    fn schedule_native_docks_fit_small_and_large_phones(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for (width, height) in [(360., 640.), (412., 915.)] {
            cx.simulate_resize(size(px(width), px(height)));
            for (screen, buttons) in [
                (
                    "schedules",
                    vec!["schedule-back", "schedule-refresh", "schedule-new"],
                ),
                ("schedule-new", vec!["schedule-review"]),
                (
                    "schedule-review",
                    vec!["schedule-confirm", "schedule-cancel"],
                ),
                ("schedule-detail", vec!["schedule-edit", "schedule-run-now"]),
                (
                    "schedule-run-confirm",
                    vec!["schedule-confirm", "schedule-cancel"],
                ),
                (
                    "schedule-delete-confirm",
                    vec!["schedule-confirm", "schedule-cancel"],
                ),
            ] {
                cx.update(|window, cx| app.update(cx, |app, cx| app.preview(screen, window, cx)));
                cx.run_until_parked();
                for selector in buttons {
                    let bounds = cx
                        .debug_bounds(selector)
                        .unwrap_or_else(|| panic!("{screen}: missing {selector}"));
                    assert!(
                        bounds.left() >= px(0.)
                            && bounds.right() <= px(width)
                            && bounds.top() >= px(0.)
                            && bounds.bottom() <= px(height),
                        "{screen} {selector} {width}x{height}: {bounds:?}"
                    );
                    assert!(
                        bounds.size.height >= px(48.),
                        "{selector}: touch target too small"
                    );
                }
            }
        }
    }
}
