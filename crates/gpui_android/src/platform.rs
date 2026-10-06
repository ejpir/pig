//! The platform: `android_main`'s thread is GPUI's main thread, blocking in
//! Android's looper between activity events, input, queued tasks and vsync ticks.

use crate::{
    dispatcher::{AndroidDispatcher, MainQueue},
    display::AndroidDisplay,
    fonts,
    frame_clock::FrameClock,
    java::{self, Java, JavaEvent},
    keys,
    lifecycle::{ActivityEvent, Lifecycle},
    touch::{Action, Contact},
    window::{AndroidWindow, WindowState},
};
use android_activity::{
    AndroidApp, InputStatus, MainEvent, PollEvent,
    input::{InputEvent, KeyAction, KeyEvent, KeyMapChar, Keycode, MotionAction, MotionEvent},
};
use anyhow::{Result, anyhow};
use futures::channel::oneshot;
use gpui::{
    Action as GpuiAction, ActivityGuard, AnyWindowHandle, AppLifecyclePhase, BackgroundExecutor,
    ClipboardEntry, ClipboardItem, ClipboardString, CursorStyle, DummyKeyboardMapper,
    ForegroundExecutor, GestureTuning, Image, ImageFormat, Keymap, Keystroke, Menu, MenuItem,
    Modifiers, PathPromptOptions, Platform, PlatformDisplay, PlatformGestures,
    PlatformKeyboardLayout, PlatformKeyboardMapper, PlatformTextSystem, PlatformWindow,
    ScrollPhysics, Task, ThermalState, WindowAppearance, WindowKind, WindowParams, point,
    popup::PopupNotSupportedError, px,
};
use gpui_wgpu::{CosmicTextSystem, GpuContext};
use ndk::configuration::UiModeNight;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::Arc,
    time::{Duration, Instant},
};

/// How long queued main-thread tasks may run before input and frames get a turn.
const TASK_BUDGET: Duration = Duration::from_millis(8);

/// Android's `ViewConfiguration` defaults, so touches feel native.
struct AndroidGestures;

impl PlatformGestures for AndroidGestures {
    fn tuning(&self) -> GestureTuning {
        GestureTuning {
            touch_slop: px(8.),
            multi_tap_interval: Duration::from_millis(300),
            multi_tap_slop: px(100.),
            long_press_duration: Duration::from_millis(400),
            scroll_physics: ScrollPhysics::android(),
            min_fling_velocity: 50.,
        }
    }
}

struct AndroidKeyboardLayout;

impl PlatformKeyboardLayout for AndroidKeyboardLayout {
    fn id(&self) -> &str {
        "android"
    }

    fn name(&self) -> &str {
        "Android"
    }
}

#[derive(Default)]
struct Callbacks {
    quit: Option<Box<dyn FnMut() -> bool>>,
    lifecycle: Option<Box<dyn FnMut(AppLifecyclePhase)>>,
    memory_warning: Option<Box<dyn FnMut()>>,
    open_urls: Option<Box<dyn FnMut(Vec<String>)>>,
}

/// A file pick waiting for the system picker.
enum Pick {
    Open(oneshot::Sender<Result<Option<Vec<PathBuf>>>>),
    Save(oneshot::Sender<Result<Option<PathBuf>>>),
}

pub struct AndroidPlatform {
    app: AndroidApp,
    java: Rc<Java>,
    main_queue: RefCell<MainQueue>,
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    text_system: Arc<CosmicTextSystem>,
    display: Rc<AndroidDisplay>,
    gpu: GpuContext,
    clock: Rc<FrameClock>,
    window: RefCell<Weak<WindowState>>,
    lifecycle: RefCell<Lifecycle>,
    callbacks: RefCell<Callbacks>,
    clipboard: RefCell<Option<ClipboardItem>>,
    picks: RefCell<HashMap<i32, Pick>>,
    next_pick: Cell<i32>,
    /// URLs the app was opened with before it registered for them.
    pending_urls: RefCell<Vec<String>>,
    /// Whether the app took the back key's last press; its release goes the same way.
    back_taken: Cell<bool>,
    /// The system bars and keyboard as Java last reported them, kept for a window
    /// opened after the report: Java reports once it attaches, often before then.
    insets: Cell<Option<([i32; 4], [i32; 4])>>,
    quitting: Cell<bool>,
}

