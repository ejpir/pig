//! A real GPUI visual prototype. App content is loaded from JSON + Rhai at runtime.
//! The surrounding conversation is illustrative, not connected to an agent.
#![allow(dead_code)]

mod runtime;

#[path = "../../../../crates/pi_desktop/src/theme.rs"]
mod theme;

use gpui::{prelude::*, *};
use runtime::{Node, Runtime};
use std::{borrow::Cow, path::PathBuf};
use theme::{MONO, SANS, Theme};

fn row() -> Div {
    div().flex().flex_row().items_center()
}
fn column() -> Div {
    div().flex().flex_col()
}
fn glyph(name: &'static str, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(15.))
        .text_color(color)
}

struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        // The same bundled icons as Pi Desktop; no disk access by scripts.
        let bytes: Option<&'static [u8]> = match path {
            "icons/chat.svg" => Some(include_bytes!("../../../../assets/icons/chat.svg")),
            "icons/plus.svg" => Some(include_bytes!("../../../../assets/icons/plus.svg")),
            "icons/chevron_down.svg" => {
                Some(include_bytes!("../../../../assets/icons/chevron_down.svg"))
            }
            "icons/chevron_right.svg" => {
                Some(include_bytes!("../../../../assets/icons/chevron_right.svg"))
            }
            "icons/folder.svg" => Some(include_bytes!("../../../../assets/icons/folder.svg")),
            "icons/sparkle.svg" => Some(include_bytes!("../../../../assets/icons/sparkle.svg")),
            "icons/send.svg" => Some(include_bytes!("../../../../assets/icons/send.svg")),
            "icons/check.svg" => Some(include_bytes!("../../../../assets/icons/check.svg")),
            "icons/attach.svg" => Some(include_bytes!("../../../../assets/icons/attach.svg")),
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }
    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

struct Mock {
    app: Runtime,
    folder: PathBuf,
    focus: FocusHandle,
    error: Option<String>,
}

impl Mock {
    fn dispatch(&mut self, key: &str, cx: &mut Context<Self>) {
        self.error = self.app.dispatch(key).err().map(|err| err.to_string());
        cx.notify();
    }

