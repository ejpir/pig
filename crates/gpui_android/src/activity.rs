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
