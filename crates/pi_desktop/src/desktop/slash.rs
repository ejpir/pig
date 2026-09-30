//! The `/` menu: the app's built-in commands and pi's prompt templates, skills, and
//! extension commands, filtered by what follows the slash in the composer.
use gpui::{App, Div, HighlightStyle, StyledText, deferred, relative};

use super::composer::{ComposerEvent, ComposerView};
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BuiltIn {
    Compact,
    Copy,
    Name,
    New,
    Reload,
}

impl BuiltIn {
    const ALL: [Self; 5] = [
        Self::Compact,
        Self::Copy,
        Self::Name,
        Self::New,
        Self::Reload,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Copy => "copy",
            Self::Name => "name",
            Self::New => "new",
            Self::Reload => "reload",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Compact => {
                "Summarize the conversation so far to free up context. Text after the command guides the summary."
            }
            Self::Copy => "Copy the last reply",
            Self::Name => "Name this session",
            Self::New => "Start a new session in this project",
            Self::Reload => "Reload extensions, skills, prompt templates, and context files",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Copy => "copy",
            Self::Name => "pencil",
            Self::New => "plus",
            Self::Reload => "box",
        }
    }

    /// Commands that use the text after them wait for Enter; the rest run when chosen.
    fn takes_text(self) -> bool {
        matches!(self, Self::Compact | Self::Name)
    }
}

/// A command attached to the composer draft.
#[derive(Clone, Debug)]
pub(super) enum Attached {
    BuiltIn(BuiltIn),
    Pi(SlashCommand),
}

impl Attached {
    pub(super) fn label(&self) -> String {
        match self {
            Self::BuiltIn(builtin) => format!("/{}", builtin.name()),
            Self::Pi(command) => match command.name.strip_prefix("skill:") {
                Some(skill) => skill.to_owned(),
                None => format!("/{}", command.name),
            },
        }
    }

    pub(super) fn icon(&self) -> &'static str {
        match self {
            Self::BuiltIn(builtin) => builtin.icon(),
            Self::Pi(command) => match command.source.as_str() {
                "skill" => "sparkle",
                "prompt" => "file",
                _ => "box",
            },
        }
    }

    pub(super) fn is_skill(&self) -> bool {
        matches!(self, Self::Pi(command) if command.source == "skill")
    }
}

#[derive(Clone)]
struct Item {
    attached: Attached,
    name: String,
    description: String,
    tag: String,
}

#[derive(Clone, Copy)]
enum MenuRow {
    Header(&'static str),
    Separator,
    Item(usize),
}

pub(super) struct Menu {
    items: Vec<Item>,
    rows: Vec<MenuRow>,
    item_rows: Vec<usize>,
    tops: Vec<f32>,
    height: f32,
    scroll: gpui::ListState,
}
impl Default for Menu {
    fn default() -> Self {
        Self {
            items: vec![],
            rows: vec![],
            item_rows: vec![],
            tops: vec![],
            height: 0.,
            scroll: gpui::ListState::new(0, gpui::ListAlignment::Top, px(28.)),
        }
    }
}

const MENU_WIDTH: f32 = 340.;
const LIST_PADDING: f32 = 4.;
const SEPARATOR_HEIGHT: f32 = 9.;
const HEADER_HEIGHT: f32 = 24.;
const ROW_HEIGHT: f32 = 28.;

/// Menu sections in display order: built-ins, then pi's command sources.
const SECTIONS: [(&str, Option<&str>); 4] = [
    ("Commands", None),
    ("Prompts", Some("prompt")),
    ("Skills", Some("skill")),
    ("Extensions", Some("extension")),
];

impl ComposerView {
    /// The text after `/` while the menu is showing.
    pub(super) fn slash_query(&self, cx: &App) -> Option<String> {
        if self.slash_dismissed || self.picker.is_some() {
            return None;
        }
        let query = self.input.read(cx).content().strip_prefix('/')?;
        (!query.contains(char::is_whitespace)).then(|| query.to_lowercase())
    }