    fn node(&self, node: &Node, path: String, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::theme(cx);
        match node {
            Node::Column {
                gap,
                width,
                children,
            } => {
                let mut view = column().gap(px(*gap)).min_w_0();
                if let Some(width) = width {
                    view = view.w(px(*width)).flex_shrink_0();
                } else {
                    view = view.flex_1();
                }
                view.children(
                    children
                        .iter()
                        .enumerate()
                        .map(|(i, child)| self.node(child, format!("{path}-{i}"), cx)),
                )
                .into_any_element()
            }
            Node::Row { gap, children } => row()
                .items_start()
                .w_full()
                .gap(px(*gap))
                .children(
                    children
                        .iter()
                        .enumerate()
                        .map(|(i, child)| self.node(child, format!("{path}-{i}"), cx)),
                )
                .into_any_element(),
            Node::Text { text, bind, style } => {
                let content = bind
                    .as_ref()
                    .map(|key| self.app.text(key))
                    .unwrap_or_else(|| text.clone().unwrap_or_default());
                let mut view = div().w_full().child(content);
                view = match style.as_str() {
                    "display" => view
                        .font_family(MONO)
                        .text_size(px(44.))
                        .line_height(px(62.))
                        .text_right()
                        .text_color(t.text),
                    "expression" => view
                        .font_family(MONO)
                        .text_size(px(13.))
                        .line_height(px(22.))
                        .text_right()
                        .text_color(t.muted),
                    "heading" => view
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.secondary),
                    "history" => view
                        .font_family(MONO)
                        .text_size(px(12.))
                        .line_height(px(23.))
                        .text_color(t.secondary),
                    _ => view
                        .text_size(px(12.))
                        .line_height(px(19.))
                        .text_color(t.muted),
                };
                view.into_any_element()
            }
            // This standalone legacy calculator mock is not the phone renderer.
            Node::Canvas { .. } | Node::Slider { .. } => div()
                .child("Graphics controls require the Android v2 renderer.")
                .into_any_element(),
            Node::Button {
                label,
                action,
                tone,
            } => {
                let (bg, fg) = match tone.as_str() {
                    "accent" => (t.accent, t.on_accent),
                    "operator" => (t.tint(t.accent), t.accent),
                    "utility" => (t.selected, t.secondary),
                    _ => (t.work_code, t.text),
                };
                let action = action.clone();
                let is_accent = tone == "accent";
                row()
                    .id(SharedString::from(path))
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .flex_1()
                    .min_w_0()
                    .h(px(48.))
                    .justify_center()
                    .rounded(px(6.))
                    .bg(bg)
                    .border_1()
                    .border_color(t.line.opacity(0.4))
                    .font_family(SANS)
                    .text_size(px(19.))
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(move |s| s.bg(if is_accent { t.accent_hover } else { t.hover }))
                    .active(move |s| s.bg(t.focus))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        this.dispatch(&action, cx);
                    }))
                    .child(label.clone())
                    .into_any_element()
            }
        }
    }

    fn calculator(&self, cx: &mut Context<Self>) -> Div {
        let t = theme::theme(cx);
        let app_view = self.node(&self.app.definition.view, "app".into(), cx);
        column()
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(t.line)
            .bg(t.panel)
            .child(
                row()
                    .h(px(48.))
                    .px(px(18.))
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(glyph("sparkle", t.accent))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.app.definition.title.clone()),
                    )
                    .child(div().flex_1())
                    .child(
                        row()
                            .gap(px(5.))
                            .text_size(px(11.))
                            .text_color(t.green)
                            .child(div().size(px(5.)).rounded_full().bg(t.green))
                            .child("Native app"),
                    ),
            )
            .child(div().p(px(20.)).child(app_view))
            .child(
                row()
                    .h(px(36.))
                    .px(px(18.))
                    .border_t_1()
                    .border_color(t.line)
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(t.muted)
                    .child("Keyboard supported")
                    .child(div().flex_1())
                    .child(
                        row()
                            .id("reload-app")
                            .role(Role::Button)
                            .aria_label("Reload app definition and script")
                            .cursor_pointer()
                            .gap(px(4.))
                            .text_color(t.accent)
                            .on_click(cx.listener(|this, _, _, cx| {
                                match Runtime::load(&this.folder) {
                                    Ok(app) => {
                                        this.app = app;
                                        this.error = None;
                                    }
                                    Err(err) => this.error = Some(err.to_string()),
                                }
                                cx.notify();
                            }))
                            .child("Reload script")
                            .child(glyph("chevron_right", t.accent)),
                    ),
            )
    }

    fn sidebar(&self, t: Theme) -> Div {
        column()
            .w(px(206.))
            .h_full()
            .flex_shrink_0()
            .bg(t.panel)
            .border_r_1()
            .border_color(t.line)
            .child(
                row()
                    .h(px(54.))
                    .px(px(18.))
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.accent)
                            .child("π"),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Pi"),
                    )
                    .child(div().flex_1())
                    .child(glyph("plus", t.muted)),
            )
            .child(
                div()
                    .px(px(18.))
                    .pt(px(20.))
                    .pb(px(12.))
                    .text_size(px(11.))
                    .text_color(t.muted)
                    .child("Projects"),
            )
            .child(
                row()
                    .h(px(34.))
                    .px(px(16.))
                    .gap(px(8.))
                    .child(glyph("chevron_down", t.muted))
                    .child(glyph("folder", t.accent))
                    .child(div().text_size(px(13.)).child("Little tools")),
            )
            .child(
                row()
                    .mx(px(8.))
                    .px(px(12.))
                    .h(px(36.))
                    .gap(px(8.))
                    .bg(t.selected)
                    .rounded(px(5.))
                    .child(glyph("chat", t.accent))
                    .child(div().text_size(px(13.)).child("A calculator, please")),
            )
            .child(
                row()
                    .mx(px(8.))
                    .px(px(12.))
                    .h(px(36.))
                    .gap(px(8.))
                    .text_color(t.muted)
                    .child(glyph("chat", t.muted))
                    .child(div().text_size(px(13.)).child("Weekend groceries")),
            )
            .child(div().flex_1())
            .child(
                column()
                    .p(px(18.))
                    .gap(px(6.))
                    .border_t_1()
                    .border_color(t.line)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.secondary)
                            .child("Made for the conversation"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.muted)
                            .child("Native GPUI · scripted content"),
                    ),
            )
    }
}

