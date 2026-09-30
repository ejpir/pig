use super::*;
use crate::desktop::panels::SessionPage;

#[gpui::test]
fn tree_navigation_is_session_local_and_never_submits_the_hidden_draft(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let first = tab(&desktop, 0, &cx);
    set_draft(&first, "Keep this draft", &mut cx);
    let before = first
        .controller
        .read_with(&cx, |c, _| c.model().messages.len());
    let bounds = cx.debug_bounds("tab-tree").unwrap();
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("tree-view").is_some());
    let tree = first.view.read_with(&cx, |v, _| v.tree.clone());
    assert_eq!(
        tree.read_with(&cx, |v, _| v.selected.clone()).as_deref(),
        Some("live-answer")
    );
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    assert_eq!(
        tree.read_with(&cx, |v, _| v.selected.clone()).as_deref(),
        Some("live-user")
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(draft(&first, &cx), "Keep this draft");
    assert_eq!(
        first
            .controller
            .read_with(&cx, |c, _| c.model().messages.len()),
        before
    );
    let second_id = open(&desktop, "/demo/other", None, &mut cx);
    select(&desktop, 0, &mut cx);
    assert_eq!(first.view.read_with(&cx, |v, _| v.page), SessionPage::Tree);
    assert_eq!(
        tree.read_with(&cx, |v, _| v.selected.clone()).as_deref(),
        Some("live-user")
    );
    assert_eq!(
        tab(&desktop, second_id, &cx)
            .view
            .read_with(&cx, |v, _| v.page),
        SessionPage::Thread
    );
}

#[gpui::test]
fn tree_rows_match_the_study_pitch_and_filters_keep_selection_valid(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let first = tab(&desktop, 0, &cx);
    let bounds = cx.debug_bounds("tab-tree").unwrap();
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let a = cx.debug_bounds("tree-entry-0").unwrap();
    let b = cx.debug_bounds("tree-entry-1").unwrap();
    assert_eq!(a.size.height, px(36.));
    assert_eq!(b.origin.y - a.origin.y, px(36.));
    // One segmented control, not five separate bordered buttons.
    let first_filter = cx.debug_bounds("tree-filter-0").unwrap();
    let next_filter = cx.debug_bounds("tree-filter-1").unwrap();
    // TestAppContext's synthetic font metrics differ from the native font. Test
    // connected segments here; exact native geometry is checked under Xvfb.
    assert!(f32::from(next_filter.left() - first_filter.right()).abs() <= 1.);
    let user = cx.debug_bounds("tree-filter-2").unwrap();
    cx.simulate_click(user.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let selected = first
        .view
        .read_with(&cx, |v, cx| v.tree.read(cx).selected.clone())
        .unwrap();
    first.controller.read_with(&cx, |c, _| {
        assert_eq!(
            c.model()
                .history
                .as_ref()
                .unwrap()
                .entry(&selected)
                .unwrap()["message"]["role"],
            "user"
        )
    });
}
