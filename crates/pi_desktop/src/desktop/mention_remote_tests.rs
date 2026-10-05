use super::*;
use crate::desktop::{session::SessionController, session_view::SessionView};
use gpui::{TestAppContext, VisualTestContext, WindowOptions};
use pi_core::{remote_files::Client, ssh::SshTarget, transport::Launch};

struct Fixture {
    composer: Entity<ComposerView>,
    files: Entity<super::super::files::FilesView>,
    log: std::path::PathBuf,
    _directory: tempfile::TempDir,
    visual: VisualTestContext,
}
fn setup(cx: &mut TestAppContext) -> Fixture {
    setup_with_list_error(cx, false)
}
fn setup_with_list_error(cx: &mut TestAppContext, list_error: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(
        directory.path().join("src/remote.rs"),
        "LOCAL SECRET - must not appear",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/unreadable.rs"),
        "LOCAL FALLBACK - forbidden",
    )
    .unwrap();
    // Same-looking desktop and remote cwd is deliberate. Every byte for preview
    // must come from RPC, never from the real files created above.
    let target = SshTarget::new(
        "test".into(),
        directory.path().to_string_lossy().into_owned(),
    )
    .unwrap();
    let log = directory.path().join("requests.jsonl");
    let mut env = vec![(
        "PI_DESKTOP_TEST_FILE_LOG".into(),
        log.clone().into_os_string(),
    )];
    if list_error {
        env.push(("PI_DESKTOP_TEST_FILE_LIST_ERROR".into(), "1".into()));
    }
    let client = Arc::new(
        Client::from_launch(
            &target,
            Launch {
                program: if cfg!(windows) { "python" } else { "python3" }.into(),
                args: vec![
                    "-u".into(),
                    "-c".into(),
                    include_str!("../../tests/fake_files.py").into(),
                ],
                cwd: std::env::temp_dir(),
                env,
                request_timeout: Duration::from_secs(5),
                extension: None,
            },
        )
        .unwrap(),
    );
    let controller = cx.update(|cx| {
        cx.set_global(Theme::new(false));
        super::super::init(cx);
        cx.new(|cx| SessionController::remote_test(target, cx))
    });
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|cx| SessionView::new(controller, "", cx))
        })
        .unwrap()
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let view = window.root(&mut visual).unwrap();
    let (composer, files) = view.read_with(&visual, |view, _| {
        (view.composer.clone(), view.files.clone())
    });
    files.update(&mut visual, |files, _| files.use_remote_client(client));
    Fixture {
        composer,
        files,
        log,
        _directory: directory,
        visual,
    }
}
fn query(fixture: &mut Fixture, text: &str) {
    fixture
        .composer
        .update_in(&mut fixture.visual, |composer, window, cx| {
            composer
                .input
                .update(cx, |input, cx| input.set_content(text.to_owned(), cx));
            composer.input.focus_handle(cx).focus(window, cx);
        });
    fixture.visual.run_until_parked();
}
fn requests(fixture: &Fixture) -> Vec<serde_json::Value> {
    std::fs::read_to_string(&fixture.log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
#[gpui::test]
fn ssh_at_menu_reports_tree_errors_instead_of_silently_hiding_files(cx: &mut TestAppContext) {
    let mut fixture = setup_with_list_error(cx, true);
    query(&mut fixture, "@");
    fixture.files.read_with(&fixture.visual, |files, _| {
        assert!(files.file_entries().is_empty());
        assert!(
            files
                .browser_status()
                .unwrap()
                .contains("Remote directory unavailable")
        );
    });
    fixture.composer.read_with(&fixture.visual, |composer, _| {
        assert!(composer.mention_open());
        assert!(composer.mention.items.is_empty());
    });
    assert!(fixture.visual.debug_bounds("mention-menu").is_some());
    assert!(
        !requests(&fixture)
            .iter()
            .any(|request| request["type"] == "files_read")
    );
}

#[gpui::test]
fn ssh_at_files_load_on_first_use_preview_remote_bytes_and_insert_relative_paths(
    cx: &mut TestAppContext,
) {
    let mut fixture = setup(cx);
    query(&mut fixture, "Compare @remote");
    fixture.composer.read_with(&fixture.visual, |composer, cx| {
        assert!(composer.mention_open());
        assert_eq!(
            composer.mention.items[0].choice.mention().body,
            Body::Path {
                path: "src/remote.rs".into(),
                line: None
            }
        );
        let preview = &composer.mention.previews["src/remote.rs"];
        assert_eq!(preview.summary, "2 lines · Rust");
        assert_eq!(
            preview.lines,
            vec!["// remote host only", "    fn main() {}"]
        );
        assert!(composer.language_project(cx).is_none());
    });
    assert!(fixture.visual.debug_bounds("mention-item-0").is_some());
    fixture
        .composer
        .update_in(&mut fixture.visual, |composer, window, cx| {
            composer.choose_mention(0, window, cx)
        });
    fixture.visual.run_until_parked();
    fixture.composer.read_with(&fixture.visual, |composer, cx| {
        let input = composer.input.read(cx);
        assert_eq!(
            super::super::mentions::prompt(input.content(), input.chips(), &composer.mentions),
            "Compare @src/remote.rs"
        );
        assert_eq!(composer.draft_mentions(cx)[0].2, "path · pi reads it");
        assert!(!composer.mention_open());
    });
    assert!(
        requests(&fixture)
            .iter()
            .any(|request| request["type"] == "files_list")
    );
    assert!(
        requests(&fixture)
            .iter()
            .any(|request| request["type"] == "files_read" && request["path"] == "src/remote.rs")
    );
    assert!(
        !fixture
            .files
            .read_with(&fixture.visual, |files, _| files.has_tabs()),
        "preview must not open an editor/project"
    );
}
#[gpui::test]
fn ssh_directory_mentions_and_unicode_completion_do_not_attach_contents(cx: &mut TestAppContext) {
    let mut fixture = setup(cx);
    query(&mut fixture, "Inspect @设计/");
    fixture.composer.read_with(&fixture.visual, |composer, _| {
        let mention = composer.mention.items[0].choice.mention();
        assert_eq!(mention.kind, Kind::Directory);
        assert_eq!(
            mention.body,
            Body::Path {
                path: "docs/设计/".into(),
                line: None
            }
        );
        assert!(composer.mention.previews.is_empty());
    });
    assert!(
        !requests(&fixture)
            .iter()
            .any(|request| request["type"] == "files_read")
    );
    fixture
        .composer
        .update_in(&mut fixture.visual, |composer, window, cx| {
            composer.choose_mention(0, window, cx)
        });
    fixture.composer.read_with(&fixture.visual, |composer, cx| {
        let input = composer.input.read(cx);
        assert_eq!(
            super::super::mentions::prompt(input.content(), input.chips(), &composer.mentions),
            "Inspect @docs/设计/"
        );
    });
    query(&mut fixture, "Open @./docs/设计/hello");
    fixture
        .composer
        .update_in(&mut fixture.visual, |composer, window, cx| {
            composer.complete_mention(window, cx)
        });
    fixture.composer.read_with(&fixture.visual, |composer, cx| {
        assert_eq!(
            composer.input.read(cx).content(),
            "Open `docs/设计/hello world.md` "
        );
        assert!(!composer.mention_open());
    });
}
#[gpui::test]
fn ssh_preview_failure_never_falls_back_to_desktop_reads_or_blocks_a_mention(
    cx: &mut TestAppContext,
) {
    let mut fixture = setup(cx);
    query(&mut fixture, "Inspect @unreadable");
    fixture.composer.read_with(&fixture.visual, |composer, _| {
        let preview = &composer.mention.previews["src/unreadable.rs"];
        assert!(preview.summary.contains("Remote permission denied"));
        assert!(preview.lines.is_empty());
    });
    fixture
        .composer
        .update_in(&mut fixture.visual, |composer, window, cx| {
            composer.choose_mention(0, window, cx)
        });
    fixture.composer.read_with(&fixture.visual, |composer, cx| {
        let input = composer.input.read(cx);
        assert!(composer.mention.pending.is_empty());
        assert_eq!(
            super::super::mentions::prompt(input.content(), input.chips(), &composer.mentions),
            "Inspect @src/unreadable.rs"
        );
    });
}
