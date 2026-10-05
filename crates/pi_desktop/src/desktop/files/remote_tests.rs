use super::*;
use pi_core::remote_files::Document;

#[gpui::test]
fn ssh_editor_buffers_are_detached_and_keep_edits_on_conflict(cx: &mut gpui::TestAppContext) {
    let target = pi_core::ssh::SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
    let root = target.identity();
    cx.update(|cx| { cx.set_global(Theme::new(false)); crate::desktop::init(cx); });
    let files = cx.new(|cx| FilesView::new(root.clone(), false, cx).with_remote(target));
    let buffer = cx.update(|cx| pi_editor::detached_buffer(std::path::Path::new("code.rs"), "fn main() {}\r\n", cx).unwrap());
    assert!(buffer.read_with(cx, |buffer,_| buffer.file().is_none()));
    assert!(!buffer.read_with(cx, |buffer,_| buffer.is_dirty()));
    files.update(cx, |files,cx| {
        files.apply_remote_document(root.join("code.rs"), buffer.clone(), Document {path:"code.rs".into(),text:"first\r\n".into(), revision:"first-hash".into()},cx);
        files.add_tab(root.join("code.rs"),buffer.clone(),cx);
        assert!(files.host.is_none());
        assert!(!files.mutation_enabled());
        assert!(pi_editor::language_project(&root,cx).is_none());
    });
    buffer.update(cx, |buffer,cx| buffer.set_text("unsaved",cx));
    assert!(files.read_with(cx, |files,cx| files.has_unsaved(cx)));
    buffer.update(cx, |buffer,_| buffer.set_conflict());
    assert_eq!(buffer.read_with(cx, |buffer,_| buffer.text()), "unsaved");
    assert!(buffer.read_with(cx, |buffer,_| buffer.has_conflict()));
    files.update(cx, |files,cx| files.apply_remote_document(root.join("code.rs"),buffer.clone(),Document {path:"code.rs".into(),text:"reloaded\r\n".into(),revision:"latest".into()},cx));
    assert!(!buffer.read_with(cx, |buffer,_| buffer.has_conflict() || buffer.is_dirty()));
    assert_eq!(buffer.read_with(cx, |buffer,_| buffer.line_ending()),language::LineEnding::Windows);
}
