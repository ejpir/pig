//! The phone app: a stack of screens, at most one bottom sheet over them, a
//! short notice drawer below the app bar, and a computer's sessions underneath,
//! or the sample ones.
//!
//! Back closes the sheet, then the search, then the screen; on the first
//! screen it leaves the app, as Android expects.

use crate::{
    alerts::{self, Link},
    composer::{Attachment, Composer, ComposerEvent, compact_instructions},
    live::{Live, TransportDiagnostic, Update},
    model::{Answer, Computer, SessionId, StageKind, State},
    motion::SwipeMotion,
    prefs::{Prefs, SavedCommand},
    projects::{FileView, ProjectBrowser, Row as ProjectRow},
    remote,
    ssh::{self, Address, Connection, Identity},
    store::{Event, Store},
    text_area::{TextArea, TextAreaEvent},
    theme::{Appearance, SANS, Theme, theme},
};
use gpui::{
    Animation, AnimationExt, App, Context, Edges, Entity, FocusHandle, Focusable, ListAlignment,
    ListState, PathPromptOptions, Pixels, Role, ScrollHandle, SharedString, Subscription, Task,
    TextInputAction, TextInputConfiguration, Window, WindowAppearance, WindowVisibility, actions,
    div, prelude::*, px, relative,
};
use gpui_android::activity;
use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

actions!(pi_android, [GoBack]);

async fn open_remote_helper(
    address: Address,
    identity: Identity,
    known: Option<String>,
) -> anyhow::Result<(Address, Connection, remote::Helper)> {
    let connection = Connection::open(address.clone(), identity, known).await?;
    let helper = remote::find(&connection).await?;
    Ok((address, connection, helper))
}

async fn open_remote_helper_with_legacy_fallback(
    address: Address,
    fallback: Option<Address>,
    identity: Identity,
    known: Option<String>,
) -> anyhow::Result<(Address, Connection, remote::Helper)> {
    match open_remote_helper(address, identity.clone(), known.clone()).await {
        Err(error)
            if matches!(error.downcast_ref(), Some(ssh::Failure::Unreachable(_)))
                && fallback.is_some() =>
        {
            open_remote_helper(fallback.unwrap(), identity, known).await
        }
        result => result,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Connect,
    Sessions,
    Schedules,
    ScheduleEdit,
    ScheduleDetail,
    ScheduleConfirm,
    ScheduleHistory,
    Projects,
    Start,
    /// The native editor workspace, with or without an active file.
    File,
    Thread(SessionId),
    /// A live shell call, bound by an opaque screen token to its stable tool ID.
    Command(SessionId, u64),
    /// A session-owned helper account shell. Its token/view survives hiding.
    Terminal(SessionId, u64),
    Review(SessionId),
    History(SessionId),
    Diagnostics(SessionId),
    /// Every subagent across all of a session's turns and hand-offs.
    Subagents(SessionId),
    /// One session-owned swarm: map, addressed exchanges and peer roster.
    Swarm(SessionId, CrewPick),
    /// An immutable swarm/peer binding; Pick is only its original presentation slot.
    SwarmPeer(SessionId, Pick, u64),
    /// One subagent of a session, with exact-target input when durable APIs support it.
    Subagent(SessionId, Pick),
    Settings,
}

/// A subagent by where it shows: the turn, the hand-off in it, and its place there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pick {
    pub turn: usize,
    pub handoff: usize,
    pub index: usize,
}

/// A crew within the originating session; UI state additionally binds its exact swarm ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CrewPick {
    pub turn: usize,
    pub handoff: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReviewListShape {
    pub(crate) session: SessionId,
    pub(crate) call: Option<(usize, StageKind, usize)>,
    pub(crate) file: usize,
    pub(crate) rows: usize,
    /// The selected diff row after which the inline follow-up editor appears.
    pub(crate) comment_after: Option<(usize, usize)>,
    /// Changes whenever staged cards on this file change.
    pub(crate) comments_key: u64,
    pub(crate) wrap: bool,
    pub(crate) added: u32,
    pub(crate) removed: u32,
    pub(crate) content_key: u64,
}

/// One locally staged inline section. Sections remain separate in the review UI
/// and are encoded together only when the reviewer sends the final follow-up.
#[derive(Clone, Debug)]
pub(crate) struct ReviewComment {
    pub(crate) id: u64,
    pub(crate) path: String,
    pub(crate) anchor: (usize, usize),
    pub(crate) lines: String,
    pub(crate) text: String,
    pub(crate) attachments: Vec<Attachment>,
}

/// Getting back to the saved computer when the app opens, in place of setup.
#[derive(Clone, Debug)]
pub(crate) struct Resume {
    pub(crate) address: String,
    /// Tries so far, counting the one under way.
    pub(crate) attempt: u32,
    /// When the next try starts on its own, after a failed one.
    pub(crate) retry_at: Option<Instant>,
}

/// Which composer a sheet acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Start,
    Thread(SessionId),
    Review(SessionId),
}

fn composer_target_active(target: Target, route: Route) -> bool {
    match (target, route) {
        (Target::Start, Route::Start) => true,
        (Target::Review(id), Route::Review(shown)) | (Target::Thread(id), Route::Thread(shown)) => {
            id == shown
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    Details(SessionId),
    Attach(Target),
    Model,
    Thinking,
    Project,
    /// Pick the folder that becomes the editor's working directory.
    EditorRoot,
    /// Browse only files below the editor's working directory.
    EditorFiles,
    FileStructure,
    FileRecoveries,
    More(SessionId),
    Models,
    Resources,
    Activity(SessionId, usize, StageKind),
    /// One tool call: session, turn, its stage, which of the stage's tools.
    Call(SessionId, usize, StageKind, usize),
    /// The run line of a turn, pulled up from the working strip.
    Run(SessionId, usize),
    Delete(SessionId),
    RestoreHistory(SessionId, usize),
    EnableJj(SessionId),
    Image(Target, usize),
    /// An image submitted with a prompt: session, turn, which image.
    PromptImage(SessionId, usize, usize),
    /// An image a tool returned: session, turn, which.
    ToolImage(SessionId, usize, usize),
    /// What the app logged lately, from Settings.
    Logs,
    Computers,
    /// Text opened by an explicit source/output viewer, outside chat.
    SelectText,
    InlineSource,
    InlineUpdate,
    InlinePermissions,
}

#[derive(Clone, Debug)]
pub(crate) enum JjHistoryState {
    Loading,
    Loaded(remote::JjHistory),
    Failed(SharedString),
}

pub(crate) struct RenderStats {
    pub(crate) visible: bool,
    last_frame: Option<Instant>,
    intervals: VecDeque<Duration>,
    build_times: VecDeque<Duration>,
    refresh_rate: Option<f32>,
    max_refresh_rate: Option<f32>,
    last_rate_read: Option<Instant>,
    frames_since_label: u8,
    cached_label: SharedString,
}

impl Default for RenderStats {
    fn default() -> Self {
        Self {
            visible: false,
            last_frame: None,
            intervals: VecDeque::new(),
            build_times: VecDeque::new(),
            refresh_rate: None,
            max_refresh_rate: None,
            last_rate_read: None,
            frames_since_label: 0,
            cached_label: "".into(),
        }
    }
}

impl RenderStats {
    fn observe(&mut self, now: Instant) {
        let Some(last) = self.last_frame.replace(now) else {
            return;
        };
        let interval = now.duration_since(last);
        // Idle time is not a slow frame. Start a fresh active-render window.
        if interval > Duration::from_millis(250) {
            self.intervals.clear();
            self.frames_since_label = 10;
            return;
        }
        if self.intervals.len() == 120 {
            self.intervals.pop_front();
        }
        self.intervals.push_back(interval);
        self.frames_since_label = self.frames_since_label.saturating_add(1);
    }

    fn reset(&mut self) {
        self.last_frame = None;
        self.intervals.clear();
        self.build_times.clear();
        self.frames_since_label = 10;
        self.cached_label = "".into();
    }

    fn observe_build(&mut self, elapsed: Duration) {
        if self.build_times.len() == 120 {
            self.build_times.pop_front();
        }
        self.build_times.push_back(elapsed);
    }

    fn refresh_display_rates(&mut self, now: Instant) {
        if self
            .last_rate_read
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2))
        {
            return;
        }
        self.refresh_rate = activity::refresh_rate();
        self.max_refresh_rate = activity::max_refresh_rate();
        self.last_rate_read = Some(now);
        self.frames_since_label = 10;
    }

    fn label(&mut self) -> SharedString {
        // Updating the text at 10-12 Hz is readable without sorting and
        // formatting on every frame that the diagnostic is measuring.
        if self.frames_since_label < 10 && !self.cached_label.is_empty() {
            return self.cached_label.clone();
        }
        self.frames_since_label = 0;
        let seconds: f32 = self.intervals.iter().map(Duration::as_secs_f32).sum();
        let fps = if seconds > 0. {
            self.intervals.len() as f32 / seconds
        } else {
            0.
        };
        let latest = self
            .intervals
            .back()
            .map_or(0., |duration| duration.as_secs_f32() * 1_000.);
        let mut sorted: Vec<_> = self.intervals.iter().copied().collect();
        sorted.sort_unstable();
        let p95 = if sorted.is_empty() {
            0.
        } else {
            sorted[(sorted.len() - 1) * 95 / 100].as_secs_f32() * 1_000.
        };
        let (rate, late) = self.refresh_rate.map_or_else(
            || ("? Hz".to_string(), 0),
            |rate| {
                // Allow normal scheduler/vsync jitter; count frames that
                // clearly missed the next refresh opportunity.
                let late_after = Duration::from_secs_f32(1. / rate).mul_f32(1.25);
                let late = self
                    .intervals
                    .iter()
                    .filter(|interval| **interval > late_after)
                    .count();
                let label = self.max_refresh_rate.map_or_else(
                    || format!("{rate:.0} Hz"),
                    |max| {
                        if (max - rate).abs() < 0.5 {
                            format!("{rate:.0} Hz")
                        } else {
                            format!("{rate:.0} Hz ({max:.0} max)")
                        }
                    },
                );
                (label, late)
            },
        );
        let build = self
            .build_times
            .back()
            .map_or(0.0, |t| t.as_secs_f32() * 1000.0);
        let mut builds: Vec<_> = self.build_times.iter().copied().collect();
        builds.sort_unstable();
        let build95 = if builds.is_empty() {
            0.0
        } else {
            builds[(builds.len() - 1) * 95 / 100].as_secs_f32() * 1000.0
        };
        self.cached_label = format!(
            "view {fps:.0} FPS / {rate}\ngap {latest:.1} ms · p95 {p95:.1} · late {late}/{}\nbuild {build:.1} ms · p95 {build95:.1}",
            self.intervals.len()
        )
        .into();
        self.cached_label.clone()
    }
}

pub struct PhoneApp {
    pub(crate) inline_pages: std::cell::RefCell<crate::inline_page::InlinePages>,
    pub(crate) visual_apps: std::cell::RefCell<crate::visual_apps::VisualApps>,
    /// The turn whose Visualize app is shown full screen.
    pub(crate) expanded_visual: Option<(SessionId, usize)>,
    pub(crate) schedules: crate::schedules::Schedules,
    pub(crate) store: Option<Store>,
    pub(crate) routes: Vec<Route>,
    pub(crate) sheet: Option<Sheet>,
    pub(crate) swiping_session: Option<(SessionId, SwipeMotion)>,
    pub(crate) deleting_session: Option<SessionId>,
    pub(crate) jj_histories: HashMap<SessionId, JjHistoryState>,
    pub(crate) restoring_history: Option<(SessionId, usize)>,
    pub(crate) enabling_jj: Option<SessionId>,
    pub(crate) sheet_motion: SwipeMotion,
    pub(crate) closing_sheet: Option<Sheet>,
    pub(crate) sheet_height: Rc<Cell<Pixels>>,
    /// The new-session sheet as it is dragged down to close.
    pub(crate) start_motion: SwipeMotion,
    pub(crate) start_height: Rc<Cell<Pixels>>,
    /// Zoom and pan of the image a sheet shows, and where it is on screen.
    pub(crate) image_zoom: f32,
    pub(crate) image_pan: gpui::Point<Pixels>,
    pub(crate) image_box: Rc<Cell<gpui::Bounds<Pixels>>>,
    pub(crate) sheet_scroll: ScrollHandle,
    /// Retained rows for the one activity bottom sheet that can be open.
    pub(crate) activity_view: Option<crate::screens::sheets::ActivitySheetState>,
    pub(crate) expanded_turns: HashMap<(SessionId, usize), bool>,
    /// Finished prompts shown in full instead of on one line.
    pub(crate) expanded_prompts: HashSet<(SessionId, usize)>,
    /// Compaction summaries opened in the thread: session, turn, step of its flow.
    pub(crate) expanded_compactions: HashSet<(SessionId, usize, usize)>,
    /// Reasoning opened in the thread: session, turn, step of its flow.
    pub(crate) expanded_thinking: HashSet<(SessionId, usize, usize)>,
    /// Finished turns whose calls are shown in the thread again.
    pub(crate) expanded_streams: HashSet<(SessionId, usize)>,
    /// Folded runs of calls opened in the thread: session, turn, first step.
    pub(crate) expanded_calls: HashSet<(SessionId, usize, usize)>,
    /// Subagents whose screen was opened, drawn like a session: an ID of the
    /// phone's own for each (session, conversation), and how it is now.
    pub(crate) subagent_ids: HashMap<(SessionId, String), SessionId>,
    /// Synthetic child IDs descend monotonically so deleting one child cannot
    /// make a later child reuse an ID still retained by another virtual list.
    pub(crate) next_subagent_session_id: u32,
    pub(crate) subagent_sessions: HashMap<SessionId, crate::model::Session>,
    /// Last fetched child revision projected into each synthetic session.
    pub(crate) subagent_projected_revisions: HashMap<SessionId, u64>,
    /// Drafts belong to exact child conversations, never to the parent composer.
    pub(crate) subagent_composers: HashMap<(SessionId, String), Entity<Composer>>,
    /// Questions put off with Later: the composer shows instead of the card.
    pub(crate) questions_later: HashSet<SessionId>,
    /// Home shows what you've seen while something needs you.
    pub(crate) seen_open: bool,
    /// Report cards showing every changed file, not just the first few.
    pub(crate) all_files: HashSet<SessionId>,
    /// Hand-off cards, by session, turn and hand-off: folded to their header,
    /// or showing every subagent instead of the first few.
    pub(crate) folded_handoffs: HashSet<(SessionId, usize, usize)>,
    pub(crate) all_subagents: HashSet<(SessionId, usize, usize)>,
    pub(crate) swarm_input_targets: HashMap<(SessionId, String), crate::live::SubagentInputTarget>,
    pub(crate) next_swarm_peer: u64,
    pub(crate) swarm_peer_bindings: HashMap<u64, crate::screens::swarm::SwarmPeerBinding>,
    pub(crate) swarm_views: HashMap<(SessionId, CrewPick), crate::screens::swarm::SwarmScreenState>,
    pub(crate) next_command_view: u64,
    pub(crate) command_views: HashMap<u64, crate::screens::command::CommandScreenState>,
    pub(crate) next_terminal_view: u64,
    pub(crate) terminal_views: HashMap<u64, crate::screens::terminal::TerminalScreenState>,
    /// A sheet transition gives focus back to the app once, before its fields can focus.
    sheet_focus_pending: bool,
    /// Open disclosures in the details sheet.
    pub(crate) expanded: HashSet<&'static str>,
    pub(crate) notice: Option<SharedString>,
    notice_generation: usize,
    pub(crate) render_stats: RenderStats,
    pub(crate) focus: FocusHandle,
    pub(crate) address: Entity<TextArea>,
    /// Text shown by explicit source viewers in the legacy selection sheet.
    pub(crate) selectable: Entity<TextArea>,
    /// The one Markdown block currently being selected directly in the thread.
    pub(crate) inline_selectable: Entity<TextArea>,
    pub(crate) inline_selection: Option<String>,
    pub(crate) connect_error: Option<SharedString>,
    pub(crate) connecting: bool,
    /// Identifies the connection attempt that is still allowed to update the app.
    connection_generation: u64,
    /// Enrollment progress and the code that must match the computer.
    pub(crate) pairing_status: Option<SharedString>,
    /// The phone's public key, once made: one line for authorized_keys.
    pub(crate) phone_key: Option<String>,
    /// The computer turned down the phone's key on the last try.
    pub(crate) key_refused: bool,
    /// The connect screen shows manual SSH key setup instead of QR pairing.
    pub(crate) manual_setup: bool,
    /// First run chose "On your computer": the pairing steps, then the QR.
    pub(crate) pairing: bool,
    /// Set while the app gets back to the saved computer; the connect screen
    /// then shows that instead of first-run setup.
    pub(crate) resuming: Option<Resume>,
    _retry: Option<Task<()>>,
    /// Where the key and settings live.
    pub(crate) data_dir: PathBuf,
    #[cfg(feature = "local-backend")]
    pub(crate) local_providers: crate::provider_accounts::Panel,
    #[cfg(feature = "local-backend")]
    pub(crate) local_frame: u64,
    #[cfg(feature = "local-backend")]
    pub(crate) local_private: bool,
    #[cfg(feature = "local-backend")]
    pub(crate) parked_local: Option<Store>,
    #[cfg(feature = "local-backend")]
    parked_local_threads: HashMap<SessionId, Entity<Composer>>,
    #[cfg(feature = "local-backend")]
    parked_local_start: Option<(String, Vec<crate::composer::Attachment>)>,
    /// Finds a folder or file in the project browser, or goes to a path.
    pub(crate) folder: Entity<TextArea>,
    reconnecting: bool,
    last_listed: Instant,
    last_reconnect: Instant,
    /// Carries what the computer sends into the store.
    _pump: Option<Task<()>>,
    pub(crate) search: Entity<TextArea>,
    pub(crate) model_search: Entity<TextArea>,
    /// Only populated by the isolated preview harness.
    pub(crate) preview_models: Vec<pi_core::protocol::Model>,
    /// Sanitized records used only by the isolated preview harness.
    pub(crate) preview_transport_diagnostics: HashMap<SessionId, VecDeque<TransportDiagnostic>>,
    pub(crate) searching: bool,
    pub(crate) start: Entity<Composer>,
    pub(crate) start_visible_height: Option<gpui::Pixels>,
    pub(crate) project: usize,
    pub(crate) project_browser: ProjectBrowser,
    /// The remote folder that bounds every editor read, save, and file-drawer action.
    pub(crate) editor_root: Option<String>,
    /// The shared browser is repurposed for the editor only while this is set.
    pub(crate) project_browser_editor: bool,
    /// Retained rows for the project sheet; only the viewport is laid out.
    pub(crate) project_list: ListState,
    pub(crate) project_tree_rows: Vec<ProjectRow>,
    pub(crate) project_overview_shape: Option<(usize, bool)>,
    pub(crate) project_list_offset: Pixels,
    pub(crate) file_view: Option<FileView>,
    /// Inactive editor tabs. The active buffer stays in `file_view` to keep
    /// existing route and preview ownership simple.
    pub(crate) file_tabs: Vec<FileView>,
    pub(crate) file_recoveries: crate::projects::FileRecoveryState,
    pub(crate) file_recovery_delete_armed: Option<String>,
    /// The current File route was pushed from the project chooser, so Back
    /// restores that chooser. Workspace resumes explicitly clear this.
    pub(crate) file_return_to_project: bool,
    pub(crate) file_exit_armed: bool,
    file_exit_generation: u64,
    /// Tool images decoded once, by key.
    pub(crate) tool_images: std::cell::RefCell<HashMap<String, crate::screens::ShownImage>>,
    /// Image keys whose cards have actually intersected a viewport. Keeping
    /// this separate from layout prevents full-history measurement from
    /// decoding legacy inline images or starting remote hydration.
    pub(crate) tool_image_demands: HashSet<String>,
    /// Page posters asked for, so each is drawn once.
    pub(crate) posters_drawing: std::cell::RefCell<std::collections::HashSet<PathBuf>>,
    /// Invalidates file replies when another file opens.
    pub(crate) file_generation: u64,
    /// Invalidates project-tree replies when the selected folder changes.
    pub(crate) project_files_generation: u64,
    pub(crate) threads: HashMap<SessionId, Entity<Composer>>,
    /// Cleared compact commands, retained until the backend accepts or rejects them.
    pending_compacts: HashMap<SessionId, (Target, String)>,
    /// Sample sessions have their own model selection, just like live sessions.
    sample_models: HashMap<SessionId, (String, String)>,
    pub(crate) review: Entity<Composer>,
    /// Session that owns the retained review draft and staged sections.
    pub(crate) review_session: Option<SessionId>,
    pub(crate) review_file: usize,
    /// One edit/write call shown as a focused full-screen diff instead of the
    /// session-wide file review: turn, stage and tool index.
    pub(crate) review_call: Option<(usize, StageKind, usize)>,
    /// Tapped review lines, as (hunk, line) of the shown file.
    pub(crate) review_lines: BTreeSet<(usize, usize)>,
    /// Inline sections accumulated for one final follow-up.
    pub(crate) review_comments: Vec<ReviewComment>,
    pub(crate) review_next_comment_id: u64,
    /// Retained vertical virtualization for the current session/call diff.
    pub(crate) review_list: gpui::ListState,
    /// Identity and dimensions of the diff represented by `review_list`.
    pub(crate) review_list_shape: Option<ReviewListShape>,
    /// Last virtual-list offset, for settled-fling touch filtering.
    pub(crate) review_list_offset: Pixels,
    scrolls: HashMap<Route, ScrollHandle>,
    pub(crate) thread_lists: HashMap<SessionId, gpui::ListState>,
    pub(crate) thread_list_shapes: HashMap<SessionId, (usize, usize, bool)>,
    /// Large thread opens and reopens present loading before layout; cached
    /// reopens retain their exact full-history measurements.
    pub(crate) thread_loading: HashMap<SessionId, u64>,
    pub(crate) thread_loading_release_scheduled: HashMap<SessionId, u64>,
    /// Monotonic within this app instance so detached callbacks from an old
    /// connection cannot release a newer load that reused the same session ID.
    pub(crate) thread_loading_generation: u64,
    /// Height-relevant failed-prompt text, retained across navigation.
    pub(crate) thread_failed_layouts: HashMap<SessionId, Vec<(String, String)>>,
    /// Shape changes spliced during a fling and awaiting exact remeasurement.
    pub(crate) thread_pending_remeasure: HashSet<SessionId>,
    /// Immutable transcript snapshots reused for every frame of a fling.
    pub(crate) thread_snapshots: HashMap<SessionId, Arc<crate::model::Session>>,
    /// Last virtual-list offset, for settled-fling touch filtering.
    pub(crate) thread_list_offsets: HashMap<SessionId, Pixels>,
    pub(crate) prefs_path: Option<PathBuf>,
    pub(crate) visible: bool,
    pub(crate) window_active: bool,
    /// The text of the working notification, while one is shown.
    working_posted: Option<String>,
    /// Live transcript changes held until scrolling/direct manipulation settles.
    deferred_updates: Vec<(Option<SessionId>, Update)>,
    deferred_flush_scheduled: bool,
    last_tick: Instant,
    /// Holds the sample sessions still, for previews.
    pub(crate) paused: bool,
    /// A paused fixture whose sample sessions run anyway, for recording.
    pub(crate) playing: bool,
    subscriptions: Vec<Subscription>,
    _ticker: Task<()>,
}

