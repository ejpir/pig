//! The activity's window.
//!
//! Android owns the native window: it is created after launch, destroyed when
//! the app goes to the background and recreated on return. The renderer
//! outlives it, so the glyph atlas and pipelines survive, and only the wgpu
//! surface is replaced.
//!
//! Text input goes through the on-screen keyboard's mirror (see `ime`): a tap
//! on a focused text input shows the keyboard, keyboard edits become input
//! handler calls, and after every turn of the event loop the app's text is
//! compared with the mirror and sent to it if they differ.

use crate::{
    display::AndroidDisplay,
    frame_clock::FrameClock,
    ime::{self, AppText, Edit, ImeState, Mirror},
    java::Java,
    touch::Touches,
};
use android_activity::AndroidApp;
use gpui::{
    AnyWindowHandle, Bounds, Capslock, DevicePixels, DispatchEventResult, Edges, GpuSpecs,
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, Pixels, PlatformAtlas, PlatformDisplay,
    PlatformInput, PlatformInputHandler, PlatformWindow, Point, PromptButton, PromptLevel,
    RequestFrameOptions, Scene, Size, TextInputConfiguration, TextInputStateChange, TouchEvent,
    TouchId, TouchPhase, WindowAppearance, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea, WindowInsets, WindowVisibility, px, size,
};
use gpui_wgpu::{GpuContext, WgpuRenderer, WgpuSurfaceConfig, wgpu};
use ndk::native_window::NativeWindow;
use raw_window_handle as rwh;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

/// The native window as wgpu needs it: a window handle and a display handle.
#[derive(Clone, Debug)]
struct Surface(NativeWindow);

impl rwh::HasWindowHandle for Surface {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        rwh::HasWindowHandle::window_handle(&self.0)
    }
}

impl rwh::HasDisplayHandle for Surface {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        Ok(rwh::DisplayHandle::android())
    }
}

impl Surface {
    fn size(&self) -> Size<DevicePixels> {
        size(
            DevicePixels(self.0.width().max(1)),
            DevicePixels(self.0.height().max(1)),
        )
    }

    fn config(&self) -> WgpuSurfaceConfig {
        WgpuSurfaceConfig {
            size: self.size(),
            transparent: false,
            // Never block on a swapchain image while Android tears the window down.
            preferred_present_mode: Some(wgpu::PresentMode::Mailbox),
        }
    }
}

type InputCallback = Box<dyn FnMut(PlatformInput) -> DispatchEventResult>;
type ResizeCallback = Box<dyn FnMut(Size<Pixels>, f32)>;

#[derive(Default)]
struct Callbacks {
    request_frame: Option<Box<dyn FnMut(RequestFrameOptions)>>,
    input: Option<InputCallback>,
    active_status_change: Option<Box<dyn FnMut(bool)>>,
    visibility_change: Option<Box<dyn FnMut(WindowVisibility)>>,
    resize: Option<ResizeCallback>,
    close: Option<Box<dyn FnOnce()>>,
    appearance_changed: Option<Box<dyn FnMut()>>,
    insets_changed: Option<Box<dyn FnMut(WindowInsets)>>,
    back: Option<Box<dyn FnMut()>>,
}

/// Calls a stored callback without holding the borrow, since GPUI may register
/// or replace callbacks from inside one.
macro_rules! with_callback {
    ($state:expr, $field:ident, |$callback:ident| $body:expr) => {{
        let taken = $state.callbacks.borrow_mut().$field.take();
        taken.map(|mut $callback| {
            let result = $body;
            let mut callbacks = $state.callbacks.borrow_mut();
            if callbacks.$field.is_none() {
                callbacks.$field = Some($callback);
            }
            result
        })
    }};
}

/// How far a touch may travel and still count as a tap that shows the keyboard.
const TAP_SLOP: f64 = 8.;

