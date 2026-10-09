//! One Markdown implementation for Pi on desktop and Android: Zed's renderer,
//! its native tree-sitter grammars, stable document identities while streaming,
//! and Pi's semantic syntax palette. Platform crates only choose dimensions.

use crate::{SyntaxPalette, code_source};
use gpui::{
    App, AppContext as _, ElementId, Entity, FontStyle, FontWeight, Global, HighlightStyle, Hsla,
    KeyBinding, ParentElement as _, SharedString, Styled as _, Window, div,
};
use language::{LanguageRegistry, LoadedLanguage};
use markdown::{
    CodeBlockRenderer, CopyButtonVisibility, Markdown, MarkdownElement, MarkdownOptions,
    MarkdownStyle, WrapButtonVisibility,
};
use std::{borrow::Cow, collections::HashMap, sync::Arc};
use zed_theme::SyntaxTheme;

struct CodeLanguages(Arc<LanguageRegistry>);

impl Global for CodeLanguages {}

/// Languages bundled in both apps. This intentionally matches the desktop set.
const CODE_LANGUAGES: [&str; 20] = [
    "bash",
    "c",
    "cpp",
    "css",
    "diff",
    "go",
    "gomod",
    "gowork",
    "javascript",
    "jsdoc",
    "json",
    "jsonc",
    "markdown",
    "markdown-inline",
    "python",
    "regex",
    "rust",
    "tsx",
    "typescript",
    "yaml",
];

/// Initializes Zed's Markdown prerequisites and the exact same native language
/// registry on desktop and Android. No language servers or subprocesses run.
pub fn init(cx: &mut App) {
    if cx.has_global::<CodeLanguages>() {
        return;
    }
    settings::init(cx);
    theme_settings::init(zed_theme::LoadThemes::JustBase, cx);
    let languages = Arc::new(LanguageRegistry::new(cx.background_executor().clone()));
    languages.register_native_grammars(grammars::native_grammars());
    for name in CODE_LANGUAGES {
        let config = grammars::load_config(name);
        languages.register_language(
            config.name.clone(),
            config.grammar.clone(),
            config.matcher.clone(),
            config.hidden,
            None,
            Arc::new(move || {
                let config = config.clone();
                Box::pin(async move {
                    Ok(LoadedLanguage {
                        config,
                        queries: grammars::load_queries(name),
                        context_provider: None,
                        toolchain_provider: None,
                        manifest_name: None,
                    })
                })
            }),
        );
    }
    cx.set_global(CodeLanguages(languages));
    cx.bind_keys([KeyBinding::new(
        "secondary-c",
        markdown::Copy,
        Some("Markdown"),
    )]);
}

pub fn languages(cx: &App) -> Arc<LanguageRegistry> {
    cx.global::<CodeLanguages>().0.clone()
}

/// Language captures resolve colors through the active theme, so every palette
/// change must update the registry along with `GlobalTheme`.
pub fn set_language_theme(theme: Arc<zed_theme::Theme>, cx: &App) {
    cx.global::<CodeLanguages>().0.set_theme(theme);
}

/// The semantic syntax mapping previously owned by the desktop-only module.
pub fn syntax(palette: SyntaxPalette) -> SyntaxTheme {
    let color = |color: Hsla| HighlightStyle {
        color: Some(color),
        ..Default::default()
    };
    SyntaxTheme::new(
        [
            ("attribute", color(palette.amber)),
            ("boolean", color(palette.keyword)),
            (
                "comment",
                HighlightStyle {
                    font_style: Some(FontStyle::Italic),
                    ..color(palette.faint)
                },
            ),
            ("constant", color(palette.amber)),
            ("constructor", color(palette.steel)),
            (
                "diff.plus",
                HighlightStyle {
                    background_color: Some(palette.added),
                    ..color(palette.green)
                },
            ),
            (
                "diff.minus",
                HighlightStyle {
                    background_color: Some(palette.removed),
                    ..color(palette.coral)
                },
            ),
            ("embedded", color(palette.plain)),
            (
                "emphasis",
                HighlightStyle {
                    font_style: Some(FontStyle::Italic),
                    ..Default::default()
                },
            ),
            (
                "emphasis.strong",
                HighlightStyle {
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                },
            ),
            ("function", color(palette.accent)),
            ("keyword", color(palette.keyword)),
            ("label", color(palette.amber)),
            ("link_text", color(palette.accent)),
            ("link_uri", color(palette.accent)),
            ("number", color(palette.amber)),
            ("operator", color(palette.muted)),
            ("property", color(palette.plain)),
            ("punctuation", color(palette.muted)),
            ("string", color(palette.string)),
            ("string.escape", color(palette.amber)),
            ("string.special", color(palette.string)),
            ("tag", color(palette.keyword)),
            (
                "title",
                HighlightStyle {
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..color(palette.text)
                },
            ),
            ("type", color(palette.steel)),
            ("variable", color(palette.plain)),
            ("variable.special", color(palette.keyword)),
        ]
        .map(|(name, style)| (name.to_owned(), style)),
    )
}

pub type LinkHandler = std::rc::Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

