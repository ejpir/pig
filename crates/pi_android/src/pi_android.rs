//! Pi on an Android phone, after design/android: follow the sessions on a
//! computer, answer Pi's questions, review its changes and start new work.
//!
//! The phone connects to a computer over SSH with its own key (`ssh`), uses
//! Pi Desktop's helper there (`remote`), and follows its durable sessions
//! (`live`, `projection`). Sample sessions that run on their own (`demo`) are
//! there to look around without a computer, and for previews.

mod alerts;
mod app;
mod assets;
mod attachments;
mod composer;
mod demo;
mod live;
mod message;
mod model;
mod motion;
mod pages;
mod pairing;
mod prefs;
mod preview;
mod projection;
mod projects;
mod prompt;
mod remote;
mod screens;
mod scroll;
mod ssh;
mod store;
mod testing;
mod text_area;
mod theme;
mod ui;

pub use app::{GoBack, PhoneApp};
pub use assets::Assets;
pub use preview::SCREENS;

use gpui::{App, AppContext, KeyBinding, WindowOptions};
use std::path::PathBuf;

/// Fonts and key bindings; call once before opening the window.
pub fn init(cx: &mut App) {
    #[cfg(feature = "ui-test")]
    cx.set_global(testing::State::default());
    if let Err(error) = assets::load_fonts(cx) {
        log::error!("Could not load the fonts: {error:#}");
    }
    use text_area::*;
    let context = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("enter", Enter, context),
        KeyBinding::new("shift-enter", Newline, context),
        KeyBinding::new("ctrl-v", Paste, context),
        KeyBinding::new("ctrl-a", SelectAll, context),
        KeyBinding::new("ctrl-c", CopySelection, context),
        KeyBinding::new("ctrl-x", CutSelection, context),
        // Android's back button and gesture arrive as the "back" key.
        KeyBinding::new("back", GoBack, None),
        KeyBinding::new("escape", GoBack, None),
    ]);
}

/// Opens the app's window. `data_dir` keeps the settings; `urls` brings the
/// `pi://` links that notifications open.
pub fn open(cx: &mut App, data_dir: Option<PathBuf>, urls: async_channel::Receiver<String>) {
    let opened = cx.open_window(WindowOptions::default(), |window, cx| {
        cx.new(|cx| PhoneApp::new(prefs::path(data_dir), window, cx))
    });
    let window = match opened {
        Ok(window) => window,
        Err(error) => {
            log::error!("Could not open the window: {error:#}");
            return;
        }
    };
    cx.spawn(async move |cx| {
        while let Ok(url) = urls.recv().await {
            window
                .update(cx, |app, window, cx| app.open_url(&url, window, cx))
                .ok();
        }
    })
    .detach();
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: gpui_android::AndroidApp) {
    use gpui::Application;
    use std::rc::Rc;

    gpui_android::init_logging("pi", log::LevelFilter::Info);
    let data_dir = app.internal_data_path();
    let platform = match gpui_android::AndroidPlatform::new(app) {
        Ok(platform) => platform,
        Err(error) => {
            log::error!("Could not start GPUI: {error:#}");
            return;
        }
    };
    let application = Application::with_platform(Rc::new(platform)).with_assets(Assets);
    let (sender, urls) = async_channel::unbounded();
    application.on_open_urls(move |opened| {
        for url in opened {
            sender.try_send(url).ok();
        }
    });
    application.run(move |cx: &mut App| {
        init(cx);
        open(cx, data_dir, urls);
    });
    // The activity is gone. Android may call `android_main` again in this
    // process for the next launch; start that from a fresh process instead.
    std::process::exit(0);
}
