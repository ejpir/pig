//! Shared visual primitives for session tabs; no session or navigation ownership.
use super::*;
use crate::markdown_view::{self, Documents, Source};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionPage {
    #[default]
    Thread,
    Changes,
    Tree,
    Context,
}
impl SessionPage {
    pub const ALL: [Self; 4] = [Self::Thread, Self::Changes, Self::Tree, Self::Context];
    pub fn name(self) -> &'static str {
        match self {
            Self::Thread => "THREAD",
            Self::Changes => "CHANGES",
            Self::Tree => "TREE",
            Self::Context => "CONTEXT",
        }
    }
}

pub fn heading(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    theme: Theme,
) -> gpui::Div {
    v_flex()
        .gap(px(5.))
        .mb(px(18.))
        .child(
            div()
                .font_family(SERIF)
                .italic()
                .text_size(px(22.))
                .line_height(px(30.))
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme.muted)
                .child(subtitle.into()),
        )
}
pub fn note(text: impl Into<SharedString>, theme: Theme) -> gpui::Div {
    div()
        .text_size(px(11.))
        .line_height(px(17.))
        .text_color(theme.faint)
        .child(text.into())
}
pub fn empty(title: &str, detail: &str, theme: Theme) -> gpui::Div {
    v_flex()
        .p(px(24.))
        .gap(px(8.))
        .child(
            div()
                .font_family(SERIF)
                .italic()
                .text_size(px(22.))
                .child(title.to_owned()),
        )
        .child(note(detail.to_owned(), theme))
}
pub fn input_box(input: Entity<TextInput>, theme: Theme) -> gpui::Div {
    div()
        .w_full()
        .p(px(8.))
        .bg(theme.canvas)
        .border_1()
        .border_color(theme.chip_line)
        .rounded(px(5.))
        .child(input)
}

/// Independently cached selectable Markdown/code with a bounded preview and full raw copy.
pub struct DocumentView {
    documents: Documents,
    raw: String,
    code: Option<&'static str>,
    compact: bool,
    review: bool,
    wrap: Option<bool>,
    subscription: Option<gpui::Subscription>,
}
impl DocumentView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            documents: Documents::default(),
            raw: String::new(),
            code: None,
            compact: false,
            review: false,
            wrap: None,
            subscription: None,
        };
        this.set("".into(), None, cx);
        this
    }
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    pub fn review(mut self) -> Self {
        self.review = true;
        self
    }
    pub fn wrapped(mut self) -> Self {
        self.wrap = Some(true);
        self
    }
    pub fn unwrapped(mut self) -> Self {
        self.wrap = Some(false);
        self
    }
    pub fn is_wrapped(&self) -> bool {
        self.wrap == Some(true)
    }
    pub fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.wrap = Some(!self.is_wrapped());
        cx.notify();
    }
    pub fn selected_source(&self, cx: &App) -> Option<String> {
        self.documents
            .get("body")
            .and_then(|document| document.read(cx).selected_source().map(str::to_owned))
    }
    pub fn set(&mut self, raw: String, code: Option<&'static str>, cx: &mut Context<Self>) {
        if self.raw == raw && self.code == code && self.subscription.is_some() {
            return;
        }
        self.raw = raw;
        self.code = code;
        let mut end = self.raw.len().min(65536);
        while !self.raw.is_char_boundary(end) {
            end -= 1;
        }
        let preview = &self.raw[..end];
        self.documents.sync(
            vec![(
                "body".into(),
                match code {
                    Some(language) => Source::Code {
                        text: preview,
                        language,
                    },
                    None => Source::Markdown(preview.into()),
                },
            )],
            cx,
        );
        if let Some(document) = self.documents.get("body") {
            self.subscription = Some(cx.observe(document, |_, _, cx| cx.notify()));
        }
        cx.notify();
    }
}
impl Render for DocumentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let Some(document) = self.documents.get("body") else {
            return div().into_any_element();
        };
        let mut style = if self.code.is_some() {
            markdown_view::output_style(theme, window, cx)
        } else {
            markdown_view::style(theme, theme.secondary, window, cx)
        };
        if self.compact {
            style.base_text_style.font_size = px(11.5).into();
            style.base_text_style.line_height = px(17.5).into();
            style.container_style = gpui::StyleRefinement::default()
                .text_size(px(11.5))
                .line_height(px(17.5));
            style.paragraph_line_height = px(17.5).into();
            style.paragraph_spacing = px(4.);
            if self.code.is_some() {
                style.code_block = style.code_block.p(px(0.)).line_height(px(17.5));
            }
        }
        if self.review {
            style.base_text_style.font_size = px(13.).into();
            style.base_text_style.line_height = px(24.).into();
            style.container_style = gpui::StyleRefinement::default()
                .font_family(MONO)
                .text_size(px(13.))
                .line_height(px(24.));
            style.paragraph_line_height = px(24.).into();
            style.code_block = gpui::StyleRefinement::default()
                .font_family(MONO)
                .text_size(px(13.))
                .line_height(px(24.))
                .py(px(0.))
                .bg(gpui::transparent_black());
        }
        if let Some(wrap) = self.wrap {
            style.code_block_overflow_x_scroll = !wrap;
        }
        let element = markdown_view::element(document, style);
        let element = if self.wrap.is_some() {
            element.without_wrap_control()
        } else {
            element
        };
        let raw = self.raw.clone();
        v_flex()
            .w_full()
            .min_w_0()
            .when(self.raw.len() > 65536, |v| {
                v.child(note("Preview truncated to 64 KiB.", theme)).child(
                    button("copy-full-document", "Copy full text", theme)
                        .on_click(move |_, _, cx| markdown_view::copy(&raw, cx)),
                )
            })
            .child(markdown_view::with_menu(
                "panel-document",
                Some(document.clone()),
                "Copy block",
                self.documents.copy_range("body"),
                element.into_any_element(),
            ))
            .into_any_element()
    }
}