#[derive(gpui::IntoElement)]
pub struct DocumentElement {
    document: Entity<Markdown>,
    style: MarkdownStyle,
    copy: CopyButtonVisibility,
    wrap: WrapButtonVisibility,
    on_link: Option<LinkHandler>,
}

impl DocumentElement {
    pub fn without_wrap_control(mut self) -> Self {
        self.wrap = WrapButtonVisibility::Hidden;
        self
    }

    pub fn on_link(mut self, handler: LinkHandler) -> Self {
        self.on_link = Some(handler);
        self
    }

    pub fn controls(mut self, copy: CopyButtonVisibility, wrap: WrapButtonVisibility) -> Self {
        self.copy = copy;
        self.wrap = wrap;
        self
    }
}

impl gpui::RenderOnce for DocumentElement {
    fn render(self, _: &mut Window, cx: &mut App) -> impl gpui::IntoElement {
        use gpui::{Focusable as _, InteractiveElement as _};
        let focus = self.document.focus_handle(cx);
        div()
            .id(ElementId::View(self.document.entity_id()))
            .w_full()
            .key_context("Markdown")
            .track_focus(&focus)
            .child({
                let element = MarkdownElement::new(self.document, self.style);
                let element = match self.on_link {
                    Some(handler) => {
                        element.on_url_click(move |url, window, cx| handler(url, window, cx))
                    }
                    None => element,
                };
                element.code_block_renderer(CodeBlockRenderer::Default {
                    copy_button_visibility: self.copy,
                    wrap_button_visibility: self.wrap,
                    border: false,
                })
            })
    }
}

pub fn element(document: &Entity<Markdown>, style: MarkdownStyle) -> DocumentElement {
    DocumentElement {
        document: document.clone(),
        style,
        copy: CopyButtonVisibility::VisibleOnHover,
        wrap: WrapButtonVisibility::VisibleOnHover,
        on_link: None,
    }
}

pub enum Source<'a> {
    Markdown(Cow<'a, str>),
    Text(Cow<'a, str>),
    Code { text: &'a str, language: &'a str },
}

/// Stable Markdown entities preserve selection, code wrapping and incremental
/// streaming state instead of reparsing a reply into ad-hoc UI blocks.
#[derive(Default)]
pub struct Documents {
    entities: HashMap<SharedString, Entity<Markdown>>,
    code: HashMap<SharedString, (String, std::ops::Range<usize>)>,
}

impl Documents {
    pub fn sync(&mut self, sources: Vec<(SharedString, Source)>, cx: &mut App) {
        let keys: std::collections::HashSet<_> =
            sources.iter().map(|(key, _)| key.clone()).collect();
        self.entities.retain(|key, _| keys.contains(key));
        self.code.retain(|key, _| keys.contains(key));
        self.update(sources, cx);
    }

    pub fn update(&mut self, sources: Vec<(SharedString, Source)>, cx: &mut App) {
        let languages = languages(cx);
        for (key, source) in sources {
            let (source, plain) = match source {
                Source::Markdown(source) => {
                    self.code.remove(&key);
                    // Zed's Markdown draws only base64 `data:` images.
                    let source = match crate::embed_svg(&source) {
                        Cow::Borrowed(_) => source,
                        Cow::Owned(embedded) => Cow::Owned(embedded),
                    };
                    (source, false)
                }
                Source::Text(source) => {
                    self.code.remove(&key);
                    (source, true)
                }
                Source::Code { text, language } => {
                    if self.code.get(&key).is_some_and(|(previous, range)| {
                        previous == language
                            && self.entities.get(&key).is_some_and(|entity| {
                                entity.read(cx).source().get(range.clone()) == Some(text)
                            })
                    }) {
                        continue;
                    }
                    let (source, range) = code_source(text, language);
                    self.code.insert(key.clone(), (language.to_owned(), range));
                    (Cow::Owned(source), false)
                }
            };
            let entity = match self.entities.remove(&key) {
                Some(entity) => {
                    entity.update(cx, |markdown, cx| {
                        let previous = markdown.source();
                        if previous.as_ref() != source {
                            match source.strip_prefix(previous.as_ref()) {
                                Some(delta) => markdown.append(delta, cx),
                                None => markdown.replace(source.into_owned(), cx),
                            }
                        }
                    });
                    entity
                }
                None if plain => cx.new(|cx| Markdown::new_text(source.into_owned().into(), cx)),
                None => cx.new(|cx| {
                    Markdown::new_with_options(
                        source.into_owned().into(),
                        Some(languages.clone()),
                        None,
                        MarkdownOptions {
                            render_mermaid_diagrams: true,
                            ..Default::default()
                        },
                        cx,
                    )
                }),
            };
            self.entities.insert(key, entity);
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.entities.remove(key);
        self.code.remove(key);
    }

    pub fn get(&self, key: &str) -> Option<&Entity<Markdown>> {
        self.entities.get(key)
    }

    pub fn copy_range(&self, key: &str) -> Option<std::ops::Range<usize>> {
        self.code.get(key).map(|(_, range)| range.clone())
    }

    pub fn copy_source<'a>(&'a self, key: &str, cx: &'a App) -> Option<&'a str> {
        let source = self.get(key)?.read(cx).source();
        match self.copy_range(key) {
            Some(range) => source.get(range),
            None => Some(source.as_ref()),
        }
    }
}
