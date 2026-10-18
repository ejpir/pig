//! Interactive account shell carried by the helper's framed terminal endpoint.

use crate::{
    app::{PhoneApp, Route},
    model::SessionId,
    remote, ssh,
    theme::theme,
    ui,
};
use gpui::{
    AnyElement, Context, Entity, Focusable, SharedString, Task, Window, div, prelude::*, px,
};
use pi_core::terminal_transport::{
    ClientMessage, Dimensions, MAX_INPUT_BYTES, ServerDecoder, ServerMessage,
};
use pi_terminal::{ExternalPty, ExternalTerminalBackend, PathStyle, TerminalBounds, TerminalView};
use std::{
    borrow::Cow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const OUTBOUND_FRAMES: usize = 64;

struct RemotePty {
    outbound: async_channel::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
    dimensions: Mutex<Dimensions>,
}

impl RemotePty {
    fn new() -> (Arc<Self>, async_channel::Receiver<Vec<u8>>) {
        let (outbound, receiver) = async_channel::bounded(OUTBOUND_FRAMES);
        (
            Arc::new(Self {
                outbound,
                closed: Arc::new(AtomicBool::new(false)),
                dimensions: Mutex::new(Dimensions {
                    columns: 80,
                    rows: 24,
                }),
            }),
            receiver,
        )
    }

    fn dimensions(&self) -> Dimensions {
        self.dimensions
            .lock()
            .map(|value| *value)
            .unwrap_or(Dimensions {
                columns: 80,
                rows: 24,
            })
    }

    fn enqueue(&self, message: ClientMessage) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let Ok(frame) = message.encode() else {
            self.shutdown();
            return;
        };
        if self.outbound.try_send(frame).is_err() {
            // Never let a paste or stalled transport grow an unbounded UI-side
            // queue. Closing is safer than silently dropping terminal input.
            self.shutdown();
        }
    }
}

impl ExternalPty for RemotePty {
    fn write(&self, bytes: Cow<'static, [u8]>) {
        for chunk in bytes.chunks(MAX_INPUT_BYTES) {
            self.enqueue(ClientMessage::Input(chunk.to_vec()));
        }
    }

    fn resize(&self, bounds: TerminalBounds) {
        let columns = u16::try_from(bounds.num_columns().clamp(1, 1_000)).unwrap_or(1_000);
        let rows = u16::try_from(bounds.num_lines().clamp(1, 1_000)).unwrap_or(1_000);
        if let Ok(dimensions) = Dimensions::new(columns, rows) {
            if let Ok(mut current) = self.dimensions.lock() {
                *current = dimensions;
            }
            self.enqueue(ClientMessage::Resize(dimensions));
        }
    }

