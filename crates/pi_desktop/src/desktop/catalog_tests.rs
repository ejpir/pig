use super::app_views::AppView;
use super::*;

fn show(desktop: &Entity<Desktop>, view: AppView, cx: &mut VisualTestContext) {
    show_inspector(desktop, cx);
    workspace(desktop, cx).update(cx, |workspace, cx| workspace.show_view(view, cx));
    cx.run_until_parked();
}
#[gpui::test]
fn catalog_confirmation_blocks_background_navigation_and_search(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    click("settings-tools", &mut cx);
    let answer = cx.update(|window, cx| {
        window.prompt(
            gpui::PromptLevel::Warning,
            "Confirm package operation",
            Some("No code runs in this test."),
            &["Cancel", "Continue"],
            cx,
        )
    });
    cx.run_until_parked();
    let nav = cx.debug_bounds("nav-Models").unwrap();
    cx.simulate_click(nav.center(), gpui::Modifiers::default());
    cx.simulate_keystrokes("ctrl-k ctrl-shift-d");
    cx.run_until_parked();
    assert!(workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.view.is_none()));
    assert!(cx.debug_bounds("confirmation-dialog").is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("confirmation-dialog").is_none());
    drop(answer);
}

#[gpui::test]
fn trust_restart_hint_matches_the_effective_process_state(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    show(&desktop, AppView::Resources, &mut cx);
    for trusted in [false, true] {
        receive(
            &session,
            json!({"type":"response","command":"get_project_trust","success":true,"data":{
                "cwd":"/demo/repos/pi","trusted":trusted,"hasProjectResources":true,
                "savedDecision":{"path":"/demo/repos/pi","decision":true}
            }}),
            &mut cx,
        );
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("trust-decision-pending").is_some(),
            !trusted
        );
        assert_eq!(cx.debug_bounds("trust-decision-active").is_some(), trusted);
    }
}

#[gpui::test]
fn multiline_sessions_stay_in_rows_and_sort_menu_is_above_them(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    let message =
        "Fix the language server error\n\nCannot find name agentStartMs.\u{2028}Keep the original.";
    receive(
        &session,
        json!({"type":"response","command":"list_sessions","success":true,"data":{"sessions":[
            {"id":"multi","path":"/demo/multi.jsonl","cwd":"/demo/repos/pi","firstMessage":message,"modified":"2026-09-29T12:00:00Z"},
            {"id":"named","path":"/demo/named.jsonl","cwd":"/demo/repos/pi","name":"A named\nsession","firstMessage":"Original text","modified":"2026-09-29T11:00:00Z"}
        ]}}),
        &mut cx,
    );
    show(&desktop, AppView::Sessions, &mut cx);
    let row = cx.debug_bounds("session-row-0").unwrap();
    let title = cx.debug_bounds("session-title-0").unwrap();
    assert!(
        title.origin.y >= row.origin.y
            && title.origin.y + title.size.height <= row.origin.y + row.size.height
    );
    let sort = cx.debug_bounds("session-sort").unwrap();
    cx.simulate_click(sort.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let name = cx.debug_bounds("session-sort-Name").unwrap();
    cx.simulate_click(name.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("session-sort-menu").is_none(),
        "click must reach menu, not the row below"
    );
    assert!(session.controller.read_with(&cx, |controller, _| {
        controller
            .model()
            .saved
            .iter()
            .any(|saved| saved.first_message == message)
    }));
    cx.simulate_click(sort.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("session-sort-menu").is_none());
}

fn catalog(session: &Tab, cx: &mut VisualTestContext) {
    receive(
        session,
        json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"id":"vision","provider":"a","contextWindow":200000,"maxTokens":32000,"reasoning":true,"input":["text","image"],"cost":{"input":3,"output":15}},
            {"id":"text","provider":"b","contextWindow":128000,"reasoning":false,"input":["text"]}
        ]}}),
        cx,
    );
    receive(
        session,
        json!({"type":"response","command":"get_settings","success":true,"data":{"scopedModels":["a/vision"]}}),
        cx,
    );
    receive(
        session,
        json!({"type":"response","command":"list_packages","success":true,"data":{"packages":[
            {"source":"npm:@test/kit","scope":"user","installedPath":"/offline/kit","filtered":false}
        ]}}),
        cx,
    );
    receive(
        session,
        json!({"type":"response","command":"get_commands","success":true,"data":{"commands":[
            {"name":"skill:test","source":"skill","description":"Offline skill","sourceInfo":{"path":"/offline/SKILL.md","scope":"user","origin":"top-level","source":"auto"}}
        ]}}),
        cx,
    );
    cx.run_until_parked();
}