impl AndroidPlatform {
    /// Call from `android_main`, on the thread that will run the app.
    pub fn new(app: AndroidApp) -> Result<Self> {
        let waker = app.create_waker();
        let (dispatcher, main_queue) = AndroidDispatcher::new(move || waker.wake());
        let dispatcher = Arc::new(dispatcher);
        let text_system = Arc::new(CosmicTextSystem::new_without_system_fonts("Roboto"));
        let started = Instant::now();
        let fonts = fonts::system_fonts();
        let count = fonts.len();
        text_system.add_fonts(fonts)?;
        log::info!(
            "Loaded {count} system font files in {:?}",
            started.elapsed()
        );
        let java = Rc::new(Java::attach(&app));
        java::set_current(&java);
        Ok(Self {
            clock: Rc::new(FrameClock::new(app.create_waker())?),
            java,
            app,
            background_executor: BackgroundExecutor::new(dispatcher.clone()),
            foreground_executor: ForegroundExecutor::new(dispatcher),
            main_queue: RefCell::new(main_queue),
            text_system,
            display: Rc::default(),
            gpu: GpuContext::default(),
            window: RefCell::default(),
            lifecycle: RefCell::default(),
            callbacks: RefCell::default(),
            clipboard: RefCell::default(),
            picks: RefCell::default(),
            next_pick: Cell::new(0),
            pending_urls: RefCell::default(),
            back_taken: Cell::new(false),
            insets: Cell::new(None),
            quitting: Cell::new(false),
        })
    }

    fn window(&self) -> Option<Rc<WindowState>> {
        self.window.borrow().upgrade()
    }

    /// Waits in the looper and handles what woke it. Returns false once the
    /// activity is being destroyed.
    fn turn(&self, timeout: Option<Duration>) -> bool {
        let mut input = false;
        let mut destroyed = false;
        self.app.poll_events(timeout, |event| {
            if let PollEvent::Main(event) = event {
                match event {
                    MainEvent::InputAvailable => input = true,
                    MainEvent::Destroy => destroyed = true,
                    event => self.handle_main_event(event),
                }
            }
        });
        if input {
            self.handle_input();
        }
        for event in java::take_events() {
            self.java_event(event);
        }
        if self.clock.take_tick()
            && let Some(window) = self.window()
        {
            window.frame(false);
        }
        if let Some(window) = self.window() {
            window.settle();
        }
        !destroyed
    }

    fn java_event(&self, event: JavaEvent) {
        let event = match event {
            JavaEvent::Picked { request, result } => return self.picked(request, result),
            JavaEvent::OpenUrl(url) => return self.open_urls(vec![url]),
            JavaEvent::Insets { bars, ime } => {
                self.insets.set(Some((bars, ime)));
                event
            }
            event => event,
        };
        let Some(window) = self.window() else {
            return;
        };
        match event {
            JavaEvent::Ime {
                seq,
                push_id,
                state,
            } => window.apply_ime(seq, push_id, state),
            // Done, send, go and the rest arrive as enter, like the return key.
            JavaEvent::EditorAction(action) => {
                log::debug!("keyboard action {action}");
                window.press("enter");
            }
            JavaEvent::Insets { bars, ime } => window.set_reported_insets(bars, ime),
            JavaEvent::Picked { .. } | JavaEvent::OpenUrl(_) => {}
        }
    }

    fn open_urls(&self, urls: Vec<String>) {
        let callback = self.callbacks.borrow_mut().open_urls.take();
        let Some(mut callback) = callback else {
            self.pending_urls.borrow_mut().extend(urls);
            return;
        };
        callback(urls);
        self.callbacks
            .borrow_mut()
            .open_urls
            .get_or_insert(callback);
    }

    /// A request number for Android's picker, which allows 16 bits.
    fn pick_request(&self, pick: Pick) -> i32 {
        let request = self.next_pick.get() % 0xFFFF + 1;
        self.next_pick.set(request);
        self.picks.borrow_mut().insert(request, pick);
        request
    }

    fn picked(&self, request: i32, result: Result<Option<Vec<String>>, String>) {
        let Some(pick) = self.picks.borrow_mut().remove(&request) else {
            return;
        };
        let result = result.map_err(|error| anyhow!("Could not use the picked files: {error}"));
        match pick {
            Pick::Open(sender) => {
                let paths = result
                    .map(|paths| paths.map(|paths| paths.into_iter().map(PathBuf::from).collect()));
                sender.send(paths).ok();
            }
            Pick::Save(sender) => {
                let path = result.map(|paths| {
                    paths
                        .and_then(|paths| paths.into_iter().next())
                        .map(PathBuf::from)
                });
                sender.send(path).ok();
            }
        }
    }