impl Render for Mock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx);
        let card = self.calculator(cx);
        let error = self.error.clone();
        row()
            .id("mock")
            .track_focus(&self.focus)
            .size_full()
            .items_start()
            .font_family(SANS)
            .text_size(px(14.))
            .text_color(t.text)
            .bg(t.canvas)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let stroke = &event.keystroke;
                if stroke.modifiers.control || stroke.modifiers.platform || stroke.modifiers.alt {
                    return;
                }
                let key = match stroke.key.as_str() {
                    "enter" => "=",
                    "escape" => "AC",
                    "backspace" => "backspace",
                    _ => stroke.key_char.as_deref().unwrap_or(&stroke.key),
                };
                if key.len() == 1 && "0123456789.+-*/%=".contains(key)
                    || matches!(key, "AC" | "backspace")
                {
                    this.dispatch(key, cx);
                    cx.stop_propagation();
                }
            }))
            .child(self.sidebar(t))
            .child(
                column()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        row()
                            .h(px(54.))
                            .px(px(24.))
                            .gap(px(9.))
                            .bg(t.bar)
                            .border_b_1()
                            .border_color(t.line)
                            .child(glyph("chat", t.muted))
                            .child("A calculator, please")
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(t.muted)
                                    .child("Local · visual prototype"),
                            )
                            .child(
                                row()
                                    .id("theme-toggle")
                                    .role(Role::Button)
                                    .aria_label("Toggle color theme")
                                    .ml(px(16.))
                                    .px(px(8.))
                                    .py(px(4.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .hover(move |s| s.bg(t.hover))
                                    .child(if t.light { "Evening" } else { "Moonstone" })
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.set_global(Theme::new(!theme::theme(cx).light));
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        row()
                            .h(px(38.))
                            .px(px(24.))
                            .gap(px(24.))
                            .border_b_1()
                            .border_color(t.line)
                            .child(
                                div()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_b_2()
                                    .border_color(t.accent)
                                    .child("Thread"),
                            )
                            .child(div().text_color(t.muted).child("Changes")),
                    )
                    .child(
                        column()
                            .id("thread")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .items_center()
                            .child(
                                column()
                                    .w_full()
                                    .max_w(px(624.))
                                    .px(px(24.))
                                    .pt(px(26.))
                                    .pb(px(20.))
                                    .gap(px(16.))
                                    .child(
                                        row()
                                            .gap(px(10.))
                                            .text_size(px(12.))
                                            .text_color(t.muted)
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(t.secondary)
                                                    .child("You"),
                                            )
                                            .child("Just now"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(16.))
                                            .child("Build me a calculator I can use right here."),
                                    )
                                    .child(
                                        row()
                                            .pt(px(8.))
                                            .gap(px(8.))
                                            .text_size(px(12.))
                                            .child(glyph("sparkle", t.accent))
                                            .child(
                                                div().font_weight(FontWeight::SEMIBOLD).child("Pi"),
                                            )
                                            .child(
                                                div()
                                                    .text_color(t.muted)
                                                    .child("Made a little tool for you"),
                                            ),
                                    )
                                    .child(div().text_color(t.secondary).child(
                                        "Here you go. Click the keys, or use your keyboard.",
                                    ))
                                    .child(card)
                                    .when_some(error, |view, error| {
                                        view.child(div().text_color(t.coral).child(error))
                                    })
                                    .child(
                                        row()
                                            .gap(px(6.))
                                            .text_size(px(12.))
                                            .text_color(t.muted)
                                            .child(glyph("check", t.green))
                                            .child("Ready to use. No browser, no new window."),
                                    ),
                            ),
                    )
                    .child(
                        column()
                            .w_full()
                            .items_center()
                            .px(px(24.))
                            .pb(px(16.))
                            .pt(px(8.))
                            .child(
                                column()
                                    .w_full()
                                    .max_w(px(576.))
                                    .gap(px(14.))
                                    .p(px(14.))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(t.line_strong)
                                    .bg(t.composer)
                                    .child(div().text_color(t.muted).child("Ask Pi to change it…"))
                                    .child(
                                        row()
                                            .gap(px(10.))
                                            .text_size(px(11.))
                                            .text_color(t.muted)
                                            .child(glyph("attach", t.muted))
                                            .child("Claude Opus")
                                            .child(glyph("chevron_down", t.muted))
                                            .child("·")
                                            .child("High")
                                            .child(div().flex_1())
                                            .child(div().child("Composer is illustrative"))
                                            .child(glyph("send", t.accent)),
                                    ),
                            ),
                    )
                    .child(
                        row()
                            .h(px(26.))
                            .px(px(16.))
                            .bg(t.status)
                            .border_t_1()
                            .border_color(t.line)
                            .text_size(px(11.))
                            .text_color(t.muted)
                            .gap(px(6.))
                            .child(div().size(px(5.)).rounded_full().bg(t.green))
                            .child("Script runs locally")
                            .child(div().flex_1())
                            .child("JSON layout → GPUI   ·   Rhai behavior"),
                    ),
            )
    }
}

fn main() -> anyhow::Result<()> {
    let mut folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../app");
    let mut light = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--light" => light = true,
            "--app" => {
                folder = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow::anyhow!("--app requires a directory"))?
            }
            _ => anyhow::bail!("Unknown option: {arg}"),
        }
    }
    let app = Runtime::load(&folder)?;
    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx| {
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!(
                        "../../../../assets/fonts/IBMPlexSans-Regular.ttf"
                    )),
                    Cow::Borrowed(include_bytes!(
                        "../../../../assets/fonts/IBMPlexSans-SemiBold.ttf"
                    )),
                    Cow::Borrowed(include_bytes!(
                        "../../../../assets/fonts/CommitMono-Regular.otf"
                    )),
                ])
                .expect("load bundled fonts");
            cx.set_global(Theme::new(light));
            let bounds = Bounds::centered(None, size(px(1120.), px(960.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(800.), px(700.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Pi · Scripted calculator".into()),
                        ..Default::default()
                    }),
                    app_id: Some("dev.pi.scripted-calculator-mock".into()),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let focus = cx.focus_handle();
                        window.focus(&focus, cx);
                        Mock {
                            app,
                            folder,
                            focus,
                            error: None,
                        }
                    })
                },
            )
            .expect("open native mock window");
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
