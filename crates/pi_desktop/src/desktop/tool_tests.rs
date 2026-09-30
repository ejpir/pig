use super::*;

#[gpui::test]
fn tool_details_start_collapsed_and_support_drag_keyboard_and_context_menu_copy(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/selection", None, &mut cx);
    let a = tab(&desktop, id, &cx);
    receive(
        &a,
        json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"edit","name":"edit","arguments":{"path":"a.txt","oldText":"old","newText":"EDIT_SELECTED"}},
            {"type":"toolCall","id":"write","name":"write","arguments":{"path":"b.txt","content":"WRITE_SELECTED text\nsecond line"}},
            {"type":"toolCall","id":"bash","name":"bash","arguments":{"command":"printf BASH_SELECTED"}}
        ]}}),
        &mut cx,
    );
    receive(
        &a,
        json!({"type":"tool_execution_end","toolCallId":"edit","result":{"content":[{"type":"text","text":"Edited file"}],"details":{"diff":"-1 old\n+1 EDIT_SELECTED text"}}}),
        &mut cx,
    );
    receive(
        &a,
        json!({"type":"tool_execution_end","toolCallId":"write","result":{"content":[{"type":"text","text":"Wrote file"}]}}),
        &mut cx,
    );
    receive(
        &a,
        json!({"type":"tool_execution_end","toolCallId":"bash","result":{"content":[{"type":"text","text":"BASH_SELECTED text\nsecond line"}]}}),
        &mut cx,
    );
    cx.run_until_parked();
    for name in [
        "tool-details-edit",
        "tool-details-write",
        "tool-details-bash",
    ] {
        assert!(cx.debug_bounds(name).is_none());
    }
    a.transcript.read_with(&cx, |view, _| {
        assert!(
            view.documents.get("tool:edit:diff").is_none(),
            "collapsed details should not be parsed"
        )
    });
    expand_activity_for_tool(&a, "edit", &mut cx);
    for (header, details, key, marker) in [
        (
            "tool-header-edit",
            "tool-edit-diff",
            "tool:edit:diff",
            "EDIT_SELECTED",
        ),
        (
            "tool-header-write",
            "tool-write-content",
            "tool:write:content",
            "WRITE_SELECTED",
        ),
        (
            "tool-header-bash",
            "tool-bash-output",
            "tool:bash:output",
            "BASH_SELECTED",
        ),
    ] {
        let header_bounds = cx.debug_bounds(header).unwrap();
        cx.simulate_click(header_bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        let document = a
            .transcript
            .read_with(&cx, |view, _| view.documents.get(key).unwrap().clone());
        let bounds = cx.debug_bounds(details).unwrap();
        let mut selected_at = None;
        // The renderer owns the code toolbar; find a rendered line without assuming
        // a platform-specific font baseline underneath it.
        for dy in (4..f32::from(bounds.size.height) as usize).step_by(4) {
            let start = bounds.origin + point(px(1.), px(dy as f32));
            let end = point(bounds.origin.x + bounds.size.width - px(50.), start.y);
            cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
            cx.simulate_mouse_move(end, Some(MouseButton::Left), gpui::Modifiers::default());
            cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::default());
            cx.run_until_parked();
            if document.read_with(&cx, |document, _| {
                document
                    .selected_source()
                    .is_some_and(|source| source.contains(marker))
            }) {
                selected_at = Some(start);
                break;
            }
        }
        let position = selected_at.unwrap_or_else(|| {
            let selection = document.read_with(&cx, |document, _| document.selected_source().map(str::to_owned));
            panic!("{key}: dragging never selected {marker}; bounds={bounds:?}, last selection={selection:?}")
        });
        // Exercise native-like frame reuse before dispatching the copy shortcut.
        for _ in 0..3 {
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
            cx.run_until_parked();
        }
        cx.simulate_keystrokes("secondary-c");
        let copied = cx.read(|cx| {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .unwrap()
        });
        assert!(copied.contains(marker), "keyboard copy: {copied:?}");
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into())));
        cx.simulate_mouse_down(position, MouseButton::Right, gpui::Modifiers::default());
        cx.simulate_mouse_up(position, MouseButton::Right, gpui::Modifiers::default());
        cx.run_until_parked();
        // Zed's deferred context menu focuses after two platform frames.
        for _ in 0..2 {
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
            cx.run_until_parked();
        }
        cx.simulate_keystrokes("home enter");
        cx.run_until_parked();
        let copied = cx.read(|cx| {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .unwrap()
        });
        assert!(
            copied.contains(marker),
            "{key} context menu copy: {copied:?}"
        );
        let header_bounds = cx.debug_bounds(header).unwrap();
        cx.simulate_click(header_bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds(details).is_none());
    }
}
