//! The phone app: a stack of screens, at most one bottom sheet over them, a
//! short notice at the bottom, and a computer's sessions underneath, or the
//! sample ones.
//!
//! Back closes the sheet, then the search, then the screen; on the first
//! screen it leaves the app, as Android expects.

use crate::{
    alerts::{self, Link},
    composer::{Attachment, Composer, ComposerEvent, Layout},
    live::{Live, Update},
    model::{Answer, Computer, SessionId, State},
    prefs::Prefs,
    remote,
    ssh::{self, Address, Connection, Identity},
    store::{Event, Store},
    text_area::{TextArea, TextAreaEvent},
    theme::{Appearance, SANS, Theme, theme},
};
use gpui::{
    Animation, AnimationExt, App, Context, Edges, Entity, FocusHandle, Focusable, Image,
    ImageFormat, PathPromptOptions, Pixels, ScrollHandle, SharedString, Subscription, Task,
    TextInputAction, TextInputConfiguration, Window, WindowAppearance, WindowVisibility, actions,
    div, ease_out_quint, prelude::*, px, relative,
};
use gpui_android::activity;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

actions!(pi_android, [GoBack]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Connect,
    Sessions,
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
    Project,
    More(SessionId),
    Models,
    Resources,
}

pub struct PhoneApp {
    pub(crate) store: Option<Store>,
    pub(crate) routes: Vec<Route>,
    pub(crate) sheet: Option<Sheet>,
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
    /// The phone's public key, once made: one line for authorized_keys.
    pub(crate) phone_key: Option<String>,
    /// The computer turned down the phone's key on the last try.
    pub(crate) key_refused: bool,
    /// Where the key and settings live.
    data_dir: PathBuf,
    /// A project folder typed into the project sheet.
    pub(crate) folder: Entity<TextArea>,
    reconnecting: bool,
    last_listed: Instant,
    last_reconnect: Instant,
    /// Carries what the computer sends into the store.
    _pump: Option<Task<()>>,
    pub(crate) search: Entity<TextArea>,
    pub(crate) searching: bool,
    pub(crate) start: Entity<Composer>,
    pub(crate) project: usize,
    pub(crate) threads: HashMap<SessionId, Entity<Composer>>,
    pub(crate) review: Entity<Composer>,
    pub(crate) review_file: usize,
    /// Tapped review lines, as (hunk, line) of the shown file.
    pub(crate) review_lines: BTreeSet<(usize, usize)>,
    scrolls: HashMap<Route, ScrollHandle>,
    prefs_path: Option<PathBuf>,
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
        let start = cx.new(|cx| Composer::new("Describe the change…", Layout::Full, cx));
        let review = cx.new(|cx| Composer::new("Ask for a revision…", Layout::Compact, cx));
        let subscriptions = vec![
            cx.subscribe_in(&address, window, |this, _, event, window, cx| match event {
                TextAreaEvent::Submit => this.connect(window, cx),
                TextAreaEvent::Changed => {
                    this.connect_error = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&search, |_, _, _: &TextAreaEvent, cx| cx.notify()),
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
            choice: None,
            expanded: HashSet::new(),
            notice: None,
            notice_generation: 0,
            focus: cx.focus_handle(),
            address,
            connect_error: None,
            connecting: false,
            phone_key,
            key_refused: false,
            data_dir,
            folder,
            reconnecting: false,
            last_listed: Instant::now(),
            last_reconnect: Instant::now(),
            _pump: None,
            search,
            searching: false,
            start,
            project: 0,
            threads: HashMap::new(),
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
        self.routes = vec![Route::Sessions, Route::Thread(id)];
        self.entered(window, cx);
    }

    fn entered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = None;
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
        if let Sheet::Question(_) = sheet {
            self.choice = None;
        }
        self.sheet = Some(sheet);
        cx.notify();
    }

    pub(crate) fn close_sheet(&mut self, cx: &mut Context<Self>) {
        self.sheet = None;
        cx.notify();
    }

    /// One step back; false when there is nothing to go back to.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.sheet.take().is_some() {
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
        self.store = Some(Store::sample(Computer::from_address(address)));
        self.routes = vec![Route::Sessions];
        self.threads.clear();
        self._pump = None;
    }

    pub(crate) fn open_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn connected(
        &mut self,
        address: Address,
        connection: Connection,
        helper: remote::Helper,
        listed: Vec<remote::Listed>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fingerprint = connection.fingerprint.clone();
        let prefs = self.prefs(cx);
        let mut live = Live::new(connection, helper, address.to_string(), prefs.model.clone());
        live.thinking = prefs.thinking.clone();
        let mut computer = Computer::from_address(&address.to_string());
        computer.pi_version = None;
        let mut store = Store::live(computer, live);
        store.apply(vec![(None, Update::Listed(Ok(listed)))]);
        self.store = Some(store);
        self.routes = vec![Route::Sessions];
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

    /// The model new sessions start with.
    pub(crate) fn choose_model(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some(live) = self.store.as_mut().and_then(|store| store.live.as_mut()) {
            live.model = name.clone();
        }
        self.update_prefs(cx, |prefs| prefs.model = name);
    }

    /// Leaves the computer: back to Connect, its host key forgotten.
    pub(crate) fn forget_computer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = self
            .store
            .as_ref()
            .map(|store| store.computer.address.clone());
        self.store = None;
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

    /// The folder typed into the project sheet becomes the project.
    pub(crate) fn add_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self
            .folder
            .read(cx)
            .text()
            .trim()
            .trim_end_matches('/')
            .to_owned();
        if !path.starts_with('/') {
            self.notify_user("Type the folder's full path, starting with /", cx);
            return;
        }
        if let Some(store) = &mut self.store {
            self.project = store.add_project(&path);
        }
        self.folder.update(cx, |field, cx| {
            field.take(cx);
        });
        window.dismiss_virtual_keyboard();
        self.close_sheet(cx);
    }

