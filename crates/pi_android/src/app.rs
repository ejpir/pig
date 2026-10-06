//! The phone app: a stack of screens, at most one bottom sheet over them, a
//! short notice at the bottom, and a computer's sessions underneath, or the
//! sample ones.
//!
//! Back closes the sheet, then the search, then the screen; on the first
//! screen it leaves the app, as Android expects.

use crate::{
    alerts::{self, Link},
    composer::{Attachment, Composer, ComposerEvent},
    live::{Live, Update},
    model::{Answer, Computer, SessionId, State},
    motion::SwipeMotion,
    prefs::{Prefs, SavedCommand},
    projects::{FileView, ProjectBrowser},
    remote,
    ssh::{self, Address, Connection, Identity},
    store::{Event, Store},
    text_area::{TextArea, TextAreaEvent},
    theme::{Appearance, SANS, Theme, theme},
};
use gpui::{
    App, Context, Edges, Entity, FocusHandle, Focusable, PathPromptOptions, Pixels, ScrollHandle,
    SharedString, Subscription, Task, TextInputAction, TextInputConfiguration, Window,
    WindowAppearance, WindowVisibility, actions, div, prelude::*, px, relative,
};
use gpui_android::activity;
use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

actions!(pi_android, [GoBack]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Connect,
    Sessions,
    Projects,
    Start,
    /// A file from the project browser, read-only.
    File,
    Thread(SessionId),
    Review(SessionId),
    History(SessionId),
    Settings,
}

/// Which composer a sheet acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Start,
    Thread(SessionId),
    Review,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    Details(SessionId),
    Attach(Target),
    Model,
    Thinking,
    Project,
    More(SessionId),
    Models,
    Resources,
    Activity(SessionId, usize, usize),
    Delete(SessionId),
    RestoreHistory(SessionId, usize),
    EnableJj(SessionId),
    Image(Target, usize),
    /// An image a tool returned: session, turn, which.
    ToolImage(SessionId, usize, usize),
    /// What the app logged lately, from Settings.
    Logs,
    Computers,
    /// Text from the thread, to select part of and copy.
    SelectText,
}

#[derive(Clone, Debug)]
pub(crate) enum JjHistoryState {
    Loading,
    Loaded(remote::JjHistory),
    Failed(SharedString),
}