    /// Activity events. Window teardown must finish before this returns:
    /// Android reclaims the native window right after.
    fn handle_main_event(&self, event: MainEvent<'_>) {
        let window = self.window();
        match event {
            MainEvent::Start => self.lifecycle_event(ActivityEvent::Start),
            MainEvent::Resume { .. } => self.lifecycle_event(ActivityEvent::Resume),
            MainEvent::Pause => self.lifecycle_event(ActivityEvent::Pause),
            MainEvent::Stop => self.lifecycle_event(ActivityEvent::Stop),
            MainEvent::GainedFocus => self.lifecycle_event(ActivityEvent::GainedFocus),
            MainEvent::LostFocus => self.lifecycle_event(ActivityEvent::LostFocus),
            MainEvent::InitWindow { .. } => {
                if let (Some(window), Some(native)) = (&window, self.app.native_window()) {
                    window.attach(native);
                }
                self.lifecycle_event(ActivityEvent::WindowCreated);
            }
            MainEvent::TerminateWindow { .. } => {
                if let Some(window) = &window {
                    window.detach();
                }
                self.lifecycle_event(ActivityEvent::WindowDestroyed);
            }
            MainEvent::WindowResized { .. } | MainEvent::ConfigChanged { .. } => {
                if let Some(window) = &window {
                    window.measure();
                    window.set_appearance(self.window_appearance());
                }
            }
            MainEvent::RedrawNeeded { .. } => {
                if let Some(window) = &window {
                    window.frame(true);
                }
            }
            MainEvent::ContentRectChanged { .. } | MainEvent::InsetsChanged { .. } => {
                if let Some(window) = &window {
                    window.refresh_insets();
                }
            }
            MainEvent::LowMemory => {
                let callback = self.callbacks.borrow_mut().memory_warning.take();
                if let Some(mut callback) = callback {
                    callback();
                    self.callbacks
                        .borrow_mut()
                        .memory_warning
                        .get_or_insert(callback);
                }
            }
            _ => {}
        }
    }

    fn lifecycle_event(&self, event: ActivityEvent) {
        let transition = self.lifecycle.borrow_mut().apply(event);
        if let Some(phase) = transition.phase {
            let callback = self.callbacks.borrow_mut().lifecycle.take();
            if let Some(mut callback) = callback {
                callback(phase);
                self.callbacks
                    .borrow_mut()
                    .lifecycle
                    .get_or_insert(callback);
            }
        }
        let Some(window) = self.window() else {
            return;
        };
        if let Some(active) = transition.active {
            window.set_active(active);
        }
        if let Some(visibility) = transition.visibility {
            window.set_visibility(visibility);
        }
    }

    fn handle_input(&self) {
        let Ok(mut events) = self.app.input_events_iter() else {
            return;
        };
        while events.next(|event| match event {
            InputEvent::MotionEvent(motion) => self.motion(motion),
            InputEvent::KeyEvent(key) => self.key(key),
            _ => InputStatus::Unhandled,
        }) {}
    }

    fn key(&self, key: &KeyEvent<'_>) -> InputStatus {
        let Some(window) = self.window() else {
            return InputStatus::Unhandled;
        };
        let down = match key.action() {
            KeyAction::Down => true,
            KeyAction::Up => false,
            _ => return InputStatus::Unhandled,
        };
        if key.key_code() == Keycode::Back {
            if window.wants_back() {
                if !down {
                    window.back();
                }
                return InputStatus::Handled;
            }
            // Otherwise the app sees a "back" key, which it can bind to go back a
            // step. If nothing takes it, Android leaves the app as usual.
            if down && key.repeat_count() == 0 {
                let back = Keystroke {
                    modifiers: Modifiers::default(),
                    key: "back".into(),
                    key_char: None,
                };
                self.back_taken.set(window.key(back, true, false));
            }
            return if self.back_taken.get() {
                InputStatus::Handled
            } else {
                InputStatus::Unhandled
            };
        }
        let Some(name) = keys::key_name(key.key_code().into()) else {
            return InputStatus::Unhandled;
        };
        let meta = key.meta_state();
        let modifiers = Modifiers {
            control: meta.ctrl_on(),
            alt: meta.alt_on(),
            shift: meta.shift_on(),
            platform: meta.meta_on(),
            function: meta.function_on(),
        };
        let key_char = keys::types_text(name)
            .then(|| self.key_char(key, name, modifiers.shift))
            .flatten();
        let keystroke = Keystroke {
            modifiers,
            key: name.into(),
            key_char,
        };
        if window.key(keystroke, down, key.repeat_count() > 0) {
            InputStatus::Handled
        } else {
            InputStatus::Unhandled
        }
    }