#[gpui::test]
fn model_catalog_filters_and_inspector_follow_selection_without_changing_model(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    let before = session.controller.read_with(&cx, |controller, _| {
        controller
            .model()
            .state
            .model
            .as_ref()
            .map(|m| m.id.clone())
    });
    set_draft(&session, "Keep this draft", &mut cx);
    catalog(&session, &mut cx);
    show(&desktop, AppView::Models, &mut cx);
    assert!(cx.debug_bounds("models-screen").is_some());
    assert!(cx.debug_bounds("model-detail-a/vision").is_some());
    let heading = cx.debug_bounds("model-price-heading").unwrap();
    let price = cx.debug_bounds("model-price-0").unwrap();
    assert_eq!(heading.origin.x, price.origin.x);
    assert_eq!(heading.size.width, price.size.width);
    assert_eq!(
        cx.debug_bounds("model-cycle").unwrap().size,
        size(px(30.), px(18.))
    );
    let row = cx.debug_bounds("model-row-1").unwrap();
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("model-detail-b/text").is_some(),
        "inspector must redraw independently of shell"
    );
    let images = cx.debug_bounds("model-filter-3").unwrap();
    cx.simulate_click(images.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("model-row-1").is_none());
    assert!(cx.debug_bounds("model-detail-a/vision").is_some());
    assert_eq!(
        before,
        session.controller.read_with(&cx, |controller, _| controller
            .model()
            .state
            .model
            .as_ref()
            .map(|m| m.id.clone()))
    );
    assert_eq!(draft(&session, &cx), "Keep this draft");
    cx.simulate_resize(size(px(1000.), px(740.)));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("models-screen").unwrap();
    assert!(bounds.origin.x + bounds.size.width <= px(1000.));
    cx.simulate_keystrokes("ctrl-shift-d");
    cx.run_until_parked();
    assert!(workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.view.is_none()));
    assert!(cx.debug_bounds("session-diagnostics").is_some());
}

#[gpui::test]
fn resource_tabs_copy_original_metadata_and_switch_sessions_without_leaking(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    catalog(&session, &mut cx);
    show(&desktop, AppView::Resources, &mut cx);
    let user = cx.debug_bounds("resource-scope-user").unwrap();
    cx.simulate_click(user.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let packages = cx.debug_bounds("resource-tab-Packages").unwrap();
    cx.simulate_click(packages.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("resource-row-0").is_some());
    let before = cx.debug_bounds("resource-row-0").unwrap();
    session.controller.update(&mut cx, |controller, cx| {
        controller.notice(
            "Package configuration changed. Use Reload resources to apply it to this session.",
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("resource-row-0").unwrap(),
        before,
        "notifications must not push table rows down"
    );
    let toast = cx.debug_bounds("catalog-toast").unwrap();
    assert!(toast.size.width <= px(440.));
    let close = cx.debug_bounds("catalog-toast-close").unwrap();
    cx.simulate_click(close.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("catalog-toast").is_none());
    let row = cx.debug_bounds("resource-row-0").unwrap();
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let copy = cx.debug_bounds("resource-copy").unwrap();
    cx.simulate_click(copy.center(), gpui::Modifiers::default());
    let copied = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(copied.contains("npm:@test/kit") && copied.contains("/offline/kit"));
    let skills = cx.debug_bounds("resource-tab-Skills").unwrap();
    cx.simulate_click(skills.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let row = cx.debug_bounds("resource-row-0").unwrap();
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let copy = cx.debug_bounds("resource-copy").unwrap();
    cx.simulate_click(copy.center(), gpui::Modifiers::default());
    assert!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("/offline/SKILL.md")
    );
    let other = open(&desktop, "/demo/different-catalog", None, &mut cx);
    let other = tab(&desktop, other, &cx);
    receive(
        &other,
        json!({"type":"response","command":"get_commands","success":true,"data":{"commands":[]}}),
        &mut cx,
    );
    show(&desktop, AppView::Resources, &mut cx);
    assert!(cx.debug_bounds("resource-row-0").is_none());
    assert!(cx.debug_bounds("resource-copy").is_none());
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.tabs.len()),
        2
    );
}

#[gpui::test]
fn global_search_stays_global_inside_app_views(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    show(&desktop, AppView::Sessions, &mut cx);
    cx.simulate_keystrokes("secondary-k");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("global-search").is_some(),
        "Ctrl/Cmd+K works from any view"
    );
    cx.simulate_input("models");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.view),
        Some(AppView::Models)
    );
}
