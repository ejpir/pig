//! Session transport diagnostics: a bounded, sanitized, local-only record list
//! on its own screen so Details stays compact while records continue arriving.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route},
    live::{
        TRANSPORT_DIAGNOSTIC_MAX_BYTES, TRANSPORT_DIAGNOSTIC_MAX_ENTRIES, TransportDiagnostic,
        TransportDirection,
    },
    model::SessionId,
    theme::{MONO, Theme, theme},
    ui::{self, icon},
};
use gpui::{
    AnyElement, Context, Div, FontWeight, SharedString, Stateful, Window, div, prelude::*, px,
};
use std::{collections::VecDeque, time::Duration};

pub(super) fn transport_diagnostics(
    app: &PhoneApp,
    id: SessionId,
) -> Option<&VecDeque<TransportDiagnostic>> {
    app.store
        .as_ref()
        .and_then(|store| store.live.as_ref())
        .and_then(|live| live.transport_diagnostics(id))
        .or_else(|| app.preview_transport_diagnostics.get(&id))
}

pub(super) fn transport_summary(entries: &VecDeque<TransportDiagnostic>) -> String {
    let Some(latest) = entries.back() else {
        return "No records".into();
    };
    let direction = match latest.direction {
        TransportDirection::Outbound => "OUT",
        TransportDirection::Inbound => "IN",
    };
    let record_type: String = latest.record_type.chars().take(16).collect();
    let ellipsis = if record_type.chars().count() < latest.record_type.chars().count() {
        "…"
    } else {
        ""
    };
    format!("{} · {direction} {record_type}{ellipsis}", entries.len())
}

fn transport_timestamp(milliseconds: u64) -> String {
    let since_epoch = Duration::from_millis(milliseconds);
    format!(
        "{}:{:02}.{:03}",
        crate::model::clock_at(since_epoch),
        since_epoch.as_secs() % 60,
        milliseconds % 1_000
    )
}