pub struct PhoneApp {
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
    sheet_height: Rc<Cell<Pixels>>,
    /// The new-session sheet as it is dragged down to close.
    pub(crate) start_motion: SwipeMotion,
    pub(crate) start_height: Rc<Cell<Pixels>>,
    /// Zoom and pan of the image a sheet shows, and where it is on screen.
    pub(crate) image_zoom: f32,
    pub(crate) image_pan: gpui::Point<Pixels>,
    pub(crate) image_box: Rc<Cell<gpui::Bounds<Pixels>>>,
    pub(crate) sheet_scroll: ScrollHandle,
    pub(crate) expanded_turns: HashMap<(SessionId, usize), bool>,
    /// Finished prompts shown in full instead of on one line.
    pub(crate) expanded_prompts: HashSet<(SessionId, usize)>,
    /// Questions put off with Later: the composer shows instead of the card.
    pub(crate) questions_later: HashSet<SessionId>,
    /// Report cards showing every changed file, not just the first few.
    pub(crate) all_files: HashSet<SessionId>,
    /// A sheet transition gives focus back to the app once, before its fields can focus.
    sheet_focus_pending: bool,
    /// The answer picked in the question sheet; sent only with Answer.
    pub(crate) choice: Option<Answer>,
    /// Open disclosures in the details sheet.
    pub(crate) expanded: HashSet<&'static str>,
    pub(crate) notice: Option<SharedString>,
    notice_generation: usize,
    pub(crate) focus: FocusHandle,
    pub(crate) address: Entity<TextArea>,
    /// The text a long press opened, to select and copy parts of.
    pub(crate) selectable: Entity<TextArea>,
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
    /// Where the key and settings live.
    pub(crate) data_dir: PathBuf,
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
    pub(crate) searching: bool,
    pub(crate) start: Entity<Composer>,
    pub(crate) start_visible_height: Option<gpui::Pixels>,
    pub(crate) project: usize,
    pub(crate) project_browser: ProjectBrowser,
    pub(crate) file_view: Option<FileView>,
    /// Tool images decoded once, by key.
    pub(crate) tool_images: std::cell::RefCell<HashMap<String, crate::screens::ShownImage>>,
    /// Page posters asked for, so each is drawn once.
    pub(crate) posters_drawing: std::cell::RefCell<std::collections::HashSet<PathBuf>>,
    /// Invalidates file replies when another file opens.
    pub(crate) file_generation: u64,
    /// Invalidates project-tree replies when the selected folder changes.
    pub(crate) project_files_generation: u64,
    pub(crate) threads: HashMap<SessionId, Entity<Composer>>,
    /// Sample sessions have their own model selection, just like live sessions.
    sample_models: HashMap<SessionId, (String, String)>,
    pub(crate) review: Entity<Composer>,
    pub(crate) review_file: usize,
    /// Tapped review lines, as (hunk, line) of the shown file.
    pub(crate) review_lines: BTreeSet<(usize, usize)>,
    scrolls: HashMap<Route, ScrollHandle>,
    pub(crate) prefs_path: Option<PathBuf>,
    visible: bool,
    /// The text of the working notification, while one is shown.
    working_posted: Option<String>,
    last_tick: Instant,
    /// Holds the sample sessions still, for previews.
    pub(crate) paused: bool,
    subscriptions: Vec<Subscription>,
    _ticker: Task<()>,
}

