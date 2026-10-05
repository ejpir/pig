//! The phone app in a phone-sized desktop window, to work on the screens
//! without a phone:
//!
//!     cargo run -p pi_android --example preview -- [screen]
//!
//! where the screen is one of `PhoneApp::preview`'s names, like `waiting` or
//! `review`. Without one, the app starts as it does on the phone.

#[cfg(not(target_os = "android"))]
fn main() {
    use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
    use pi_android::PhoneApp;

    let screen = std::env::args().nth(1);
    if let Some(screen) = &screen
        && !pi_android::SCREENS.contains(&screen.as_str())
    {
        eprintln!(
            "No screen named {screen}. The screens: {}",
            pi_android::SCREENS.join(", ")
        );
        std::process::exit(2);
    }
    gpui_platform::application()
        .with_assets(pi_android::Assets)
        .run(move |cx: &mut App| {
            pi_android::init(cx);
            let bounds = Bounds::centered(None, size(px(412.), px(915.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..WindowOptions::default()
            };
            let opened = cx.open_window(options, |window, cx| {
                cx.new(|cx| {
                    let mut app = PhoneApp::new(None, window, cx);
                    if let Some(screen) = &screen {
                        app.preview(screen, window, cx);
                    }
                    app
                })
            });
            if let Err(error) = opened {
                eprintln!("Could not open the window: {error:#}");
                cx.quit();
            }
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            cx.activate(true);
        });
}

#[cfg(target_os = "android")]
fn main() {}