pub(crate) struct WindowState {
    pub handle: AnyWindowHandle,
    app: AndroidApp,
    java: Rc<Java>,
    display: Rc<AndroidDisplay>,
    clock: Rc<FrameClock>,
    gpu: GpuContext,
    renderer: RefCell<WgpuRenderer>,
    surface: RefCell<Option<Surface>>,
    device_size: Cell<Size<DevicePixels>>,
    scale: Cell<f32>,
    /// The display's fastest refresh rate, requested while the window draws.
    refresh_rate: Option<f32>,
    insets: RefCell<WindowInsets>,
    /// Whether Java reports insets; otherwise they come from the content area.
    reported_insets: Cell<bool>,
    appearance: Cell<WindowAppearance>,
    active: Cell<bool>,
    visibility: Cell<WindowVisibility>,
    last_touch: Cell<Point<Pixels>>,
    touches: RefCell<Touches>,
    /// A touch that may still end as a tap.
    tap: Cell<Option<(TouchId, Point<Pixels>)>>,
    /// Where a tap ended; show the keyboard if it landed on the focused text input.
    tapped: Cell<Option<Point<Pixels>>>,
    /// Whether a text input has focus.
    text_focus: Cell<bool>,
    /// Fulfilled after drawing and synchronizing the newly focused field.
    keyboard_requested: Cell<bool>,
    mirror: RefCell<Mirror>,
    /// A key changed the text since the last sync.
    keyed: Cell<bool>,
    back_enabled: Cell<bool>,
    input_handler: RefCell<Option<PlatformInputHandler>>,
    callbacks: RefCell<Callbacks>,
}

pub(crate) struct AndroidWindow(pub Rc<WindowState>);

