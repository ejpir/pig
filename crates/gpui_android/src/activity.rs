//! What an app can ask of the activity beyond GPUI's platform: notifications,
//! the permission to post them, and the color of the system bars' icons.
//!
//! Tapping a notification, or one of its actions, opens its URL in the app;
//! register [`gpui::Application::on_open_urls`] to receive it. Everything here
//! must be called on the main thread, and does nothing on other hosts or
//! without `GpuiActivity`.

/// How a channel's notifications interrupt. Android fixes a channel's
/// importance when the channel is first used; the user can change it later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Importance {
    /// In the shade only, without sound or a status bar icon.
    Low,
    /// Sound and a status bar icon.
    Default,
    /// Also appears over the current app.
    High,
}

impl Importance {
    /// `NotificationManager.IMPORTANCE_*`.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(crate) fn android(self) -> i32 {
        match self {
            Self::Low => 2,
            Self::Default => 3,
            Self::High => 4,
        }
    }
}

/// A notification channel, which users can mute or adjust in the system settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    pub id: &'static str,
    pub name: &'static str,
    pub importance: Importance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    /// Posting again with the same id replaces the notification.
    pub id: i32,
    pub channel: Channel,
    pub title: String,
    pub text: String,
    /// A short line in the header, after the app's name.
    pub subtext: Option<String>,
    /// Opened when the notification is tapped.
    pub url: String,
    /// Buttons, as labels and the URLs they open.
    pub actions: Vec<(String, String)>,
    /// Stays until cancelled, and is not dismissed by tapping it.
    pub ongoing: bool,
    /// The accent for the icon and actions, as 0xRRGGBB.
    pub color: u32,
}

/// Posts or replaces a notification. Returns false if it could not be posted;
/// a user who turned notifications off still gets true.
pub fn notify(notification: &Notification) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.post_notification(notification))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = notification;
        false
    }
}

pub fn cancel_notification(id: i32) {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.cancel_notification(id);
    }
    #[cfg(not(target_os = "android"))]
    let _ = id;
}

/// Whether the user lets the app post notifications.
pub fn notifications_enabled() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.notifications_enabled())
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Whether Android's system animation duration scale is disabled.
/// Other platforms retain GPUI's own reduced-motion setting.
pub fn reduced_motion() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.reduced_motion())
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// The refresh rate Android reports to this app, including rate overrides.
pub fn refresh_rate() -> Option<f32> {
    #[cfg(target_os = "android")]
    {
        crate::java::current().and_then(|java| java.refresh_rate())
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// The fastest refresh rate offered at the display's current resolution.
///
/// GPUI Android requests this rate while drawing. The system may still select
/// a lower active rate because of power, thermal, or platform policy.
pub fn max_refresh_rate() -> Option<f32> {
    #[cfg(target_os = "android")]
    {
        crate::java::current().and_then(|java| java.max_refresh_rate())
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// Asks the user for permission to notify, on Android 13 and later, unless
/// they already answered. Ask when notifications become useful to them.
pub fn request_notification_permission() {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.request_notifications();
    }
}

/// The system bars draw their icons over the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarIcons {
    /// Dark icons for a light app in night mode, light ones otherwise: the default.
    System,
    /// Dark icons, for a light app.
    Dark,
    /// Light icons, for a dark app.
    Light,
}

/// Sets the system bars' icons for an app whose theme differs from the system's.
pub fn set_bar_icons(icons: BarIcons) {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.set_bar_icons(match icons {
            BarIcons::System => -1,
            BarIcons::Light => 0,
            BarIcons::Dark => 1,
        });
    }
    #[cfg(not(target_os = "android"))]
    let _ = icons;
}

/// The phone's short buzz for a long press, as when it selects text. It
/// follows the system's touch feedback setting.
pub fn long_press_feedback() {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.long_press_feedback();
    }
}