pub(crate) fn is_scrolled_to_bottom(scroll: &ScrollHandle) -> bool {
    scroll.bounds().size.height <= px(0.) || scroll.max_offset().y + scroll.offset().y <= px(2.)
}

impl PhoneApp {
    pub fn new(prefs_path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::visual_profile::install_trace();
        #[cfg(target_os = "android")]
        cx.set_reduce_motion(activity::reduced_motion());
        let prefs = prefs_path.as_deref().map(Prefs::load).unwrap_or_default();
        let address = cx.new(|cx| {
            TextArea::single_line(
                "user@computer.local",
                TextInputConfiguration {
                    autocorrect: false,
                    autocapitalize: gpui::Autocapitalize::None,
                    suggestions: false,
                    input_action: TextInputAction::Go,
                },
                cx,
            )
        });
        let search = cx.new(|cx| {
            TextArea::single_line(
                "Search sessions",
                TextInputConfiguration {
                    autocorrect: false,
                    autocapitalize: gpui::Autocapitalize::None,
                    suggestions: true,
                    input_action: TextInputAction::Search,
                },
                cx,
            )
        });
        let model_search = cx.new(|cx| {
            TextArea::single_line(
                "Search models or providers",
                TextInputConfiguration {
                    autocorrect: false,
                    autocapitalize: gpui::Autocapitalize::None,
                    suggestions: false,
                    input_action: TextInputAction::Search,
                },
                cx,
            )
        });
        let folder = cx.new(|cx| {
            TextArea::single_line(
                "Find a folder or file, or type a path",
                TextInputConfiguration {
                    autocorrect: false,
                    autocapitalize: gpui::Autocapitalize::None,
                    suggestions: false,
                    input_action: TextInputAction::Search,
                },
                cx,
            )
        });
        let start = cx.new(|cx| {
            let mut composer = Composer::new("Describe the change…", cx);
            composer.set_new_task();
            composer
        });
        let review = cx.new(|cx| Composer::new("Add a follow-up…", cx));
        let selectable = cx.new(|cx| TextArea::read_only(12, cx));
        let inline_selectable = cx.new(|cx| TextArea::read_only(100_000, cx));
        let subscriptions = vec![
            cx.subscribe(&selectable, |this, _, event: &TextAreaEvent, cx| {
                if *event == TextAreaEvent::Copied {
                    this.close_sheet(cx);
                    this.notify_user("Copied", cx);
                }
            }),
            cx.subscribe(&inline_selectable, |this, _, event: &TextAreaEvent, cx| {
                if matches!(event, TextAreaEvent::Copied | TextAreaEvent::Dismissed) {
                    let copied = *event == TextAreaEvent::Copied;
                    this.inline_selection = None;
                    if copied {
                        this.notify_user("Copied", cx);
                    } else {
                        cx.notify();
                    }
                }
            }),
            cx.subscribe_in(&address, window, |this, _, event, window, cx| match event {
                TextAreaEvent::Submit => this.connect(window, cx),
                TextAreaEvent::Changed(_) => {
                    this.connect_error = None;
                    cx.notify();
                }
                TextAreaEvent::SelectionChanged
                | TextAreaEvent::Copied
                | TextAreaEvent::Dismissed => {}
            }),
            cx.subscribe(&search, |_, _, _: &TextAreaEvent, cx| cx.notify()),
            cx.subscribe(&model_search, |this, _, event: &TextAreaEvent, cx| {
                if matches!(event, TextAreaEvent::Changed(_)) {
                    this.sheet_scroll.set_offset(gpui::Point::default());
                }
                cx.notify();
            }),
            cx.subscribe_in(&folder, window, |this, _, event, window, cx| match event {
                TextAreaEvent::Submit => this.submit_folder_search(window, cx),
                TextAreaEvent::Changed(_) => {
                    this.sheet_scroll.set_offset(gpui::Point::default());
                    cx.notify();
                }
                TextAreaEvent::SelectionChanged
                | TextAreaEvent::Copied
                | TextAreaEvent::Dismissed => {}
            }),
            cx.subscribe_in(&start, window, |this, _, event, window, cx| {
                this.composer_event(Target::Start, event, window, cx)
            }),
            cx.subscribe_in(&review, window, |this, _, event, window, cx| {
                if let Route::Review(id) = this.route() {
                    this.composer_event(Target::Review(id), event, window, cx)
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)),
            cx.observe_window_visibility(window, |this, visibility, _, cx| {
                this.set_visible(visibility, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.window_active = window.is_window_active();
                if !this.window_active {
                    crate::scroll::cancel_motion();
                    this.visual_apps
                        .borrow_mut()
                        .projection_deferred(this.route(), false);
                }
                // Rollback on blur and rearm visible animation on activation.
                cx.notify();
            }),
        ];
        // Tests read the screen from what each tick writes, so it stays current.
        let interval = if cfg!(feature = "ui-test") {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(1)
        };
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        let computer = prefs.computer.clone();
        let sample = prefs.sample;
        let data_dir = prefs_path.as_deref().and_then(Path::parent).map_or_else(
            || std::env::temp_dir().join("pi-android"),
            Path::to_path_buf,
        );
        #[cfg(feature = "local-backend")]
        crate::local_chat::configure(&data_dir);
        // On the phone the key is made at once, to show; elsewhere on connecting.
        let phone_key = prefs_path
            .is_some()
            .then(|| Identity::load_or_create(&data_dir.join("id_ed25519")))
            .and_then(|identity| {
                identity
                    .inspect_err(|error| log::error!("No key for SSH: {error:#}"))
                    .ok()
            })
            .map(|identity| identity.public_line());
        cx.set_global(prefs);
        let mut app = Self {
            schedules: crate::schedules::Schedules::default(),
            store: None,
            routes: vec![Route::Connect],
            sheet: None,
            swiping_session: None,
            deleting_session: None,
            jj_histories: HashMap::new(),
            restoring_history: None,
            enabling_jj: None,
            sheet_motion: SwipeMotion::at(1.),
            closing_sheet: None,
            sheet_height: Rc::new(Cell::new(px(0.))),
            start_motion: SwipeMotion::at(0.),
            start_height: Rc::new(Cell::new(px(0.))),
            image_zoom: 1.,
            image_pan: gpui::Point::default(),
            image_box: Rc::default(),
            sheet_scroll: ScrollHandle::new(),
            activity_view: None,
            expanded_turns: HashMap::new(),
            expanded_prompts: HashSet::new(),
            expanded_compactions: HashSet::new(),
            expanded_thinking: HashSet::new(),
            expanded_streams: HashSet::new(),
            expanded_calls: HashSet::new(),
            subagent_ids: HashMap::new(),
            next_subagent_session_id: u32::MAX,
            subagent_sessions: HashMap::new(),
            subagent_projected_revisions: HashMap::new(),
            subagent_composers: HashMap::new(),
            questions_later: HashSet::new(),
            seen_open: false,
            all_files: HashSet::new(),
            folded_handoffs: HashSet::new(),
            all_subagents: HashSet::new(),
            swarm_input_targets: HashMap::new(),
            next_swarm_peer: 1,
            swarm_peer_bindings: HashMap::new(),
            swarm_views: HashMap::new(),
            next_command_view: 1,
            command_views: HashMap::new(),
            next_terminal_view: 1,
            terminal_views: HashMap::new(),
            sheet_focus_pending: false,
            expanded: HashSet::new(),
            notice: None,
            notice_generation: 0,
            render_stats: RenderStats::default(),
            focus: cx.focus_handle(),
            address,
            selectable,
            inline_selectable,
            inline_selection: None,
            connect_error: None,
            connecting: false,
            connection_generation: 0,
            pairing_status: None,
            phone_key,
            key_refused: false,
            manual_setup: false,
            pairing: false,
            resuming: None,
            _retry: None,
            data_dir,
            #[cfg(feature = "local-backend")]
            local_providers: Default::default(),
            #[cfg(feature = "local-backend")]
            local_frame: 0,
            #[cfg(feature = "local-backend")]
            local_private: false,
            #[cfg(feature = "local-backend")]
            parked_local: None,
            #[cfg(feature = "local-backend")]
            parked_local_threads: HashMap::new(),
            #[cfg(feature = "local-backend")]
            parked_local_start: None,
            folder,
            reconnecting: false,
            last_listed: Instant::now(),
            last_reconnect: Instant::now(),
            _pump: None,
            search,
            model_search,
            preview_models: Vec::new(),
            preview_transport_diagnostics: HashMap::new(),
            searching: false,
            start,
            start_visible_height: None,
            project: 0,
            project_browser: ProjectBrowser::default(),
            editor_root: None,
            project_browser_editor: false,
            project_list: ListState::new(0, ListAlignment::Top, px(240.))
                .with_uniform_item_height(px(48.)),
            project_tree_rows: Vec::new(),
            project_overview_shape: None,
            project_list_offset: px(0.),
            file_view: None,
            file_tabs: Vec::new(),
            file_recoveries: crate::projects::FileRecoveryState::Idle,
            file_recovery_delete_armed: None,
            file_return_to_project: false,
            file_exit_armed: false,
            file_exit_generation: 0,
            tool_images: Default::default(),
            tool_image_demands: Default::default(),
            inline_pages: Default::default(),
            visual_apps: std::cell::RefCell::new(crate::visual_apps::VisualApps::load(
                prefs_path.as_deref(),
            )),
            expanded_visual: None,
            posters_drawing: Default::default(),
            file_generation: 0,
            project_files_generation: 0,
            threads: HashMap::new(),
            pending_compacts: HashMap::new(),
            sample_models: HashMap::new(),
            review,
            review_session: None,
            review_file: 0,
            review_call: None,
            review_lines: BTreeSet::new(),
            review_comments: Vec::new(),
            review_next_comment_id: 1,
            review_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(128.))
                .with_uniform_item_height(px(24.)),
            review_list_shape: None,
            review_list_offset: px(0.),
            scrolls: HashMap::new(),
            thread_lists: HashMap::new(),
            thread_list_shapes: HashMap::new(),
            thread_loading: HashMap::new(),
            thread_loading_release_scheduled: HashMap::new(),
            thread_loading_generation: 0,
            thread_failed_layouts: HashMap::new(),
            thread_pending_remeasure: HashSet::new(),
            thread_snapshots: HashMap::new(),
            thread_list_offsets: HashMap::new(),
            prefs_path,
            visible: window.visibility().is_visible(),
            window_active: window.is_window_active(),
            working_posted: None,
            deferred_updates: Vec::new(),
            deferred_flush_scheduled: false,
            last_tick: Instant::now(),
            paused: false,
            playing: false,
            subscriptions,
            _ticker: ticker,
        };
        app.apply_theme(window, cx);
        app.apply_return_sends(cx);
        #[cfg(feature = "local-backend")]
        let prefer_local = app.data_dir.join("local-backend-preferred").is_file();
        #[cfg(not(feature = "local-backend"))]
        let prefer_local = false;
        if prefer_local {
            #[cfg(feature = "local-backend")]
            app.select_local_backend(window, cx);
        } else if let Some(address) = computer {
            app.address
                .update(cx, |field, cx| field.set_text(address.clone(), cx));
            if sample {
                app.open_store(&address, cx);
            } else {
                app.resume(address, window, cx);
            }
        }
        window.focus(&app.focus, cx);
        app
    }

    pub fn route(&self) -> Route {
        self.routes.last().copied().unwrap_or(Route::Connect)
    }

    pub(crate) fn toggle_render_stats(&mut self, cx: &mut Context<Self>) {
        self.render_stats.visible = !self.render_stats.visible;
        self.render_stats.reset();
        if self.render_stats.visible {
            self.render_stats.refresh_display_rates(Instant::now());
        }
        cx.notify();
    }

    pub(crate) fn scroll(&mut self, route: Route) -> ScrollHandle {
        self.scrolls.entry(route).or_default().clone()
    }

    pub(crate) fn reset_swarm_views(&mut self) {
        self.swarm_views.clear();
        self.swarm_peer_bindings.clear();
        self.swarm_input_targets.clear();
        self.command_views.clear();
        self.activity_view = None;
        self.scrolls
            .retain(|route, _| !matches!(route, Route::Swarm(_, _) | Route::SwarmPeer(_, _, _)));
    }

    /// Follow content updates only while the reader is at the bottom. Doing
    /// this unconditionally in render would undo upward swipes and activity expansion.
    fn keep_latest_in_view(&mut self) {
        match self.route() {
            Route::Thread(_) => {
                let scroll = self.scroll(self.route());
                if is_scrolled_to_bottom(&scroll) {
                    scroll.scroll_to_bottom();
                }
            }
            Route::Command(_, token) => {
                if let Some(view) = self.command_views.get(&token)
                    && view.tab == crate::screens::command::CommandTab::Output
                {
                    if view.output_wrap {
                        if is_scrolled_to_bottom(&view.output_vertical) {
                            view.output_vertical.scroll_to_bottom();
                        }
                    } else if view.output_list.is_following_tail() {
                        view.output_list.scroll_to_end();
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn prefs<'a>(&self, cx: &'a App) -> &'a Prefs {
        cx.global::<Prefs>()
    }

    pub(crate) fn update_prefs(&mut self, cx: &mut Context<Self>, update: impl FnOnce(&mut Prefs)) {
        update(cx.global_mut::<Prefs>());
        // Preview fixtures freely vary appearance, models and computers. They
        // must never replace the real app state stored on the phone.
        if !self.paused
            && let Some(path) = &self.prefs_path
        {
            cx.global::<Prefs>().save(path);
        }
        cx.notify();
    }

    fn command_catalog_for_path(&self, path: &str, cx: &App) -> Option<Vec<SavedCommand>> {
        let store = self.store.as_ref()?;
        let live = store.live.as_ref()?;
        if let Some(commands) = live.commands_for_path(path) {
            return Some(commands.iter().map(SavedCommand::from).collect());
        }
        Some(
            self.prefs(cx)
                .command_catalogs
                .get(&store.computer.address)
                .and_then(|projects| projects.get(path))
                .cloned()
                .unwrap_or_default(),
        )
    }

    pub(crate) fn command_catalog_for_project(
        &self,
        project: usize,
        cx: &App,
    ) -> Option<Vec<SavedCommand>> {
        if !self.backend_supports("workspace_browse") {
            return Some(Vec::new());
        }
        let path = self.store.as_ref()?.projects.get(project)?.path.clone();
        self.command_catalog_for_path(&path, cx)
    }

    pub(crate) fn command_catalog_for_session(
        &self,
        id: SessionId,
        cx: &App,
    ) -> Option<Vec<SavedCommand>> {
        let store = self.store.as_ref()?;
        if let Some(commands) = store.commands(id) {
            return Some(commands.iter().map(SavedCommand::from).collect());
        }
        let live = store.live.as_ref()?;
        let path = live.session_cwd(id)?.to_owned();
        self.command_catalog_for_path(&path, cx)
    }

    fn compact_available_for_session(&self, id: SessionId) -> bool {
        self.store
            .as_ref()
            .is_some_and(|store| store.supports_compact(id))
    }

    pub(crate) fn compact_pending(&self, id: SessionId) -> bool {
        self.pending_compacts.contains_key(&id)
    }

    fn compact_for_session(&self, id: SessionId, draft: &str) -> Option<Option<String>> {
        let store = self.store.as_ref()?;
        if store.live.is_none() && !store.is_local() {
            return None;
        }
        let parsed = compact_instructions(draft, false)?;
        let Some(commands) = store.commands(id) else {
            // Do not turn an action into model text while its possible session
            // shadow is still unknown. Live keeps the draft and asks to wait.
            return Some(parsed);
        };
        let shadowed = commands
            .iter()
            .any(|command| command.name.trim_start_matches('/') == "compact");
        (!shadowed).then_some(parsed)
    }

    pub(crate) fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let appearance = self.prefs(cx).appearance;
        let system_dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let dark = match appearance {
            Appearance::System => system_dark,
            Appearance::Evening => true,
            Appearance::Moonstone => false,
        };
        let palette = Theme::new(dark);
        cx.set_global(palette);
        crate::theme::sync_terminal_theme(palette, cx);
        activity::set_bar_icons(match appearance {
            Appearance::System => activity::BarIcons::System,
            _ if dark => activity::BarIcons::Light,
            _ => activity::BarIcons::Dark,
        });
        self.recolor_file_editors(cx);
        cx.notify();
    }

    pub(crate) fn apply_return_sends(&mut self, cx: &mut Context<Self>) {
        let sends = self.prefs(cx).return_sends;
        let composers = [self.start.clone(), self.review.clone()]
            .into_iter()
            .chain(self.threads.values().cloned())
            .chain(self.subagent_composers.values().cloned());
        for composer in composers {
            let area = composer.read(cx).area.clone();
            area.update(cx, |area, _| area.enter_sends = sends);
        }
    }

    // Navigation

    pub(crate) fn push(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        self.routes.push(route);
        self.entered(window, cx);
    }

    /// Replaces navigation while deterministically closing terminals excluded
    /// by the replacement (for example when changing stores or following a
    /// direct link). Ordinary Back deliberately does not use this cleanup.
    fn replace_routes(&mut self, routes: Vec<Route>, cx: &mut Context<Self>) {
        let retained: HashSet<u64> = routes
            .iter()
            .filter_map(|route| match route {
                Route::Terminal(_, token) => Some(*token),
                _ => None,
            })
            .collect();
        let removed: Vec<u64> = self
            .terminal_views
            .keys()
            .copied()
            .filter(|token| !retained.contains(token))
            .collect();
        self.routes = routes;
        for token in removed {
            self.close_terminal_view(token, cx);
        }
    }

    /// Goes to a session from anywhere, with the list behind it.
    pub(crate) fn show_session(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_routes(vec![Route::Sessions], cx);
        self.push_session(id, window, cx);
    }

    /// Opens an ordinary session while preserving the caller's return stack
    /// (for example, schedule history). Session setup must match the list opener.
    pub(crate) fn push_session(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(store) = &mut self.store {
            store.watch(id);
        }
        if !self.scrolls.contains_key(&Route::Thread(id)) {
            self.scroll(Route::Thread(id)).scroll_to_bottom();
        }
        self.push(Route::Thread(id), window, cx);
    }

    /// Tells the connection which session is on screen: idle ones elsewhere
    /// let go of their channel.
    fn show_live(&mut self) {
        let shown = match self.route() {
            Route::Thread(id)
            | Route::Command(id, _)
            | Route::Terminal(id, _)
            | Route::Review(id)
            | Route::History(id)
            | Route::Diagnostics(id)
            | Route::Subagents(id)
            | Route::Swarm(id, _)
            | Route::SwarmPeer(id, _, _)
            | Route::Subagent(id, _) => Some(id),
            _ => None,
        };
        if let Some(store) = &mut self.store {
            store.show(shown);
        }
    }

    fn entered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inline_selection = None;
        self.show_live();
        if let Route::Thread(id) = self.route() {
            self.begin_thread_loading(id);
        }
        self.swiping_session = None;
        self.close_sheet(cx);
        self.searching = false;
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        if let Route::Thread(id)
        | Route::Command(id, _)
        | Route::Terminal(id, _)
        | Route::Review(id)
        | Route::History(id)
        | Route::Diagnostics(id)
        | Route::Subagents(id)
        | Route::Swarm(id, _)
        | Route::SwarmPeer(id, _, _)
        | Route::Subagent(id, _) = self.route()
        {
            activity::cancel_notification(alerts::question_id(id));
            activity::cancel_notification(alerts::finished_id(id));
            // A question waiting in the session takes the composer's place.
            self.questions_later.remove(&id);
            self.note_seen(id, cx);
        }
        cx.notify();
    }

    /// You have looked at a session as it is now: what ended there leaves
    /// Home's Needs you.
    pub(crate) fn note_seen(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let Some(store) = &mut self.store else {
            return;
        };
        if store.mark_seen(id) && !store.is_sample() && !store.is_local() {
            self.save_seen(cx);
        }
    }

    /// Keeps what you have seen on this computer for the next time the app opens.
    fn save_seen(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &mut self.store else {
            return;
        };
        let mut seen = crate::prefs::Seen {
            since: store.seen_since,
            sessions: store
                .seen
                .iter()
                .map(|(key, at)| (key.clone(), *at))
                .collect(),
        };
        seen.trim();
        store.seen_since = seen.since;
        store.seen.retain(|key, _| seen.sessions.contains_key(key));
        let address = store.computer.address.clone();
        self.update_prefs(cx, |prefs| {
            prefs.seen.insert(address, seen);
        });
    }

    pub(crate) fn open_sheet(&mut self, sheet: Sheet, cx: &mut Context<Self>) {
        const RESOURCES: &str = "resources";
        let required = match sheet {
            Sheet::Resources => Some(RESOURCES),
            Sheet::Project | Sheet::EditorRoot | Sheet::EditorFiles => Some("workspace_browse"),
            Sheet::RestoreHistory(_, _) | Sheet::EnableJj(_) => Some("file_history"),
            _ => None,
        };
        if required.is_some_and(|feature| !self.backend_supports(feature)) {
            self.notify_user("This backend does not support that feature", cx);
            return;
        }
        self.inline_selection = None;
        self.closing_sheet = None;
        // A different sheet can be much taller than the last measured one. Until
        // this panel is laid out, use the viewport fallback so its first frame is
        // completely offscreen instead of flashing stale content into view.
        self.sheet_height.set(px(0.));
        self.sheet_motion.settle(0.);
        self.sheet_scroll = ScrollHandle::new();
        if !matches!(sheet, Sheet::Activity(..)) {
            self.activity_view = None;
        }
        if matches!(sheet, Sheet::Details(_)) {
            self.expanded.remove("details-rail");
            self.expanded.remove("details-context");
        }
        if sheet == Sheet::Model {
            self.model_search
                .update(cx, |area, cx| area.set_text("", cx));
        }
        self.sheet = Some(sheet);
        // The attachment chooser has only two rows; animating it used to
        // rebuild the potentially huge route underneath for dozens of frames.
        if matches!(sheet, Sheet::Attach(_)) {
            self.sheet_motion.finish();
        }
        self.image_zoom = 1.;
        self.image_pan = gpui::Point::default();
        match sheet {
            Sheet::Project => {
                if self.project_browser_editor {
                    self.project_browser.clear();
                }
                self.project_browser_editor = false;
                self.open_project_browser(false, cx);
            }
            Sheet::EditorRoot => {
                self.project_browser.clear();
                self.project_browser_editor = true;
                self.folder.update(cx, |area, cx| area.set_text("", cx));
                self.go_to_folder(self.home_folder(), cx);
            }
            Sheet::EditorFiles => {
                let Some(root) = self.editor_root.clone() else {
                    self.notify_user("Choose an editor folder first", cx);
                    self.sheet = None;
                    return;
                };
                if !self.project_browser_editor
                    || self.project_browser.root.as_deref() != Some(root.as_str())
                {
                    self.project_browser.clear();
                    self.project_browser_editor = true;
                    self.project_browser.root = Some(root.clone());
                    self.load_editor_tree(root, cx);
                }
                self.folder.update(cx, |area, cx| area.set_text("", cx));
            }
            _ => {}
        }
        self.sheet_focus_pending = true;
        cx.notify();
    }

    pub(crate) fn close_sheet(&mut self, cx: &mut Context<Self>) {
        if let Some(sheet) = self.sheet.take() {
            self.closing_sheet = Some(sheet);
            self.sheet_motion.settle(1.);
            if matches!(sheet, Sheet::Attach(_)) {
                self.sheet_motion.finish();
            }
        }
        self.sheet_focus_pending = true;
        cx.notify();
    }

    fn animate_panels(&mut self, window: &Window, cx: &Context<Self>) {
        let now = Instant::now();
        let reduced = cx.reduce_motion();
        self.sheet_motion.tick(now, reduced);
        self.start_motion.tick(now, reduced);
        if self.sheet.is_none() && !self.sheet_motion.animating() {
            if matches!(self.closing_sheet, Some(Sheet::Activity(..))) {
                self.activity_view = None;
            }
            self.closing_sheet = None;
        }
        if let Some((_, motion)) = &mut self.swiping_session {
            motion.tick(now, reduced);
            if !motion.dragging() && !motion.animating() {
                self.swiping_session = None;
            }
        }
        if self.sheet_motion.animating()
            || self.start_motion.animating()
            || self
                .swiping_session
                .as_ref()
                .is_some_and(|(_, motion)| motion.animating())
        {
            window.request_animation_frame();
        }
    }

    /// A flick that closes a panel leaves GPUI flinging; the next touch would
    /// only catch the fling and never tap. A touch off screen catches it now.
    pub(crate) fn stop_fling(window: &mut Window) {
        window.on_next_frame(|window, cx| {
            let touch = |phase| {
                gpui::PlatformInput::Touch(gpui::TouchEvent {
                    id: gpui::TouchId(u64::MAX),
                    phase,
                    position: gpui::point(px(-10_000.), px(-10_000.)),
                    predicted_position: None,
                    force: None,
                })
            };
            window.dispatch_event(touch(gpui::TouchPhase::Started), cx);
            window.dispatch_event(touch(gpui::TouchPhase::Cancelled), cx);
        });
    }

    /// Capture dismissal for the gesture's lifetime, not the moving panel's
    /// hitbox. Moving a panel away from the starting finger must not lose the
    /// remaining events. Sheet content gets normal scrolling until its top.
    pub(crate) fn dismiss_gesture(&self, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let sheet_height = self.sheet_height.clone();
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                sheet_height.set(bounds.size.height);
                window.on_mouse_event(move |event: &gpui::ScrollWheelEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Capture {
                        return;
                    }
                    let _ = view.update(cx, |this, cx| {
                        if this.sheet.is_none() {
                            return;
                        }
                        let delta = event.delta.pixel_delta(px(20.));
                        let (along, across) = (delta.y, delta.x);
                        let shown_sheet = this.sheet.or(this.closing_sheet);
                        let project_list = matches!(
                            shown_sheet,
                            Some(Sheet::Project | Sheet::EditorRoot | Sheet::EditorFiles)
                        ) && this.folder.read(cx).text().trim().is_empty();
                        let activity_list = matches!(shown_sheet, Some(Sheet::Activity(..)))
                            .then(|| this.activity_view.as_ref())
                            .flatten();
                        let (scroll_max, scroll_bounds, scroll_offset) = if project_list {
                            (
                                this.project_list.max_offset_for_scrollbar().y,
                                this.project_list.viewport_bounds(),
                                this.project_list.scroll_px_offset_for_scrollbar().y,
                            )
                        } else if let Some(activity) = activity_list {
                            (
                                activity.list.max_offset_for_scrollbar().y,
                                activity.list.viewport_bounds(),
                                activity.list.scroll_px_offset_for_scrollbar().y,
                            )
                        } else {
                            (
                                this.sheet_scroll.max_offset().y,
                                this.sheet_scroll.bounds(),
                                this.sheet_scroll.offset().y,
                            )
                        };
                        let motion = &mut this.sheet_motion;
                        let extent = bounds.size.height.max(px(1.));
                        if event.touch_phase == gpui::TouchPhase::Started {
                            if !bounds.contains(&event.position)
                                || (this.image_zoom > 1.01
                                    && this.image_box.get().contains(&event.position))
                                || along <= px(0.)
                                || along.abs() < across.abs()
                                || (scroll_max > px(1.)
                                    && scroll_bounds.contains(&event.position)
                                    && event.position.x >= scroll_bounds.right() - px(18.))
                                || (event.position.y > bounds.top() + px(36.)
                                    && scroll_offset < px(-1.))
                            {
                                return;
                            }
                            motion.begin_drag();
                        }
                        if !motion.dragging() {
                            return;
                        }
                        cx.stop_propagation();
                        // A flick's last sample barely moves; judge its speed before it.
                        let flung = motion.flung();
                        motion.drag_by(along / extent);
                        if event.touch_phase == gpui::TouchPhase::Cancelled {
                            motion.settle(0.);
                        } else if event.touch_phase == gpui::TouchPhase::Ended {
                            let close = flung
                                || motion.flung()
                                || extent * motion.position() > px(85.).min(extent * 0.4);
                            if close {
                                Self::stop_fling(window);
                                this.close_sheet(cx);
                            } else {
                                motion.settle(0.);
                            }
                        }
                        cx.notify();
                    });
                });
            },
        )
        .absolute()
        .inset_0()
    }

    /// One step back; false when there is nothing to go back to.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.inline_selection.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        #[cfg(feature = "local-backend")]
        if self.local_providers.visible {
            self.close_provider_accounts();
            self.tick_provider_accounts(cx);
            cx.notify();
            return true;
        }
        if self.sheet.is_some() {
            self.close_sheet(cx);
            window.dismiss_virtual_keyboard();
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        if self.close_expanded_visual(window, cx) {
            return true;
        }
        if self.inline_pages.borrow_mut().expanded.take().is_some() {
            window.dismiss_virtual_keyboard();
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        if self.searching {
            self.searching = false;
            self.search.update(cx, |search, cx| {
                search.take(cx);
            });
            window.dismiss_virtual_keyboard();
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        if self.route() == Route::Connect && self.manual_setup {
            self.manual_setup = false;
            window.dismiss_virtual_keyboard();
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        // From the pairing steps back to where Pi should work.
        if self.route() == Route::Connect && self.pairing && self.resuming.is_none() {
            self.pairing = false;
            cx.notify();
            return true;
        }
        if self.route() == Route::File
            && !self.file_exit_armed
            && self.editor_workspace_needs_attention(cx)
        {
            self.file_exit_armed = true;
            self.file_exit_generation += 1;
            let generation = self.file_exit_generation;
            self.notify_user(
                "Unsaved or unconfirmed file changes remain. Tap Back again to return; this workspace will stay open.",
                cx,
            );
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(4)).await;
                this.update(cx, |this, _| {
                    if this.file_exit_generation == generation {
                        this.file_exit_armed = false;
                    }
                })
                .ok();
            })
            .detach();
            return true;
        }
        if self.routes.len() > 1 {
            let left = self.routes.pop();
            if left == Some(Route::ScheduleConfirm) {
                self.schedules.confirmation = None;
                self.schedules.consent = false;
            }
            if let Some(Route::Subagent(id, _) | Route::SwarmPeer(id, _, _)) = left {
                self.follow_subagent(id, None);
            }
            if let Some(route @ Route::SwarmPeer(_, _, token)) = left {
                self.swarm_peer_bindings.remove(&token);
                self.scrolls.remove(&route);
            }
            if let Some(Route::Command(_, token)) = left {
                self.command_views.remove(&token);
            }
            // A terminal is session-owned rather than route-owned. Popping its
            // screen hides it while its shell, scrollback and zoom/pan state
            // remain available from the session's Open terminal action.
            self.show_live();
            if let Route::Thread(id) = self.route() {
                self.begin_thread_loading(id);
            }
            if left == Some(Route::File) {
                if std::mem::take(&mut self.file_return_to_project) {
                    self.open_sheet(Sheet::Project, cx);
                }
                self.file_exit_armed = false;
            }
            if !matches!(self.route(), Route::Review(_)) {
                self.review_call = None;
                self.review_lines.clear();
                if matches!(left, Some(Route::Review(_))) {
                    self.review
                        .update(cx, |review, cx| review.set_lines(None, cx));
                    self.review_list_shape = None;
                }
            }
            window.dismiss_virtual_keyboard();
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        false
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        if !self.back(window, cx) {
            cx.propagate();
        }
    }

    pub(crate) fn notify_user(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        let auto_dismiss = text.chars().count() <= 80;
        self.notice = Some(text);
        self.notice_generation += 1;
        let generation = self.notice_generation;
        if auto_dismiss {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                this.update(cx, |this, cx| {
                    if this.notice_generation == generation {
                        this.notice = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    // Connecting

    /// The sample sessions, for trying the app without a computer.
    pub(crate) fn open_store(&mut self, address: &str, cx: &mut Context<Self>) -> bool {
        self.install_store(Store::sample(Computer::from_address(address)), cx)
    }
    pub(crate) fn install_store(&mut self, store: Store, cx: &mut Context<Self>) -> bool {
        self.replace_store(Some(store), cx)
    }
    pub(crate) fn preserve_local_drafts(&mut self, cx: &mut Context<Self>) {
        #[cfg(feature = "local-backend")]
        if self.is_local() {
            // Preserve the command before parking its cleared composer. Parked
            // backend completions have no active composer/notification target.
            for (_, (target, draft)) in std::mem::take(&mut self.pending_compacts) {
                self.restore_compact_draft(target, &draft, cx);
            }
            self.parked_local_start = Some(self.start.update(cx, |c, cx| {
                c.cancel_imports(cx);
                (c.area.read(cx).text().to_owned(), c.attachments().to_vec())
            }));
            for composer in self.threads.values() {
                composer.update(cx, |c, cx| c.cancel_imports(cx));
            }
        }
        #[cfg(not(feature = "local-backend"))]
        let _ = cx;
    }
    pub(crate) fn replace_store(&mut self, store: Option<Store>, cx: &mut Context<Self>) -> bool {
        let retaining_editor = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref().map(|_| store.computer.address.as_str()))
            .zip(
                store
                    .as_ref()
                    .and_then(|store| store.live.as_ref().map(|_| store.computer.address.as_str())),
            )
            .is_some_and(|(current, next)| current == next);
        if !retaining_editor && self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before changing computers",
                cx,
            );
            return false;
        }
        let retained_editor = retaining_editor.then(|| {
            (
                self.editor_root.take(),
                self.file_view.take(),
                std::mem::take(&mut self.file_tabs),
            )
        });

        #[cfg(feature = "local-backend")]
        if store.as_ref().is_some_and(|store| store.local.is_none()) {
            let _ = std::fs::remove_file(self.data_dir.join("local-backend-preferred"));
        }
        #[cfg(feature = "local-backend")]
        if self.is_local()
            && let Some(mut previous) = self.store.take()
        {
            previous.retire_generation();
            let ids: Vec<_> = previous.sessions.iter().map(|s| s.id).collect();
            self.parked_local_threads = ids
                .into_iter()
                .filter_map(|id| self.threads.remove(&id).map(|c| (id, c)))
                .collect();
            self.parked_local = Some(previous);
        }
        self.reconnecting = false;
        // Connected: the next first-run screen starts from where Pi should work.
        self.pairing = false;
        self.reset_inline_pages();
        self.reset_swarm_views();
        self.schedules.invalidate();
        self.project_browser.clear();
        self.project_browser_editor = false;
        self.editor_root = None;
        self.project_files_generation += 1;
        self.file_generation += 1;
        self.file_view = None;
        self.file_tabs.clear();
        self.file_return_to_project = false;
        self.file_exit_armed = false;
        self.project = 0;
        self.deferred_updates.clear();
        self.replace_routes(
            vec![if store.is_some() {
                Route::Sessions
            } else {
                Route::Connect
            }],
            cx,
        );
        self.store = store;
        self.review_session = None;
        self.review_lines.clear();
        self.review_comments.clear();
        self.review_list_shape = None;
        self.threads.clear();
        self.pending_compacts.clear();
        self.sample_models.clear();
        self.preview_transport_diagnostics.clear();
        self.jj_histories.clear();
        self.restoring_history = None;
        self.enabling_jj = None;
        self.swiping_session = None;
        self.deleting_session = None;
        self.scrolls.clear();
        self.thread_lists.clear();
        self.thread_list_shapes.clear();
        self.thread_loading.clear();
        self.thread_loading_release_scheduled.clear();
        self.thread_failed_layouts.clear();
        self.thread_pending_remeasure.clear();
        self.thread_snapshots.clear();
        self.thread_list_offsets.clear();
        // Child projections and drafts are scoped to the old store. Parent and
        // synthetic IDs can be reused by a different computer or Local store.
        self.subagent_ids.clear();
        self.subagent_sessions.clear();
        self.subagent_projected_revisions.clear();
        self.subagent_composers.clear();
        self._pump = None;

        if let Some((root, active, tabs)) = retained_editor {
            self.editor_root = root;
            self.file_view = active;
            self.file_tabs = tabs;
            if let Some(root) = self.editor_root.clone() {
                self.project_browser_editor = true;
                self.project_browser.root = Some(root.clone());
                self.load_editor_tree(root, cx);
            }
        }
        true
    }

    pub(crate) fn open_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before opening the samples",
                cx,
            );
            return;
        }
        self.preserve_local_drafts(cx);
        if self.is_local() {
            self.start.update(cx, |start, cx| start.clear(cx));
        }
        self.cancel_connection_attempt();
        activity::cancel_notification(alerts::HELPER_UPDATE_ID);
        self.paused = false;
        let typed = self.address.read(cx).text().trim().to_owned();
        let address = if typed.is_empty() {
            "you@studio-mac.local".to_owned()
        } else {
            typed
        };
        if !self.open_store(&address, cx) {
            return;
        }
        self.start.update(cx, |start, _| {
            start.set_workspace(true);
            start.use_files(None);
            start.use_commands(None);
        });
        self.update_prefs(cx, |prefs| {
            prefs.computer = Some(address.clone());
            prefs.sample = true;
        });
        self.entered(window, cx);
    }

    fn identity(&mut self) -> anyhow::Result<Identity> {
        let identity = Identity::load_or_create(&self.data_dir.join("id_ed25519"))?;
        self.phone_key = Some(identity.public_line());
        Ok(identity)
    }

    fn begin_connection_attempt(&mut self) -> u64 {
        self.schedules.invalidate();
        self.connection_generation = self.connection_generation.wrapping_add(1);
        self.connecting = true;
        self.connection_generation
    }

    /// Cached screens must not present an in-flight reconnect as fresh state.
    pub(crate) fn is_reconnecting(&self) -> bool {
        self.reconnecting
    }

    /// Retires an in-flight connection without relying on its transport being cancellable.
    /// Its task may finish, but the generation check prevents stale navigation or state.
    pub(crate) fn cancel_connection_attempt(&mut self) {
        self.schedules.invalidate();
        self.connection_generation = self.connection_generation.wrapping_add(1);
        self.connecting = false;
        self.reconnecting = false;
        self.pairing_status = None;
        self.stop_resuming();
    }

    fn stop_resuming(&mut self) {
        self.resuming = None;
        self._retry = None;
    }

    pub(crate) fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        let entered_address = self.address.read(cx).text().trim().to_owned();
        let address = match Address::parse(&entered_address) {
            Ok(address) => address,
            Err(error) => {
                self.connect_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let retaining_editor = self.store.as_ref().is_some_and(|store| {
            store.live.is_some() && address.matches_alias(&store.computer.address)
        });
        if !retaining_editor && self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before changing computers",
                cx,
            );
            return;
        }
        let identity = match self.identity() {
            Ok(identity) => identity,
            Err(error) => {
                self.connect_error = Some(format!("{error:#}").into());
                cx.notify();
                return;
            }
        };
        let legacy_fallback = self
            .resuming
            .as_ref()
            .filter(|resume| resume.address.trim() == entered_address)
            .and_then(|_| Address::legacy_port_candidate(&entered_address))
            .filter(|fallback| fallback != &address);
        let known = self
            .prefs(cx)
            .host_keys
            .iter()
            .find(|(saved, _)| {
                address.matches_alias(saved)
                    || legacy_fallback
                        .as_ref()
                        .is_some_and(|fallback| fallback.matches_alias(saved))
            })
            .map(|(_, key)| key.clone());
        let generation = self.begin_connection_attempt();
        self.pairing_status = None;
        self.connect_error = None;
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let reached = async {
                let (address, connection, helper) = open_remote_helper_with_legacy_fallback(
                    address,
                    legacy_fallback,
                    identity,
                    known,
                )
                .await?;
                let listed = if helper.can_list_sessions() {
                    remote::sessions(&connection, &helper).await?
                } else {
                    Vec::new()
                };
                anyhow::Ok((address, connection, helper, listed))
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                if this.connection_generation != generation {
                    return;
                }
                this.connecting = false;
                match reached {
                    Ok((address, connection, helper, listed)) => {
                        this.connected(address, connection, helper, listed, window, cx)
                    }
                    Err(error) => {
                        this.key_refused =
                            matches!(error.downcast_ref(), Some(ssh::Failure::KeyRefused));
                        this.connect_error = Some(format!("{error:#}").into());
                        if this.route() != Route::Connect {
                            this.routes = vec![Route::Connect];
                        }
                        // A refused key or a changed host key needs the person,
                        // not another try.
                        let lasting = matches!(
                            error.downcast_ref(),
                            Some(ssh::Failure::KeyRefused | ssh::Failure::HostKeyChanged { .. })
                        );
                        if !lasting {
                            this.schedule_resume(window, cx);
                        }
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Connects to the saved computer, trying again until it answers.
    fn resume(&mut self, address: String, window: &mut Window, cx: &mut Context<Self>) {
        self.resuming = Some(Resume {
            address,
            attempt: 0,
            retry_at: None,
        });
        self.retry_resume(window, cx);
    }

    /// Tries the saved computer again now.
    pub(crate) fn retry_resume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        let Some(resume) = &mut self.resuming else {
            return;
        };
        resume.attempt += 1;
        resume.retry_at = None;
        let address = resume.address.clone();
        self._retry = None;
        self.address
            .update(cx, |field, cx| field.set_text(address, cx));
        self.connect(window, cx);
    }

    /// After a failed try, the next one, sooner at first: the phone's network
    /// or VPN is often still coming up when the app opens.
    fn schedule_resume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(resume) = &mut self.resuming else {
            return;
        };
        let delay = Duration::from_secs(match resume.attempt {
            0 | 1 => 5,
            2 => 10,
            3 => 20,
            _ => 30,
        });
        resume.retry_at = Some(Instant::now() + delay);
        self._retry = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update_in(cx, |this, window, cx| this.retry_resume(window, cx))
                .ok();
        }));
    }

    /// Stops getting back to the saved computer and shows setup, to pair
    /// another. The saved one stays until another connects.
    pub(crate) fn leave_resume(&mut self, cx: &mut Context<Self>) {
        self.cancel_connection_attempt();
        self.connect_error = None;
        self.key_refused = false;
        self.manual_setup = false;
        // To pair another computer, straight to its steps.
        self.pairing = true;
        cx.notify();
    }

    /// Opens the dedicated camera scanner. Its result comes back through the
    /// same URL path as a QR opened by Android's system camera.
    pub(crate) fn scan_computer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        self.connect_error = None;
        if !activity::scan_qr() {
            self.connect_error = Some("This device couldn't open the QR scanner".into());
        }
        cx.notify();
    }

    fn pair(
        &mut self,
        offer: pi_core::pairing::Offer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A scanned QR is for setting up, so it shows there, error and all.
        self.cancel_connection_attempt();
        if let Err(error) = offer.check_time() {
            self.connect_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        let address = Address::from_parts(offer.user.clone(), offer.hosts[0].clone(), offer.port);
        let retaining_editor = self.store.as_ref().is_some_and(|store| {
            store.live.is_some() && store.computer.address == address.to_string()
        });
        if !retaining_editor && self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before changing computers",
                cx,
            );
            return;
        }
        let identity = match self.identity() {
            Ok(identity) => identity,
            Err(error) => {
                self.connect_error = Some(format!("{error:#}").into());
                cx.notify();
                return;
            }
        };
        let public_key = identity.public_line();
        let code = pi_core::pairing::confirmation_code(&offer.id, &public_key);
        self.address
            .update(cx, |field, cx| field.set_text(address.to_string(), cx));
        let generation = self.begin_connection_attempt();
        self.key_refused = false;
        self.connect_error = None;
        // A code from the system camera too: its progress is on the pairing step.
        self.pairing = true;
        self.pairing_status = Some(format!("Confirm {code} on the computer").into());
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        cx.notify();

        let device_name = activity::device_name().unwrap_or_else(|| "Android phone".into());
        cx.spawn_in(window, async move |this, cx| {
            let reached = crate::pairing::enroll(offer, identity, public_key, device_name).await;
            this.update_in(cx, |this, window, cx| {
                if this.connection_generation != generation {
                    return;
                }
                this.connecting = false;
                match reached {
                    Ok((address, connection, helper, listed)) => {
                        this.pairing_status = Some("Paired securely".into());
                        this.connected(address, connection, helper, listed, window, cx);
                    }
                    Err(error) => {
                        this.pairing_status = None;
                        this.connect_error = Some(format!("{error:#}").into());
                        this.routes = vec![Route::Connect];
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn connected(
        &mut self,
        address: Address,
        connection: Connection,
        helper: remote::Helper,
        listed: Vec<remote::Listed>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pairing_status = None;
        self.stop_resuming();
        let canonical_address = address.to_string();
        // Migrate every equivalent authenticated spelling together. Host-key
        // trust, seen state, projects, and command catalogs must not split when
        // IPv6 gains brackets or normalizes its numeric spelling.
        let aliases: BTreeSet<String> = {
            let prefs = self.prefs(cx);
            prefs
                .host_keys
                .keys()
                .chain(prefs.seen.keys())
                .chain(prefs.projects.keys())
                .chain(prefs.command_catalogs.keys())
                .filter(|saved| saved.as_str() != canonical_address && address.matches_alias(saved))
                .cloned()
                .collect()
        };
        if !aliases.is_empty() {
            self.update_prefs(cx, |prefs| {
                for alias in &aliases {
                    if let Some(key) = prefs.host_keys.remove(alias) {
                        prefs
                            .host_keys
                            .entry(canonical_address.clone())
                            .or_insert(key);
                    }
                    if let Some(seen) = prefs.seen.remove(alias) {
                        prefs.seen.entry(canonical_address.clone()).or_insert(seen);
                    }
                    if let Some(project) = prefs.projects.remove(alias) {
                        prefs
                            .projects
                            .entry(canonical_address.clone())
                            .or_insert(project);
                    }
                    if let Some(catalogs) = prefs.command_catalogs.remove(alias) {
                        let current = prefs
                            .command_catalogs
                            .entry(canonical_address.clone())
                            .or_default();
                        for (project, commands) in catalogs {
                            current.entry(project).or_insert(commands);
                        }
                    }
                }
                if prefs
                    .computer
                    .as_ref()
                    .is_some_and(|saved| address.matches_alias(saved))
                {
                    prefs.computer = Some(canonical_address.clone());
                }
            });
        }
        let fingerprint = connection.fingerprint.clone();
        let prefs = self.prefs(cx);
        let wanted = prefs
            .model_provider
            .as_ref()
            .zip(prefs.model_id.as_ref())
            .map(|(provider, id)| format!("{provider}/{id}"))
            .unwrap_or_else(|| prefs.model.clone());
        let helper_release = helper
            .release
            .as_ref()
            .map(|release| format!("Helper {release}"));
        let mut live = Live::new(connection, helper, address.to_string(), wanted);
        live.thinking = prefs.thinking.clone();
        if live.helper.can_attach() {
            live.refresh_models();
            live.refresh_commands();
        }
        let mut computer = Computer::from_address(&address.to_string());
        computer.pi_version = helper_release;
        let mut store = Store::live(computer, live);
        let first_look = match prefs.seen.get(&store.computer.address) {
            Some(seen) => {
                store.seen = seen.sessions.clone().into_iter().collect();
                store.seen_since = seen.since;
                false
            }
            None => true,
        };
        store.apply(vec![(None, Update::Listed(Ok(listed)))]);
        let project = prefs
            .projects
            .get(&address.to_string())
            .map(|path| store.add_project(path))
            .unwrap_or(0);
        self.preserve_local_drafts(cx);
        if self.is_local() {
            self.start.update(cx, |start, cx| start.clear(cx));
        }
        if !self.install_store(store, cx) {
            return;
        }
        // Sessions that end while the phone is away are new to you then.
        if first_look {
            self.save_seen(cx);
        }
        self.project = project;
        self.replace_routes(vec![Route::Sessions, Route::Projects], cx);
        self.threads.clear();
        self.pending_compacts.clear();
        self.jj_histories.clear();
        self.restoring_history = None;
        self.enabling_jj = None;
        self.start.update(cx, |start, _| {
            start.set_workspace(true);
            start.use_files(Some(Vec::new()));
            start.use_commands(Some(Vec::new()));
        });
        self.key_refused = false;
        self.last_listed = Instant::now();
        self.update_prefs(cx, |prefs| {
            prefs.computer = Some(canonical_address.clone());
            prefs.sample = false;
            prefs
                .host_keys
                .insert(canonical_address.clone(), fingerprint);
        });
        self.load_project_files(self.project, cx);
        self.start_pump(cx);
        activity::request_notification_permission();
        self.update_helper_notification(cx);
        self.entered(window, cx);
        self.open_project_browser(true, cx);
    }

    /// Carries the computer's updates into the store as they come.
    fn start_pump(&mut self, cx: &mut Context<Self>) {
        let Some(updates) = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .map(|live| live.updates.clone())
        else {
            return;
        };
        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Ok(first) = updates.recv().await {
                // Lexical guard ends before the next recv().await.
                let _trace = gpui_android::trace::scope(c"pi.app.pump");
                let mut batch = vec![first];
                while let Ok(more) = updates.try_recv() {
                    batch.push(more);
                }
                if this
                    .update(cx, |this, cx| this.take_updates(batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn take_updates(&mut self, batch: Vec<(Option<SessionId>, Update)>, cx: &mut Context<Self>) {
        // Once a queue exists, every later batch joins it even if motion has
        // just ended. The one flush task then preserves protocol order.
        if self.projection_deferred() || self.deferred_flush_scheduled {
            self.deferred_updates.extend(batch);
            if !self.deferred_flush_scheduled {
                self.deferred_flush_scheduled = true;
                cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(50))
                            .await;
                        let Ok(done) = this.update(cx, |this, cx| {
                            if this.projection_deferred() {
                                return false;
                            }
                            this.deferred_flush_scheduled = false;
                            let batch = std::mem::take(&mut this.deferred_updates);
                            if !batch.is_empty() {
                                this.apply_updates(batch, cx);
                            }
                            true
                        }) else {
                            break;
                        };
                        if done {
                            break;
                        }
                    }
                })
                .detach();
            }
            return;
        }
        self.apply_updates(batch, cx);
    }

    fn apply_updates(&mut self, batch: Vec<(Option<SessionId>, Update)>, cx: &mut Context<Self>) {
        let _trace = gpui_android::trace::scope(c"pi.app.updates");
        self.keep_latest_in_view();
        let global_commands = batch
            .iter()
            .any(|(id, update)| id.is_none() && matches!(update, Update::CommandCatalog(_, Ok(_))));
        let command_sessions: HashSet<SessionId> = batch
            .iter()
            .filter_map(|(id, update)| {
                (*id).filter(|_| {
                    matches!(update, Update::Record(_, record) if record["type"] == "response" && record["command"] == "get_commands")
                })
            })
            .collect();
        let compact_responses: Vec<(SessionId, bool)> = batch
            .iter()
            .filter_map(|(id, update)| {
                let id = (*id)?;
                match update {
                    Update::Record(_, record)
                        if record["type"] == "response" && record["command"] == "compact" =>
                    {
                        Some((id, record["success"] == true))
                    }
                    _ => None,
                }
            })
            .collect();
        let ended_sessions: HashSet<SessionId> = batch
            .iter()
            .filter_map(|(id, update)| matches!(update, Update::Ended(_, _)).then_some((*id)?))
            .collect();
        let Some(store) = &mut self.store else {
            return;
        };
        let events = store.apply(batch);
        for event in events {
            self.alert(event, cx);
        }
        let catalogs = self.store.as_ref().and_then(|store| {
            let live = store.live.as_ref()?;
            let mut catalogs: HashMap<String, Vec<SavedCommand>> = HashMap::new();
            if global_commands && let Some(project) = store.projects.get(self.project) {
                catalogs.insert(
                    project.path.clone(),
                    live.commands.iter().map(SavedCommand::from).collect(),
                );
            }
            for id in &command_sessions {
                if let (Some(path), Some(commands)) = (live.session_cwd(*id), live.commands(*id)) {
                    catalogs.insert(
                        path.to_owned(),
                        commands.iter().map(SavedCommand::from).collect(),
                    );
                }
            }
            Some((store.computer.address.clone(), catalogs))
        });
        if let Some((computer, catalogs)) = catalogs
            && !catalogs.is_empty()
        {
            self.update_prefs(cx, |prefs| {
                prefs
                    .command_catalogs
                    .entry(computer)
                    .or_default()
                    .extend(catalogs);
            });
        }
        let refresh_commands: HashSet<SessionId> = if global_commands {
            self.threads.keys().copied().collect()
        } else {
            command_sessions
        };
        for id in refresh_commands {
            let commands = self.command_catalog_for_session(id, cx);
            let compact = self.compact_available_for_session(id);
            if let Some(composer) = self.threads.get(&id) {
                composer.update(cx, |composer, _| {
                    composer.use_commands(commands);
                    composer.set_session_commands(compact);
                });
            }
        }
        for (id, success) in compact_responses {
            let Some((target, draft)) = self.pending_compacts.remove(&id) else {
                continue;
            };
            if !success {
                self.restore_compact_draft(target, &draft, cx);
            }
        }
        for id in ended_sessions {
            if let Some((target, draft)) = self.pending_compacts.remove(&id) {
                self.restore_compact_draft(target, &draft, cx);
            }
        }
        self.update_working_notification(cx);
        cx.notify();
    }

    /// Connects again after the connection dropped; watched sessions attach again.
    fn reconnect(&mut self, cx: &mut Context<Self>) {
        let Some(address) = self
            .store
            .as_ref()
            .and_then(|store| Address::parse(&store.computer.address).ok())
        else {
            return;
        };
        let Ok(identity) = self.identity() else {
            return;
        };
        let host = address.to_string();
        let generation = self.connection_generation;
        let known = self.prefs(cx).host_keys.get(&host).cloned();
        self.reconnecting = true;
        self.last_reconnect = Instant::now();
        cx.spawn(async move |this, cx| {
            let reached = async {
                let connection = Connection::open(address, identity, known).await?;
                let helper = remote::find(&connection).await?;
                anyhow::Ok((connection, helper))
            }
            .await;
            this.update(cx, |this, cx| {
                if this.connection_generation != generation
                    || this
                        .store
                        .as_ref()
                        .is_none_or(|store| store.computer.address != host || store.is_sample())
                {
                    return;
                }
                this.reconnecting = false;
                let Some(store) = &mut this.store else {
                    return;
                };
                let mut helper_changed = false;
                let mut file_editing = None;
                match reached {
                    Ok((connection, helper)) => {
                        let helper_release = helper
                            .release
                            .as_ref()
                            .map(|release| format!("Helper {release}"));
                        file_editing = Some(helper.can_edit_files());
                        store.computer.pi_version = helper_release.clone();
                        store.computer.connected = true;
                        if let Some(computer) = store.computers.first_mut() {
                            computer.pi_version = helper_release;
                            computer.connected = true;
                        }
                        if let Some(live) = &mut store.live {
                            live.resume(connection, helper);
                        }
                        this.last_listed = Instant::now();
                        helper_changed = true;
                    }
                    Err(error) => {
                        log::info!("Reconnecting failed: {error:#}");
                        store.computer.connected = false;
                        if let Some(computer) = store.computers.first_mut() {
                            computer.connected = false;
                        }
                    }
                }
                if let Some(file_editing) = file_editing {
                    this.refresh_file_editing_capability(file_editing, cx);
                }
                if helper_changed {
                    this.update_helper_notification(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    #[cfg(feature = "local-backend")]
    pub(crate) fn select_local_backend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_local() && self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before switching to Local",
                cx,
            );
            return;
        }
        #[cfg(feature = "local-backend")]
        if !self.local_providers.is_synthetic() {
            let _ = std::fs::write(self.data_dir.join("local-backend-preferred"), "local");
        }
        self.close_sheet(cx);
        self.closing_sheet = None;
        self.cancel_connection_attempt();
        if self.is_local() {
            self.close_provider_accounts();
            self.paused = false;
            self.entered(window, cx);
            return;
        }
        self.close_provider_accounts();
        self.start.update(cx, |c, cx| c.clear(cx));
        if let Some(backend) = self.parked_local.take() {
            self.install_store(backend, cx);
        } else {
            let synthetic = self.local_providers.is_synthetic();
            let Some(vault) = crate::provider_accounts::vault(synthetic) else {
                self.notify_user("Android credential storage is unavailable", cx);
                return;
            };
            self.install_store(
                Store::local(crate::local_chat::Local::new(
                    pi_local::auth::Provider::Claude,
                    vault,
                    synthetic,
                )),
                cx,
            );
        }
        if let Some((text, attachments)) = self.parked_local_start.take() {
            self.start.update(cx, |c, cx| {
                c.set_text(&text, cx);
                for attachment in attachments {
                    c.attach(attachment, cx);
                }
            });
        }
        self.threads
            .extend(std::mem::take(&mut self.parked_local_threads));
        self.paused = false;
        self.start.update(cx, |c, _| {
            c.set_workspace(true);
            c.use_files(Some(Vec::new()));
            c.use_commands(Some(Vec::new()));
        });
        if !self
            .store
            .as_ref()
            .unwrap()
            .local
            .as_ref()
            .unwrap()
            .synthetic
        {
            self.local_private = true;
        }
        self.entered(window, cx);
    }

    pub(crate) fn backend_supports(&self, feature: &str) -> bool {
        self.store
            .as_ref()
            .is_some_and(|backend| backend.supports(feature))
    }
    pub(crate) fn is_local(&self) -> bool {
        self.store.as_ref().is_some_and(Store::is_local)
    }

    pub(crate) fn model_session(&self) -> Option<SessionId> {
        match self.route() {
            Route::Thread(id)
            | Route::Command(id, _)
            | Route::Terminal(id, _)
            | Route::Review(id)
            | Route::History(id)
            | Route::Diagnostics(id)
            | Route::Subagents(id)
            | Route::Swarm(id, _)
            | Route::SwarmPeer(id, _, _)
            | Route::Subagent(id, _) => Some(id),
            _ => None,
        }
    }

    /// The current session's confirmed model, or defaults on New session.
    pub(crate) fn model_settings(&self, cx: &App) -> (String, String) {
        if let Some(settings) = self
            .store
            .as_ref()
            .and_then(|store| store.model_settings(self.model_session()))
        {
            return settings;
        }
        if self.route() == Route::ScheduleEdit
            && let Some(draft) = &self.schedules.draft
        {
            return (draft.model_id.clone(), String::new());
        }
        if let Some(id) = self.model_session() {
            if let Some(live) = self.store.as_ref().and_then(|store| store.live.as_ref()) {
                return live
                    .model_settings(id)
                    .unwrap_or_else(|| ("Loading model…".into(), "…".into()));
            }
            if let Some(settings) = self.sample_models.get(&id) {
                return settings.clone();
            }
        }
        let prefs = self.prefs(cx);
        (prefs.model.clone(), prefs.thinking.clone())
    }

    pub(crate) fn choose_model(
        &mut self,
        name: String,
        provider: String,
        model_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.route() == Route::ScheduleEdit
            && let Some(draft) = &mut self.schedules.draft
        {
            draft.provider = provider;
            draft.model_id = model_id;
            cx.notify();
            return;
        }
        let id = self.model_session();
        let (_, thinking) = self.model_settings(cx);
        if let Some(store) = self.store.as_mut().filter(|store| !store.is_sample()) {
            if let Err(error) = store.set_model(id, &provider, &model_id) {
                self.notify_user(error, cx);
                return;
            }
            if id.is_some() || store.is_local() {
                cx.notify();
                return;
            }
        } else if let Some(id) = id {
            self.sample_models.insert(id, (name, thinking));
            cx.notify();
            return;
        }
        self.update_prefs(cx, |prefs| {
            prefs.model = name;
            prefs.model_provider = Some(provider);
            prefs.model_id = Some(model_id);
        });
    }

    pub(crate) fn choose_thinking(&mut self, level: &str, cx: &mut Context<Self>) {
        let id = self.model_session();
        let (model, _) = self.model_settings(cx);
        if let Some(store) = self.store.as_mut().filter(|store| !store.is_sample()) {
            if let Err(error) = store.set_thinking(id, level) {
                self.notify_user(error, cx);
                return;
            }
            if id.is_some() || store.is_local() {
                cx.notify();
                return;
            }
        } else if let Some(id) = id {
            self.sample_models.insert(id, (model, level.to_owned()));
            cx.notify();
            return;
        }
        self.update_prefs(cx, |prefs| prefs.thinking = level.to_owned());
    }

    /// Leaves the computer: back to Connect, its host key forgotten.
    pub(crate) fn forget_computer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_workspace_needs_attention(cx) {
            self.notify_user(
                "Save or resolve every open file before forgetting this computer",
                cx,
            );
            return;
        }
        if self.is_local() {
            self.start.update(cx, |start, cx| start.clear(cx));
        }
        if let Some(host) = self.schedule_host() {
            self.schedules.caches.remove(&host);
        }
        self.schedules.invalidate();
        let address = self
            .store
            .as_ref()
            .map(|store| store.computer.address.clone());
        self.reset_inline_pages();
        self.store = None;
        self.deferred_updates.clear();
        self.thread_lists.clear();
        self.thread_list_shapes.clear();
        self.thread_loading.clear();
        self.thread_loading_release_scheduled.clear();
        self.thread_failed_layouts.clear();
        self.thread_pending_remeasure.clear();
        self.thread_snapshots.clear();
        self.thread_list_offsets.clear();
        self.project_browser.clear();
        self.project_browser_editor = false;
        self.editor_root = None;
        self.file_generation += 1;
        self.file_view = None;
        self.file_tabs.clear();
        self.file_return_to_project = false;
        self.file_exit_armed = false;
        self._pump = None;
        self.threads.clear();
        self.pending_compacts.clear();
        self.jj_histories.clear();
        self.restoring_history = None;
        self.enabling_jj = None;
        self.update_prefs(cx, |prefs| {
            prefs.computer = None;
            prefs.sample = false;
            if let Some(address) = &address {
                prefs.host_keys.remove(address);
            }
        });
        self.replace_routes(vec![Route::Connect], cx);
        self.entered(window, cx);
    }

    /// Goes to a typed `~/…` or `/…` path; otherwise closes the keyboard
    /// over what was found.
    pub(crate) fn submit_folder_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.folder.read(cx).text().trim().to_owned();
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        if !(query.starts_with('~') || query.starts_with('/')) {
            return;
        }
        match self.full_path(&query) {
            Some(path) => {
                self.folder.update(cx, |area, cx| area.set_text("", cx));
                self.go_to_folder(path, cx);
            }
            None => self.notify_user("Type the folder's full path, starting with ~ or /", cx),
        }
    }

    // Sample sessions and notifications

    pub(crate) fn scroll_in_motion(&self) -> bool {
        // Every vertical viewport reports the same touch/momentum lifecycle.
        // Freeze remote geometry changes and ticker repaints on any screen the
        // person is actively manipulating, not only in a conversation.
        crate::scroll::in_motion()
    }

    /// Keep transcript snapshot freezing separate from direct manipulation:
    /// local script/animation frames must continue while remote geometry waits.
    fn projection_deferred(&self) -> bool {
        let visual = self.visual_apps.borrow_mut().projection_deferred(
            self.route(),
            self.visible
                && self.window_active
                && self.sheet.is_none()
                && self.closing_sheet.is_none(),
        );
        self.scroll_in_motion() || visual
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        #[cfg(feature = "local-backend")]
        if let Some(backend) = &mut self.parked_local {
            let _ = backend.tick(Duration::ZERO);
        }
        let _trace = gpui_android::trace::scope(c"pi.app.tick");
        #[cfg(all(feature = "local-backend", feature = "ui-test"))]
        self.tick_local_chat(cx);
        #[cfg(feature = "local-backend")]
        self.tick_provider_accounts(cx);
        // Framework duration-scale updates can arrive while we stay visible.
        // Poll at the existing low-frequency ticker, never on animation frames.
        #[cfg(target_os = "android")]
        if self.visible {
            cx.set_reduce_motion(activity::reduced_motion());
        }
        #[cfg(feature = "ui-test")]
        self.write_fixture_state(cx);
        let now = Instant::now();
        // Flush queued records first after release rather than projecting stale
        // state here and then reconstructing it again in the pending flush.
        let projection_deferred = self.projection_deferred() || self.deferred_flush_scheduled;
        let paused = self.paused && !self.playing;
        if !paused && !projection_deferred {
            self.keep_latest_in_view();
        }
        if self
            .resuming
            .as_ref()
            .is_some_and(|resume| resume.retry_at.is_some())
        {
            // The countdown to the next try.
            cx.notify();
        }
        if paused {
            self.last_tick = now;
            return;
        }
        let Some(store) = self.store.as_mut() else {
            self.last_tick = now;
            return;
        };
        if let Some(live) = &mut store.live {
            let closed = live.connection.is_closed();
            if store.computer.connected == closed {
                store.computer.connected = !closed;
                cx.notify();
            }
            if !closed && !self.reconnecting {
                live.poll_recovery(now);
                live.poll_subagents(now);
            }
            if self.visible && !self.reconnecting {
                if closed {
                    if self.last_reconnect.elapsed() > Duration::from_secs(5) {
                        self.reconnect(cx);
                    }
                    return;
                }
                if self.last_listed.elapsed() > Duration::from_secs(20) {
                    live.refresh();
                    self.last_listed = now;
                }
            }
        }
        let Some(store) = self.store.as_mut() else {
            self.last_tick = now;
            return;
        };
        #[cfg(feature = "local-backend")]
        let local_idle = store.local.is_some();
        #[cfg(not(feature = "local-backend"))]
        let local_idle = false;
        if !local_idle && store.running().next().is_none() {
            self.last_tick = now;
            return;
        }
        // A live projection or sample transition can resize the content even
        // without a remote record. Keep the elapsed interval intact and apply
        // it once the manipulation and momentum have ended.
        if projection_deferred {
            return;
        }
        let elapsed = now - self.last_tick;
        self.last_tick = now;
        for event in store.tick(elapsed) {
            self.alert(event, cx);
        }
        self.update_working_notification(cx);
        cx.notify();
    }

    fn viewing(&self, id: SessionId) -> bool {
        self.visible
            && matches!(self.route(), Route::Thread(shown) | Route::Command(shown, _) | Route::Terminal(shown, _) | Route::Review(shown) | Route::History(shown) | Route::Diagnostics(shown) | Route::Subagents(shown) | Route::Swarm(shown, _) | Route::SwarmPeer(shown, _, _) | Route::Subagent(shown, _) if shown == id)
    }

    fn alert(&mut self, event: Event, cx: &mut Context<Self>) {
        #[cfg(feature = "local-backend")]
        if let Event::Compacted(id, error) = &event {
            if let Some((target, draft)) = self.pending_compacts.remove(id)
                && error.is_some()
            {
                self.restore_compact_draft(target, &draft, cx);
            }
            self.notify_user(
                error.clone().unwrap_or_else(|| "Context compacted".into()),
                cx,
            );
            return;
        }
        let prefs = self.prefs(cx).clone();
        let accent = theme(cx).accent_rgb();
        let Some(store) = &self.store else {
            return;
        };
        if store.is_local() {
            return;
        }
        let computer = store.computer.name.clone();
        match event {
            #[cfg(feature = "local-backend")]
            Event::Compacted(..) => {}
            Event::Deleted(id) => self.finish_deletion(id, cx),
            Event::NeedsYou(id) => {
                if self.viewing(id) {
                    // The question card takes the composer's place.
                    self.questions_later.remove(&id);
                } else if prefs.notify_questions
                    && let Some(notification) = store
                        .session(id)
                        .and_then(|s| alerts::question(s, &computer, accent))
                {
                    activity::notify(&notification);
                }
            }
            Event::Problem(session, text) => {
                if session.is_none_or(|id| self.viewing(id) || self.route() == Route::Start) {
                    self.notify_user(text, cx);
                }
            }
            Event::Finished(id) => {
                activity::cancel_notification(alerts::question_id(id));
                if self.viewing(id) {
                    // You watched it end.
                    self.note_seen(id, cx);
                } else if prefs.notify_finished
                    && let Some(session) = store.session(id)
                {
                    activity::notify(&alerts::finished(session, &computer, accent));
                }
            }
        }
    }

    fn update_helper_notification(&self, cx: &mut Context<Self>) {
        let update = self.store.as_ref().and_then(|store| {
            Some((
                store.computer.name.as_str(),
                store.live.as_ref()?.helper.update.as_ref()?,
            ))
        });
        if let Some((computer, update)) = update {
            activity::notify(&alerts::helper_update(
                update,
                computer,
                theme(cx).accent_rgb(),
            ));
        } else {
            activity::cancel_notification(alerts::HELPER_UPDATE_ID);
        }
    }

    fn update_working_notification(&mut self, cx: &mut Context<Self>) {
        let wanted = self.prefs(cx).notify_working && !self.visible;
        let accent = theme(cx).accent_rgb();
        // The run that moved last leads, as on Home.
        let notification = self
            .store
            .as_ref()
            .filter(|store| wanted && !store.is_local())
            .and_then(|store| {
                alerts::working(
                    store.inbox().working.into_iter(),
                    &store.computer.name,
                    accent,
                )
            });
        match notification {
            Some(notification) => {
                // Its bar moves in whole steps; post again when it does.
                let text = format!(
                    "{}\n{}\n{:?}",
                    notification.title, notification.text, notification.progress
                );
                if self.working_posted.as_ref() != Some(&text) {
                    activity::notify(&notification);
                    self.working_posted = Some(text);
                }
            }
            None => {
                if self.working_posted.take().is_some() {
                    activity::cancel_notification(alerts::WORKING_ID);
                }
            }
        }
    }

    fn set_visible(&mut self, visibility: WindowVisibility, cx: &mut Context<Self>) {
        let visible = visibility.is_visible();
        let changed = self.visible != visible;
        self.visible = visible;
        #[cfg(feature = "local-backend")]
        self.tick_provider_accounts(cx);
        if !visible {
            #[cfg(feature = "local-backend")]
            if let Some(store) = &self.store {
                store.cancel_local();
            }
            // Cancel synchronously even if Hidden→Visible happens between ticks.
            // Android may stop delivering scroll phases before the final Ended.
            crate::scroll::cancel_motion();
            self.visual_apps
                .borrow_mut()
                .projection_deferred(self.route(), false);
        }
        #[cfg(target_os = "android")]
        if visible {
            cx.set_reduce_motion(activity::reduced_motion());
        }
        if self.visible
            && let Route::Thread(id)
            | Route::Command(id, _)
            | Route::Terminal(id, _)
            | Route::Review(id)
            | Route::History(id)
            | Route::Diagnostics(id)
            | Route::Subagents(id)
            | Route::Swarm(id, _)
            | Route::SwarmPeer(id, _, _)
            | Route::Subagent(id, _) = self.route()
        {
            activity::cancel_notification(alerts::question_id(id));
            activity::cancel_notification(alerts::finished_id(id));
        }
        self.update_working_notification(cx);
        if changed {
            // Foregrounding does not otherwise invalidate GPUI's retained
            // frame. Rerender so deferred work such as a large-thread loading
            // barrier can be armed only after a visible presentation.
            cx.notify();
        }
    }

    /// A `pi://` link from a notification.
    pub fn open_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "local-backend")]
        {
            // Unknown/read-only test links must not terminate local chat or expose its frame.
            let local_link = matches!(
                url,
                "pi://test/local-providers" | "pi://test/local-providers/synthetic"
            );
            let navigation = local_link
                || url == "pi://helper-update"
                || Link::parse(url).is_some()
                || pi_core::pairing::Offer::parse(url).is_ok()
                || url
                    .strip_prefix("pi://preview/")
                    .is_some_and(|name| crate::SCREENS.contains(&name));
            if self.local_fixture_url(url, window, cx) {
                return;
            }
            if (self.is_local() || self.local_providers.visible) && !navigation {
                return;
            }
            self.close_provider_accounts();
            window.dismiss_virtual_keyboard();
            window.focus(&self.focus, cx);
            cx.notify();
            if self.provider_accounts_url(url, window, cx) {
                return;
            }
            self.close_provider_accounts();
        }
        if url == "pi://helper-update" {
            self.close_sheet(cx);
            self.replace_routes(vec![Route::Sessions], cx);
            cx.notify();
            return;
        }
        #[cfg(feature = "ui-test")]
        if let Some(request) = url.strip_prefix("pi://test/inline-inspect/") {
            if self.paused
                && self.store.as_ref().is_some_and(|store| store.is_sample())
                && cx
                    .try_global::<crate::testing::State>()
                    .is_some_and(|state| state.fixture.starts_with("inline-"))
            {
                activity::inspect_inline_page_for_test(request);
            }
            return;
        }
        #[cfg(feature = "ui-test")]
        if url == "pi://test/inline-crash" {
            if self.paused
                && self.store.as_ref().is_some_and(|store| store.is_sample())
                && cx
                    .try_global::<crate::testing::State>()
                    .is_some_and(|state| state.fixture.starts_with("inline-"))
            {
                activity::terminate_inline_page_for_test();
            }
            return;
        }
        #[cfg(feature = "ui-test")]
        if matches!(url, "pi://test/inline-evening" | "pi://test/inline-light") {
            if self.paused
                && self.store.as_ref().is_some_and(|store| store.is_sample())
                && cx
                    .try_global::<crate::testing::State>()
                    .is_some_and(|state| state.fixture.starts_with("inline-"))
            {
                self.update_prefs(cx, |prefs| {
                    prefs.appearance = if url.ends_with("inline-evening") {
                        Appearance::Evening
                    } else {
                        Appearance::Moonstone
                    }
                });
                self.apply_theme(window, cx);
                cx.notify();
            }
            return;
        }
        #[cfg(feature = "ui-test")]
        if matches!(
            url,
            "pi://test/inline-update" | "pi://test/inline-unavailable"
        ) {
            // Fixture-only mutations of an inert sample, never a remote session.
            if self.paused
                && self.store.as_ref().is_some_and(|store| store.is_sample())
                && cx
                    .try_global::<crate::testing::State>()
                    .is_some_and(|state| state.fixture.starts_with("inline-"))
                && let Some(session) = self.store.as_mut().and_then(|store| {
                    store
                        .sessions
                        .iter_mut()
                        .find(|session| session.id == SessionId(1))
                })
                && let Some(page) = session
                    .turns
                    .first_mut()
                    .and_then(|turn| turn.pages.first_mut())
            {
                if url.ends_with("inline-unavailable") {
                    page.html = None;
                } else if let Some(html) = &mut page.html {
                    *html =
                        html.replace("<title>Calculator</title>", "<title>Calculator v2</title>");
                }
                cx.notify();
            }
            return;
        }
        #[cfg(feature = "ui-test")]
        if let Some(seconds) = url.strip_prefix("pi://test/advance/") {
            // Preview sessions are held still so screenshots are deterministic.
            // Tests can move their sample-only clock explicitly to verify that
            // mounted views react to later session updates.
            if self.paused
                && self.store.as_ref().is_some_and(|store| store.is_sample())
                && let Ok(seconds) = seconds.parse::<u64>()
                && let Some(store) = &mut self.store
            {
                store.tick(Duration::from_secs(seconds));
                cx.notify();
            }
            return;
        }
        #[cfg(feature = "ui-test")]
        if url == "pi://test/notify" {
            // A sample session's finished notification, to check how it looks in the tray.
            if let Some(store) = self.store.as_ref().filter(|store| store.is_sample())
                && let Some(session) = store.sessions.first()
            {
                let accent = theme(cx).accent_rgb();
                let posted =
                    activity::notify(&alerts::finished(session, &store.computer.name, accent));
                log::info!("Test notification posted: {posted}");
            }
            return;
        }
        #[cfg(feature = "ui-test")]
        if url == "pi://test/play" {
            // Lets the sample sessions run in real time, as they do before
            // pairing, for recording a walkthrough.
            self.playing = true;
            return;
        }
        #[cfg(feature = "ui-test")]
        if url == "pi://test/grow" && self.paused {
            // A run writing a file while its reply streams: every step adds a
            // diff line, an output line and words, as a live session does.
            if let Some(session) = self
                .store
                .as_mut()
                .filter(|store| store.is_sample())
                .and_then(|store| store.sessions.iter_mut().find(|s| s.id == SessionId(1)))
            {
                let turn = session.turn_mut();
                let stage = turn.stage_mut(crate::model::StageKind::Change);
                stage.diff_path = Some("crates/pi_android/src/live.rs".into());
                let step = stage.added;
                // A live edit shows its latest eight lines, as the projection does.
                if stage.diff.len() >= 8 {
                    stage.diff.remove(0);
                }
                stage.diff.push(crate::model::DiffLine::new(
                    crate::model::LineKind::Added,
                    300 + step,
                    &format!(
                        "  const line{step} = render(block, {step}); // written as it streams"
                    ),
                ));
                stage.added += 1;
                if stage.tools.is_empty() {
                    stage.tools.push(crate::model::ToolActivity {
                        id: "grow".into(),
                        name: "write".into(),
                        target: "notes.md".into(),
                        output: String::new(),
                        finished: false,
                        failed: false,
                        added: 0,
                        removed: 0,
                        diff: Vec::new(),
                        diff_is_preview: false,
                    });
                }
                let tool = stage.tools.last_mut().unwrap();
                tool.output.push_str(&format!(
                    "wrote line {step} of notes.md, a long line that wraps on a phone\n"
                ));
                let text = turn.summary.as_ref().map(|s| s.text()).unwrap_or_default();
                turn.summary = Some(crate::model::Summary {
                    headline: String::new(),
                    body: String::new(),
                    source: Some(format!("{text} Streaming word {step} of a growing reply.")),
                });
            }
            self.keep_latest_in_view();
            cx.notify();
            return;
        }
        #[cfg(feature = "ui-test")]
        if url == "pi://test-state" && self.paused {
            // Only fixture mode exposes diagnostics, and only lengths/state:
            // never prompt contents, clipboard data, addresses or credentials.
            let session = match self.route() {
                Route::Thread(id)
                | Route::Command(id, _)
                | Route::Terminal(id, _)
                | Route::Review(id)
                | Route::History(id)
                | Route::Diagnostics(id)
                | Route::Subagents(id)
                | Route::Swarm(id, _)
                | Route::SwarmPeer(id, _, _)
                | Route::Subagent(id, _) => self.store.as_ref().and_then(|s| s.session(id)),
                _ => None,
            };
            log::info!(
                "ui-test-state {}",
                serde_json::json!({
                    "route": format!("{:?}", self.route()),
                    "routes": self.routes.iter().map(|r| format!("{r:?}")).collect::<Vec<_>>(),
                    "sheet": self.sheet.map(|s| format!("{s:?}")),
                    "start_chars": self.start.read(cx).area.read(cx).text().chars().count(),
                    "search_chars": self.search.read(cx).text().chars().count(),
                    "prompt_chars": session.and_then(|s| s.turns.last()).map(|t| t.prompt.chars().count()),
                    "reply_chars": session.and_then(|s| s.turns.last()).and_then(|t| t.summary.as_ref()).map(|s| s.text().chars().count()),
                    "turns": session.map(|s| s.turns.len()),
                })
            );
            return;
        }
        #[cfg(feature = "ui-test")]
        if let Some(name) = url.strip_prefix("pi://preview/") {
            if crate::SCREENS.contains(&name) {
                self.preview(name, window, cx);
            }
            return;
        }
        if url.starts_with(pi_core::pairing::URL_PREFIX) {
            match pi_core::pairing::Offer::parse(url) {
                Ok(offer) => self.pair(offer, window, cx),
                Err(error) => {
                    self.connect_error = Some(error.to_string().into());
                    self.replace_routes(vec![Route::Connect], cx);
                    cx.notify();
                }
            }
            return;
        }
        let Some(link) = Link::parse(url) else {
            log::warn!("Ignoring the link {url}");
            return;
        };
        let (Link::Session(id)
        | Link::Question(id)
        | Link::AllowOnce(id)
        | Link::Decline(id)
        | Link::Review(id)) = link;
        let Some(store) = &mut self.store else {
            return;
        };
        let Some(session) = store.session(id) else {
            self.notify_user("That session is no longer on this phone", cx);
            return;
        };
        let waiting = session.state == State::NeedsYou;
        match link {
            Link::AllowOnce(_) | Link::Decline(_) if waiting => {
                let answer = if matches!(link, Link::Decline(_)) {
                    Answer::Deny
                } else {
                    Answer::AllowOnce
                };
                let label = session
                    .question
                    .as_ref()
                    .and_then(|question| {
                        question
                            .choices
                            .iter()
                            .find(|choice| choice.answer == answer)
                    })
                    .map(|choice| choice.label.clone());
                store.answer(id, answer);
                activity::cancel_notification(alerts::question_id(id));
                self.show_session(id, window, cx);
                self.close_sheet(cx);
                self.notify_user(
                    match label {
                        Some(label) => format!("Answered “{label}”"),
                        None => "Answered".into(),
                    },
                    cx,
                );
            }
            Link::Review(_) => {
                self.show_session(id, window, cx);
                self.open_review(id, 0, window, cx);
            }
            _ => self.show_session(id, window, cx),
        }
    }

    // Acting on sessions

    /// Answers Pi's question with the choice tapped: the choices are the buttons.
    pub(crate) fn answer(
        &mut self,
        id: SessionId,
        choice: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(store) = &mut self.store {
            store.answer(id, choice);
        }
        activity::cancel_notification(alerts::question_id(id));
        self.close_sheet(cx);
        // "Tell Pi why in the next message", where the session is on screen.
        if choice == Answer::Deny && self.route() == Route::Thread(id) {
            self.sheet_focus_pending = false;
            let composer = self.thread_composer(id, window, cx);
            let area = composer.read(cx).area.clone();
            let focus = area.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    pub(crate) fn stop(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let Some(store) = &mut self.store else { return };
        let live = !store.is_sample();
        if let Err(error) = store.stop(id) {
            self.notify_user(error, cx);
            return;
        }
        activity::cancel_notification(alerts::question_id(id));
        self.close_sheet(cx);
        self.notify_user(if live { "Stop requested…" } else { "Stopped" }, cx);
    }

    /// Only called by the destructive button in the confirmation sheet.
    pub(crate) fn delete_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if self.deleting_session.is_some() {
            return;
        }
        let Some(store) = &self.store else { return };
        if store
            .session(id)
            .is_none_or(|session| session.state.is_running())
        {
            self.notify_user("Stop the session before deleting it.", cx);
            return;
        }
        let Some(live) = &store.live else {
            if let Some(store) = &mut self.store {
                store.remove(id);
            }
            self.finish_deletion(id, cx);
            return;
        };
        let Some(target) = live.target(id) else {
            return;
        };
        let (connection, helper, key) = (
            live.connection.clone(),
            live.helper.clone(),
            target.key.clone(),
        );
        let (send, receive) = async_channel::bounded(1);
        self.deleting_session = Some(id);
        ssh::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                remote::delete(&connection, &helper, &target),
            )
            .await
            .map_err(|_| "Deletion wasn't confirmed in time. Refresh before retrying.".to_owned())
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
            let _ = send.send(result).await;
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = receive.recv().await else {
                return;
            };
            this.update(cx, |this, cx| {
                // The user may have changed computers while the request ran.
                let same = this
                    .store
                    .as_ref()
                    .and_then(|store| store.live.as_ref())
                    .and_then(|live| live.target(id))
                    .is_some_and(|target| target.key == key);
                if !same {
                    return;
                }
                this.deleting_session = None;
                match result {
                    Ok(()) => {
                        if let Some(store) = &mut this.store {
                            store.remove(id);
                        }
                        this.finish_deletion(id, cx);
                    }
                    Err(error) => {
                        this.notify_user(error, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn finish_deletion(&mut self, id: SessionId, cx: &mut Context<Self>) {
        self.deleting_session = None;
        self.swiping_session = None;
        self.threads.remove(&id);
        self.subagent_composers
            .retain(|(session, _), _| *session != id);
        let child_ids = self
            .subagent_ids
            .iter()
            .filter_map(|((session, _), child)| (*session == id).then_some(*child))
            .collect::<Vec<_>>();
        self.subagent_ids.retain(|(session, _), _| *session != id);
        for child in child_ids {
            self.subagent_sessions.remove(&child);
            self.subagent_projected_revisions.remove(&child);
            self.thread_lists.remove(&child);
            self.thread_list_shapes.remove(&child);
            self.thread_snapshots.remove(&child);
            self.thread_list_offsets.remove(&child);
        }
        self.pending_compacts.remove(&id);
        if self.review_session == Some(id) {
            self.review_session = None;
            self.review_lines.clear();
            self.review_comments.clear();
            self.review_list_shape = None;
            self.review.update(cx, |review, cx| review.clear(cx));
        }
        self.sample_models.remove(&id);
        self.jj_histories.remove(&id);
        if self
            .restoring_history
            .is_some_and(|(session, _)| session == id)
        {
            self.restoring_history = None;
        }
        if self.enabling_jj == Some(id) {
            self.enabling_jj = None;
        }
        self.expanded_turns.retain(|(session, _), _| *session != id);
        self.expanded_streams.retain(|(session, _)| *session != id);
        self.expanded_calls.retain(|(session, _, _)| *session != id);
        self.scrolls.remove(&Route::Thread(id));
        self.thread_lists.remove(&id);
        self.thread_list_shapes.remove(&id);
        self.thread_loading.remove(&id);
        self.thread_loading_release_scheduled.remove(&id);
        self.thread_failed_layouts.remove(&id);
        self.thread_pending_remeasure.remove(&id);
        self.thread_snapshots.remove(&id);
        self.thread_list_offsets.remove(&id);
        self.scrolls.remove(&Route::Review(id));
        self.scrolls.remove(&Route::History(id));
        self.scrolls.remove(&Route::Diagnostics(id));
        self.scrolls.remove(&Route::Subagents(id));
        self.scrolls
            .retain(|route, _| !matches!(route, Route::Swarm(session, _) if *session == id));
        self.swarm_views.retain(|(session, _), _| *session != id);
        self.swarm_peer_bindings
            .retain(|_, binding| binding.id != id);
        self.swarm_input_targets
            .retain(|(session, _), _| *session != id);
        let command_tokens: HashSet<u64> = self
            .routes
            .iter()
            .filter_map(|route| match route {
                Route::Command(session, token) if *session == id => Some(*token),
                _ => None,
            })
            .collect();
        self.command_views
            .retain(|token, _| !command_tokens.contains(token));
        let terminal_tokens: Vec<u64> = self
            .terminal_views
            .iter()
            .filter_map(|(token, state)| (state.session == id).then_some(*token))
            .collect();
        for token in terminal_tokens {
            self.close_terminal_view(token, cx);
        }
        self.scrolls
            .retain(|route, _| !matches!(route, Route::SwarmPeer(session, _, _) if *session == id));
        self.preview_transport_diagnostics.remove(&id);
        self.routes.retain(|route| !matches!(route, Route::Thread(session) | Route::Command(session, _) | Route::Terminal(session, _) | Route::Review(session) | Route::History(session) | Route::Diagnostics(session) | Route::Subagents(session) | Route::Swarm(session, _) | Route::SwarmPeer(session, _, _) | Route::Subagent(session, _) if *session == id));
        if matches!(self.sheet, Some(Sheet::Delete(session) | Sheet::Activity(session, _, _)) if session == id)
        {
            self.close_sheet(cx);
            self.activity_view = None;
        }
        activity::cancel_notification(alerts::question_id(id));
        activity::cancel_notification(alerts::finished_id(id));
        self.notify_user(
            if self
                .store
                .as_ref()
                .is_some_and(|store| store.live.is_some())
            {
                "Session permanently deleted. Project files were kept."
            } else {
                if self.is_local() {
                    "Local chat removed from this phone's memory."
                } else {
                    "Sample session removed. No computer files were changed."
                }
            },
            cx,
        );
    }

    pub(crate) fn open_review(
        &mut self,
        id: SessionId,
        file: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.backend_supports("file_review") {
            return;
        }
        if self.review_session != Some(id) {
            self.review_comments.clear();
            self.review.update(cx, |review, cx| review.clear(cx));
        } else {
            self.review
                .update(cx, |review, cx| review.set_lines(None, cx));
        }
        self.review_session = Some(id);
        self.review_file = file;
        self.review_call = None;
        self.review_lines.clear();
        self.scroll(Route::Review(id))
            .set_offset(gpui::point(px(0.), px(0.)));
        self.push(Route::Review(id), window, cx);
    }

    /// Opens edit and write calls as a focused full-screen diff. Calls without
    /// an applied/requested diff keep their complete command/output sheet.
    pub(crate) fn open_call_detail(
        &mut self,
        id: SessionId,
        turn: usize,
        kind: StageKind,
        tool: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail = self
            .session(id)
            .and_then(|session| session.turns.get(turn))
            .and_then(|turn| turn.tool(kind, tool))
            .map(|tool| {
                (
                    tool.id.clone(),
                    tool.name.clone(),
                    tool.finished,
                    !tool.diff.is_empty(),
                )
            });
        let Some((tool_id, name, finished, has_diff)) = detail else {
            return;
        };
        if name == "bash" && !finished {
            self.open_command_detail(id, tool_id, window, cx);
            return;
        }
        if !matches!(name.as_str(), "edit" | "write") || !has_diff {
            self.open_sheet(Sheet::Call(id, turn, kind, tool), cx);
            return;
        }
        self.review_session = None;
        self.review_call = Some((turn, kind, tool));
        self.review_lines.clear();
        self.review_comments.clear();
        self.review.update(cx, |review, cx| review.clear(cx));
        self.scroll(Route::Review(id))
            .set_offset(gpui::point(px(0.), px(0.)));
        self.push(Route::Review(id), window, cx);
    }

    pub(crate) fn open_history(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.backend_supports("file_history") {
            return;
        }
        self.push(Route::History(id), window, cx);
        self.load_jj_history(id, cx);
    }

    pub(crate) fn open_transport_diagnostics(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll(Route::Diagnostics(id))
            .set_offset(gpui::Point::default());
        self.close_sheet(cx);
        self.push(Route::Diagnostics(id), window, cx);
    }

    pub(crate) fn load_jj_history(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if !self.backend_supports("file_history") {
            return;
        }
        let Some((connection, helper, path)) = self.store.as_ref().and_then(|store| {
            let live = store.live.as_ref()?;
            Some((
                live.connection.clone(),
                live.helper.clone(),
                live.session_cwd(id)?.to_owned(),
            ))
        }) else {
            self.jj_histories.insert(
                id,
                JjHistoryState::Loaded(remote::JjHistory {
                    version: 1,
                    path: "~/repos/sample".into(),
                    root: Some("~/repos/sample".into()),
                    available: true,
                    reason: None,
                    operations: vec![
                        remote::JjOperation {
                            id: "91ec735a1f22".into(),
                            time: 0,
                            description: "pi: tighten mobile session layout".into(),
                            kind: "pi".into(),
                        },
                        remote::JjOperation {
                            id: "82ad4c917d3b".into(),
                            time: 0,
                            description: "snapshot working copy".into(),
                            kind: "snapshot".into(),
                        },
                    ],
                }),
            );
            cx.notify();
            return;
        };
        self.jj_histories.insert(id, JjHistoryState::Loading);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = remote::jj_history(&connection, &helper, &path)
                .await
                .map_err(|error| format!("{error:#}"));
            this.update(cx, |this, cx| {
                if this
                    .store
                    .as_ref()
                    .and_then(|store| store.live.as_ref())
                    .and_then(|live| live.session_cwd(id))
                    != Some(path.as_str())
                {
                    return;
                }
                self::PhoneApp::set_jj_history_result(this, id, result, cx);
            })
            .ok();
        })
        .detach();
    }

    fn set_jj_history_result(
        &mut self,
        id: SessionId,
        result: Result<remote::JjHistory, String>,
        cx: &mut Context<Self>,
    ) {
        self.jj_histories.insert(
            id,
            match result {
                Ok(history) => JjHistoryState::Loaded(history),
                Err(error) => JjHistoryState::Failed(error.into()),
            },
        );
        cx.notify();
    }

    pub(crate) fn project_is_running(&self, id: SessionId) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        let Some(live) = &store.live else {
            return store
                .session(id)
                .is_some_and(|session| session.state.is_running());
        };
        let Some(path) = live.session_cwd(id) else {
            return false;
        };
        store
            .sessions
            .iter()
            .any(|session| session.state.is_running() && live.session_cwd(session.id) == Some(path))
    }

    pub(crate) fn restore_jj_history(
        &mut self,
        id: SessionId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if self.restoring_history.is_some() {
            return;
        }
        if self.project_is_running(id) {
            self.notify_user(
                "Stop active sessions in this project before restoring files.",
                cx,
            );
            return;
        }
        let operation = match self.jj_histories.get(&id) {
            Some(JjHistoryState::Loaded(history)) => history.operations.get(index).cloned(),
            _ => None,
        };
        let target = self.store.as_ref().and_then(|store| {
            let live = store.live.as_ref()?;
            Some((
                live.connection.clone(),
                live.helper.clone(),
                live.session_cwd(id)?.to_owned(),
            ))
        });
        let (Some(operation), Some((connection, helper, path))) = (operation, target) else {
            self.notify_user("That history point is no longer available.", cx);
            return;
        };
        self.restoring_history = Some((id, index));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = remote::restore_jj_operation(&connection, &helper, &path, &operation.id)
                .await
                .map_err(|error| format!("{error:#}"));
            this.update(cx, |this, cx| {
                this.restoring_history = None;
                match result {
                    Ok(()) => {
                        this.close_sheet(cx);
                        this.notify_user(
                            "Project files restored. jj recorded the restore so it can be undone.",
                            cx,
                        );
                        this.load_jj_history(id, cx);
                    }
                    Err(error) => this.notify_user(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn enable_jj(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if self.enabling_jj.is_some() {
            return;
        }
        if self.project_is_running(id) {
            self.notify_user(
                "Stop active sessions in this project before enabling jj.",
                cx,
            );
            return;
        }
        let Some((connection, helper, path)) = self.store.as_ref().and_then(|store| {
            let live = store.live.as_ref()?;
            Some((
                live.connection.clone(),
                live.helper.clone(),
                live.session_cwd(id)?.to_owned(),
            ))
        }) else {
            return;
        };
        self.enabling_jj = Some(id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = remote::enable_jj(&connection, &helper, &path)
                .await
                .map_err(|error| format!("{error:#}"));
            this.update(cx, |this, cx| {
                this.enabling_jj = None;
                match result {
                    Ok(history) => {
                        this.jj_histories
                            .insert(id, JjHistoryState::Loaded(history));
                        this.close_sheet(cx);
                        this.notify_user(
                            "jj file history is on. Future remote turns that edit files are recorded.",
                            cx,
                        );
                    }
                    Err(error) => this.notify_user(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn subagent_composer(
        &mut self,
        id: SessionId,
        conversation: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Composer> {
        let key = (id, conversation.to_owned());
        if let Some(composer) = self.subagent_composers.get(&key) {
            return composer.clone();
        }
        let composer = cx.new(|cx| {
            let mut composer = Composer::new(placeholder, cx);
            composer.set_message_only();
            composer
        });
        let sends = self.prefs(cx).return_sends;
        composer
            .read(cx)
            .area
            .clone()
            .update(cx, |area, _| area.enter_sends = sends);
        let conversation = conversation.to_owned();
        let swarm = match self.route() {
            Route::SwarmPeer(_, _, token) => self.swarm_peer_bindings.get(&token).cloned(),
            _ => None,
        };
        self.subscriptions.push(cx.subscribe_in(
            &composer,
            window,
            move |this, _, event, window, cx| {
                this.subagent_composer_event(id, &conversation, swarm.as_ref(), event, window, cx)
            },
        ));
        self.subagent_composers.insert(key, composer.clone());
        composer
    }

    fn subagent_composer_event(
        &mut self,
        id: SessionId,
        conversation: &str,
        swarm: Option<&crate::screens::swarm::SwarmPeerBinding>,
        event: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ComposerEvent::Send { text, attachments } => {
                let active_swarm = match self.route() {
                    Route::SwarmPeer(_, _, token) => self.swarm_peer_bindings.get(&token),
                    _ => None,
                };
                if swarm
                    .or(active_swarm)
                    .is_some_and(|binding| !self.can_send_swarm_peer(binding, conversation))
                {
                    self.notify_user("Open this exact peer with a current connection before sending. Nothing was queued.", cx);
                    return;
                }
                if swarm.or(active_swarm).is_some() {
                    let expected = self.swarm_input_targets.get(&(id, conversation.to_owned()));
                    let current = self
                        .store
                        .as_ref()
                        .and_then(|store| store.live.as_ref())
                        .and_then(|live| live.subagent_input_target(id, conversation).ok());
                    if !crate::screens::swarm::input_target_current(expected, current.as_ref()) {
                        self.notify_user("The displayed child turn changed. Review the peer before explicitly retrying; nothing was queued.", cx);
                        return;
                    }
                }
                if !attachments.is_empty() {
                    self.notify_user("Direct subagent input supports text only.", cx);
                    return;
                }
                let sent = self
                    .store
                    .as_mut()
                    .and_then(|store| store.live.as_mut())
                    .ok_or_else(|| {
                        "Direct subagent input requires a live durable session.".to_owned()
                    })
                    .and_then(|live| {
                        if matches!(
                            live.subagent_input_target(id, conversation)?,
                            crate::live::SubagentInputTarget::FollowUp
                        ) {
                            return Err("This subagent has finished. Nothing was sent.".into());
                        }
                        live.prompt_subagent(id, conversation, text.clone())
                    });
                match sent {
                    Ok(()) => {
                        window.dismiss_virtual_keyboard();
                        cx.notify();
                    }
                    Err(error) => self.notify_user(error, cx),
                }
            }
            ComposerEvent::Attach
            | ComposerEvent::ChooseModel
            | ComposerEvent::ChooseThinking
            | ComposerEvent::PreviewImage(_)
            | ComposerEvent::StageReview { .. }
            | ComposerEvent::CancelReview => self.notify_user(
                "This composer sends text only to the selected subagent.",
                cx,
            ),
        }
    }

    pub(crate) fn thread_composer(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Composer> {
        if let Some(composer) = self.threads.get(&id) {
            return composer.clone();
        }
        let compact = self.compact_available_for_session(id);
        let workspace = self.backend_supports("workspace_browse");
        let composer = cx.new(|cx| {
            let mut composer = Composer::new("Ask a follow-up…", cx);
            composer.set_session_commands(compact);
            composer.set_compact_when_idle();
            composer.set_workspace(true);
            if !workspace {
                composer.use_files(Some(Vec::new()));
            }
            composer
        });
        let commands = self.command_catalog_for_session(id, cx);
        composer.update(cx, |composer, _| composer.use_commands(commands));
        // A live session offers the files Pi touched in it.
        let files = self
            .store
            .as_ref()
            .filter(|store| !store.is_sample())
            .map(|store| {
                let mut files: Vec<String> = Vec::new();
                if let Some(session) = store.session(id) {
                    let read =
                        session
                            .turns
                            .iter()
                            .flat_map(|turn| &turn.stages)
                            .flat_map(|stage| {
                                stage
                                    .references
                                    .iter()
                                    .filter_map(|reference| match reference {
                                        crate::model::Reference::File(path) => Some(path.clone()),
                                        crate::model::Reference::Search(_) => None,
                                    })
                            });
                    for path in session
                        .files
                        .iter()
                        .map(|file| file.path.clone())
                        .chain(read)
                    {
                        if !files.contains(&path) {
                            files.push(path);
                        }
                    }
                }
                files
            });
        let remote_files = self.store.as_ref().and_then(|store| {
            let live = store.live.as_ref()?;
            Some((
                live.connection.clone(),
                live.helper.clone(),
                store.computer.address.clone(),
                live.session_cwd(id)?.to_owned(),
                files.clone().unwrap_or_default(),
            ))
        });
        if let Some((connection, helper, host, cwd, touched)) = remote_files {
            composer.update(cx, |composer, _| composer.load_files());
            cx.spawn(async move |this, cx| {
                let result = remote::project_files(&connection, &helper, &host, &cwd)
                    .await
                    .map_err(|error| format!("{error:#}"));
                this.update(cx, |this, cx| {
                    let Some(composer) = this.threads.get(&id) else {
                        return;
                    };
                    composer.update(cx, |composer, _| match result {
                        Ok(mut files) => {
                            for path in touched {
                                if !files.contains(&path) {
                                    files.push(path);
                                }
                            }
                            composer.use_files(Some(files));
                        }
                        Err(error) if !touched.is_empty() => {
                            composer.use_files(Some(touched));
                            log::warn!("Could not load project files for mentions: {error}");
                        }
                        Err(error) => composer.fail_files(error),
                    });
                    cx.notify();
                })
                .ok();
            })
            .detach();
        } else {
            composer.update(cx, |composer, _| composer.use_files(files));
        }
        let sends = self.prefs(cx).return_sends;
        composer
            .read(cx)
            .area
            .clone()
            .update(cx, |area, _| area.enter_sends = sends);
        self.subscriptions.push(cx.subscribe_in(
            &composer,
            window,
            move |this, source, event, window, cx| {
                if this
                    .threads
                    .get(&id)
                    .is_none_or(|current| current != source)
                {
                    return;
                }
                this.composer_event(Target::Thread(id), event, window, cx)
            },
        ));
        self.threads.insert(id, composer.clone());
        composer
    }

    pub(crate) fn composer(&self, target: Target) -> Option<Entity<Composer>> {
        match target {
            Target::Start => Some(self.start.clone()),
            Target::Review(id) => matches!(self.route(), Route::Review(shown) if shown == id)
                .then(|| self.review.clone()),
            Target::Thread(id) => self.threads.get(&id).cloned(),
        }
    }

    pub(crate) fn restore_compact_draft(
        &self,
        target: Target,
        draft: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(composer) = self.composer(target) {
            composer.update(cx, |composer, cx| {
                // An in-flight attachment import disables Send even when a new
                // draft exists. Never overwrite text/attachments composed later.
                if composer.area.read(cx).text().is_empty() && composer.attachments().is_empty() {
                    composer.set_text(draft, cx);
                }
            });
        }
    }

    fn composer_event(
        &mut self,
        target: Target,
        event: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.store.as_ref().is_some_and(|s| !s.is_sample()) {
            let active = composer_target_active(target, self.route());
            #[cfg(feature = "local-backend")]
            let active = active && !self.local_providers.visible;
            if !active || !self.visible || !self.window_active {
                return;
            }
        }
        match event {
            ComposerEvent::Attach => self.open_sheet(Sheet::Attach(target), cx),
            ComposerEvent::ChooseModel => self.open_sheet(Sheet::Model, cx),
            ComposerEvent::ChooseThinking => self.open_sheet(Sheet::Thinking, cx),
            ComposerEvent::PreviewImage(index) => self.open_sheet(Sheet::Image(target, *index), cx),
            ComposerEvent::StageReview { text, attachments } => {
                let Route::Review(id) = self.route() else {
                    return;
                };
                if target != Target::Review(id) {
                    return;
                }
                let Some(anchor) = self.review_lines.iter().next_back().copied() else {
                    self.notify_user("Select the lines for this comment again.", cx);
                    return;
                };
                let Some(lines) = attachments.iter().find_map(|attachment| match attachment {
                    Attachment::Lines(lines) => Some(lines.clone()),
                    _ => None,
                }) else {
                    self.notify_user("Select the lines for this comment again.", cx);
                    return;
                };
                let Some(path) = self
                    .store
                    .as_ref()
                    .and_then(|store| store.session(id))
                    .and_then(|session| session.files.get(self.review_file))
                    .map(|file| file.path.clone())
                else {
                    self.notify_user("The reviewed file is no longer available.", cx);
                    return;
                };
                let body_attachments = attachments
                    .iter()
                    .filter(|attachment| !matches!(attachment, Attachment::Lines(_)))
                    .cloned()
                    .collect::<Vec<_>>();
                let staged_attachments: usize = self
                    .review_comments
                    .iter()
                    .map(|comment| comment.attachments.len())
                    .sum();
                if staged_attachments + body_attachments.len() > 8 {
                    self.notify_user("Attach up to eight files across all review comments.", cx);
                    return;
                }
                let staged_images: usize = self
                    .review_comments
                    .iter()
                    .flat_map(|comment| &comment.attachments)
                    .filter(|attachment| matches!(attachment, Attachment::Image { .. }))
                    .count();
                let new_images = body_attachments
                    .iter()
                    .filter(|attachment| matches!(attachment, Attachment::Image { .. }))
                    .count();
                if staged_images + new_images > crate::attachments::MAX_IMAGES {
                    self.notify_user("Attach up to four images across all review comments.", cx);
                    return;
                }
                let comment_id = self.review_next_comment_id;
                self.review_next_comment_id = self.review_next_comment_id.wrapping_add(1).max(1);
                self.review_comments.push(ReviewComment {
                    id: comment_id,
                    path,
                    anchor,
                    lines,
                    text: text.clone(),
                    attachments: body_attachments,
                });
                self.review.update(cx, |composer, cx| composer.clear(cx));
                self.review_lines.clear();
                self.review_list_shape = None;
                window.dismiss_virtual_keyboard();
                cx.notify();
            }
            ComposerEvent::CancelReview => {
                if matches!(target, Target::Review(_)) {
                    self.review
                        .update(cx, |composer, cx| composer.set_lines(None, cx));
                    self.review_lines.clear();
                    self.review_list_shape = None;
                    window.dismiss_virtual_keyboard();
                    cx.notify();
                }
            }
            ComposerEvent::Send { text, attachments } => {
                let route = self.route();
                let compact = match target {
                    Target::Thread(id) => self
                        .compact_for_session(id, text)
                        .map(|instructions| (id, instructions)),
                    Target::Review(target_id) => match route {
                        Route::Review(id) if id == target_id => self
                            .compact_for_session(id, text)
                            .map(|instructions| (id, instructions)),
                        _ => None,
                    },
                    Target::Start => None,
                };
                if let Some((id, instructions)) = compact {
                    if !attachments.is_empty() {
                        self.notify_user(
                            "Remove attachments before compacting. Your draft has been kept.",
                            cx,
                        );
                        return;
                    }
                    let Some(store) = &mut self.store else {
                        return;
                    };
                    if let Err(error) = store.compact(id, instructions) {
                        self.notify_user(error, cx);
                        return;
                    }
                    self.pending_compacts.insert(id, (target, text.clone()));
                    if let Some(composer) = self.composer(target) {
                        composer.update(cx, |composer, cx| composer.clear(cx));
                    }
                    window.dismiss_virtual_keyboard();
                    self.notify_user("Compacting context", cx);
                    cx.notify();
                    return;
                }
                let Some(store) = &mut self.store else {
                    return;
                };
                let file = if let Route::Review(id) = route {
                    store
                        .session(id)
                        .and_then(|session| session.files.get(self.review_file))
                        .map(|file| file.path.clone())
                } else {
                    None
                };
                let prompt = match prompt_with_capabilities(
                    text,
                    attachments,
                    file.as_deref(),
                    store.supports("visualize"),
                ) {
                    Ok(prompt) => prompt,
                    Err(error) => {
                        self.notify_user(error, cx);
                        return;
                    }
                };
                let labels: Vec<String> = attachments.iter().map(Attachment::label).collect();
                match target {
                    Target::Start => match store.start(self.project, prompt, labels) {
                        Ok(id) => {
                            self.start.update(cx, |composer, cx| composer.clear(cx));
                            self.show_session(id, window, cx);
                        }
                        Err(error) => self.notify_user(error, cx),
                    },
                    Target::Thread(id) => {
                        let running = store.session(id).is_some_and(|s| s.state.is_running());
                        if let Err(error) = store.send(id, prompt, labels) {
                            self.notify_user(error, cx);
                            return;
                        }
                        if let Some(composer) = self.composer(target) {
                            composer.update(cx, |composer, cx| composer.clear(cx));
                        }
                        if !running {
                            if let Some(list) = self.thread_lists.get(&id) {
                                list.set_follow_mode(gpui::FollowMode::Tail);
                            } else {
                                self.scroll(Route::Thread(id)).scroll_to_bottom();
                            }
                        }
                        window.dismiss_virtual_keyboard();
                        if running {
                            self.notify_user("Queued to steer this run", cx);
                        }
                    }
                    Target::Review(target_id) => {
                        let Route::Review(id) = route else {
                            return;
                        };
                        if id != target_id {
                            return;
                        };
                        let running = store.session(id).is_some_and(|s| s.state.is_running());
                        if let Err(error) = store.follow_up(id, prompt, labels) {
                            self.notify_user(error, cx);
                            return;
                        }
                        self.review.update(cx, |composer, cx| composer.clear(cx));
                        self.review_lines.clear();
                        self.review_comments.clear();
                        self.review_session = None;
                        self.routes.pop();
                        self.entered(window, cx);
                        self.notify_user(
                            if running {
                                "Queued as a follow-up"
                            } else {
                                "Sent as a follow-up"
                            },
                            cx,
                        );
                    }
                }
                cx.notify();
            }
        }
    }

    pub(crate) fn remove_review_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self
            .review_comments
            .iter()
            .position(|comment| comment.id == id)
        {
            self.review_comments.remove(index);
            self.review_list_shape = None;
            cx.notify();
        }
    }

    pub(crate) fn submit_review_comments(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route() != Route::Review(id)
            || self.review_session != Some(id)
            || self.review_comments.is_empty()
        {
            return;
        }
        let visualize = self
            .store
            .as_ref()
            .is_some_and(|store| store.supports("visualize"));
        let (prompt, labels) = match batched_review_prompt(&self.review_comments, visualize) {
            Ok(batch) => batch,
            Err(error) => {
                self.notify_user(error, cx);
                return;
            }
        };
        let Some(store) = &mut self.store else {
            return;
        };
        let running = store
            .session(id)
            .is_some_and(|session| session.state.is_running());
        if let Err(error) = store.follow_up(id, prompt, labels) {
            self.notify_user(error, cx);
            return;
        }
        self.review.update(cx, |composer, cx| composer.clear(cx));
        self.review_lines.clear();
        self.review_comments.clear();
        self.review_session = None;
        self.review_list_shape = None;
        self.routes.pop();
        self.entered(window, cx);
        self.notify_user(
            if running {
                "Queued review comments as a follow-up"
            } else {
                "Sent review comments as a follow-up"
            },
            cx,
        );
        cx.notify();
    }

    pub(crate) fn recover_prompt(
        &mut self,
        id: SessionId,
        request_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let failed = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .and_then(|live| {
                live.failed_prompts(id)
                    .iter()
                    .find(|failed| failed.prompt.request_id == request_id)
            })
            .cloned();
        let Some(failed) = failed else {
            return;
        };
        let composer = self.thread_composer(id, window, cx);
        match composer.update(cx, |composer, cx| {
            composer.restore_prompt(&failed.prompt, cx)
        }) {
            Ok(()) => {
                if let Some(live) = self.store.as_mut().and_then(|store| store.live.as_mut()) {
                    live.recover_prompt(id, request_id);
                }
                self.notify_user(
                    "Message and images restored. Review the model and send when ready.",
                    cx,
                );
            }
            Err(error) => self.notify_user(error, cx),
        }
    }

    /// Adds files from Android's picker to a composer.
    pub(crate) fn attach_files(&mut self, target: Target, cx: &mut Context<Self>) {
        self.close_sheet(cx);
        let Some(composer) = self.composer(target) else {
            return;
        };
        let owner = self.store.as_ref().map(|s| s.generation);
        let generation = composer.update(cx, |composer, cx| composer.begin_import(cx));
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let picked = picked.await;
            let imported = match picked {
                Ok(Ok(Some(paths))) => Some(
                    cx.background_executor()
                        .spawn(async move {
                            paths
                                .iter()
                                .map(|path| crate::attachments::load(path))
                                .collect::<Vec<_>>()
                        })
                        .await,
                ),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(error)) => Some(vec![Err(format!("{error:#}"))]),
            };
            this.update(cx, |this, cx| {
                if this.store.as_ref().map(|s| s.generation) != owner
                    || this.composer(target).as_ref() != Some(&composer)
                {
                    return;
                }
                if !composer.update(cx, |composer, cx| composer.finish_import(generation, cx)) {
                    return;
                }
                if let Some(imported) = imported {
                    for attachment in imported {
                        match attachment {
                            Ok(attachment) => {
                                if let Err(error) = composer
                                    .update(cx, |composer, cx| composer.try_attach(attachment, cx))
                                {
                                    this.notify_user(error, cx);
                                }
                            }
                            Err(error) => this.notify_user(error, cx),
                        }
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Pastes the clipboard's image as an attachment, or its text into the draft.
    pub(crate) fn paste_into(&mut self, target: Target, cx: &mut Context<Self>) {
        self.close_sheet(cx);
        let Some(composer) = self.composer(target) else {
            return;
        };
        let Some(item) = cx.read_from_clipboard() else {
            self.notify_user("The clipboard is empty", cx);
            return;
        };
        let image = item.entries.iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        });
        match (image, item.text()) {
            (Some(image), _) => {
                let owner = self.store.as_ref().map(|s| s.generation);
                let generation = composer.update(cx, |composer, cx| composer.begin_import(cx));
                cx.spawn(async move |this, cx| {
                    let attachment = cx
                        .background_executor()
                        .spawn(async move {
                            crate::attachments::prepare_image(
                                format!("Pasted image.{}", image.format.extension()),
                                &image.bytes,
                            )
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        if this.store.as_ref().map(|s| s.generation) != owner
                            || this.composer(target).as_ref() != Some(&composer)
                        {
                            return;
                        }
                        if !composer
                            .update(cx, |composer, cx| composer.finish_import(generation, cx))
                        {
                            return;
                        }
                        let result = attachment.and_then(|attachment| {
                            composer.update(cx, |composer, cx| composer.try_attach(attachment, cx))
                        });
                        if let Err(error) = result {
                            this.notify_user(error, cx);
                        }
                    })
                    .ok();
                })
                .detach();
            }
            (None, Some(text)) => composer.update(cx, |composer, cx| {
                composer.area.update(cx, |area, cx| area.insert(&text, cx));
            }),
            (None, None) => self.notify_user("The clipboard has nothing to paste", cx),
        }
    }

    /// Whether long lines of code and output wrap, which the phone remembers.
    pub(crate) fn wrap_lines(&self, cx: &App) -> bool {
        self.prefs(cx).wrap_lines
    }

    /// The button that switches long lines between wrapping and scrolling sideways.
    pub(crate) fn wrap_toggle(
        &self,
        id: impl Into<gpui::ElementId>,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = theme(cx);
        let wrap = self.wrap_lines(cx);
        crate::ui::tap(id, "wrap", &colors)
            .aria_label(if wrap {
                "Scroll long lines sideways"
            } else {
                "Wrap long lines"
            })
            .when(wrap, |tap| tap.bg(colors.selected).rounded_full())
            .on_click(cx.listener(|this, _, _, cx| {
                this.update_prefs(cx, |prefs| prefs.wrap_lines = !prefs.wrap_lines);
                cx.notify();
            }))
    }

    pub(crate) fn copy(&mut self, text: String, what: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.close_sheet(cx);
        self.notify_user(format!("Copied {what}"), cx);
    }

    pub(crate) fn insets(window: &Window) -> Edges<Pixels> {
        let viewport = window.viewport_size();
        let safe = window.fully_visible_bounds();
        Edges {
            top: safe.top(),
            left: safe.left(),
            right: viewport.width - safe.right(),
            bottom: viewport.height - safe.bottom(),
        }
    }
}

/// Encode the complete draft. No attachment may silently become just a label.
#[cfg(test)]
fn prompt_for(
    text: &str,
    attachments: &[Attachment],
    file: Option<&str>,
) -> Result<crate::prompt::Prompt, String> {
    prompt_with_capabilities(text, attachments, file, true)
}
fn prompt_with_capabilities(
    text: &str,
    attachments: &[Attachment],
    file: Option<&str>,
    visualize: bool,
) -> Result<crate::prompt::Prompt, String> {
    use base64::Engine;
    let mut message = if visualize {
        crate::visualize_prompt::expand(text)
    } else {
        text.to_owned()
    };
    let mut images = Vec::new();
    for attachment in attachments {
        match attachment {
            Attachment::Image { image, .. } => {
                if image.bytes.len() > crate::attachments::MAX_IMAGE_BYTES {
                    return Err(
                        "Image exceeds the upload limit. Remove it and select it again.".into(),
                    );
                }
                images.push(pi_core::protocol::ImageContent::new(
                    base64::engine::general_purpose::STANDARD.encode(&image.bytes),
                    image.format.mime_type(),
                ));
            }
            Attachment::File { name, contents, .. } => message.push_str(&format!(
                "\n\nAttached text file: {}\nContent byte length: {}\n{contents}\nEnd of attached file: {}", serde_json::to_string(name).unwrap_or_default(), contents.len(), serde_json::to_string(name).unwrap_or_default()
            )),
            Attachment::Lines(lines) => {
                let file =
                    file.ok_or("Choose the reviewed file again before sending selected lines.")?;
                message = format!("About {lines} of {file}: {message}");
            }
        }
    }
    if images.len() > crate::attachments::MAX_IMAGES {
        return Err("Attach up to four images per message.".into());
    }
    Ok(crate::prompt::Prompt::new(message, images))
}

fn batched_review_prompt(
    comments: &[ReviewComment],
    visualize: bool,
) -> Result<(crate::prompt::Prompt, Vec<String>), String> {
    if comments.is_empty() {
        return Err("Add at least one review comment before sending.".into());
    }
    let mut sections = Vec::with_capacity(comments.len());
    let attachment_count: usize = comments
        .iter()
        .map(|comment| comment.attachments.len())
        .sum();
    if attachment_count > 8 {
        return Err("Attach up to eight files across all review comments.".into());
    }
    let mut images = Vec::new();
    let mut labels = Vec::new();
    for (index, comment) in comments.iter().enumerate() {
        let encoded =
            prompt_with_capabilities(&comment.text, &comment.attachments, None, visualize)?;
        images.extend(encoded.images);
        labels.push(format!("{} · {}", comment.path, comment.lines));
        labels.extend(comment.attachments.iter().map(Attachment::label));
        sections.push(format!(
            "## Comment {}\nFile: {}\nLocation: {}\n\n{}",
            index + 1,
            comment.path,
            comment.lines,
            encoded.message
        ));
    }
    if images.len() > crate::attachments::MAX_IMAGES {
        return Err("Attach up to four images across all review comments.".into());
    }
    let message = format!(
        "Please address these {} review comments:\n\n{}",
        comments.len(),
        sections.join("\n\n")
    );
    Ok((crate::prompt::Prompt::new(message, images), labels))
}

pub(crate) fn size_label(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.0} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

impl Render for PhoneApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _trace = gpui_android::trace::scope(c"pi.root.build");
        #[cfg(feature = "local-backend")]
        {
            self.local_frame = self.local_frame.wrapping_add(1);
        }
        #[cfg(feature = "local-backend")]
        let backend_visible = self.local_providers.visible;
        #[cfg(not(feature = "local-backend"))]
        let backend_visible = false;
        // Route/sheet transitions cancel before a retained old surface can run
        // another callback. Do not wait for the next ticker or remote record.
        self.visual_apps.borrow_mut().projection_deferred(
            self.route(),
            self.visible
                && !backend_visible
                && self.window_active
                && self.sheet.is_none()
                && self.closing_sheet.is_none(),
        );
        crate::visual_profile::enable(cx, self.render_stats.visible);
        gpui_android::frame_profile::set_enabled(self.render_stats.visible);
        let render_start = self.render_stats.visible.then(Instant::now);
        if self.render_stats.visible {
            let now = Instant::now();
            self.render_stats.refresh_display_rates(now);
            self.render_stats.observe(now);
        }
        #[cfg(feature = "ui-test")]
        if cx.has_global::<crate::testing::State>() {
            let state = cx.global_mut::<crate::testing::State>();
            // Probes describe this frame, not controls/scroll viewports left
            // behind on earlier routes. Stale bounds must never authorize taps.
            state.bounds.clear();
            state.scale = window.scale_factor();
            state.viewport = [
                window.viewport_size().width.into(),
                window.viewport_size().height.into(),
            ];
            state.pointer = [
                window.mouse_position().x.into(),
                window.mouse_position().y.into(),
            ];
        }
        self.animate_panels(window, cx);
        if std::mem::take(&mut self.sheet_focus_pending) {
            window.dismiss_virtual_keyboard();
            window.focus(&self.focus, cx);
        }
        let colors = theme(cx);
        let insets = Self::insets(window);
        let inline_slot = self.inline_pages.borrow().slot.clone();
        inline_slot.borrow_mut().take();
        let native_page_visible = self.visible
            && !backend_visible
            && self.sheet.is_none()
            && self.closing_sheet.is_none()
            && self.notice.is_none();
        let expanded_visual = self.visual_expanded_screen(window, cx);
        let expanded_page = self.inline_expanded_screen(window, cx);
        if expanded_page.is_none() {
            self.inline_pages.borrow_mut().expanded = None;
        }
        #[cfg(feature = "local-backend")]
        let backend_screen = self
            .local_providers
            .visible
            .then(|| self.provider_accounts_screen(cx).into_any_element());
        #[cfg(not(feature = "local-backend"))]
        let backend_screen: Option<gpui::AnyElement> = None;
        let screen = if let Some(screen) = backend_screen {
            screen
        } else if let Some(expanded) = expanded_visual {
            expanded.into_any_element()
        } else if let Some(expanded) = expanded_page {
            expanded.into_any_element()
        } else {
            match self.route() {
                Route::Connect => self.connect_screen(window, cx).into_any_element(),
                Route::Sessions => self.sessions_screen(window, cx).into_any_element(),
                Route::Schedules
                | Route::ScheduleEdit
                | Route::ScheduleDetail
                | Route::ScheduleConfirm
                | Route::ScheduleHistory => self.schedules_screen(window, cx),
                Route::Projects => self.projects_screen(window, cx).into_any_element(),
                Route::Start => self.start_screen(window, cx).into_any_element(),
                Route::File => self.file_screen(window, cx).into_any_element(),
                Route::Thread(id) => self.thread_screen(id, window, cx),
                Route::Command(id, token) => self.command_screen(id, token, window, cx),
                Route::Terminal(id, token) => self.terminal_screen(id, token, window, cx),
                Route::Review(id) => self.review_screen(id, window, cx),
                Route::History(id) => self.history_screen(id, window, cx),
                Route::Diagnostics(id) => self.diagnostics_screen(id, window, cx),
                Route::Subagents(id) => self.subagents_screen(id, window, cx),
                Route::Swarm(id, pick) => self.swarm_screen(id, pick, window, cx),
                Route::Subagent(id, pick) | Route::SwarmPeer(id, pick, _) => {
                    self.subagent_screen(id, pick, window, cx)
                }
                Route::Settings => self.settings_screen(window, cx).into_any_element(),
            }
        };
        let sheet = self.sheet.or(self.closing_sheet).map(|sheet| {
            let content = self.sheet_content(sheet, window, cx);
            let max_sheet_height = window.viewport_size().height - insets.top - px(24.);
            let activity_height = matches!(sheet, Sheet::Activity(..)).then(|| {
                self.activity_view
                    .as_ref()
                    .map_or(px(240.), |view| view.panel_height())
                    .min(max_sheet_height)
            });
            let progress = self.sheet_motion.position();
            let height = self.sheet_height.get();
            let sheet_drag = if height > px(0.) {
                height
            } else {
                window.viewport_size().height
            } * progress;
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .justify_end()
                .child(
                    div()
                        .id("scrim")
                        .occlude()
                        .absolute()
                        .inset_0()
                        .bg(colors.scrim)
                        // Dim the exposed rounded corners before an opening panel
                        // enters. On close, keep fading with the departing panel.
                        .opacity(sheet_scrim_opacity(self.sheet.is_some(), progress))
                        .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                )
                .child(
                    div()
                        .id("sheet")
                        .debug_selector(|| "bottom-sheet".into())
                        .relative()
                        .child(crate::testing::probe("bottom-sheet"))
                        .top(sheet_drag)
                        .when(self.sheet.is_none(), |sheet| sheet.min_h(height))
                        .occlude()
                        .max_h(max_sheet_height)
                        // Virtual lists need a definite viewport; long sheets keep their
                        // header fixed while only their retained rows scroll.
                        .when(
                            matches!(
                                sheet,
                                Sheet::Project | Sheet::EditorRoot | Sheet::EditorFiles
                            ),
                            |panel| panel.h(max_sheet_height),
                        )
                        .when_some(activity_height, |panel, height| panel.h(height))
                        .flex()
                        .flex_col()
                        .bg(colors.canvas)
                        .rounded_t(px(24.))
                        .shadow(vec![gpui::BoxShadow {
                            color: colors.shadow,
                            offset: gpui::point(px(0.), px(-10.)),
                            blur_radius: px(32.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .pt(px(8.))
                        .pb(insets.bottom + px(12.))
                        .pl(insets.left)
                        .pr(insets.right)
                        .child(
                            div()
                                .flex_none()
                                .mx_auto()
                                .mt(px(4.))
                                .mb(px(16.))
                                .w(px(32.))
                                .h(px(4.))
                                .rounded_full()
                                .bg(colors.line_strong),
                        )
                        .child(content)
                        .child(self.dismiss_gesture(cx)),
                )
                // Keep the exiting panel mounted and occluding until it is offscreen.
                .when(self.sheet.is_none(), |overlay| {
                    overlay.child(
                        div()
                            .id("closing-sheet-blocker")
                            .absolute()
                            .inset_0()
                            .occlude(),
                    )
                })
        });
        let rendering_stats = self.render_stats.visible.then(|| {
            let label: SharedString = format!(
                "{}{}",
                self.render_stats.label(),
                crate::visual_profile::label(cx)
            )
            .into();
            div()
                .id("rendering-stats")
                .debug_selector(|| "rendering-stats".into())
                .absolute()
                .top(insets.top + px(76.))
                .right(insets.right + px(10.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(gpui::rgba(0x05070ddd))
                .text_color(gpui::rgba(0xe8edf2ff))
                .text_size(px(10.))
                .line_height(px(14.))
                .font_family(crate::theme::MONO)
                .child(label)
        });
        let notice = self.notice.clone().map(|notice| {
            let generation = self.notice_generation;
            let top = insets.top + px(68.);
            let announcement = notice.clone();
            let drawer = div()
                .id("notice-drawer-layer")
                .absolute()
                .left(insets.left + px(12.))
                .right(insets.right + px(12.))
                .top(top)
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("notice-drawer")
                        .debug_selector(|| "notice-drawer".into())
                        .relative()
                        .child(crate::testing::probe("notice-drawer"))
                        .role(Role::Alert)
                        .aria_label(announcement)
                        .occlude()
                        .w_full()
                        .max_w(px(560.))
                        .min_w_0()
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .rounded(px(16.))
                        .bg(colors.composer)
                        .border_1()
                        .border_color(colors.line_strong)
                        .shadow(vec![gpui::BoxShadow {
                            color: colors.shadow,
                            offset: gpui::point(px(0.), px(8.)),
                            blur_radius: px(24.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .child(
                            div()
                                .p(px(12.))
                                .pl(px(14.))
                                .pr(px(8.))
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .items_start()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .size(px(32.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(colors.tint(colors.accent))
                                        .child(crate::ui::icon("info", 17., colors.accent)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .pt(px(4.))
                                        .text_size(px(14.))
                                        .line_height(relative(1.35))
                                        .text_color(colors.text)
                                        .child(notice),
                                )
                                .child(
                                    crate::ui::tap(("dismiss-notice", generation), "x", &colors)
                                        .role(Role::Button)
                                        .aria_label("Dismiss notification")
                                        .debug_selector(|| "dismiss-notice".into())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if this.notice_generation == generation {
                                                this.notice = None;
                                                this.notice_generation += 1;
                                                cx.notify();
                                            }
                                        })),
                                ),
                        ),
                );
            if cx.reduce_motion() {
                drawer.into_any_element()
            } else {
                drawer
                    .with_animation(
                        ("notice-drawer-unroll", generation),
                        Animation::new(Duration::from_millis(220)),
                        move |drawer, progress| {
                            drawer
                                .top(top - px(14.) * (1. - progress))
                                .opacity(progress)
                        },
                    )
                    .into_any_element()
            }
        });
        let root = div()
            .id("pi")
            .key_context("PhoneApp")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::go_back))
            .size_full()
            .relative()
            .bg(colors.canvas)
            .text_color(colors.text)
            .font_family(SANS)
            .text_size(px(15.))
            .line_height(relative(1.45))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .pt(insets.top)
                    .pb(insets.bottom)
                    .pl(insets.left)
                    .pr(insets.right)
                    .child(screen),
            )
            .children(sheet)
            .children(notice)
            .children(rendering_stats)
            // Publish only after actual painting. Measurement of virtual rows
            // must neither execute HTML nor leave a native window above a sheet.
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |_, _, _, _| {
                        let slot = inline_slot.borrow_mut().take();
                        activity::sync_inline_page(if native_page_visible {
                            slot.as_deref().unwrap_or("{}")
                        } else {
                            "{}"
                        });
                    },
                )
                .absolute()
                .size_full(),
            );
        if let Some(start) = render_start {
            self.render_stats.observe_build(start.elapsed());
        }
        crate::visual_profile::root(root)
    }
}

fn sheet_scrim_opacity(open: bool, progress: f32) -> f32 {
    if open { 1. } else { 1. - progress }
}

impl PhoneApp {
    pub(crate) fn show_selected_text(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selectable
            .update(cx, |area, cx| area.show_selected(text, cx));
        self.open_sheet(Sheet::SelectText, cx);
        self.sheet_focus_pending = false;
        window.focus(&self.selectable.read(cx).focus_handle(cx), cx);
    }

    /// Laid over a block of content: a long press opens `text` to select
    /// and copy all of it or part, with the phone's buzz.
    pub(crate) fn copyable(
        &self,
        text: impl Into<SharedString>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let text = text.into();
        let app = cx.entity().downgrade();
        gpui::canvas(
            |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                let text = text.clone();
                let app = app.clone();
                window.on_mouse_event(move |event: &gpui::LongPressEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble
                        || event.phase != gpui::TouchPhase::Started
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }
                    window.prevent_default();
                    activity::long_press_feedback();
                    app.update(cx, |app, cx| {
                        app.selectable
                            .update(cx, |area, cx| area.show_selected(text.to_string(), cx));
                        app.open_sheet(Sheet::SelectText, cx);
                        // The text takes focus, so its bar shows, not the app.
                        app.sheet_focus_pending = false;
                        let focus = app.selectable.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                    })
                    .ok();
                });
            },
        )
        .absolute()
        .inset_0()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn active_composer_contract_includes_review_and_rejects_stale_targets() {
        use super::{Route, Target, composer_target_active};
        use crate::model::SessionId;
        assert!(composer_target_active(Target::Start, Route::Start));
        assert!(composer_target_active(
            Target::Review(SessionId(1)),
            Route::Review(SessionId(1))
        ));
        assert!(composer_target_active(
            Target::Thread(SessionId(1)),
            Route::Thread(SessionId(1))
        ));
        assert!(!composer_target_active(
            Target::Thread(SessionId(1)),
            Route::Thread(SessionId(2))
        ));
        assert!(!composer_target_active(
            Target::Review(SessionId(1)),
            Route::Thread(SessionId(1))
        ));
        assert!(!composer_target_active(
            Target::Review(SessionId(1)),
            Route::Review(SessionId(2))
        ));
        assert!(!composer_target_active(Target::Start, Route::Sessions));
    }
    use super::{PhoneApp, RenderStats, sheet_scrim_opacity};
    use crate::{live::Update, model::SessionId};
    use gpui::{AppContext, TestAppContext};
    use std::time::{Duration, Instant};

    #[test]
    fn staged_review_sections_encode_as_one_structured_prompt() {
        use super::{ReviewComment, batched_review_prompt};
        use crate::composer::Attachment;
        let comments = vec![
            ReviewComment {
                id: 1,
                path: "src/a.rs".into(),
                anchor: (0, 1),
                lines: "lines 10–12".into(),
                text: "Handle the empty case.".into(),
                attachments: Vec::new(),
            },
            ReviewComment {
                id: 2,
                path: "src/b.rs".into(),
                anchor: (0, 0),
                lines: "line 44".into(),
                text: "Keep this documented.".into(),
                attachments: vec![Attachment::File {
                    name: "note.txt".into(),
                    size: "4 B".into(),
                    contents: "note".into(),
                }],
            },
        ];
        let (prompt, labels) = batched_review_prompt(&comments, false).unwrap();
        assert!(
            prompt
                .message
                .starts_with("Please address these 2 review comments:")
        );
        assert!(
            prompt
                .message
                .contains("File: src/a.rs\nLocation: lines 10–12")
        );
        assert!(prompt.message.contains("Handle the empty case."));
        assert!(prompt.message.contains("File: src/b.rs\nLocation: line 44"));
        assert!(prompt.message.contains("Attached text file: \"note.txt\""));
        assert_eq!(
            labels,
            ["src/a.rs · lines 10–12", "src/b.rs · line 44", "note.txt"]
        );
        assert!(prompt.images.is_empty());
    }

    #[test]
    fn visualize_expands_only_draft_text_before_attachment_and_review_encoding() {
        use crate::composer::Attachment;
        let files = [Attachment::File {
            name: "note.txt".into(),
            size: "10 B".into(),
            contents: "@Visualize".into(),
        }];
        let inert = super::prompt_for("Read this", &files, None).unwrap();
        assert!(!inert.message.contains(crate::visualize_prompt::CONTRACT));
        assert_eq!(
            super::prompt_for(&inert.message, &[], None)
                .unwrap()
                .message,
            inert.message
        );
        let active = super::prompt_for("@Visualize build a calc", &files, None).unwrap();
        assert_eq!(
            active
                .message
                .matches(crate::visualize_prompt::CONTRACT)
                .count(),
            1
        );
        assert!(
            active
                .label()
                .starts_with("@Visualize build a calc\n\nAttached text file:")
        );
        assert!(!active.label().contains("Pi native visual app request"));
        assert_eq!(
            super::prompt_for(&active.message, &[], None)
                .unwrap()
                .message,
            active.message
        );
        let review = super::prompt_for(
            "@Visualize show a counter",
            &[Attachment::Lines("lines 1–2".into())],
            Some("src/main.rs"),
        )
        .unwrap();
        assert!(
            review
                .message
                .starts_with("About lines 1–2 of src/main.rs: @Visualize")
        );
        assert_eq!(
            review
                .message
                .matches(crate::visualize_prompt::CONTRACT)
                .count(),
            1
        );
    }

    #[test]
    fn render_stats_measure_active_frames_without_counting_idle_time() {
        let mut stats = RenderStats::default();
        let start = Instant::now();
        stats.observe(start);
        stats.observe(start + Duration::from_millis(8));
        assert_eq!(stats.intervals.len(), 1);
        stats.observe(start + Duration::from_secs(1));
        assert!(stats.intervals.is_empty());
    }

    #[test]
    fn render_stats_name_current_and_max_refresh_rates() {
        let mut stats = RenderStats {
            refresh_rate: Some(60.),
            max_refresh_rate: Some(120.),
            ..RenderStats::default()
        };
        let start = Instant::now();
        stats.observe(start);
        stats.observe(start + Duration::from_millis(22));
        let label = stats.label();
        assert!(label.contains("45 FPS / 60 Hz (120 max)"), "{label}");
        assert!(label.contains("late 1/1"), "{label}");
    }

    #[test]
    fn render_stats_tolerate_small_vsync_jitter() {
        let mut stats = RenderStats {
            refresh_rate: Some(60.),
            ..RenderStats::default()
        };
        let start = Instant::now();
        stats.observe(start);
        stats.observe(start + Duration::from_millis(17));
        let label = stats.label();
        assert!(label.contains("late 0/1"), "{label}");
    }

    #[gpui::test]
    fn native_manipulation_defers_remote_records_and_release_preserves_queue_order(
        cx: &mut TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let barrier = app.update(cx, |app, _| {
            app.visual_apps
                .borrow_mut()
                .test_projection_barrier(app.route())
        });
        let lease = barrier.acquire();
        app.update(cx, |app, cx| {
            assert!(app.projection_deferred());
            app.take_updates(
                vec![(Some(SessionId(1)), Update::Ended(1, "first".into()))],
                cx,
            );
            assert!(app.deferred_flush_scheduled);
            assert_eq!(app.deferred_updates.len(), 1);
        });
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        app.read_with(cx, |app, _| assert_eq!(app.deferred_updates.len(), 1));
        drop(lease);
        app.update(cx, |app, cx| {
            assert!(!app.projection_deferred());
            app.take_updates(
                vec![(Some(SessionId(1)), Update::Ended(1, "second".into()))],
                cx,
            );
            let reasons = app
                .deferred_updates
                .iter()
                .map(|(_, update)| match update {
                    Update::Ended(_, reason) => reason.as_str(),
                    _ => "wrong update",
                })
                .collect::<Vec<_>>();
            assert_eq!(reasons, ["first", "second"]);
        });
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.deferred_flush_scheduled);
            assert!(app.deferred_updates.is_empty());
        });
    }

    #[gpui::test]
    fn interrupted_scroll_does_not_freeze_updates_or_timer_after_foregrounding(
        cx: &mut TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        app.update(cx, |app, _| {
            app.store = Some(crate::store::Store::sample(
                crate::model::Computer::from_address("sample@computer.local"),
            ));
        });
        for hidden in [false, true] {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            let (id, elapsed) = app.update(cx, |app, cx| {
                let session = app.store.as_ref().unwrap().running().next().unwrap();
                let before = (session.id, session.elapsed);
                crate::scroll::note_input(gpui::TouchPhase::Moved);
                crate::scroll::note_movement(gpui::px(100.));
                app.take_updates(
                    vec![(Some(before.0), Update::Ended(1, "queued".into()))],
                    cx,
                );
                assert!(app.projection_deferred());
                assert!(app.deferred_flush_scheduled);
                // A tick during a gesture must not change the projection.
                app.last_tick = Instant::now() - Duration::from_secs(1);
                app.tick(cx);
                assert_eq!(
                    app.store
                        .as_ref()
                        .unwrap()
                        .session(before.0)
                        .unwrap()
                        .elapsed,
                    before.1
                );
                before
            });
            // No TouchPhase::Ended arrives when the OS interrupts the gesture.
            if hidden {
                cx.simulate_visibility_change(gpui::WindowVisibility::Hidden);
                cx.simulate_visibility_change(gpui::WindowVisibility::Visible);
            } else {
                cx.deactivate_window();
                cx.update(|window, _| window.activate_window());
            }
            app.read_with(cx, |app, _| assert!(!app.projection_deferred()));
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            app.update(cx, |app, cx| {
                assert!(!app.deferred_flush_scheduled);
                assert!(app.deferred_updates.is_empty());
                app.tick(cx);
                assert!(app.store.as_ref().unwrap().session(id).unwrap().elapsed > elapsed);
            });
        }
    }

    #[gpui::test]
    fn a_later_batch_joins_an_existing_deferred_queue_in_order(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.deferred_flush_scheduled = true;
                app.deferred_updates
                    .push((Some(SessionId(1)), Update::Ended(1, "first".into())));
                app.take_updates(
                    vec![(Some(SessionId(1)), Update::Ended(1, "second".into()))],
                    cx,
                );
                let reasons = app
                    .deferred_updates
                    .iter()
                    .map(|(_, update)| match update {
                        Update::Ended(_, reason) => reason.as_str(),
                        _ => "wrong update",
                    })
                    .collect::<Vec<_>>();
                assert_eq!(reasons, ["first", "second"]);
            })
        });
    }

    #[gpui::test]
    fn child_conversations_keep_independent_drafts(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let parent = cx.new(|cx| crate::composer::Composer::new("Parent…", cx));
                parent.update(cx, |composer, cx| composer.set_text("parent draft", cx));
                app.threads.insert(SessionId(7), parent.clone());
                let first = app.subagent_composer(SessionId(7), "child-a", "Steer…", window, cx);
                first.update(cx, |composer, cx| composer.set_text("draft for A", cx));
                let second =
                    app.subagent_composer(SessionId(7), "child-b", "Follow up…", window, cx);
                second.update(cx, |composer, cx| composer.set_text("draft for B", cx));
                let first_again =
                    app.subagent_composer(SessionId(7), "child-a", "Steer…", window, cx);

                assert_eq!(first_again.read(cx).area.read(cx).text(), "draft for A");
                assert_eq!(second.read(cx).area.read(cx).text(), "draft for B");
                assert_eq!(parent.read(cx).area.read(cx).text(), "parent draft");
                assert_eq!(app.subagent_composers.len(), 2);
            })
        });
    }

    #[gpui::test]
    fn replacing_store_clears_child_projection_cache_and_drafts(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let parent = SessionId(7);
                let child = SessionId(u32::MAX);
                let cached = crate::store::Store::sample(crate::model::Computer::from_address(
                    "old@computer.local",
                ))
                .sessions
                .remove(0);
                app.subagent_ids.insert((parent, "child-a".into()), child);
                app.subagent_sessions.insert(child, cached.clone());
                app.subagent_projected_revisions.insert(child, 1);
                app.thread_snapshots
                    .insert(child, std::sync::Arc::new(cached));
                app.subagent_composer(parent, "child-a", "Steer…", window, cx);

                app.replace_store(
                    Some(crate::store::Store::sample(
                        crate::model::Computer::from_address("new@computer.local"),
                    )),
                    cx,
                );

                assert!(app.subagent_ids.is_empty());
                assert!(app.subagent_sessions.is_empty());
                assert!(app.subagent_projected_revisions.is_empty());
                assert!(app.subagent_composers.is_empty());
                assert!(app.thread_snapshots.is_empty());
            })
        });
    }

    #[test]
    fn sheet_scrim_is_immediate_on_open_and_fades_on_close() {
        assert_eq!(sheet_scrim_opacity(true, 1.), 1.);
        assert_eq!(sheet_scrim_opacity(true, 0.5), 1.);
        assert_eq!(sheet_scrim_opacity(false, 0.), 1.);
        assert_eq!(sheet_scrim_opacity(false, 0.5), 0.5);
        assert_eq!(sheet_scrim_opacity(false, 1.), 0.);
    }
    #[cfg(all(feature = "local-backend", target_os = "android"))]
    #[gpui::test]
    fn local_reselection_retires_remote_generations_without_losing_conversations(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::Target;
        use crate::store::Event;
        use pi_local::auth::{Provider, Vault, VaultScope};
        struct EmptyVault;
        impl Vault for EmptyVault {
            fn scope(&self) -> VaultScope {
                VaultScope::Synthetic
            }
            fn read(&self, _: Provider) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::json!({"generation":"0","credential":null}))
            }
            fn write(&self, _: Provider, _: &str, _: &str) -> anyhow::Result<Option<String>> {
                anyhow::bail!("unused")
            }
            fn clear(&self, _: Provider) -> anyhow::Result<()> {
                Ok(())
            }
        }
        cx.update(|cx| cx.set_reduce_motion(true));
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.open_local_chat(
                    Provider::Claude,
                    std::sync::Arc::new(EmptyVault),
                    true,
                    window,
                    cx
                ));
                let store = app.store.as_ref().unwrap().generation;
                let route = app.route();
                let attempt = app.begin_connection_attempt();
                app.reconnecting = true;
                app.select_local_backend(window, cx);
                assert!(!app.connecting && !app.reconnecting);
                assert_ne!(app.connection_generation, attempt);
                assert_eq!(app.store.as_ref().unwrap().generation, store);
                assert_eq!(app.route(), route);
                let id = app.model_session().unwrap();
                assert!(app.compact_available_for_session(id));
                assert_eq!(
                    app.compact_for_session(id, "/compact keep decisions"),
                    Some(Some("keep decisions".into()))
                );
                assert_eq!(app.compact_for_session(id, "/compactly"), None);
                let composer = app.thread_composer(id, window, cx);
                let target = Target::Thread(id);
                composer.update(cx, |c, cx| c.set_text("/compact keep decisions", cx));
                app.pending_compacts
                    .insert(id, (target, "/compact keep decisions".into()));
                composer.update(cx, |c, cx| c.clear(cx));
                app.alert(Event::Compacted(id, Some("Nothing to compact".into())), cx);
                assert!(!app.compact_pending(id));
                assert_eq!(
                    composer.read(cx).area.read(cx).text(),
                    "/compact keep decisions"
                );
                app.pending_compacts
                    .insert(id, (target, "/compact old command".into()));
                composer.update(cx, |c, cx| c.set_text("new draft", cx));
                let import = composer.update(cx, |c, cx| c.begin_import(cx));
                app.alert(Event::Compacted(id, Some("Cancelled".into())), cx);
                assert_eq!(
                    composer.read(cx).area.read(cx).text(),
                    "new draft",
                    "disabled Send during import must not replace a later draft"
                );
                composer.update(cx, |c, cx| c.finish_import(import, cx));
                app.pending_compacts.insert(id, (target, "/compact".into()));
                app.alert(Event::Compacted(id, None), cx);
                assert!(!app.compact_pending(id));
                assert_eq!(composer.read(cx).area.read(cx).text(), "new draft");
                assert_eq!(
                    super::prompt_with_capabilities(
                        "@Visualize make a calculator",
                        &[],
                        None,
                        false
                    )
                    .unwrap()
                    .message,
                    "@Visualize make a calculator"
                );
            })
        });
    }
}
