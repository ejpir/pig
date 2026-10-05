//! Reusable workbench surfaces; navigation and mutations stay with their owner.
use super::*;
use crate::theme::SANS;

pub const WORK_GUTTER: Pixels = px(24.);

pub fn work_button(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    theme: Theme,
) -> Stateful<Div> {
    button(id, text, theme)
        .h(px(30.))
        .px(px(14.))
        .bg(theme.composer)
        .border_color(theme.line)
        .text_size(px(13.))
}

/// A shared row for lightweight navigation popovers.
pub fn work_menu_item(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    glyph: &'static str,
    selected: bool,
    theme: Theme,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .role(Role::Button)
        .h(px(36.))
        .px(px(10.))
        .gap(px(10.))
        .rounded(px(4.))
        .text_size(px(13.))
        .cursor_pointer()
        .when(selected, |row| row.bg(theme.selected))
        .hover(move |row| row.bg(theme.hover))
        .child(icon(glyph, theme.muted))
        .child(text.into())
}

pub fn work_primary(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    enabled: bool,
    theme: Theme,
) -> Stateful<Div> {
    primary_button(id, text, enabled, theme)
        .h(px(30.))
        .px(px(16.))
        .text_size(px(13.))
}

pub fn work_tab(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    active: bool,
    theme: Theme,
) -> Stateful<Div> {
    let title: SharedString = title.into();
    h_flex()
        .id(id)
        .role(Role::Tab)
        .aria_label(title.clone())
        .aria_selected(active)
        .h_full()
        .gap(px(6.))
        .border_b_2()
        .border_color(if active {
            theme.secondary
        } else {
            transparent_black()
        })
        .text_size(px(13.))
        .text_color(if active { theme.text } else { theme.muted })
        .when(active, |v| v.font_weight(FontWeight::SEMIBOLD))
        .cursor_pointer()
        .child(title)
}

pub fn change_counts(added: usize, removed: usize, theme: Theme) -> Div {
    let counts = h_flex()
        .flex_shrink_0()
        .gap(px(8.))
        .font_family(MONO)
        .text_size(px(12.));
    if added == 0 && removed == 0 {
        counts.text_color(theme.muted).child("Modified")
    } else {
        counts
            .when(added > 0, |v| {
                v.child(div().text_color(theme.green).child(format!("+{added}")))
            })
            .when(removed > 0, |v| {
                v.child(div().text_color(theme.coral).child(format!("−{removed}")))
            })
    }
}

/// A compact row inside the Thread result card.
pub fn grouped_changed_file_row(
    id: impl Into<ElementId>,
    path: &str,
    added: usize,
    removed: usize,
    theme: Theme,
) -> Stateful<Div> {
    let path = std::path::Path::new(path);
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.display().to_string())
        .unwrap_or_else(|| "Project root".into());
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(format!("Review {}", path.display()))
        .w_full()
        .h(px(48.))
        .min_w_0()
        .px(px(16.))
        .gap(px(12.))
        .cursor_pointer()
        .hover(move |row| row.bg(theme.hover))
        .tooltip(ui::Tooltip::text(path.display().to_string()))
        .child(icon("file", theme.muted).size(px(14.)))
        .child(
            div()
                .w(px(200.))
                .flex_shrink_0()
                .truncate()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(name),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(MONO)
                .text_size(px(11.))
                .text_color(theme.muted)
                .child(parent),
        )
        .child(change_counts(added, removed, theme))
        .when(added + removed > 0, |row| {
            row.child(diff_blocks(added, removed, theme))
        })
        .child(icon("chevron_right", theme.muted).size(px(11.)))
}

/// A file is an object, not a log line. Full paths are available even when ellipsized.
pub fn changed_file_row(
    id: impl Into<ElementId>,
    path: &str,
    added: usize,
    removed: usize,
    selected: bool,
    theme: Theme,
) -> Stateful<Div> {
    file_row(id, path, added, removed, selected, false, theme)
}

pub fn review_file_row(
    id: impl Into<ElementId>,
    path: &str,
    added: usize,
    removed: usize,
    selected: bool,
    theme: Theme,
) -> Stateful<Div> {
    file_row(id, path, added, removed, selected, true, theme)
}

#[allow(clippy::too_many_arguments)]
fn file_row(
    id: impl Into<ElementId>,
    path: &str,
    added: usize,
    removed: usize,
    selected: bool,
    rail: bool,
    theme: Theme,
) -> Stateful<Div> {
    let path = std::path::Path::new(path);
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    let parent = path
        .parent()
        .unwrap_or(std::path::Path::new(""))
        .display()
        .to_string();
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(format!("Review {}", path.display()))
        .aria_selected(selected)
        .w_full()
        .min_w_0()
        .min_h(px(if rail { 66. } else { 61. }))
        .px(px(if rail { 12. } else { 0. }))
        .gap(px(if rail { 6. } else { 15. }))
        .rounded(px(5.))
        .when(selected, |v| v.bg(theme.selected))
        .cursor_pointer()
        .hover(move |v| v.bg(theme.hover))
        .tooltip(ui::Tooltip::text(path.display().to_string()))
        .when(!rail, |v| v.child(icon("file", theme.muted).size(px(14.))))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .font_family(SANS)
                .child(
                    div()
                        .truncate()
                        .text_size(px(if rail { 12. } else { 14. }))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                )
                .when(rail, |v| v.child(change_counts(added, removed, theme)))
                .when(!rail && !parent.is_empty(), |v| {
                    v.child(
                        div()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(theme.muted)
                            .child(parent),
                    )
                }),
        )
        .when(!rail, |v| {
            v.child(change_counts(added, removed, theme))
                .child(icon("chevron_right", theme.muted).size(px(12.)))
        })
}

/// A note about the work. A warning is a soft callout: its first clause
/// ("Observed edit, already on disk") as the title, the rest as plain text.
pub fn work_notice(text: impl Into<SharedString>, warning: bool, theme: Theme) -> Div {
    let text: SharedString = text.into();
    if !warning {
        return h_flex()
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(theme.muted)
            .child(div().flex_1().min_w_0().child(text));
    }
    let (title, rest) = text
        .split_once(" · ")
        .map(|(title, rest)| (title.to_owned(), Some(rest.replace(" · ", "; "))))
        .unwrap_or_else(|| (text.to_string(), None));
    h_flex()
        .items_start()
        .gap(px(10.))
        .px(px(14.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(theme.tint(theme.amber))
        .child(icon("info", theme.amber).size(px(14.)).mt(px(2.)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(title),
                )
                .children(rest.map(|rest| {
                    div()
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(theme.secondary)
                        .child(capitalize(&rest))
                })),
        )
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