    fn build_slash_sections(&self, query: &str, cx: &App) -> Vec<(&'static str, Vec<Item>)> {
        let commands = &self.controller.read(cx).model().commands;
        SECTIONS
            .into_iter()
            .map(|(title, source)| {
                let items: Vec<Item> = match source {
                    None => BuiltIn::ALL
                        .into_iter()
                        .map(|builtin| Item {
                            attached: Attached::BuiltIn(builtin),
                            name: builtin.name().to_owned(),
                            description: builtin.description().to_owned(),
                            tag: "built-in".into(),
                        })
                        .collect(),
                    Some(source) => commands
                        .iter()
                        .filter(|command| command.source == source)
                        .map(|command| Item {
                            name: command
                                .name
                                .strip_prefix("skill:")
                                .unwrap_or(&command.name)
                                .to_owned(),
                            description: command.description.clone().unwrap_or_default(),
                            tag: source_tag(command),
                            attached: Attached::Pi(command.clone()),
                        })
                        .collect(),
                };
                let items = items
                    .into_iter()
                    .filter(|item| item.name.to_lowercase().contains(query))
                    .collect();
                (title, items)
            })
            .filter(|(_, items): &(_, Vec<Item>)| !items.is_empty())
            .collect()
    }

    pub(super) fn refresh_slash(&mut self, cx: &App) {
        let sections = self
            .slash_query(cx)
            .map(|query| self.build_slash_sections(&query, cx))
            .unwrap_or_default();
        let mut menu = Menu::default();
        let mut top = LIST_PADDING;
        for (section, (title, items)) in sections.into_iter().enumerate() {
            if section > 0 {
                menu.tops.push(top);
                menu.rows.push(MenuRow::Separator);
                top += SEPARATOR_HEIGHT;
            }
            menu.tops.push(top);
            menu.rows.push(MenuRow::Header(title));
            top += HEADER_HEIGHT;
            for item in items {
                menu.item_rows.push(menu.rows.len());
                menu.tops.push(top);
                menu.rows.push(MenuRow::Item(menu.items.len()));
                menu.items.push(item);
                top += ROW_HEIGHT;
            }
        }
        menu.height = top + LIST_PADDING;
        menu.scroll
            .reset_with_uniform_height(menu.rows.len(), px(ROW_HEIGHT));
        self.slash_menu = menu;
        self.slash_index = 0;
    }

    /// Called whenever the composer changes; opens, filters, or closes the menu.
    pub(super) fn composer_changed(&mut self, cx: &mut Context<Self>) {
        let content = self.input.read(cx).content().to_owned();
        if !content.starts_with('/') {
            self.slash_dismissed = false;
            if let Some(stash) = self.slash_stash.take() {
                let restored = if content.is_empty() {
                    stash
                } else {
                    format!("{content} {stash}")
                };
                self.input
                    .update(cx, |input, cx| input.set_content(restored, cx));
            }
        }
        let query = self.slash_query(cx);
        if query != self.slash_seen {
            if self.slash_seen.is_none() && query.is_some() {
                // Resources can change on disk or after `/reload`; ask each time the menu opens.
                self.command(Command::GetCommands, cx);
            }
            self.slash_index = 0;
            self.slash_seen = query;
            self.refresh_slash(cx);
        }
        self.update_mentions(cx);
        cx.notify();
    }

    /// The composer's slash button: a draft in progress is kept and restored afterwards.
    pub(super) fn open_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.input.read(cx).content().to_owned();
        if !draft.starts_with('/') {
            if !draft.is_empty() {
                self.slash_stash = Some(draft);
            }
            self.input
                .update(cx, |input, cx| input.set_content("/", cx));
        }
        self.slash_dismissed = false;
        self.picker = None;
        self.input.focus_handle(cx).focus(window, cx);
        self.composer_changed(cx);
    }

    pub(super) fn dismiss_slash(&mut self, cx: &mut Context<Self>) {
        self.slash_dismissed = true;
        if let Some(stash) = self.slash_stash.take() {
            self.input
                .update(cx, |input, cx| input.set_content(stash, cx));
        }
        self.slash_seen = None;
        cx.notify();
    }

    pub(super) fn move_slash(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = self.slash_menu.items.len();
        if self.slash_query(cx).is_none() || count == 0 {
            return;
        }
        self.slash_index =
            (self.slash_index as isize + direction).rem_euclid(count as isize) as usize;
        self.slash_menu
            .scroll
            .scroll_to_reveal_item(self.slash_menu.item_rows[self.slash_index]);
        cx.notify();
    }