impl WindowState {
    /// Creates the renderer on the activity's current native window.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handle: AnyWindowHandle,
        app: AndroidApp,
        java: Rc<Java>,
        native: NativeWindow,
        gpu: GpuContext,
        display: Rc<AndroidDisplay>,
        clock: Rc<FrameClock>,
        appearance: WindowAppearance,
        active: bool,
        visibility: WindowVisibility,
    ) -> anyhow::Result<Rc<Self>> {
        let surface = Surface(native);
        let renderer = WgpuRenderer::new(gpu.clone(), &surface, surface.config(), None)?;
        let refresh_rate = java.max_refresh_rate();
        let state = Rc::new(Self {
            handle,
            scale: Cell::new(scale_factor(&app)),
            app,
            java,
            display,
            clock,
            gpu,
            renderer: RefCell::new(renderer),
            device_size: Cell::new(surface.size()),
            refresh_rate,
            insets: RefCell::default(),
            reported_insets: Cell::new(false),
            appearance: Cell::new(appearance),
            active: Cell::new(active),
            visibility: Cell::new(visibility),
            last_touch: Cell::default(),
            touches: RefCell::default(),
            tap: Cell::new(None),
            tapped: Cell::new(None),
            text_focus: Cell::new(false),
            keyboard_requested: Cell::new(false),
            keyed: Cell::new(false),
            mirror: RefCell::default(),
            back_enabled: Cell::new(false),
            input_handler: RefCell::default(),
            callbacks: RefCell::default(),
            surface: RefCell::new(None),
        });
        state.prefer_refresh_rate(&surface);
        *state.surface.borrow_mut() = Some(surface);
        state.display.set_size(state.content_size());
        *state.insets.borrow_mut() = state.content_rect_insets();
        Ok(state)
    }

    /// Lets Android switch the display to its fastest mode while this window
    /// draws; an idle window draws nothing, so this costs no power at rest.
    fn prefer_refresh_rate(&self, surface: &Surface) {
        let Some(rate) = self.refresh_rate else {
            return;
        };
        let compatibility = ndk_sys::ANativeWindow_FrameRateCompatibility::ANATIVEWINDOW_FRAME_RATE_COMPATIBILITY_DEFAULT;
        // SAFETY: the native window is alive while `surface` holds it.
        let status = unsafe {
            ndk_sys::ANativeWindow_setFrameRate(
                surface.0.ptr().as_ptr(),
                rate,
                compatibility.0 as i8,
            )
        };
        if status != 0 {
            log::debug!("Android declined a {rate} Hz frame rate ({status})");
        }
    }

    fn content_size(&self) -> Size<Pixels> {
        let device = self.device_size.get();
        let scale = self.scale.get();
        size(
            px(device.width.0 as f32 / scale),
            px(device.height.0 as f32 / scale),
        )
    }

    pub fn scale(&self) -> f32 {
        self.scale.get()
    }

    /// A new native window after launch or on returning from the background.
    pub fn attach(&self, native: NativeWindow) {
        let surface = Surface(native);
        let instance = self
            .gpu
            .borrow()
            .as_ref()
            .map(|context| context.instance.clone());
        let attached = match instance {
            Some(instance) => {
                self.renderer
                    .borrow_mut()
                    .replace_surface(&surface, surface.config(), &instance)
            }
            None => Err(anyhow::anyhow!("the GPU context is gone")),
        };
        match attached {
            Ok(()) => {
                self.prefer_refresh_rate(&surface);
                *self.surface.borrow_mut() = Some(surface);
            }
            Err(error) => log::error!("Could not draw to the new window: {error:#}"),
        }
        self.measure();
    }

    /// Android is destroying the native window; stop using it before returning.
    pub fn detach(&self) {
        self.renderer.borrow_mut().unconfigure_surface();
        self.surface.borrow_mut().take();
    }

    /// Re-reads size and density after a resize, rotation or configuration change.
    pub fn measure(&self) {
        let device_size = self.surface.borrow().as_ref().map(Surface::size);
        let scale = scale_factor(&self.app);
        let device_size = device_size.unwrap_or(self.device_size.get());
        if device_size == self.device_size.get() && scale == self.scale.get() {
            return;
        }
        self.device_size.set(device_size);
        self.scale.set(scale);
        self.renderer.borrow_mut().update_drawable_size(device_size);
        let content = self.content_size();
        self.display.set_size(content);
        with_callback!(self, resize, |callback| callback(content, scale));
        self.refresh_insets();
        self.clock.request();
    }

    /// The system bars and keyboard, from the activity's visible content area,
    /// for a plain `NativeActivity` that cannot report them separately.
    fn content_rect_insets(&self) -> WindowInsets {
        let rect = self.app.content_rect();
        let device = self.device_size.get();
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return WindowInsets::default();
        }
        let scale = self.scale.get();
        let logical = |device_px: i32| px(device_px.max(0) as f32 / scale);
        WindowInsets {
            safe_area: Edges {
                top: logical(rect.top),
                right: logical(device.width.0 - rect.right),
                bottom: logical(device.height.0 - rect.bottom),
                left: logical(rect.left),
            },
            ime: Edges::default(),
        }
    }

    /// The content area changed; only used when Java does not report insets.
    pub fn refresh_insets(&self) {
        if !self.reported_insets.get() {
            self.update_insets(self.content_rect_insets());
        }
    }

    /// Insets reported by Java, in device pixels: left, top, right, bottom.
    pub fn set_reported_insets(&self, bars: [i32; 4], ime: [i32; 4]) {
        self.reported_insets.set(true);
        let scale = self.scale.get();
        let edges = |[left, top, right, bottom]: [i32; 4]| {
            let logical = |device_px: i32| px(device_px.max(0) as f32 / scale);
            Edges {
                top: logical(top),
                right: logical(right),
                bottom: logical(bottom),
                left: logical(left),
            }
        };
        self.update_insets(WindowInsets {
            safe_area: edges(bars),
            ime: edges(ime),
        });
    }

    fn update_insets(&self, insets: WindowInsets) {
        if *self.insets.borrow() == insets {
            return;
        }
        *self.insets.borrow_mut() = insets.clone();
        with_callback!(self, insets_changed, |callback| callback(insets.clone()));
        self.clock.request();
    }

    pub fn set_appearance(&self, appearance: WindowAppearance) {
        if self.appearance.replace(appearance) != appearance {
            with_callback!(self, appearance_changed, |callback| callback());
        }
    }

    pub fn set_active(&self, active: bool) {
        self.active.set(active);
        with_callback!(self, active_status_change, |callback| callback(active));
    }

    pub fn set_visibility(&self, visibility: WindowVisibility) {
        self.visibility.set(visibility);
        with_callback!(self, visibility_change, |callback| callback(visibility));
        if visibility.is_visible() {
            self.clock.request();
        }
    }

    /// Lets GPUI draw a frame, if one can be shown. `present` repaints the last
    /// frame even when nothing changed, for a new or damaged surface.
    pub fn frame(&self, present: bool) {
        if !self.visibility.get().is_visible() || self.surface.borrow().is_none() {
            log::debug!(
                "frame skipped: {:?}, surface: {}",
                self.visibility.get(),
                self.surface.borrow().is_some()
            );
            return;
        }
        log::debug!("frame (present: {present})");
        with_callback!(self, request_frame, |callback| callback(
            RequestFrameOptions {
                require_presentation: present,
                force_render: false,
            }
        ));
        self.settle_after_frame();
    }

    pub fn touch(&self, events: Vec<TouchEvent>) {
        for event in events {
            self.last_touch.set(event.position);
            let tapped = match event.phase {
                TouchPhase::Started => {
                    self.tap.set(Some((event.id, event.position)));
                    None
                }
                TouchPhase::Moved => {
                    if let Some((id, start)) = self.tap.get()
                        && id == event.id
                        && (event.position - start).magnitude() > TAP_SLOP
                    {
                        self.tap.set(None);
                    }
                    None
                }
                TouchPhase::Ended => self
                    .tap
                    .take()
                    .is_some_and(|(id, _)| id == event.id)
                    .then_some(event.position),
                TouchPhase::Cancelled => {
                    self.tap.set(None);
                    None
                }
            };
            if self.dispatch(PlatformInput::Touch(event)).is_none() {
                log::warn!("touch dropped: GPUI has not registered for input");
            }
            // GPUI draws right after a tap, so a text input it focused has
            // already reported focus; `settle` shows the keyboard for it.
            if tapped.is_some() {
                self.tapped.set(tapped);
                self.clock.request();
            }
        }
    }

    fn dispatch(&self, input: PlatformInput) -> Option<DispatchEventResult> {
        with_callback!(self, input, |callback| callback(input))
    }

    /// Calls the focused input handler, outside of any GPUI update.
    fn with_input_handler<T>(&self, f: impl FnOnce(&mut PlatformInputHandler) -> T) -> Option<T> {
        let mut handler = self.input_handler.borrow_mut().take()?;
        let result = f(&mut handler);
        let mut slot = self.input_handler.borrow_mut();
        if slot.is_none() {
            *slot = Some(handler);
        }
        Some(result)
    }

    /// A key from a hardware keyboard, or an editing key from the on-screen
    /// one. Unhandled character keys type their character, as on Linux.
    /// Returns whether the app took the key.
    pub fn key(&self, keystroke: Keystroke, down: bool, held: bool) -> bool {
        let text = keystroke
            .key_char
            .clone()
            .filter(|_| keystroke.modifiers.is_subset_of(&Modifiers::shift()));
        let input = if down {
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke,
                is_held: held,
                prefer_character_input: false,
            })
        } else {
            PlatformInput::KeyUp(KeyUpEvent { keystroke })
        };
        let Some(result) = self.dispatch(input) else {
            return false;
        };
        let taken = if down
            && result.propagate
            && let Some(text) = text
        {
            self.with_input_handler(|handler| handler.replace_text_in_range(None, &text))
                .is_some()
        } else {
            !result.propagate
        };
        if down && taken {
            self.keyed.set(true);
        }
        taken
    }

    /// A key press with no character, such as the keyboard's action key.
    pub fn press(&self, key: &str) {
        let keystroke = Keystroke {
            modifiers: Modifiers::default(),
            key: key.into(),
            key_char: None,
        };
        self.key(keystroke.clone(), true, false);
        self.key(keystroke, false, false);
    }

    /// The keyboard reported its mirror: apply what changed to the app.
    pub fn apply_ime(&self, seq: i32, push_id: i32, state: ImeState) {
        let edits = self.mirror.borrow_mut().reported(seq, push_id, state);
        for edit in edits {
            if edit == Edit::Enter {
                self.press("enter");
                continue;
            }
            self.with_input_handler(|handler| match edit {
                Edit::Replace { range, text } => handler.replace_text_in_range(Some(range), &text),
                Edit::Compose {
                    range,
                    text,
                    selection,
                } => handler.replace_and_mark_text_in_range(Some(range), &text, Some(selection)),
                Edit::Commit => handler.unmark_text(),
                Edit::Select(range) => handler.set_selected_text_range(range),
                Edit::Enter => {}
            });
        }
    }

    /// After GPUI handled a turn of events: show the keyboard for a tapped text
    /// input, and send the app's text to the keyboard when it changed.
    pub fn settle(&self) {
        // Queue the current field before asking Android to create its input
        // connection, so a newly focused empty field never inherits old text.
        if self.text_focus.get() {
            self.sync_ime();
        }
    }

    fn settle_after_frame(&self) {
        // A looper turn is not necessarily a draw. Keep activation pending until
        // GPUI has painted the new focused input handler, including taps in the
        // composer's padding outside the glyph bounds.
        self.settle();
        let tapped = self.tapped.take();
        let requested = self.keyboard_requested.take();
        if self.text_focus.get() {
            // Tapping a button while a field has focus must not bring back a
            // keyboard the user dismissed.
            let bounds = self
                .with_input_handler(|handler| handler.element_bounds())
                .flatten();
            let on_input = tapped
                .zip(bounds)
                .is_some_and(|(position, bounds)| bounds.contains(&position));
            // Unknown bounds are not an input hit. A newly mounted field must
            // not inherit the tap that opened its screen or sheet.
            if requested || on_input {
                self.java.show_keyboard();
            }
        }
    }

    fn sync_ime(&self) {
        let keyed = self.keyed.take();
        let app = self.with_input_handler(|handler| {
            let selection = handler.selected_text_range(false)?.range;
            let marked = handler.marked_text_range();
            let editable = handler
                .text_input_editable_range()
                .or_else(|| handler.text_length_utf16().map(|length| 0..length))
                .unwrap_or(0..usize::MAX);
            let window = self.mirror.borrow().window(&selection, &editable);
            let mut adjusted = None;
            let text = handler.text_for_range(window.clone(), &mut adjusted)?;
            Some(AppText {
                text,
                start: adjusted.unwrap_or(window).start,
                selection,
                marked,
            })
        });
        let Some(Some(app)) = app else {
            return;
        };
        let push = self.mirror.borrow_mut().sync(app, keyed);
        if let Some(push) = push {
            self.java.set_text(&push);
        }
    }

    pub fn touches(&self) -> std::cell::RefMut<'_, Touches> {
        self.touches.borrow_mut()
    }

    /// The system back action. Returns whether the app took it.
    pub fn back(&self) -> bool {
        self.back_enabled.get() && with_callback!(self, back, |callback| callback()).is_some()
    }

    pub fn wants_back(&self) -> bool {
        self.back_enabled.get() && self.callbacks.borrow().back.is_some()
    }

    pub fn close(&self) {
        let close = self.callbacks.borrow_mut().close.take();
        if let Some(close) = close {
            close();
        }
    }
}