    // Sample sessions and notifications

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let elapsed = now - self.last_tick;
        self.last_tick = now;
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
            let composer = self.thread_composer(id, window, cx);
            let area = composer.read(cx).area.clone();
            let focus = area.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    pub(crate) fn stop(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if let Some(store) = &mut self.store {
            store.stop(id);
        }
        activity::cancel_notification(alerts::question_id(id));
        self.close_sheet(cx);
        self.notify_user("Stopped", cx);
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
        let composer = cx.new(|cx| Composer::new("Ask a follow-up…", Layout::Compact, cx));
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
            ComposerEvent::Send { text, attachments } => {
                let route = self.route();
                let Some(store) = &mut self.store else {
                    return;
                };
                match target {
                    Target::Start => {
                        let (text, dropped) = live_text(store, text, attachments, None);
                        match store.start(self.project, text, attachments.clone()) {
                            Ok(id) => self.show_session(id, window, cx),
                            Err(error) => self.notify_user(error, cx),
                        }
                        if dropped {
                            self.notify_user(
                                "Only the text went: files from the phone can't be sent yet",
                                cx,
                            );
                        }
                    }
                    Target::Thread(id) => {
                        let running = store.session(id).is_some_and(|s| s.state.is_running());
                        let (text, dropped) = live_text(store, text, attachments, None);
                        store.send(id, text, attachments.clone());
                        window.dismiss_virtual_keyboard();
                        if dropped {
                            self.notify_user(
                                "Only the text went: files from the phone can't be sent yet",
                                cx,
                            );
                        } else if running {
                            self.notify_user("Queued for when this run ends", cx);
                        }
                    }
                    Target::Review => {
                        let Route::Review(id) = route else {
                            return;
                        };
                        let file = store
                            .session(id)
                            .and_then(|session| session.files.get(self.review_file))
                            .map(|file| file.path.clone());
                        let (text, _) = live_text(store, text, attachments, file.as_deref());
                        store.send(id, text, attachments.clone());
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

    /// Adds files from Android's picker to a composer.
    pub(crate) fn attach_files(&mut self, target: Target, cx: &mut Context<Self>) {
        self.close_sheet(cx);
        let Some(composer) = self.composer(target) else {
            return;
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let picked = picked.await;
            this.update(cx, |this, cx| match picked {
                Ok(Ok(Some(paths))) => composer.update(cx, |composer, cx| {
                    for path in paths {
                        composer.attach(attachment_for(&path), cx);
                    }
                }),
                Ok(Ok(None)) | Err(_) => {}
                Ok(Err(error)) => this.notify_user(format!("{error:#}"), cx),
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
            (Some(image), _) => composer.update(cx, |composer, cx| {
                let name = format!("Pasted image.{}", image.format.extension());
                composer.attach(
                    Attachment::Image {
                        name,
                        image: Arc::new(image),
                    },
                    cx,
                );
            }),
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

/// What a live session is sent for a draft: the text, with reviewed lines
/// named in it. Files and images from the phone don't go yet; the flag says
/// some were left out.
fn live_text(
    store: &Store,
    text: &str,
    attachments: &[String],
    file: Option<&str>,
) -> (String, bool) {
    if store.is_sample() {
        return (text.to_owned(), false);
    }
    let mut message = text.to_owned();
    let mut dropped = false;
    for attachment in attachments {
        match (attachment.starts_with("line"), file) {
            (true, Some(file)) => message = format!("About {attachment} of {file}: {message}"),
            _ => dropped = true,
        }
    }
    (message, dropped)
}

/// A picked file as an attachment: images show as thumbnails.
fn attachment_for(path: &Path) -> Attachment {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let bytes = std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(|extension| {
            ImageFormat::from_mime_type(&format!(
                "image/{}",
                extension.to_lowercase().replace("jpg", "jpeg")
            ))
        });
    if let Some(format) = format
        && bytes < 20_000_000
        && let Ok(data) = std::fs::read(path)
    {
        return Attachment::Image {
            name,
            image: Arc::new(Image::from_bytes(format, data)),
        };
    }
    Attachment::File {
        name,
        size: size_label(bytes),
    }
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
        let colors = theme(cx);
        let insets = Self::insets(window);
        let screen = match self.route() {
            Route::Connect => self.connect_screen(window, cx).into_any_element(),
            Route::Sessions => self.sessions_screen(window, cx).into_any_element(),
            Route::Start => self.start_screen(window, cx).into_any_element(),
            Route::Thread(id) => self.thread_screen(id, window, cx),
            Route::Review(id) => self.review_screen(id, window, cx),
            Route::Settings => self.settings_screen(window, cx).into_any_element(),
        };
        let sheet = self.sheet.map(|sheet| {
            let content = self.sheet_content(sheet, window, cx);
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .justify_end()
                .child(
                    div()
                        .id("scrim")
                        .absolute()
                        .inset_0()
                        .bg(colors.scrim)
                        .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                )
                .child(
                    div()
                        .id("sheet")
                        .relative()
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
                        .with_animation(
                            SharedString::from(format!("sheet-{sheet:?}")),
                            Animation::new(Duration::from_millis(220))
                                .with_easing(ease_out_quint()),
                            |sheet, delta| {
                                sheet.top(px(48. * (1. - delta))).opacity(0.4 + 0.6 * delta)
                            },
                        ),
                )
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