    /// The character a key types with its modifiers, from the keyboard's layout.
    fn key_char(&self, key: &KeyEvent<'_>, name: &str, shift: bool) -> Option<String> {
        let mapped = self
            .app
            .device_key_character_map(key.device_id())
            .ok()
            .and_then(|map| map.get(key.key_code(), key.meta_state()).ok());
        match mapped {
            Some(KeyMapChar::Unicode(character)) => Some(character.to_string()),
            Some(KeyMapChar::CombiningAccent(_) | KeyMapChar::None) => None,
            // No layout for this device: assume a US keyboard.
            None if name == "space" => Some(" ".into()),
            None if shift => Some(name.to_uppercase()),
            None => Some(name.into()),
        }
    }

    fn motion(&self, motion: &MotionEvent<'_>) -> InputStatus {
        let Some(window) = self.window() else {
            return InputStatus::Unhandled;
        };
        let action = match motion.action() {
            MotionAction::Down => Action::Down,
            MotionAction::PointerDown => Action::PointerDown(motion.pointer_index()),
            MotionAction::Move => Action::Move,
            MotionAction::PointerUp => Action::PointerUp(motion.pointer_index()),
            MotionAction::Up => Action::Up,
            MotionAction::Cancel => Action::Cancel,
            _ => Action::Other,
        };
        if action == Action::Other {
            return InputStatus::Unhandled;
        }
        let scale = window.scale();
        let contact = |id: i32, x: f32, y: f32, pressure: f32| Contact {
            id,
            position: point(px(x / scale), px(y / scale)),
            force: Some(pressure.clamp(0., 1.)),
        };
        let current: Vec<Contact> = motion
            .pointers()
            .map(|p| contact(p.pointer_id(), p.x(), p.y(), p.pressure()))
            .collect();
        // Batched samples, regrouped from per pointer to per moment.
        let mut history: Vec<Vec<Contact>> = Vec::new();
        if action == Action::Move {
            for pointer in motion.pointers() {
                for (moment, sample) in pointer.history().enumerate() {
                    if history.len() <= moment {
                        history.push(Vec::new());
                    }
                    history[moment].push(contact(
                        pointer.pointer_id(),
                        sample.x(),
                        sample.y(),
                        sample.pressure(),
                    ));
                }
            }
        }
        let events = window.touches().translate(action, &history, &current);
        log::debug!(
            "motion {action:?}: {} pointers, {} batched samples -> {:?}",
            current.len(),
            history.len(),
            events
                .iter()
                .map(|event| (event.id.0, event.phase, event.position))
                .collect::<Vec<_>>()
        );
        window.touch(events);
        InputStatus::Handled
    }
}