fn scale_factor(app: &AndroidApp) -> f32 {
    // Android's density-independent pixel is 1/160 inch.
    app.config().density().map_or(1., |dpi| dpi as f32 / 160.)
}

impl rwh::HasWindowHandle for AndroidWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let surface = self.0.surface.borrow();
        let native = surface.as_ref().ok_or(rwh::HandleError::Unavailable)?;
        let raw = rwh::AndroidNdkWindowHandle::new(native.0.ptr().cast());
        // SAFETY: the pointer stays valid while the surface is attached, which
        // is as long as GPUI may use the handle between lifecycle events.
        Ok(unsafe { rwh::WindowHandle::borrow_raw(rwh::RawWindowHandle::AndroidNdk(raw)) })
    }
}

impl rwh::HasDisplayHandle for AndroidWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        Ok(rwh::DisplayHandle::android())
    }
}

impl PlatformWindow for AndroidWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(Point::default(), self.0.content_size())
    }

    fn is_maximized(&self) -> bool {
        true
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Fullscreen(self.bounds())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.0.content_size()
    }

    fn resize(&mut self, _size: Size<Pixels>) {}

    fn scale_factor(&self) -> f32 {
        self.0.scale.get()
    }

    fn appearance(&self) -> WindowAppearance {
        self.0.appearance.get()
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(self.0.display.clone())
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.0.last_touch.get()
    }

    fn modifiers(&self) -> Modifiers {
        Modifiers::default()
    }

    fn capslock(&self) -> Capslock {
        Capslock::default()
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        *self.0.input_handler.borrow_mut() = Some(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.input_handler.borrow_mut().take()
    }

    /// GPUI draws its own prompt when the platform has none.
    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<futures::channel::oneshot::Receiver<usize>> {
        None
    }

    fn activate(&self) {}

    fn is_active(&self) -> bool {
        self.0.active.get()
    }

    fn visibility(&self) -> WindowVisibility {
        self.0.visibility.get()
    }

    fn is_hovered(&self) -> bool {
        false
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        WindowBackgroundAppearance::Opaque
    }

    fn set_title(&mut self, _title: &str) {}

    fn set_background_appearance(&self, _background_appearance: WindowBackgroundAppearance) {}

    fn minimize(&self) {}

    fn zoom(&self) {}

    fn toggle_fullscreen(&self) {}

    fn is_fullscreen(&self) -> bool {
        true
    }

    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        let clock = self.0.clock.clone();
        Some(Rc::new(move || clock.request()))
    }

    fn schedule_frame(&self) {
        self.0.clock.request();
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.callbacks.borrow_mut().request_frame = Some(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        self.0.callbacks.borrow_mut().input = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.callbacks.borrow_mut().active_status_change = Some(callback);
    }

    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        self.0.callbacks.borrow_mut().visibility_change = Some(callback);
    }

    fn on_hover_status_change(&self, _callback: Box<dyn FnMut(bool)>) {}

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.callbacks.borrow_mut().resize = Some(callback);
    }

    fn on_moved(&self, _callback: Box<dyn FnMut()>) {}

    fn on_should_close(&self, _callback: Box<dyn FnMut() -> bool>) {}

    fn on_hit_test_window_control(&self, _callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.0.callbacks.borrow_mut().close = Some(callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.callbacks.borrow_mut().appearance_changed = Some(callback);
    }

    fn draw(&self, scene: &Scene) {
        let mut renderer = self.0.renderer.borrow_mut();
        if renderer.device_lost() {
            if let Some(surface) = self.0.surface.borrow().as_ref()
                && let Err(error) = renderer.recover(surface)
            {
                log::warn!("GPU recovery failed; retrying on the next frame: {error:#}");
            }
            self.0.clock.request();
            return;
        }
        renderer.draw(scene);
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.renderer.borrow().sprite_atlas().clone()
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        // The screen rotates, so its subpixel order is not fixed.
        false
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.0.renderer.borrow().gpu_specs()
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {}

    fn insets(&self) -> WindowInsets {
        self.0.insets.borrow().clone()
    }

    fn on_insets_changed(&self, callback: Box<dyn FnMut(WindowInsets)>) {
        self.0.callbacks.borrow_mut().insets_changed = Some(callback);
    }

    fn set_back_handler(&self, callback: Box<dyn FnMut()>) {
        self.0.callbacks.borrow_mut().back = Some(callback);
    }

    fn set_back_enabled(&self, enabled: bool) {
        self.0.back_enabled.set(enabled);
    }

    fn show_soft_keyboard(&self) {
        self.0.keyboard_requested.set(true);
        self.0.clock.request();
    }

    fn hide_soft_keyboard(&self) {
        self.0.keyboard_requested.set(false);
        self.0.tapped.set(None);
        self.0.java.hide_keyboard();
    }

    /// Called while GPUI draws, so the input handler is not queried here;
    /// `settle` syncs the keyboard once the frame is done.
    fn text_input_state_changed(&self, change: TextInputStateChange) {
        match change {
            TextInputStateChange::FocusGained => {
                self.0.text_focus.set(true);
                self.0.mirror.borrow_mut().reset();
            }
            TextInputStateChange::FocusLost => {
                self.0.keyboard_requested.set(false);
                self.0.text_focus.set(false);
                self.0.mirror.borrow_mut().reset();
                self.0.java.hide_keyboard();
            }
            TextInputStateChange::SelectionChanged | TextInputStateChange::ContentChanged => {}
        }
    }

    fn set_text_input_configuration(&mut self, configuration: TextInputConfiguration) {
        let (input_type, ime_options) = ime::editor_info(&configuration);
        self.0.java.configure_keyboard(input_type, ime_options);
    }
}
