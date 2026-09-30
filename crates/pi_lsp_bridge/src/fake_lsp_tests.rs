//! Checker behaviour against Zed's fake language server and FakeFs. Built only with
//! `--features fake-lsp`: Zed's test-support features, unified into the whole
//! workspace's test build, break crates such as `workspace` that `pi_editor` links.
use super::tests::ask;
use super::*;
use fs::{FakeFs, Fs as _};
use futures::StreamExt as _;
use gpui::TestAppContext;
use language::FakeLspAdapter;
use lsp::{Uri, notification};
use project::lsp_store::OpenLspBufferHandle;
use serde_json::json;
use settings::SettingsStore;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use util::path;

/// A server that checks the whole project on open and save, like `cargo check`:
/// a line containing `ERR` is an error in its file, and `BREAK <name>` puts an
/// error in the file `<name>`. Every known file's errors are published each time.
fn fake_checker() -> FakeLspAdapter {
    FakeLspAdapter {
        capabilities: lsp::ServerCapabilities {
            text_document_sync: Some(lsp::TextDocumentSyncCapability::Options(
                lsp::TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(lsp::TextDocumentSyncKind::FULL),
                    save: Some(lsp::TextDocumentSyncSaveOptions::SaveOptions(
                        lsp::SaveOptions {
                            include_text: Some(true),
                        },
                    )),
                    ..Default::default()
                },
            )),
            ..Default::default()
        },
        initializer: Some(Box::new(|server| {
            // File name to (URI, text).
            let files = Arc::new(Mutex::new(BTreeMap::<String, (Uri, String)>::new()));
            let publish = {
                let server = server.clone();
                move |files: &BTreeMap<String, (Uri, String)>| {
                    for (name, (uri, text)) in files {
                        let mut diagnostics = errors_in(text);
                        for (other, (_, text)) in files {
                            if text.contains(&format!("BREAK {name}")) {
                                diagnostics.push(error(0, &format!("E0308 broken by {other}")));
                            }
                        }
                        server.notify::<notification::PublishDiagnostics>(
                            lsp::PublishDiagnosticsParams {
                                uri: uri.clone(),
                                diagnostics,
                                version: None,
                            },
                        );
                    }
                }
            };
            server.handle_notification::<notification::DidOpenTextDocument, _>({
                let (files, publish) = (files.clone(), publish.clone());
                move |params, _| {
                    let mut files = files.lock().unwrap();
                    let uri = params.text_document.uri;
                    files.insert(file_name(&uri), (uri, params.text_document.text));
                    publish(&files);
                }
            });
            server.handle_notification::<notification::DidSaveTextDocument, _>(move |params, _| {
                let mut files = files.lock().unwrap();
                let uri = params.text_document.uri;
                files.insert(file_name(&uri), (uri, params.text.unwrap_or_default()));
                publish(&files);
            });
        })),
        ..Default::default()
    }
}

fn file_name(uri: &Uri) -> String {
    uri.to_string().rsplit('/').next().unwrap_or("").to_owned()
}

fn errors_in(text: &str) -> Vec<lsp::Diagnostic> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains("ERR"))
        .map(|(row, line)| error(row as u32, line.trim()))
        .collect()
}

fn error(row: u32, message: &str) -> lsp::Diagnostic {
    lsp::Diagnostic {
        range: lsp::Range::new(lsp::Position::new(row, 0), lsp::Position::new(row, 1)),
        severity: Some(lsp::DiagnosticSeverity::ERROR),
        message: message.into(),
        ..Default::default()
    }
}

/// A project whose server is running and has `b.rs` open.
async fn project(
    files: serde_json::Value,
    cx: &mut TestAppContext,
) -> (Arc<FakeFs>, Entity<Project>, OpenLspBufferHandle) {
    cx.update(|cx| {
        let settings = SettingsStore::test(cx);
        cx.set_global(settings);
        release_channel::init(release_channel::AppVersion::load("0.0.0", None, None), cx);
    });
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(path!("/dir"), files).await;
    let project = Project::test(fs.clone(), [Path::new(path!("/dir"))], cx).await;
    let languages = project.read_with(cx, |p, _| p.languages().clone());
    languages.add(language::rust_lang());
    let mut servers = languages.register_fake_lsp("Rust", fake_checker());
    let (_, open) = project
        .update(cx, |p, cx| {
            p.open_local_buffer_with_lsp(path!("/dir/b.rs"), cx)
        })
        .await
        .unwrap();
    servers.next().await.unwrap();
    cx.run_until_parked();
    (fs, project, open)
}

fn checker(project: &Entity<Project>, cx: &mut TestAppContext) -> Entity<Checker> {
    let project = project.clone();
    cx.new(|_| Checker::new(move |_| Some(project.clone())))
}

#[gpui::test]
async fn an_edit_reports_its_file_and_new_errors_elsewhere(cx: &mut TestAppContext) {
    let (fs, project, _open) = project(
        json!({ "a.rs": "fn a() {}\n", "b.rs": "fn b() {}\n// ERR old\n" }),
        cx,
    )
    .await;
    let checker = checker(&project, cx);

    fs.save(
        Path::new(path!("/dir/a.rs")),
        &"fn a() {}\n// ERR mismatched types\n// BREAK b.rs\n".into(),
        Default::default(),
    )
    .await
    .unwrap();
    let text = ask(
        &checker,
        json!({"op": "file", "path": path!("/dir/a.rs")}),
        cx,
    )
    .await;
    assert_eq!(
        text,
        "Language server errors in a.rs:\n  2:1 // ERR mismatched types\n\
         New errors in other files:\n  b.rs:1:1 E0308 broken by a.rs\n",
        "b.rs's older error is not new"
    );

    fs.save(
        Path::new(path!("/dir/a.rs")),
        &"fn a() {}\n".into(),
        Default::default(),
    )
    .await
    .unwrap();
    let text = ask(
        &checker,
        json!({"op": "file", "path": path!("/dir/a.rs")}),
        cx,
    )
    .await;
    assert_eq!(text, "a.rs has no language server errors now.\n");

    // Saying nothing is the usual answer.
    let text = ask(
        &checker,
        json!({"op": "file", "path": path!("/dir/a.rs")}),
        cx,
    )
    .await;
    assert_eq!(text, "");
}

#[gpui::test]
async fn a_run_end_reports_files_changed_without_edit(cx: &mut TestAppContext) {
    let (fs, project, _open) = project(json!({ "a.rs": "", "b.rs": "" }), cx).await;
    let checker = checker(&project, cx);
    assert_eq!(ask(&checker, json!({"op": "run_end"}), cx).await, "");

    // As if `bash` ran a code generator: no edit or write names these files.
    fs.insert_file(path!("/dir/generated.rs"), b"// ERR from bash\n".to_vec())
        .await;
    fs.insert_file(path!("/dir/notes.md"), b"ERR is not code\n".to_vec())
        .await;
    cx.run_until_parked();
    let text = ask(&checker, json!({"op": "run_end"}), cx).await;
    assert_eq!(
        text,
        "Language server errors that appeared during this run:\n  generated.rs:1:1 // ERR from bash\n"
    );
    assert_eq!(ask(&checker, json!({"op": "run_end"}), cx).await, "");
}
