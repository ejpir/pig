use gpui::{App, AssetSource, SharedString};
use std::borrow::Cow;

pub struct Assets;

macro_rules! icons {
    ($($name:ident),* $(,)?) => { &[$((stringify!($name), include_bytes!(concat!("../../../assets/icons/", stringify!($name), ".svg")) as &[u8])),*] };
}

const ICONS: &[(&str, &[u8])] = icons![
    folder,
    thread,
    terminal,
    file,
    check,
    plus,
    close,
    git_branch,
    settings,
    chevron_right,
    chevron_down,
    magnifying_glass,
    stop,
    pencil,
    sparkle,
    copy,
    attach,
    slash,
    box,
    list_tree,
    threads_sidebar_right_open,
    threads_sidebar_right_closed,
    threads_sidebar_left_open,
    threads_sidebar_left_closed,
    maximize,
    minimize,
    ellipsis,
    load_circle,
    queue,
    compact,
    spinner_track,
    spinner_arc,
    info,
    chat,
    warning,
    text_wrap,
    text_unwrap,
    dash,
    undo,
    redo,
    git_commit,
    file_add,
    folder_add,
    trash,
    list_collapse,
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| path == format!("icons/{name}.svg"))
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }
    fn list(&self, _path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .map(|(name, _)| format!("icons/{name}.svg").into())
            .collect())
    }
}

pub fn load_fonts(cx: &App) -> anyhow::Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/IBMPlexSans-Regular.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/IBMPlexSans-SemiBold.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/IBMPlexSans-Italic.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/CommitMono-Regular.otf"
        )),
    ])
}