impl Platform for AndroidPlatform {
    fn background_executor(&self) -> BackgroundExecutor {
        self.background_executor.clone()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.foreground_executor.clone()
    }

    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.text_system.clone()
    }

    fn gestures(&self) -> Option<Rc<dyn PlatformGestures>> {
        Some(Rc::new(AndroidGestures))
    }

    /// Launches once the activity has a window, then runs until Android
    /// destroys the activity or the app quits. Returning from `android_main`
    /// afterwards finishes the activity.
    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        while self.app.native_window().is_none() {
            if !self.turn(None) {
                return;
            }
        }
        on_finish_launching();
        while !self.quitting.get() {
            let pending = self.main_queue.borrow_mut().run(TASK_BUDGET);
            if !self.turn(pending.then_some(Duration::ZERO)) {
                break;
            }
        }
        if let Some(window) = self.window() {
            window.close();
        }
    }

    fn quit(&self) {
        let callback = self.callbacks.borrow_mut().quit.take();
        let quit = callback.is_none_or(|mut callback| callback());
        if quit {
            self.quitting.set(true);
            self.app.create_waker().wake();
        }
    }

    fn restart(&self, _binary_path: Option<PathBuf>, _arguments: Vec<std::ffi::OsString>) {}

    fn activate(&self, _ignoring_other_apps: bool) {}

    fn hide(&self) {}

    fn hide_other_apps(&self) {}

    fn unhide_other_apps(&self) {}

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        vec![self.display.clone()]
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(self.display.clone())
    }

    fn active_window(&self) -> Option<AnyWindowHandle> {
        self.window().map(|window| window.handle)
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        params: WindowParams,
    ) -> Result<Box<dyn PlatformWindow>> {
        match params.kind {
            WindowKind::Normal => {}
            WindowKind::AnchoredPopup(_) => return Err(PopupNotSupportedError.into()),
            _ => return Err(anyhow!("Android shows one window per activity")),
        }
        if self.window().is_some() {
            return Err(anyhow!("Android shows one window per activity"));
        }
        let native = self
            .app
            .native_window()
            .ok_or_else(|| anyhow!("the activity has no window while in the background"))?;
        let lifecycle = self.lifecycle.borrow();
        let state = WindowState::new(
            handle,
            self.app.clone(),
            self.java.clone(),
            native,
            self.gpu.clone(),
            self.display.clone(),
            self.clock.clone(),
            self.window_appearance(),
            lifecycle.is_active(),
            lifecycle.visibility(),
        )?;
        drop(lifecycle);
        if let Some((bars, ime)) = self.insets.get() {
            state.set_reported_insets(bars, ime);
        }
        *self.window.borrow_mut() = Rc::downgrade(&state);
        self.clock.request();
        Ok(Box::new(AndroidWindow(state)))
    }

    fn window_appearance(&self) -> WindowAppearance {
        match self.app.config().ui_mode_night() {
            UiModeNight::Yes => WindowAppearance::Dark,
            _ => WindowAppearance::Light,
        }
    }

    fn open_url(&self, url: &str) {
        self.java.open_url(url);
    }

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.callbacks.borrow_mut().open_urls = Some(callback);
        let pending = std::mem::take(&mut *self.pending_urls.borrow_mut());
        if !pending.is_empty() {
            self.open_urls(pending);
        }
    }

    fn register_url_scheme(&self, _url: &str) -> Task<Result<()>> {
        Task::ready(Err(anyhow!(
            "URL schemes are declared in the app's manifest"
        )))
    }

    /// Opened documents are copied into the app's cache under their own names,
    /// so the paths are good for reading; changes to them stay in the copy.
    /// Android apps cannot open folders as paths.
    fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        let (sender, receiver) = oneshot::channel();
        if !options.files {
            sender
                .send(Err(anyhow!("Android apps can open files, not folders")))
                .ok();
            return receiver;
        }
        let request = self.pick_request(Pick::Open(sender));
        if !self.java.pick_files(request, options.multiple) {
            self.picked(request, Err("there is no file picker".into()));
        }
        receiver
    }

    /// The path is in the app's cache; each time the app finishes writing it,
    /// the file is copied to the chosen document.
    fn prompt_for_new_path(
        &self,
        _directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        let (sender, receiver) = oneshot::channel();
        let request = self.pick_request(Pick::Save(sender));
        if !self
            .java
            .pick_save_file(request, suggested_name.unwrap_or("Untitled"))
        {
            self.picked(request, Err("there is no file picker".into()));
        }
        receiver
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        false
    }

    fn reveal_path(&self, _path: &Path) {}

    fn open_with_system(&self, _path: &Path) {}

    fn on_quit(&self, callback: Box<dyn FnMut() -> bool>) {
        self.callbacks.borrow_mut().quit = Some(callback);
    }

    fn on_reopen(&self, _callback: Box<dyn FnMut()>) {}

    fn on_system_sleep(&self, _callback: Box<dyn FnMut()>) {}

    fn on_system_wake(&self, _callback: Box<dyn FnMut()>) {}

    fn on_app_lifecycle(&self, callback: Box<dyn FnMut(AppLifecyclePhase)>) {
        self.callbacks.borrow_mut().lifecycle = Some(callback);
    }

    fn on_memory_warning(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().memory_warning = Some(callback);
    }

    fn set_menus(&self, _menus: Vec<Menu>, _keymap: &Keymap) {}

    fn set_dock_menu(&self, _menu: Vec<MenuItem>, _keymap: &Keymap) {}

    fn on_app_menu_action(&self, _callback: Box<dyn FnMut(&dyn GpuiAction)>) {}

    fn on_will_open_app_menu(&self, _callback: Box<dyn FnMut()>) {}

    fn on_validate_app_menu_command(&self, _callback: Box<dyn FnMut(&dyn GpuiAction) -> bool>) {}

    fn thermal_state(&self) -> ThermalState {
        ThermalState::Nominal
    }

    fn on_thermal_state_change(&self, _callback: Box<dyn FnMut()>) {}

    fn prevent_idle_sleep(&self, _reason: &str) -> Task<Result<ActivityGuard>> {
        Task::ready(Ok(ActivityGuard::noop()))
    }

    fn compositor_name(&self) -> &'static str {
        "Android"
    }

    fn app_path(&self) -> Result<PathBuf> {
        Ok(std::env::current_exe()?)
    }

    fn path_for_auxiliary_executable(&self, _name: &str) -> Result<PathBuf> {
        Err(anyhow!("Android apps cannot ship auxiliary executables"))
    }

    fn set_cursor_style(&self, _style: CursorStyle) {}

    fn hide_cursor_until_mouse_moves(&self) {}

    fn is_cursor_visible(&self) -> bool {
        false
    }

    fn should_auto_hide_scrollbars(&self) -> bool {
        true
    }

    /// Text and images. Without the Java activity, the clipboard stays inside the app.
    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        if !self.java.is_available() {
            return self.clipboard.borrow().clone();
        }
        let mut entries = Vec::new();
        if let Some((mime, bytes)) = self.java.clipboard_image()
            && let Some(format) = ImageFormat::from_mime_type(&mime)
        {
            entries.push(ClipboardEntry::Image(Image::from_bytes(format, bytes)));
        }
        if let Some(text) = self.java.clipboard_text() {
            entries.push(ClipboardEntry::String(ClipboardString::new(text)));
        }
        (!entries.is_empty()).then_some(ClipboardItem { entries })
    }

    fn write_to_clipboard(&self, item: ClipboardItem) {
        if !self.java.is_available() {
            *self.clipboard.borrow_mut() = Some(item);
            return;
        }
        let image = item.entries.iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        });
        match (image, item.text()) {
            (Some(image), text) => self.java.set_clipboard_image(
                &image.bytes,
                image.format.mime_type(),
                image.format.extension(),
                text.as_deref(),
            ),
            (None, Some(text)) => self.java.set_clipboard_text(&text),
            (None, None) => {}
        }
    }

    fn write_credentials(&self, _url: &str, _username: &str, _password: &[u8]) -> Task<Result<()>> {
        Task::ready(Err(anyhow!(
            "credential storage is not supported on Android yet"
        )))
    }

    fn read_credentials(&self, _url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        Task::ready(Ok(None))
    }

    fn delete_credentials(&self, _url: &str) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(AndroidKeyboardLayout)
    }

    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        Rc::new(DummyKeyboardMapper)
    }

    fn on_keyboard_layout_change(&self, _callback: Box<dyn FnMut()>) {}
}