impl PhoneApp {
    pub fn new(prefs_path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let review = cx.new(|cx| Composer::new("Ask for a revision…", cx));
        let selectable = cx.new(|cx| TextArea::read_only(12, cx));
        let subscriptions = vec![
            cx.subscribe(&selectable, |this, _, event: &TextAreaEvent, cx| {
                if *event == TextAreaEvent::Copied {
                    this.close_sheet(cx);
                    this.notify_user("Copied", cx);
                }
            }),
            cx.subscribe_in(&address, window, |this, _, event, window, cx| match event {
                TextAreaEvent::Submit => this.connect(window, cx),
                TextAreaEvent::Changed => {
                    this.connect_error = None;
                    cx.notify();
                }
                TextAreaEvent::Copied => {}
            }),
            cx.subscribe(&search, |_, _, _: &TextAreaEvent, cx| cx.notify()),
            cx.subscribe(&model_search, |this, _, event: &TextAreaEvent, cx| {
                if *event == TextAreaEvent::Changed {
                    this.sheet_scroll.set_offset(gpui::Point::default());
                }
                cx.notify();
            }),
            cx.subscribe_in(&folder, window, |this, _, event, window, cx| match event {
                TextAreaEvent::Submit => this.submit_folder_search(window, cx),
                TextAreaEvent::Changed => {
                    this.sheet_scroll.set_offset(gpui::Point::default());
                    cx.notify();
                }
                TextAreaEvent::Copied => {}
            }),
            cx.subscribe_in(&start, window, |this, _, event, window, cx| {
                this.composer_event(Target::Start, event, window, cx)
            }),
            cx.subscribe_in(&review, window, |this, _, event, window, cx| {
                this.composer_event(Target::Review, event, window, cx)
            }),
            cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)),
            cx.observe_window_visibility(window, |this, visibility, _, cx| {
                this.set_visible(visibility, cx)
            }),
        ];
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
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
            expanded_turns: HashMap::new(),
            expanded_prompts: HashSet::new(),
            questions_later: HashSet::new(),
            all_files: HashSet::new(),
            sheet_focus_pending: false,
            choice: None,
            expanded: HashSet::new(),
            notice: None,
            notice_generation: 0,
            focus: cx.focus_handle(),
            address,
            selectable,
            connect_error: None,
            connecting: false,
            connection_generation: 0,
            pairing_status: None,
            phone_key,
            key_refused: false,
            manual_setup: false,
            data_dir,
            folder,
            reconnecting: false,
            last_listed: Instant::now(),
            last_reconnect: Instant::now(),
            _pump: None,
            search,
            model_search,
            preview_models: Vec::new(),
            searching: false,
            start,
            start_visible_height: None,
            project: 0,
            project_browser: ProjectBrowser::default(),
            file_view: None,
            tool_images: Default::default(),
            posters_drawing: Default::default(),
            file_generation: 0,
            project_files_generation: 0,
            threads: HashMap::new(),
            sample_models: HashMap::new(),
            review,
            review_file: 0,
            review_lines: BTreeSet::new(),
            scrolls: HashMap::new(),
            prefs_path,
            visible: window.visibility().is_visible(),
            working_posted: None,
            last_tick: Instant::now(),
            paused: false,
            subscriptions,
            _ticker: ticker,
        };
        app.apply_theme(window, cx);
        app.apply_return_sends(cx);
        if let Some(address) = computer {
            app.address
                .update(cx, |field, cx| field.set_text(address.clone(), cx));
            if sample {
                app.open_store(&address);
            } else {
                app.connect(window, cx);
            }
        }
        window.focus(&app.focus, cx);
        app
    }

    pub fn route(&self) -> Route {
        self.routes.last().copied().unwrap_or(Route::Connect)
    }

    pub(crate) fn scroll(&mut self, route: Route) -> ScrollHandle {
        self.scrolls.entry(route).or_default().clone()
    }

    /// Follow content updates only while the reader is at the bottom. Doing
    /// this in render would also undo upward swipes and activity expansion.
    fn keep_latest_in_view(&mut self) {
        if let Route::Thread(_) = self.route() {
            let scroll = self.scroll(self.route());
            if scroll.bounds().size.height > px(0.)
                && scroll.max_offset().y + scroll.offset().y <= px(2.)
            {
                scroll.scroll_to_bottom();
            }
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
        let path = self.store.as_ref()?.projects.get(project)?.path.clone();
        self.command_catalog_for_path(&path, cx)
    }

    pub(crate) fn command_catalog_for_session(
        &self,
        id: SessionId,
        cx: &App,
    ) -> Option<Vec<SavedCommand>> {
        let store = self.store.as_ref()?;
        let live = store.live.as_ref()?;
        if let Some(commands) = live.commands(id) {
            return Some(commands.iter().map(SavedCommand::from).collect());
        }
        let path = live.session_cwd(id)?.to_owned();
        self.command_catalog_for_path(&path, cx)
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
        activity::set_bar_icons(match appearance {
            Appearance::System => activity::BarIcons::System,
            _ if dark => activity::BarIcons::Light,
            _ => activity::BarIcons::Dark,
        });
        cx.notify();
    }

    pub(crate) fn apply_return_sends(&mut self, cx: &mut Context<Self>) {
        let sends = self.prefs(cx).return_sends;
        let composers = [self.start.clone(), self.review.clone()]
            .into_iter()
            .chain(self.threads.values().cloned());
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

    /// Goes to a session from anywhere, with the list behind it.
    pub(crate) fn show_session(
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
        self.routes = vec![Route::Sessions, Route::Thread(id)];
        self.entered(window, cx);
    }

    fn entered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.swiping_session = None;
        self.close_sheet(cx);
        self.searching = false;
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        if let Route::Thread(id) | Route::Review(id) | Route::History(id) = self.route() {
            activity::cancel_notification(alerts::question_id(id));
            activity::cancel_notification(alerts::finished_id(id));
            // A question waiting in the session takes the composer's place.
            self.questions_later.remove(&id);
        }
        cx.notify();
    }

    pub(crate) fn open_sheet(&mut self, sheet: Sheet, cx: &mut Context<Self>) {
        self.closing_sheet = None;
        self.sheet_motion.settle(0.);
        self.sheet_scroll = ScrollHandle::new();
        if sheet == Sheet::Model {
            self.model_search
                .update(cx, |area, cx| area.set_text("", cx));
        }
        self.sheet = Some(sheet);
        self.image_zoom = 1.;
        self.image_pan = gpui::Point::default();
        if sheet == Sheet::Project {
            self.open_project_browser(false, cx);
        }
        self.sheet_focus_pending = true;
        cx.notify();
    }

    pub(crate) fn close_sheet(&mut self, cx: &mut Context<Self>) {
        if let Some(sheet) = self.sheet.take() {
            self.closing_sheet = Some(sheet);
            self.sheet_motion.settle(1.);
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
                        let motion = &mut this.sheet_motion;
                        let extent = bounds.size.height.max(px(1.));
                        if event.touch_phase == gpui::TouchPhase::Started {
                            if !bounds.contains(&event.position)
                                || (this.image_zoom > 1.01
                                    && this.image_box.get().contains(&event.position))
                                || along <= px(0.)
                                || along.abs() < across.abs()
                                || (this.sheet_scroll.max_offset().y > px(1.)
                                    && this.sheet_scroll.bounds().contains(&event.position)
                                    && event.position.x
                                        >= this.sheet_scroll.bounds().right() - px(18.))
                                || (event.position.y > bounds.top() + px(36.)
                                    && this.sheet_scroll.offset().y < px(-1.))
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
        if self.sheet.is_some() {
            self.close_sheet(cx);
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
        if self.routes.len() > 1 {
            let left = self.routes.pop();
            if left == Some(Route::File)
                && self.file_view.as_ref().is_some_and(|view| view.from_sheet)
            {
                self.open_sheet(Sheet::Project, cx);
            }
            if !matches!(self.route(), Route::Review(_)) {
                self.review_lines.clear();
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
        self.notice = Some(text.into());
        self.notice_generation += 1;
        let generation = self.notice_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update(cx, |this, cx| {
                if this.notice_generation == generation {
                    this.notice = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // Connecting

    /// The sample sessions, for trying the app without a computer.
    pub(crate) fn open_store(&mut self, address: &str) {
        self.project_browser.clear();
        self.project_files_generation += 1;
        self.project = 0;
        self.store = Some(Store::sample(Computer::from_address(address)));
        self.routes = vec![Route::Sessions];
        self.threads.clear();
        self.sample_models.clear();
        self.jj_histories.clear();
        self.restoring_history = None;
        self.enabling_jj = None;
        self.swiping_session = None;
        self.deleting_session = None;
        self.scrolls.clear();
        self._pump = None;
    }

    pub(crate) fn open_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_connection_attempt();
        self.paused = false;
        let typed = self.address.read(cx).text().trim().to_owned();
        let address = if typed.is_empty() {
            "you@studio-mac.local".to_owned()
        } else {
            typed
        };
        self.open_store(&address);
        self.start.update(cx, |start, _| {
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
        self.connection_generation = self.connection_generation.wrapping_add(1);
        self.connecting = true;
        self.connection_generation
    }

    /// Retires an in-flight connection without relying on its transport being cancellable.
    /// Its task may finish, but the generation check prevents stale navigation or state.
    pub(crate) fn cancel_connection_attempt(&mut self) {
        self.connection_generation = self.connection_generation.wrapping_add(1);
        self.connecting = false;
        self.pairing_status = None;
    }

    pub(crate) fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        let address = match Address::parse(self.address.read(cx).text()) {
            Ok(address) => address,
            Err(error) => {
                self.connect_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let identity = match self.identity() {
            Ok(identity) => identity,
            Err(error) => {
                self.connect_error = Some(format!("{error:#}").into());
                cx.notify();
                return;
            }
        };
        let known = self.prefs(cx).host_keys.get(&address.to_string()).cloned();
        let generation = self.begin_connection_attempt();
        self.pairing_status = None;
        self.connect_error = None;
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let reached = async {
                let connection = Connection::open(address.clone(), identity, known).await?;
                let helper = remote::find(&connection).await?;
                let listed = remote::sessions(&connection, &helper).await?;
                anyhow::Ok((connection, helper, listed))
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                if this.connection_generation != generation {
                    return;
                }
                this.connecting = false;
                match reached {
                    Ok((connection, helper, listed)) => {
                        this.connected(address, connection, helper, listed, window, cx)
                    }
                    Err(error) => {
                        this.key_refused =
                            matches!(error.downcast_ref(), Some(ssh::Failure::KeyRefused));
                        this.connect_error = Some(format!("{error:#}").into());
                        if this.route() != Route::Connect {
                            this.routes = vec![Route::Connect];
                        }
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
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
        if let Err(error) = offer.check_time() {
            self.connect_error = Some(error.to_string().into());
            cx.notify();
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
        let address = Address {
            user: offer.user.clone(),
            host: offer.hosts[0].clone(),
            port: offer.port,
        };
        self.address
            .update(cx, |field, cx| field.set_text(address.to_string(), cx));
        let generation = self.begin_connection_attempt();
        self.key_refused = false;
        self.connect_error = None;
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
        let fingerprint = connection.fingerprint.clone();
        let prefs = self.prefs(cx);
        let wanted = prefs
            .model_provider
            .as_ref()
            .zip(prefs.model_id.as_ref())
            .map(|(provider, id)| format!("{provider}/{id}"))
            .unwrap_or_else(|| prefs.model.clone());
        let mut live = Live::new(connection, helper, address.to_string(), wanted);
        live.thinking = prefs.thinking.clone();
        live.refresh_models();
        live.refresh_commands();
        let mut computer = Computer::from_address(&address.to_string());
        computer.pi_version = None;
        let mut store = Store::live(computer, live);
        store.apply(vec![(None, Update::Listed(Ok(listed)))]);
        self.project = prefs
            .projects
            .get(&address.to_string())
            .map(|path| store.add_project(path))
            .unwrap_or(0);
        self.project_browser.clear();
        self.store = Some(store);
        self.routes = vec![Route::Sessions, Route::Projects];
        self.threads.clear();
        self.jj_histories.clear();
        self.restoring_history = None;
        self.enabling_jj = None;
        self.start.update(cx, |start, _| {
            start.use_files(Some(Vec::new()));
            start.use_commands(Some(Vec::new()));
        });
        self.key_refused = false;
        self.last_listed = Instant::now();
        self.update_prefs(cx, |prefs| {
            prefs.computer = Some(address.to_string());
            prefs.sample = false;
            prefs.host_keys.insert(address.to_string(), fingerprint);
        });
        self.load_project_files(self.project, cx);
        self.start_pump(cx);
        activity::request_notification_permission();
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
        self.keep_latest_in_view();
        let global_commands = batch
            .iter()
            .any(|(id, update)| id.is_none() && matches!(update, Update::CommandCatalog(Ok(_))));
        let command_sessions: HashSet<SessionId> = batch
            .iter()
            .filter_map(|(id, update)| {
                (*id).filter(|_| {
                    matches!(update, Update::Record(_, record) if record["type"] == "response" && record["command"] == "get_commands" && record["success"] == true)
                })
            })
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
        let known = self.prefs(cx).host_keys.get(&address.to_string()).cloned();
        self.reconnecting = true;
        self.last_reconnect = Instant::now();
        cx.spawn(async move |this, cx| {
            let connection = Connection::open(address, identity, known).await;
            this.update(cx, |this, cx| {
                this.reconnecting = false;
                let Some(store) = &mut this.store else {
                    return;
                };
                match connection {
                    Ok(connection) => {
                        if let Some(live) = &mut store.live {
                            live.resume(connection);
                        }
                        store.computer.connected = true;
                        this.last_listed = Instant::now();
                    }
                    Err(error) => {
                        log::info!("Reconnecting failed: {error:#}");
                        store.computer.connected = false;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn model_session(&self) -> Option<SessionId> {
        match self.route() {
            Route::Thread(id) | Route::Review(id) | Route::History(id) => Some(id),
            _ => None,
        }
    }

    /// The current session's confirmed model, or defaults on New session.
    pub(crate) fn model_settings(&self, cx: &App) -> (String, String) {
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
        if let Some(id) = self.model_session() {
            let (_, thinking) = self.model_settings(cx);
            if let Some(live) = self.store.as_mut().and_then(|store| store.live.as_mut()) {
                if let Err(error) = live.set_model(id, &provider, &model_id) {
                    self.notify_user(error, cx);
                }
            } else {
                self.sample_models.insert(id, (name, thinking));
            }
            cx.notify();
            return;
        }
        if let Some(live) = self.store.as_mut().and_then(|store| store.live.as_mut()) {
            live.model = format!("{provider}/{model_id}");
        }
        self.update_prefs(cx, |prefs| {
            prefs.model = name;
            prefs.model_provider = Some(provider);
            prefs.model_id = Some(model_id);
        });
    }

    pub(crate) fn choose_thinking(&mut self, level: &str, cx: &mut Context<Self>) {
        if let Some(id) = self.model_session() {
            let (model, _) = self.model_settings(cx);
            if let Some(live) = self.store.as_mut().and_then(|store| store.live.as_mut()) {
                if let Err(error) = live.set_thinking(id, level) {
                    self.notify_user(error, cx);
                }
            } else {
                self.sample_models.insert(id, (model, level.to_owned()));
            }
            cx.notify();
            return;
        }
        if let Some(live) = self.store.as_mut().and_then(|store| store.live.as_mut()) {
            live.thinking = level.to_owned();
        }
        self.update_prefs(cx, |prefs| prefs.thinking = level.to_owned());
    }

    /// Leaves the computer: back to Connect, its host key forgotten.
    pub(crate) fn forget_computer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = self
            .store
            .as_ref()
            .map(|store| store.computer.address.clone());
        self.store = None;
        self.project_browser.clear();
        self._pump = None;
        self.threads.clear();
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
        self.routes = vec![Route::Connect];
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

    fn tick(&mut self, cx: &mut Context<Self>) {
        #[cfg(feature = "ui-test")]
        self.write_fixture_state(cx);
        let now = Instant::now();
        let elapsed = now - self.last_tick;
        self.last_tick = now;
        if !self.paused {
            self.keep_latest_in_view();
        }
        let Some(store) = self.store.as_mut().filter(|_| !self.paused) else {
            return;
        };
        if let Some(live) = &store.live {
            let closed = live.connection.is_closed();
            if store.computer.connected == closed {
                store.computer.connected = !closed;
                cx.notify();
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
            return;
        };
        if store.running().next().is_none() {
            return;
        }
        for event in store.tick(elapsed) {
            self.alert(event, cx);
        }
        self.update_working_notification(cx);
        cx.notify();
    }

    fn viewing(&self, id: SessionId) -> bool {
        self.visible
            && matches!(self.route(), Route::Thread(shown) | Route::Review(shown) | Route::History(shown) if shown == id)
    }

    fn alert(&mut self, event: Event, cx: &mut Context<Self>) {
        let prefs = self.prefs(cx).clone();
        let accent = theme(cx).accent_rgb();
        let Some(store) = &self.store else {
            return;
        };
        let computer = store.computer.name.clone();
        match event {
            Event::Deleted(id) => self.finish_deletion(id, cx),
            Event::NeedsYou(id) => {
                if self.viewing(id) {
                    // The question card takes the composer's place.
                    self.questions_later.remove(&id);
                    self.choice = None;
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
                if !self.viewing(id)
                    && prefs.notify_finished
                    && let Some(session) = store.session(id)
                {
                    activity::notify(&alerts::finished(session, &computer, accent));
                }
            }
        }
    }

    fn update_working_notification(&mut self, cx: &mut Context<Self>) {
        let wanted = self.prefs(cx).notify_working && !self.visible;
        let accent = theme(cx).accent_rgb();
        let notification = self
            .store
            .as_ref()
            .filter(|_| wanted)
            .and_then(|store| alerts::working(store.running(), &store.computer.name, accent));
        match notification {
            Some(notification) => {
                let text = format!("{}\n{}", notification.title, notification.text);
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
        self.visible = visibility.is_visible();
        if self.visible
            && let Route::Thread(id) | Route::Review(id) | Route::History(id) = self.route()
        {
            activity::cancel_notification(alerts::question_id(id));
            activity::cancel_notification(alerts::finished_id(id));
        }
        self.update_working_notification(cx);
    }

    /// A `pi://` link from a notification.
    pub fn open_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
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
                Route::Thread(id) | Route::Review(id) | Route::History(id) => {
                    self.store.as_ref().and_then(|s| s.session(id))
                }
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
                    self.routes = vec![Route::Connect];
                    cx.notify();
                }
            }
            return;
        }
        let Some(link) = Link::parse(url) else {
            log::warn!("Ignoring the link {url}");
            return;
        };
        let (Link::Session(id) | Link::Question(id) | Link::AllowOnce(id) | Link::Review(id)) =
            link;
        let Some(store) = &mut self.store else {
            return;
        };
        let Some(session) = store.session(id) else {
            self.notify_user("That session is no longer on this phone", cx);
            return;
        };
        let waiting = session.state == State::NeedsYou;
        match link {
            Link::AllowOnce(_) if waiting => {
                store.answer(id, Answer::AllowOnce);
                activity::cancel_notification(alerts::question_id(id));
                self.show_session(id, window, cx);
                self.close_sheet(cx);
                self.notify_user("Allowed once", cx);
            }
            Link::Review(_) => {
                self.show_session(id, window, cx);
                self.open_review(id, 0, window, cx);
            }
            _ => self.show_session(id, window, cx),
        }
    }

    // Acting on sessions

    pub(crate) fn answer(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choice.take() else {
            return;
        };
        if let Some(store) = &mut self.store {
            store.answer(id, choice);
        }
        activity::cancel_notification(alerts::question_id(id));
        self.close_sheet(cx);
        // "Tell Pi why in the next message."
        if choice == Answer::Deny {
            self.sheet_focus_pending = false;
            let composer = self.thread_composer(id, window, cx);
            let area = composer.read(cx).area.clone();
            let focus = area.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    pub(crate) fn stop(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let Some(store) = &mut self.store else { return };
        let live = store.live.is_some();
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
        self.scrolls.remove(&Route::Thread(id));
        self.scrolls.remove(&Route::Review(id));
        self.scrolls.remove(&Route::History(id));
        self.routes.retain(|route| !matches!(route, Route::Thread(session) | Route::Review(session) | Route::History(session) if *session == id));
        if self.sheet == Some(Sheet::Delete(id)) {
            self.close_sheet(cx);
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
                "Sample session removed. No computer files were changed."
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
        self.review_file = file;
        self.review_lines.clear();
        self.review
            .update(cx, |review, cx| review.set_lines(None, cx));
        self.push(Route::Review(id), window, cx);
    }

    pub(crate) fn open_history(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push(Route::History(id), window, cx);
        self.load_jj_history(id, cx);
    }

    pub(crate) fn load_jj_history(&mut self, id: SessionId, cx: &mut Context<Self>) {
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

    pub(crate) fn thread_composer(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Composer> {
        if let Some(composer) = self.threads.get(&id) {
            return composer.clone();
        }
        let composer = cx.new(|cx| Composer::new("Ask a follow-up…", cx));
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
            move |this, _, event, window, cx| {
                this.composer_event(Target::Thread(id), event, window, cx)
            },
        ));
        self.threads.insert(id, composer.clone());
        composer
    }

    pub(crate) fn composer(&self, target: Target) -> Option<Entity<Composer>> {
        match target {
            Target::Start => Some(self.start.clone()),
            Target::Review => Some(self.review.clone()),
            Target::Thread(id) => self.threads.get(&id).cloned(),
        }
    }

    fn composer_event(
        &mut self,
        target: Target,
        event: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ComposerEvent::Attach => self.open_sheet(Sheet::Attach(target), cx),
            ComposerEvent::ChooseModel => self.open_sheet(Sheet::Model, cx),
            ComposerEvent::ChooseThinking => self.open_sheet(Sheet::Thinking, cx),
            ComposerEvent::PreviewImage(index) => self.open_sheet(Sheet::Image(target, *index), cx),
            ComposerEvent::Send { text, attachments } => {
                let route = self.route();
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
                let prompt = match prompt_for(text, attachments, file.as_deref()) {
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
                            self.scroll(Route::Thread(id)).scroll_to_bottom();
                        }
                        window.dismiss_virtual_keyboard();
                        if running {
                            self.notify_user("Queued for when this run ends", cx);
                        }
                    }
                    Target::Review => {
                        let Route::Review(id) = route else {
                            return;
                        };
                        if let Err(error) = store.send(id, prompt, labels) {
                            self.notify_user(error, cx);
                            return;
                        }
                        self.review.update(cx, |composer, cx| composer.clear(cx));
                        self.review_lines.clear();
                        self.routes.pop();
                        self.entered(window, cx);
                        self.notify_user("Sent as a follow-up", cx);
                    }
                }
                cx.notify();
            }
        }
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
fn prompt_for(
    text: &str,
    attachments: &[Attachment],
    file: Option<&str>,
) -> Result<crate::prompt::Prompt, String> {
    use base64::Engine;
    let mut message = text.to_owned();
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
                "\n\nAttached text file: {name}\n{contents}\nEnd of attached file: {name}"
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

pub(crate) fn size_label(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.0} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

impl Render for PhoneApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "ui-test")]
        if cx.has_global::<crate::testing::State>() {
            let state = cx.global_mut::<crate::testing::State>();
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
        let screen = match self.route() {
            Route::Connect => self.connect_screen(window, cx).into_any_element(),
            Route::Sessions => self.sessions_screen(window, cx).into_any_element(),
            Route::Projects => self.projects_screen(window, cx).into_any_element(),
            Route::Start => self.start_screen(window, cx).into_any_element(),
            Route::File => self.file_screen(window, cx).into_any_element(),
            Route::Thread(id) => self.thread_screen(id, window, cx),
            Route::Review(id) => self.review_screen(id, window, cx),
            Route::History(id) => self.history_screen(id, window, cx),
            Route::Settings => self.settings_screen(window, cx).into_any_element(),
        };
        let sheet = self.sheet.or(self.closing_sheet).map(|sheet| {
            let content = self.sheet_content(sheet, window, cx);
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
                        .opacity(1. - progress)
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
                        .max_h(window.viewport_size().height - insets.top - px(24.))
                        // The tree rolls out without the sheet growing under it.
                        .when(sheet == Sheet::Project, |panel| {
                            panel.h(window.viewport_size().height - insets.top - px(24.))
                        })
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
        let notice = self.notice.clone().map(|notice| {
            div()
                .absolute()
                .left(insets.left + px(16.))
                .right(insets.right + px(16.))
                .bottom(insets.bottom + px(84.))
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(16.))
                        .py(px(12.))
                        .rounded(px(12.))
                        .bg(colors.text)
                        .text_color(colors.canvas)
                        .text_size(px(14.))
                        .child(notice),
                )
        });
        div()
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
    }
}

impl PhoneApp {
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