fn field(label: &'static str, value: impl Into<SharedString>, colors: &Theme) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(10.))
        .child(
            div()
                .w(px(68.))
                .flex_none()
                .text_size(px(11.5))
                .line_height(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .line_height(px(17.))
                .font_family(MONO)
                .text_color(colors.secondary)
                .child(value.into()),
        )
}

fn record(index: usize, entry: TransportDiagnostic, colors: &Theme) -> Stateful<Div> {
    let (direction, hue) = match entry.direction {
        TransportDirection::Outbound => ("OUT →", colors.accent),
        TransportDirection::Inbound => ("← IN", colors.green),
    };
    let command = entry.command.unwrap_or_else(|| "—".into());
    let correlation = if entry.has_id {
        "present · value omitted"
    } else {
        "—"
    };
    let preview = entry.preview;
    ui::card(colors)
        .id(("transport-diagnostic", index))
        .debug_selector(move || format!("transport-diagnostic-{index}"))
        .p(px(14.))
        .child(
            div()
                .debug_selector(move || format!("transport-diagnostic-header-{index}"))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(11.5))
                .line_height(px(16.))
                .font_family(MONO)
                .child(
                    div()
                        .flex_none()
                        .text_color(hue)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(direction),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(colors.muted)
                        .child(transport_timestamp(entry.at_millis)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(colors.muted)
                        .child(format!("{} B", entry.byte_size)),
                ),
        )
        .child(
            div()
                .debug_selector(move || format!("transport-diagnostic-fields-{index}"))
                .mt(px(10.))
                .pt(px(10.))
                .border_t_1()
                .border_color(colors.line)
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(field("Type", entry.record_type, colors))
                .child(field("Command", command, colors))
                .child(field("Correlation", correlation, colors)),
        )
        .when_some(preview, |record, preview| {
            record.child(
                div()
                    .debug_selector(move || format!("transport-diagnostic-preview-{index}"))
                    .mt(px(10.))
                    .p(px(10.))
                    .rounded(px(10.))
                    .bg(colors.raised)
                    .text_size(px(11.5))
                    .line_height(px(17.))
                    .font_family(MONO)
                    .text_color(colors.muted)
                    .child(preview),
            )
        })
}

impl PhoneApp {
    pub(crate) fn diagnostics_screen(
        &mut self,
        id: SessionId,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Diagnostics(id));
        let session = self.store.as_ref().and_then(|store| store.session(id));
        let subtitle = session
            .map(|session| SharedString::from(session.title.clone()))
            .unwrap_or_else(|| "Session unavailable".into());
        let missing_session = session.is_none();
        let entries = transport_diagnostics(self, id)
            .map(|entries| entries.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let count = entries.len();
        let count_label = match count {
            0 => "No transport records yet".to_owned(),
            1 => "1 transport record · newest first".to_owned(),
            _ => format!("{count} transport records · newest first"),
        };
        let back = ui::tap("back", "back", &colors).on_click(cx.listener(|this, _, window, cx| {
            this.back(window, cx);
        }));
        let records = entries
            .into_iter()
            .rev()
            .enumerate()
            .map(|(index, entry)| record(index, entry, &colors));
        div()
            .id("transport-diagnostics-screen")
            .debug_selector(|| "transport-diagnostics-screen".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(ui::appbar(
                back,
                "Transport diagnostics",
                Some(subtitle),
                &colors,
            ))
            .child(
                scroll_area(("transport-diagnostics", id.0 as usize), &scroll).child(
                    div()
                        .px(px(20.))
                        .pt(px(8.))
                        .pb(px(24.))
                        .when(missing_session, |body| {
                            body.child(
                                ui::card(&colors)
                                    .id("transport-diagnostics-missing-session")
                                    .debug_selector(|| {
                                        "transport-diagnostics-missing-session".into()
                                    })
                                    .p(px(14.))
                                    .border_color(colors.coral.opacity(0.55))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(10.))
                                            .child(icon("alert", 18., colors.coral))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child("This session is no longer available"),
                                            ),
                                    )
                                    .child(
                                        ui::hint(
                                            "Transport records may disappear when a session is removed or its in-memory watch ends.",
                                            &colors,
                                        )
                                        .mt(px(6.)),
                                    ),
                            )
                        })
                        .child(ui::label(count_label, &colors).mt(px(8.)))
                        .child(
                            ui::hint(
                                format!(
                                    "This local in-memory log keeps at most {} records / {} KiB. It shows bounded structure and counts only; message text, tool I/O, image bytes, errors, raw paths, headers, credentials and identifiers are never retained.",
                                    TRANSPORT_DIAGNOSTIC_MAX_ENTRIES,
                                    TRANSPORT_DIAGNOSTIC_MAX_BYTES / 1024
                                ),
                                &colors,
                            )
                            .mt(px(6.)),
                        )
                        .when(count == 0, |body| {
                            body.child(
                                ui::card(&colors)
                                    .id("transport-diagnostics-empty")
                                    .debug_selector(|| "transport-diagnostics-empty".into())
                                    .mt(px(18.))
                                    .p(px(18.))
                                    .flex()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(icon("code", 20., colors.muted))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child("Waiting for transport activity"),
                                            )
                                            .child(
                                                ui::hint(
                                                    "New sanitized records appear here while this session communicates with the computer.",
                                                    &colors,
                                                )
                                                .mt(px(2.)),
                                            ),
                                    ),
                            )
                        })
                        .when(count > 0, |body| {
                            body.child(
                                div()
                                    .mt(px(18.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(12.))
                                    .children(records),
                            )
                        }),
                ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::transport_summary;
    use crate::live::{TransportDiagnostic, TransportDirection};
    use std::collections::VecDeque;

    fn entry(direction: TransportDirection, record_type: &str) -> TransportDiagnostic {
        TransportDiagnostic {
            at_millis: 0,
            direction,
            record_type: record_type.into(),
            command: None,
            has_id: false,
            byte_size: 0,
            preview: None,
        }
    }

    #[test]
    fn details_summary_is_compact_and_describes_the_newest_record() {
        assert_eq!(transport_summary(&VecDeque::new()), "No records");
        assert_eq!(
            transport_summary(&VecDeque::from([entry(
                TransportDirection::Outbound,
                "prompt",
            )])),
            "1 · OUT prompt"
        );
        assert_eq!(
            transport_summary(&VecDeque::from([
                entry(TransportDirection::Outbound, "prompt"),
                entry(
                    TransportDirection::Inbound,
                    "a-very-long-transport-record-type",
                ),
            ])),
            "2 · IN a-very-long-tran…"
        );
    }
}