/// The newest log lines kept in memory, so an app can show them.
const RECENT_LOGS: usize = 1000;

static RECENT: std::sync::Mutex<std::collections::VecDeque<String>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// What was logged lately, oldest first: "15:53:50 W live: …".
pub fn recent_logs() -> Vec<String> {
    RECENT
        .lock()
        .map(|lines| lines.iter().cloned().collect())
        .unwrap_or_default()
}

/// Logcat, plus the newest lines in memory.
struct Logger {
    logcat: android_logger::AndroidLogger,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.logcat.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        self.logcat.log(record);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs()) as libc::time_t;
        // SAFETY: `localtime_r` reads `now` and writes only into `tm`.
        let tm = unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&now, &mut tm);
            tm
        };
        let module = record
            .module_path()
            .map(|path| path.rsplit("::").next().unwrap_or(path))
            .unwrap_or("");
        let line = format!(
            "{:02}:{:02}:{:02} {} {module}: {}",
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            &record.level().as_str()[..1],
            record.args()
        );
        if let Ok(mut lines) = RECENT.lock() {
            if lines.len() == RECENT_LOGS {
                lines.pop_front();
            }
            lines.push_back(line);
        }
    }

    fn flush(&self) {}
}

/// Logs to logcat under `tag` and reports panics there, which otherwise vanish.
/// The newest lines also stay in memory: see `recent_logs`.
pub fn init_logging(tag: &str, level: log::LevelFilter) {
    let logger = Logger {
        logcat: android_logger::AndroidLogger::new(
            android_logger::Config::default()
                .with_max_level(level)
                .with_tag(tag),
        ),
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("{info}");
        default_hook(info);
    }));
}
