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
    prefs::Prefs,
    projects::ProjectBrowser,
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
    Thread(SessionId),
    Review(SessionId),
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
    Question(SessionId),
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
    Image(Target, usize),
}

pub struct PhoneApp {
    pub(crate) store: Option<Store>,
    pub(crate) routes: Vec<Route>,
    pub(crate) sheet: Option<Sheet>,
    pub(crate) drawer_open: bool,
    pub(crate) swiping_session: Option<(SessionId, SwipeMotion)>,
    pub(crate) deleting_session: Option<SessionId>,
    pub(crate) drawer_motion: SwipeMotion,
    pub(crate) sheet_motion: SwipeMotion,
    pub(crate) closing_sheet: Option<Sheet>,
    sheet_height: Rc<Cell<Pixels>>,
    pub(crate) sheet_scroll: ScrollHandle,
    pub(crate) drawer_scroll: ScrollHandle,
    pub(crate) expanded_turns: HashMap<(SessionId, usize), bool>,
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
    pub(crate) connect_error: Option<SharedString>,
    pub(crate) connecting: bool,
    /// Enrollment progress and the code that must match the computer.
    pub(crate) pairing_status: Option<SharedString>,
    /// The phone's public key, once made: one line for authorized_keys.
    pub(crate) phone_key: Option<String>,
    /// The computer turned down the phone's key on the last try.
    pub(crate) key_refused: bool,
    /// Where the key and settings live.
    pub(crate) data_dir: PathBuf,
    /// A project folder typed into the project sheet.
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
                "/Users/you/repos/app",
                TextInputConfiguration {
                    autocorrect: false,
                    autocapitalize: gpui::Autocapitalize::None,
                    suggestions: false,
                    input_action: TextInputAction::Done,
                },
                cx,
            )
        });
        let start = cx.new(|cx| Composer::new("Describe the change…", cx));
        let review = cx.new(|cx| Composer::new("Ask for a revision…", cx));
        let subscriptions = vec![
            cx.subscribe_in(&address, window, |this, _, event, window, cx| match event {
                TextAreaEvent::Submit => this.connect(window, cx),
                TextAreaEvent::Changed => {
                    this.connect_error = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&search, |_, _, _: &TextAreaEvent, cx| cx.notify()),
            cx.subscribe(&model_search, |this, _, event: &TextAreaEvent, cx| {
                if *event == TextAreaEvent::Changed {
                    this.sheet_scroll.set_offset(gpui::Point::default());
                }
                cx.notify();
            }),
            cx.subscribe_in(&folder, window, |this, _, event, window, cx| {
                if *event == TextAreaEvent::Submit {
                    this.add_folder(window, cx);
                }
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
            drawer_open: false,
            swiping_session: None,
            deleting_session: None,
            drawer_motion: SwipeMotion::at(1.),
            sheet_motion: SwipeMotion::at(1.),
            closing_sheet: None,
            sheet_height: Rc::new(Cell::new(px(0.))),
            sheet_scroll: ScrollHandle::new(),
            drawer_scroll: ScrollHandle::new(),
            expanded_turns: HashMap::new(),
            sheet_focus_pending: false,
            choice: None,
            expanded: HashSet::new(),
            notice: None,
            notice_generation: 0,
            focus: cx.focus_handle(),
            address,
            connect_error: None,
            connecting: false,
            pairing_status: None,
            phone_key,
            key_refused: false,
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
        if let Some(path) = &self.prefs_path {
            cx.global::<Prefs>().save(path);
        }
        cx.notify();
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
        cx.set_global(Theme::new(dark));
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
        self.close_drawer(cx);
        self.close_sheet(cx);
        self.searching = false;
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        if let Route::Thread(id) | Route::Review(id) = self.route() {
            activity::cancel_notification(alerts::question_id(id));
            activity::cancel_notification(alerts::finished_id(id));
            // A question waiting in the session opens with it.
            if self
                .store
                .as_ref()
                .and_then(|store| store.session(id))
                .is_some_and(|s| s.state == State::NeedsYou)
                && self.route() == Route::Thread(id)
            {
                self.open_sheet(Sheet::Question(id), cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn open_sheet(&mut self, sheet: Sheet, cx: &mut Context<Self>) {
        self.drawer_open = false;
        self.drawer_motion = SwipeMotion::at(1.);
        self.closing_sheet = None;
        self.sheet_motion.settle(0.);
        self.sheet_scroll = ScrollHandle::new();
        if sheet == Sheet::Model {
            self.model_search
                .update(cx, |area, cx| area.set_text("", cx));
        }
        if let Sheet::Question(_) = sheet {
            self.choice = None;
        }
        self.sheet = Some(sheet);
        if sheet == Sheet::Project
            && self.project_browser.directory.is_none()
            && !self.project_browser.loading
        {
            self.browse_projects(None, cx);
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

    pub(crate) fn close_drawer(&mut self, cx: &mut Context<Self>) {
        if self.drawer_open {
            self.drawer_open = false;
            self.drawer_motion.settle(1.);
            cx.notify();
        }
    }

    fn animate_panels(&mut self, window: &Window, cx: &Context<Self>) {
        let now = Instant::now();
        let reduced = cx.reduce_motion();
        self.drawer_motion.tick(now, reduced);
        self.sheet_motion.tick(now, reduced);
        if self.sheet.is_none() && !self.sheet_motion.animating() {
            self.closing_sheet = None;
        }
        if let Some((_, motion)) = &mut self.swiping_session {
            motion.tick(now, reduced);
            if !motion.dragging() && !motion.animating() {
                self.swiping_session = None;
            }
        }
        if self.drawer_motion.animating()
            || self.sheet_motion.animating()
            || self
                .swiping_session
                .as_ref()
                .is_some_and(|(_, motion)| motion.animating())
        {
            window.request_animation_frame();
        }
    }

    /// Capture dismissal for the gesture's lifetime, not the moving panel's
    /// hitbox. Moving a panel away from the starting finger must not lose the
    /// remaining events. Sheet content gets normal scrolling until its top.
    pub(crate) fn dismiss_gesture(&self, drawer: bool, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let sheet_height = self.sheet_height.clone();
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if !drawer {
                    sheet_height.set(bounds.size.height);
                }
                window.on_mouse_event(move |event: &gpui::ScrollWheelEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Capture {
                        return;
                    }
                    let _ = view.update(cx, |this, cx| {
                        if (drawer && !this.drawer_open) || (!drawer && this.sheet.is_none()) {
                            return;
                        }
                        let delta = event.delta.pixel_delta(px(20.));
                        let (along, across) = if drawer {
                            (-delta.x, delta.y)
                        } else {
                            (delta.y, delta.x)
                        };
                        let motion = if drawer {
                            &mut this.drawer_motion
                        } else {
                            &mut this.sheet_motion
                        };
                        let extent = if drawer {
                            bounds.size.width
                        } else {
                            bounds.size.height
                        }
                        .max(px(1.));
                        if event.touch_phase == gpui::TouchPhase::Started {
                            if !bounds.contains(&event.position)
                                || along <= px(0.)
                                || along.abs() < across.abs()
                                || (!drawer
                                    && this.sheet_scroll.max_offset().y > px(1.)
                                    && this.sheet_scroll.bounds().contains(&event.position)
                                    && event.position.x
                                        >= this.sheet_scroll.bounds().right() - px(18.))
                                || (!drawer
                                    && event.position.y > bounds.top() + px(36.)
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
                        motion.drag_by(along / extent);
                        if event.touch_phase == gpui::TouchPhase::Cancelled {
                            motion.settle(0.);
                        } else if event.touch_phase == gpui::TouchPhase::Ended {
                            let close = extent * motion.position() > px(85.).min(extent * 0.4);
                            if !close {
                                motion.settle(0.);
                            } else if drawer {
                                this.close_drawer(cx);
                            } else {
                                this.close_sheet(cx);
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
        if self.drawer_open {
            self.close_drawer(cx);
            return true;
        }
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
        if self.routes.len() > 1 {
            self.routes.pop();
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
        self.project = 0;
        self.store = Some(Store::sample(Computer::from_address(address)));
        self.routes = vec![Route::Sessions];
        self.threads.clear();
        self.sample_models.clear();
        self.swiping_session = None;
        self.deleting_session = None;
        self.scrolls.clear();
        self._pump = None;
    }

    pub(crate) fn open_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.paused = false;
        let typed = self.address.read(cx).text().trim().to_owned();
        let address = if typed.is_empty() {
            "you@studio-mac.local".to_owned()
        } else {
            typed
        };
        self.open_store(&address);
        self.start.update(cx, |start, _| start.use_files(None));
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
        self.connecting = true;
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
        if self.connecting {
            return;
        }
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
        self.connecting = true;
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
        self.start
            .update(cx, |start, _| start.use_files(Some(Vec::new())));
        self.key_refused = false;
        self.last_listed = Instant::now();
        self.update_prefs(cx, |prefs| {
            prefs.computer = Some(address.to_string());
            prefs.sample = false;
            prefs.host_keys.insert(address.to_string(), fingerprint);
        });
        self.start_pump(cx);
        activity::request_notification_permission();
        self.entered(window, cx);
        self.browse_projects(None, cx);
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
        let Some(store) = &mut self.store else {
            return;
        };
        for event in store.apply(batch) {
            self.alert(event, cx);
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
            Route::Thread(id) | Route::Review(id) => Some(id),
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

    /// A manually entered path is opened and checked by the remote browser.
    pub(crate) fn add_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut path = self.folder.read(cx).text().trim().to_owned();
        if let Some(home) = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .map(|live| &live.helper.home)
        {
            if path == "~" {
                path = home.clone();
            } else if let Some(rest) = path.strip_prefix("~/") {
                path = format!("{home}/{rest}");
            }
        }
        if !path.starts_with('/') {
            self.notify_user("Type the folder's full path, starting with /", cx);
            return;
        }
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        self.browse_projects(Some(path), cx);
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
            && matches!(self.route(), Route::Thread(shown) | Route::Review(shown) if shown == id)
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
                    if self.sheet.is_none() {
                        self.open_sheet(Sheet::Question(id), cx);
                    }
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
            && let Route::Thread(id) | Route::Review(id) = self.route()
        {
            activity::cancel_notification(alerts::question_id(id));
            activity::cancel_notification(alerts::finished_id(id));
        }
        self.update_working_notification(cx);
    }

    /// A `pi://` link from a notification.
    pub fn open_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "ui-test")]
        if url == "pi://test-state" && self.paused {
            // Only fixture mode exposes diagnostics, and only lengths/state:
            // never prompt contents, clipboard data, addresses or credentials.
            let session = match self.route() {
                Route::Thread(id) | Route::Review(id) => {
                    self.store.as_ref().and_then(|s| s.session(id))
                }
                _ => None,
            };
            log::info!(
                "ui-test-state {}",
                serde_json::json!({
                    "route": format!("{:?}", self.route()),
                    "routes": self.routes.iter().map(|r| format!("{r:?}")).collect::<Vec<_>>(),
                    "drawer": self.drawer_open,
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
        self.expanded_turns.retain(|(session, _), _| *session != id);
        self.scrolls.remove(&Route::Thread(id));
        self.scrolls.remove(&Route::Review(id));
        self.routes.retain(|route| !matches!(route, Route::Thread(session) | Route::Review(session) if *session == id));
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
        composer.update(cx, |composer, _| composer.use_files(files));
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
            ComposerEvent::Stop => {
                let id = match target {
                    Target::Thread(id) => Some(id),
                    Target::Review => self.model_session(),
                    Target::Start => None,
                };
                if let Some(id) = id {
                    self.stop(id, cx);
                }
            }
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
            Route::Thread(id) => self.thread_screen(id, window, cx),
            Route::Review(id) => self.review_screen(id, window, cx),
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
                                .mb(px(14.))
                                .w(px(36.))
                                .h(px(4.))
                                .rounded_full()
                                .bg(colors.line_strong),
                        )
                        .child(content)
                        .child(self.dismiss_gesture(false, cx)),
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
        let drawer =
            (self.drawer_open || self.drawer_motion.animating()).then(|| self.drawer(window, cx));
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
            .children(drawer)
            .children(notice)
    }
}
