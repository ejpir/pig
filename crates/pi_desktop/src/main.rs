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
    ssh: Option<String>,
    remote_backend: pi_core::ssh::RemoteBackend,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            demo: false,
            light: false,
            cwd: std::env::current_dir()?,
            project: false,
            ssh: None,
            remote_backend: pi_core::ssh::RemoteBackend::Pi,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--demo" => options.demo = true,
                "--light" => options.light = true,
                "--project" => {
                    options.cwd =
                        PathBuf::from(args.next().context("--project needs a directory")?);
                    options.project = true;
                }
                "--ssh" => options.ssh = Some(args.next().context("--ssh needs a host alias")?),
                "--remote-backend" => {
                    options.remote_backend = match args.next().as_deref() {
                        Some("pi") => pi_core::ssh::RemoteBackend::Pi,
                        Some("durable") => pi_core::ssh::RemoteBackend::Durable,
                        _ => bail!("--remote-backend needs pi or durable (experimental)"),
                    }
                }
                "--help" | "-h" => {
                    println!(
                        "pi-desktop [--demo] [--light] [--project DIR] [--ssh HOST] [--remote-backend pi|durable]\n\n--remote-backend durable selects the experimental durable engine for a NEW SSH session.\n--ssh opens DIR on an SSH host, installing a detached per-user Rust helper.\nSSH uses verified hosts and keys/agent. Remote agents survive disconnects.\n\nNormal mode reopens the sessions open at the last quit, or starts a session in DIR.\nSessions run the pi built into release builds, else pi from PATH, with Pi Desktop's extension.\nFor development, PI_DESKTOP_PI names a pi executable, or PI_DESKTOP_RPC_ENTRY a JavaScript\nentry such as ../pi/packages/coding-agent/dist/cli.js (run with PI_DESKTOP_NODE or node).\n--demo is offline and never starts pi."
                    );
                    std::process::exit(0);
                }
                _ => bail!("Unknown option: {arg}"),
            }
        }
        if let Some(host) = &options.ssh {
            anyhow::ensure!(
                !options.demo && options.project,
                "--ssh needs --project REMOTE_DIR and cannot be used with --demo"
            );
            pi_core::ssh::SshTarget::new(host.clone(), options.cwd.to_string_lossy().into_owned())?;
        } else {
            anyhow::ensure!(
                options.remote_backend == pi_core::ssh::RemoteBackend::Pi,
                "--remote-backend durable requires --ssh"
            );
            options.cwd = options.cwd.canonicalize()?;
            anyhow::ensure!(options.cwd.is_dir(), "Project must be a directory");
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
                        // The compact header draws smaller controls; keep AppKit's
                        // standard buttons off-canvas while preserving native window behavior.
                        traffic_light_position: Some(gpui::point(px(-100.), px(10.))),
                    }),
                    app_id: Some("dev.pi.desktop".into()),
                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    icon: window_icon(),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        desktop::Desktop::new_with_targets(
                            sessions,
                            active,
                            options.demo,
                            window,
                            cx,
                        )
                    })
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
fn startup_sessions(options: &Options, cx: &App) -> (Vec<prefs::OpenSession>, usize) {
    let mut sessions = Vec::new();
    let mut active = 0;
    if !options.demo && prefs::flag(cx, "general.reopenSessions", None) {
        let (open, was_active) = cx.global::<prefs::Prefs>().open_sessions();
        for (index, session) in open.into_iter().enumerate() {
            if session.remote.is_none() && !session.cwd.is_dir() {
                continue;
            }
            if index == was_active {
                active = sessions.len();
            }
            let saved = if session.remote.is_some() {
                None
            } else {
                session
                    .saved
                    .filter(|saved| std::path::Path::new(&saved.path).is_file())
            };
            sessions.push(prefs::OpenSession {
                cwd: session.cwd,
                saved,
                remote: session.remote,
            });
        }
    }
    if options.project || sessions.is_empty() {
        active = sessions.len();
        let remote = options.ssh.as_ref().map(|host| {
            let mut target = pi_core::ssh::SshTarget::new(
                host.clone(),
                options.cwd.to_string_lossy().into_owned(),
            )
            .expect("validated SSH options");
            target.backend = options.remote_backend;
            target
        });
        let cwd = remote
            .as_ref()
            .map(|target| target.identity())
            .unwrap_or_else(|| options.cwd.clone());
        sessions.push(prefs::OpenSession {
            cwd,
            saved: None,
            remote,
        });
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
