#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod assets;
mod bundled_backend;
mod components;
mod desktop;
mod input;
mod markdown_view;
mod prefs;
mod presentation;
mod theme;

use anyhow::{Context, Result, bail};
use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use std::path::PathBuf;

struct Options {
    demo: bool,
    light: bool,
    cwd: PathBuf,
    /// `--project` was given: open it even when sessions are reopened.
    project: bool,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            demo: false,
            light: false,
            cwd: std::env::current_dir()?,
            project: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--demo" => options.demo = true,
                "--light" => options.light = true,
                "--project" => {
                    options.cwd =
                        PathBuf::from(args.next().context("--project needs a directory")?)
                            .canonicalize()?;
                    options.project = true;
                }
                "--help" | "-h" => {
                    println!(
                        "pi-desktop [--demo] [--light] [--project DIR]\n\nNormal mode reopens the sessions open at the last quit, or starts a session in DIR.\nSessions run the backend built into release builds, else pi --mode rpc from PATH.\nSettings → General chooses another backend; environment variables win over it.\nSet PI_DESKTOP_RPC_ENTRY to the absolute path to ../pi/packages/coding-agent/dist/cli.js,\nor PI_DESKTOP_PI to a pi executable. --demo is offline and never starts pi."
                    );
                    std::process::exit(0);
                }
                _ => bail!("Unknown option: {arg}"),
            }
        }
        if !options.cwd.is_dir() {
            bail!("Project must be a directory");
        }
        Ok(options)
    }
}

fn main() -> Result<()> {
    // Zed's Project shell probe invokes the embedding executable, not `zed`.
    // Handle this before GPUI, logging, option parsing or any Pi process starts.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--printenv")
    {
        let environment: std::collections::BTreeMap<_, _> = std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        serde_json::to_writer(std::io::stdout().lock(), &environment)?;
        return Ok(());
    }
    env_logger::init();
    let options = Options::parse()?;
    // Blocking: the first launch of a build unpacks it (well under a second).
    let bundled_backend = bundled_backend::unpack();
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            if let Err(error) = assets::load_fonts(cx) {
                eprintln!("Could not load fonts: {error}");
                cx.quit();
                return;
            }
            let mut prefs = prefs::Prefs::load();
            prefs.force_light = options.light;
            prefs.bundled_backend = bundled_backend.clone();
            cx.set_global(prefs);
            cx.set_global(theme::Theme::new(prefs::light(cx)));
            let (sessions, active) = startup_sessions(&options, cx);
            desktop::init(cx);
            let bounds = Bounds::centered(None, size(px(1344.), px(740.)), cx);
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(960.), px(600.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("pi desktop".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: Some(gpui::point(px(16.), px(20.))),
                    }),
                    app_id: Some("dev.pi.desktop".into()),
                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    icon: window_icon(),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| desktop::Desktop::new(sessions, active, options.demo, window, cx))
                },
            );
            if let Err(error) = result {
                eprintln!("Could not open desktop: {error}");
                cx.quit();
                return;
            }
            #[cfg(target_os = "macos")]
            set_dock_icon();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}

/// The sessions to open and which to show: those open at the last quit when
/// `general.reopenSessions` is on, and `--project` or the working folder. A
/// session whose file is gone reopens as a new session in its folder.
fn startup_sessions(
    options: &Options,
    cx: &App,
) -> (
    Vec<(PathBuf, Option<pi_core::protocol::SavedSession>)>,
    usize,
) {
    let mut sessions = Vec::new();
    let mut active = 0;
    if !options.demo && prefs::flag(cx, "general.reopenSessions", None) {
        let (open, was_active) = cx.global::<prefs::Prefs>().open_sessions();
        for (index, session) in open.into_iter().enumerate() {
            if !session.cwd.is_dir() {
                continue;
            }
            if index == was_active {
                active = sessions.len();
            }
            let saved = session
                .saved
                .filter(|saved| std::path::Path::new(&saved.path).is_file());
            sessions.push((session.cwd, saved));
        }
    }
    if options.project || sessions.is_empty() {
        active = sessions.len();
        sessions.push((options.cwd.clone(), None));
    }
    (sessions, active)
}

/// The bundle's AppIcon.icns covers packaged launches; an unbundled `cargo run` would
/// otherwise show a generic executable in the Dock.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::{AnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    const PNG: &[u8] = include_bytes!("../../../assets/app-icon/app-icon-512.png");
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(PNG);
    let Some(icon) = NSImage::initWithData(NSImage::alloc(), &data) else {
        log::error!("Could not decode the Dock icon");
        return;
    };
    // SAFETY: AppKit requires the main thread, which `main_thread` proves.
    unsafe { NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&icon)) };
}

/// X11 window icon. Wayland compositors and launchers use the `.desktop` entry
/// matching the window's app id instead; see packaging/linux.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn window_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    const PNG: &[u8] = include_bytes!("../../../assets/app-icon/app-icon-128.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(icon) => Some(std::sync::Arc::new(icon.into_rgba8())),
        Err(error) => {
            log::error!("Could not decode the window icon: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn embedded_window_icon_decodes() {
        let icon = super::window_icon().expect("window icon");
        assert_eq!(icon.dimensions(), (128, 128));
    }
}