    fn shutdown(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.outbound.close();
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum TerminalStatus {
    Connecting,
    Live,
    Exited(Option<u32>),
    Failed(SharedString),
}

impl TerminalStatus {
    fn label(&self) -> SharedString {
        match self {
            Self::Connecting => "Connecting…".into(),
            Self::Live => "Live".into(),
            Self::Exited(Some(code)) => format!("Exited · {code}").into(),
            Self::Exited(None) => "Exited".into(),
            Self::Failed(error) => error.clone(),
        }
    }
}

pub(crate) struct TerminalScreenState {
    pub(crate) session: SessionId,
    pub(crate) view: Entity<TerminalView>,
    pub(crate) status: TerminalStatus,
    pty: Arc<RemotePty>,
    pub(crate) _connection: Task<()>,
}

fn path_style(platform: Option<&str>) -> PathStyle {
    if platform == Some("windows-amd64") {
        PathStyle::Windows
    } else {
        PathStyle::Unix
    }
}

impl PhoneApp {
    pub(crate) fn terminal_available(&self, id: SessionId) -> bool {
        self.terminal_views
            .values()
            .any(|state| state.session == id)
            || self.store.as_ref().is_some_and(|store| {
                store.live.as_ref().is_some_and(|live| {
                    !live.connection.is_closed()
                        && live.helper.can_open_terminal()
                        && live.session_cwd(id).is_some()
                })
            })
    }

    pub(crate) fn open_terminal(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((token, active)) = self.terminal_views.iter().find_map(|(token, state)| {
            (state.session == id).then_some((
                *token,
                matches!(
                    &state.status,
                    TerminalStatus::Connecting | TerminalStatus::Live
                ),
            ))
        }) {
            if active {
                self.close_sheet(cx);
                self.push(Route::Terminal(id, token), window, cx);
                return;
            }
            // A finished transport cannot be reconnected or safely replayed.
            // Dispose it and let this explicit open create a fresh shell.
            self.close_terminal_view(token, cx);
        }

        let Some((connection, helper, cwd)) = self.store.as_ref().and_then(|store| {
            let live = store.live.as_ref()?;
            Some((
                live.connection.clone(),
                live.helper.clone(),
                live.session_cwd(id)?.to_owned(),
            ))
        }) else {
            self.notify_user("A live session folder is needed to open a terminal.", cx);
            return;
        };
        if let Err(error) = helper.require_terminal() {
            self.notify_user(error.to_string(), cx);
            return;
        }

        self.close_sheet(cx);
        let token = self.next_terminal_view;
        self.next_terminal_view = self.next_terminal_view.wrapping_add(1).max(1);
        let (pty, outbound) = RemotePty::new();
        let terminal = pi_terminal::spawn_external(
            ExternalTerminalBackend {
                pty: pty.clone(),
                path_style: path_style(helper.platform.as_deref()),
            },
            window,
            cx,
        );
        let view = cx.new(|cx| TerminalView::new(terminal.clone(), window, cx));
        let task_pty = pty.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = async {
                let pipe = remote::open_terminal(&connection, &helper).await?;
                pipe.input
                    .send(
                        ClientMessage::Open {
                            // Layout normally runs while the SSH exec channel is
                            // opening. Start the shell at that real viewport size
                            // so its greeting is not painted at 80 columns and
                            // then clipped by an immediate narrower resize.
                            dimensions: task_pty.dimensions(),
                            cwd,
                        }
                        .encode()?,
                    )
                    .await?;

                let input = pipe.input.clone();
                let closed = task_pty.closed.clone();
                ssh::spawn(async move {
                    while let Ok(frame) = outbound.recv().await {
                        if closed.load(Ordering::Acquire) || input.send(frame).await.is_err() {
                            break;
                        }
                    }
                    if closed.load(Ordering::Acquire)
                        && let Ok(close) = ClientMessage::Close.encode()
                    {
                        let _ = input.send(close).await;
                    }
                    input.close();
                });

                let mut decoder = ServerDecoder::new();
                let mut final_message = false;
                'transport: while let Ok(bytes) = pipe.output.recv().await {
                    for message in decoder.push(&bytes)? {
                        match message {
                            ServerMessage::Opened => {
                                this.update(cx, |this, cx| {
                                    if let Some(state) = this.terminal_views.get_mut(&token) {
                                        state.status = TerminalStatus::Live;
                                        cx.notify();
                                    }
                                })?;
                            }
                            ServerMessage::Output(bytes) => {
                                terminal.update(cx, |terminal, cx| {
                                    terminal.write_pty_output(&bytes, cx)
                                });
                            }
                            ServerMessage::Exit { code } => {
                                final_message = true;
                                this.update(cx, |this, cx| {
                                    if let Some(state) = this.terminal_views.get_mut(&token) {
                                        state.status = TerminalStatus::Exited(code);
                                        cx.notify();
                                    }
                                })?;
                                break 'transport;
                            }
                            ServerMessage::Error(error) => {
                                final_message = true;
                                this.update(cx, |this, cx| {
                                    if let Some(state) = this.terminal_views.get_mut(&token) {
                                        state.status = TerminalStatus::Failed(error.into());
                                        cx.notify();
                                    }
                                })?;
                                break 'transport;
                            }
                        }
                    }
                }
                if !final_message {
                    decoder.finish()?;
                    let reason = pipe
                        .ended
                        .recv()
                        .await
                        .unwrap_or_else(|_| "The terminal connection closed".into());
                    anyhow::bail!(reason);
                }
                anyhow::Ok(())
            }
            .await;

            task_pty.shutdown();
            terminal.update(cx, |terminal, _| terminal.release_pty_resources());
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    if let Some(state) = this.terminal_views.get_mut(&token)
                        && !matches!(
                            state.status,
                            TerminalStatus::Exited(_) | TerminalStatus::Failed(_)
                        )
                    {
                        state.status = TerminalStatus::Failed(error.to_string().into());
                        cx.notify();
                    }
                })
                .ok();
            }
        });
        self.terminal_views.insert(
            token,
            TerminalScreenState {
                session: id,
                view,
                status: TerminalStatus::Connecting,
                pty,
                _connection: task,
            },
        );
        self.push(Route::Terminal(id, token), window, cx);
    }

    pub(crate) fn close_terminal_view(&mut self, token: u64, cx: &mut Context<Self>) {
        if let Some(state) = self.terminal_views.remove(&token) {
            state.view.update(cx, |view, cx| view.disconnect(cx));
            state.pty.shutdown();
        }
    }

    fn terminal_key(&mut self, token: u64, key: &'static str, cx: &mut Context<Self>) {
        if let Some(state) = self.terminal_views.get(&token) {
            state.view.update(cx, |view, cx| view.send_key(key, cx));
        }
    }

    fn terminal_copy(&mut self, token: u64, cx: &mut Context<Self>) {
        if let Some(state) = self.terminal_views.get(&token) {
            state.view.update(cx, |view, cx| view.copy_selection(cx));
        }
    }

    fn terminal_paste(&mut self, token: u64, cx: &mut Context<Self>) {
        if let Some(state) = self.terminal_views.get(&token) {
            state.view.update(cx, |view, cx| view.paste_clipboard(cx));
        }
    }

    fn terminal_keyboard(&mut self, token: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = self.terminal_views.get(&token) {
            let focus = state.view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.request_virtual_keyboard();
        }
    }

    fn terminal_close(&mut self, token: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.close_terminal_view(token, cx);
        self.back(window, cx);
    }

    pub(crate) fn terminal_screen(
        &mut self,
        _id: SessionId,
        token: u64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let back = ui::tap("terminal-back", "back", &colors).on_click(cx.listener(
            |this, _, window, cx| {
                this.back(window, cx);
            },
        ));
        let Some(state) = self.terminal_views.get(&token) else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(ui::appbar(back, "Terminal", Some("Closed".into()), &colors))
                .child(div().flex_1())
                .into_any_element();
        };
        let status = state.status.label();
        let view = state.view.clone();
        let key = |id: &'static str, label: &'static str, key: &'static str| {
            div()
                .id((id, token as usize))
                .h(px(40.))
                .min_w(px(44.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .bg(colors.raised)
                .text_size(px(13.))
                .text_color(colors.plain)
                .active(|style| style.bg(colors.selected))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.terminal_key(token, key, cx)))
        };
        let action = |id: &'static str,
                      label: &'static str,
                      action: fn(&mut Self, u64, &mut Context<Self>)| {
            div()
                .id((id, token as usize))
                .h(px(40.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .bg(colors.raised)
                .text_size(px(13.))
                .text_color(colors.plain)
                .active(|style| style.bg(colors.selected))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| action(this, token, cx)))
        };
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .child(ui::appbar(back, "Terminal", Some(status), &colors))
            .child(div().flex_1().min_h_0().p(px(8.)).child(view))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .px(px(8.))
                    .pt(px(6.))
                    .pb(px(10.))
                    .border_t_1()
                    .border_color(colors.line)
                    .children([
                        key("terminal-esc", "Esc", "escape"),
                        key("terminal-tab", "Tab", "tab"),
                        key("terminal-ctrl-c", "Ctrl-C", "ctrl-c"),
                        key("terminal-ctrl-d", "Ctrl-D", "ctrl-d"),
                        key("terminal-ctrl-z", "Ctrl-Z", "ctrl-z"),
                        key("terminal-left", "←", "left"),
                        key("terminal-up", "↑", "up"),
                        key("terminal-down", "↓", "down"),
                        key("terminal-right", "→", "right"),
                        action("terminal-copy", "Copy", Self::terminal_copy),
                        action("terminal-paste", "Paste", Self::terminal_paste),
                    ])
                    .child(
                        div()
                            .id(("terminal-keyboard", token as usize))
                            .h(px(40.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .bg(colors.raised)
                            .text_size(px(13.))
                            .text_color(colors.plain)
                            .active(|style| style.bg(colors.selected))
                            .child("Keyboard")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.terminal_keyboard(token, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(("terminal-close", token as usize))
                            .h(px(40.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .bg(colors.raised)
                            .text_size(px(13.))
                            .text_color(colors.plain)
                            .active(|style| style.bg(colors.selected))
                            .child("Close")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.terminal_close(token, window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn binary_terminal_input_is_split_into_bounded_frames() {
        let (pty, receiver) = RemotePty::new();
        let mut bytes = vec![0xff; MAX_INPUT_BYTES + 1];
        bytes[0] = 0;
        pty.write(Cow::Owned(bytes.clone()));
        let mut decoder = pi_core::terminal_transport::ClientDecoder::new();
        let first = decoder.push(&receiver.try_recv().unwrap()).unwrap();
        let second = decoder.push(&receiver.try_recv().unwrap()).unwrap();
        assert_eq!(
            first,
            vec![ClientMessage::Input(bytes[..MAX_INPUT_BYTES].to_vec())]
        );
        assert_eq!(
            second,
            vec![ClientMessage::Input(bytes[MAX_INPUT_BYTES..].to_vec())]
        );
    }

    #[test]
    fn outbound_overflow_closes_instead_of_dropping_or_growing() {
        let (pty, _receiver) = RemotePty::new();
        for _ in 0..=OUTBOUND_FRAMES {
            pty.enqueue(ClientMessage::Input(vec![1]));
        }
        assert!(pty.closed.load(Ordering::Acquire));
        assert!(pty.outbound.is_closed());
    }

    #[gpui::test]
    fn back_hides_a_terminal_and_open_restores_the_same_view(cx: &mut TestAppContext) {
        cx.update(|cx| {
            settings::init(cx);
            crate::init(cx);
        });
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        let id = SessionId(17);
        let token = 23;
        let receiver = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let (pty, receiver) = RemotePty::new();
                let terminal = pi_terminal::spawn_external(
                    ExternalTerminalBackend {
                        pty: pty.clone(),
                        path_style: PathStyle::Unix,
                    },
                    window,
                    cx,
                );
                let view = cx.new(|cx| TerminalView::new(terminal, window, cx));
                app.terminal_views.insert(
                    token,
                    TerminalScreenState {
                        session: id,
                        view,
                        status: TerminalStatus::Live,
                        pty,
                        _connection: Task::ready(()),
                    },
                );
                app.routes = vec![Route::Thread(id), Route::Terminal(id, token)];
                receiver
            })
        });

        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.back(window, cx));
                assert_eq!(app.route(), Route::Thread(id));
                assert!(app.terminal_views.contains_key(&token));

                app.open_terminal(id, window, cx);
                assert_eq!(app.route(), Route::Terminal(id, token));
                assert_eq!(app.terminal_views.len(), 1);
                assert!(
                    !app.terminal_views[&token]
                        .pty
                        .closed
                        .load(Ordering::Acquire)
                );

                app.terminal_views.get_mut(&token).unwrap().status =
                    TerminalStatus::Exited(Some(0));
                assert!(app.back(window, cx));
                app.open_terminal(id, window, cx);
                assert!(
                    app.terminal_views.is_empty(),
                    "a dead view is discarded before a fresh connection is attempted"
                );
            })
        });

        drop(receiver);
    }
}