    pub(super) fn choose_slash(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.slash_query(cx).is_none() {
            return;
        }
        let Some(item) = self.slash_menu.items.get(index).cloned() else {
            return;
        };
        let draft = self.slash_stash.take().unwrap_or_default();
        self.input
            .update(cx, |input, cx| input.set_content(draft, cx));
        self.slash_seen = None;
        match item.attached {
            Attached::BuiltIn(builtin) if !builtin.takes_text() => {
                self.run_builtin(builtin, String::new(), cx)
            }
            attached => self.attached = Some(attached),
        }
        self.input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// `/name text` typed in full runs the built-in, unless pi has a command of that name.
    pub(super) fn typed_builtin(&self, draft: &str, cx: &App) -> Option<(BuiltIn, String)> {
        let rest = draft.strip_prefix('/')?;
        let (name, text) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let builtin = BuiltIn::ALL
            .into_iter()
            .find(|builtin| builtin.name() == name)?;
        let shadowed = self
            .controller
            .read(cx)
            .model()
            .commands
            .iter()
            .any(|command| command.name == name);
        (!shadowed).then(|| (builtin, text.trim().to_owned()))
    }

    pub(super) fn run_builtin(&mut self, builtin: BuiltIn, text: String, cx: &mut Context<Self>) {
        match builtin {
            BuiltIn::Compact => {
                self.command(
                    Command::Compact {
                        custom_instructions: (!text.is_empty()).then_some(text),
                    },
                    cx,
                );
            }
            BuiltIn::Name if text.is_empty() => {
                self.controller.update(cx, |controller, cx| {
                    controller.notice("Type the new name after /name.", cx)
                });
                self.attached = Some(Attached::BuiltIn(builtin));
                cx.notify();
            }
            BuiltIn::Name => {
                self.command(
                    Command::SetSessionName {
                        name: text,
                        session_path: None,
                    },
                    cx,
                );
            }
            BuiltIn::Copy => {
                let model = self.controller.read(cx).model();
                let reply = model
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "assistant")
                    .map(|message| {
                        message["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|block| block["type"] == "text")
                            .filter_map(|block| block["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .filter(|reply| !reply.is_empty());
                match reply {
                    Some(reply) => cx.write_to_clipboard(ClipboardItem::new_string(reply)),
                    None => self.controller.update(cx, |controller, cx| {
                        controller.notice("There is no reply to copy yet.", cx)
                    }),
                }
                cx.notify();
            }
            BuiltIn::New => cx.emit(ComposerEvent::NewSession),
            BuiltIn::Reload => {
                self.command(Command::Reload, cx);
            }
        }
    }

    fn slash_row(&self, row: usize, cx: &Context<Self>, theme: Theme) -> AnyElement {
        match self.slash_menu.rows[row] {
            MenuRow::Separator => div()
                .w_full()
                .h(px(SEPARATOR_HEIGHT))
                .flex()
                .items_center()
                .child(div().w_full().h(px(1.)).bg(theme.line))
                .into_any_element(),
            MenuRow::Header(title) => h_flex()
                .w_full()
                .h(px(HEADER_HEIGHT))
                .items_end()
                .px(px(14.))
                .pb(px(3.))
                .text_size(px(12.))
                .text_color(theme.muted)
                .child(title)
                .into_any_element(),
            MenuRow::Item(index) => {
                let item = &self.slash_menu.items[index];
                let selected = index == self.slash_index;
                let row =
                    h_flex()
                        .w_full()
                        .id(("slash-item", index))
                        .debug_selector(move || format!("slash-item-{index}"))
                        .h(px(ROW_HEIGHT))
                        .px(px(8.))
                        .gap(px(8.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(selected, |row| row.bg(theme.selected))
                        .hover(move |row| row.bg(theme.hover))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.slash_index != index {
                                this.slash_index = index;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_slash(index, window, cx)
                        }))
                        .child(
                            icon(
                                item.attached.icon(),
                                if selected { theme.accent } else { theme.muted },
                            )
                            .size(px(14.)),
                        )
                        .child(matched_name(
                            &item.name,
                            self.slash_seen.as_deref().unwrap_or(""),
                            theme,
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(MONO)
                                .text_size(px(11.))
                                .text_color(theme.faint)
                                .child(item.tag.clone()),
                        );
                div().w_full().px(px(6.)).child(row).into_any_element()
            }
        }
    }
    pub(super) fn slash_view(&self, cx: &Context<Self>, theme: Theme) -> Option<AnyElement> {
        let query = self.slash_query(cx)?;
        let surface = if theme.light { theme.chip } else { theme.bar };
        let view = cx.entity().downgrade();
        let list = if self.slash_menu.items.is_empty() {
            div()
                .px(px(12.))
                .py(px(8.))
                .text_size(px(12.))
                .text_color(theme.faint)
                .child(format!("No commands match /{query}"))
                .into_any_element()
        } else {
            div()
                .id("slash-list")
                .debug_selector(|| "slash-list".into())
                .h(px(self.slash_menu.height.min(320.)))
                .w_full()
                .child(
                    gpui::list(self.slash_menu.scroll.clone(), move |index, _, cx| {
                        view.update(cx, |this, cx| this.slash_row(index, cx, theme))
                            .unwrap_or_else(|_| div().into_any_element())
                    })
                    .size_full()
                    .py(px(LIST_PADDING)),
                )
                .into_any_element()
        };
        let panel = |element: Div| {
            element
                .rounded(px(8.))
                .border_1()
                .border_color(theme.chip_line)
                .bg(surface)
                .shadow_lg()
        };
        let description = self
            .slash_menu
            .items
            .get(self.slash_index)
            .filter(|item| !item.description.is_empty())
            .map(|item| {
                let offset = self.slash_menu.scroll.logical_scroll_top();
                let scroll = self
                    .slash_menu
                    .tops
                    .get(offset.item_ix)
                    .copied()
                    .unwrap_or(self.slash_menu.height)
                    + f32::from(offset.offset_in_item);
                let top = 1. + self.slash_menu.tops[self.slash_menu.item_rows[self.slash_index]]
                    - scroll
                    + LIST_PADDING;
                (item.description.clone(), px(top.max(0.)))
            });
        let menu = div()
            .id("slash-menu")
            .absolute()
            .left(px(8.))
            .bottom(relative(1.))
            .mb(px(6.))
            .child(
                panel(v_flex().w(px(MENU_WIDTH)).occlude())
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_slash(cx)))
                    .child(list)
                    .child(
                        div()
                            .border_t_1()
                            .border_color(theme.line)
                            .px(px(14.))
                            .py(px(5.))
                            .font_family(MONO)
                            .text_size(px(10.))
                            .text_color(theme.faint)
                            .child("↑↓ select · ⏎ choose · ⇥ complete · esc dismiss"),
                    ),
            )
            .when_some(description, |menu, (description, top)| {
                menu.child(
                    panel(
                        div()
                            .absolute()
                            .left(px(MENU_WIDTH + 8.))
                            .top(top)
                            .w(px(300.)),
                    )
                    .debug_selector(|| "slash-description".into())
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(theme.secondary)
                    .child(description),
                )
            });
        Some(deferred(menu).with_priority(1).into_any_element())
    }
}

/// The package that provides a command, else the settings scope it came from.
fn source_tag(command: &SlashCommand) -> String {
    let Some(info) = &command.source_info else {
        return String::new();
    };
    if info.origin != "package" {
        return info.scope.clone();
    }
    // npm:@acme/git-guard@1.4.0 -> git-guard; git:github.com/org/repo@v1 -> repo
    let spec = info
        .source
        .split_once(':')
        .map_or(info.source.as_str(), |(_, spec)| spec);
    let name = spec.rsplit('/').next().unwrap_or(spec);
    name.split('@')
        .find(|part| !part.is_empty())
        .unwrap_or(name)
        .to_owned()
}

fn matched_name(name: &str, query: &str, theme: Theme) -> impl IntoElement {
    // Only highlight when lowercasing keeps byte offsets, i.e. for ASCII names.
    let highlight = (!query.is_empty() && name.is_ascii())
        .then(|| name.to_ascii_lowercase().find(query))
        .flatten()
        .map(|start| {
            (
                start..start + query.len(),
                HighlightStyle {
                    color: Some(theme.accent),
                    ..Default::default()
                },
            )
        });
    div()
        .flex_shrink_0()
        .font_family(MONO)
        .text_size(px(12.))
        .text_color(theme.text)
        .child(StyledText::new(name.to_owned()).with_highlights(highlight))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_core::protocol::SourceInfo;

    fn command(source: &str, origin: &str, scope: &str) -> SlashCommand {
        SlashCommand {
            name: "commit".into(),
            description: None,
            source: "extension".into(),
            source_info: Some(SourceInfo {
                path: "/x".into(),
                source: source.into(),
                scope: scope.into(),
                origin: origin.into(),
            }),
        }
    }

    #[test]
    fn source_tags_name_the_package_or_the_scope() {
        assert_eq!(
            source_tag(&command("npm:@acme/git-guard@1.4.0", "package", "user")),
            "git-guard"
        );
        assert_eq!(
            source_tag(&command(
                "git:github.com/nicobailon/pi-mcp-adapter@v0.9",
                "package",
                "user"
            )),
            "pi-mcp-adapter"
        );
        assert_eq!(
            source_tag(&command("npm:left-pad", "package", "user")),
            "left-pad"
        );
        assert_eq!(
            source_tag(&command("local", "top-level", "project")),
            "project"
        );
    }
}