/// Opens the app's full-screen QR camera. A decoded URL arrives through
/// `Application::on_open_urls`, exactly like a system-camera deep link.
pub fn scan_qr() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.scan_qr())
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Opens an HTML page full screen: a sandboxed web view, without access to the
/// app or its files, and the page's source a tab away. `site` is JSON,
/// `{"path": "demo/index.html", "pages": {"demo/about.html": "<html>…"}}`:
/// the page's own path, and pages its links may open in the same view, by
/// path. Other links leave for the browser. Returns false if it could not be
/// opened.
pub fn show_page(
    title: &str,
    html: &str,
    dark: bool,
    source: bool,
    poster: &str,
    site: &str,
) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current()
            .is_some_and(|java| java.show_page(title, html, dark, source, poster, site))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (title, html, dark, source, poster, site);
        false
    }
}

/// Whether this platform can host an interactive, offline page inside GPUI.
pub fn supports_inline_pages() -> bool {
    cfg!(target_os = "android")
}

/// Whether the Java host is healthy. Failures are recoverable, but do not
/// automatically retry an unavailable provider or a crashing document.
pub fn inline_pages_available() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.inline_pages_available())
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Registers an immutable page document without executing it. The native host
/// retains a bounded document cache; query it again before publishing a slot.
/// `revision` changes only when the user explicitly loads a different document.
pub fn prepare_inline_page(id: &str, revision: &str, html: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.prepare_inline_page(id, revision, html))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (id, revision, html);
        false
    }
}

/// Publishes the actually painted slot, in physical window pixels, as JSON.
/// An empty object hides the host without destroying its running document.
/// Calls must follow painting, not virtual-list measurement. This is an
/// offline, scripts-only host: no network, file access, permissions or bridge.
pub fn sync_inline_page(slot: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.sync_inline_page(slot))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = slot;
        false
    }
}

/// Fixture-only, read-only native document inspection; not a page bridge.
#[doc(hidden)]
pub fn inspect_inline_page_for_test(request: &str) {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.inspect_inline_page_for_test(request);
    }
    #[cfg(not(target_os = "android"))]
    let _ = request;
}

/// Fixture harness only: terminate the isolated browser renderer, not the app.
#[doc(hidden)]
pub fn terminate_inline_page_for_test() {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.terminate_inline_page_for_test();
    }
}

/// Discards all inline document state when the app changes computer/store.
pub fn clear_inline_pages() {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.clear_inline_pages();
    }
}

/// Draws a page offscreen and saves the top of it, at its card's shape, as a
/// PNG at `path`; the file appears a moment later. False where there is no
/// web view.
pub fn render_poster(html: &str, path: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.render_poster(html, path))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (html, path);
        false
    }
}

/// A friendly hardware name shown while approving a newly paired phone.
pub fn device_name() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        crate::java::current().and_then(|java| java.device_name())
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// Sensitive native provider-code interaction. All calls are on the GPUI main thread.
pub fn show_provider_code(request: &str) {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.show_provider_code(request);
    }
    #[cfg(not(target_os = "android"))]
    let _ = request;
}
pub fn dismiss_provider_code(request: &str) {
    #[cfg(target_os = "android")]
    if let Some(java) = crate::java::current() {
        java.dismiss_provider_code(request);
    }
    #[cfg(not(target_os = "android"))]
    let _ = request;
}
pub fn take_provider_code(request: &str) -> Option<String> {
    #[cfg(target_os = "android")]
    {
        crate::java::current()?.take_provider_code(request)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = request;
        None
    }
}

/// Explicitly copy a short sign-in code with Android's sensitive-clipboard marker.
/// Returns false if the platform/clipboard is unavailable. Never logs the code.
pub fn copy_provider_code(code: &str) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.copy_provider_code(code))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = code;
        false
    }
}

/// APK debuggability, independent of Rust optimization. Missing Android bridges fail
/// closed; host previews use the Rust build profile.
pub fn is_debug_build() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::java::current().is_some_and(|java| java.is_debug_build())
    }
    #[cfg(not(target_os = "android"))]
    {
        cfg!(debug_assertions)
    }
}
