use super::*;

const METRICS: &str =
    "TPS 87.3 tok/s. out 2,344, in 8,132, cache r/w 13,824/0, total 24,300, 26.9s";

#[gpui::test]
fn run_metrics_stay_in_the_small_footer_without_a_banner_or_session_leak(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    receive(
        &a,
        json!({"type":"extension_ui_request","method":"notify","notifyType":"info","message":METRICS}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("session-notice").is_none());
    let footer = cx.debug_bounds("status-turn-metrics").unwrap();
    assert!(footer.origin.y >= px(716.));
    assert!(footer.size.height <= px(24.));
    assert!(footer.size.width > px(100.));
    a.controller.read_with(&cx, |c, _| {
        assert_eq!(c.model().turn_metrics.as_deref(), Some(METRICS));
        assert!(c.model().notice.is_none());
    });
    receive(
        &a,
        json!({"type":"extension_ui_request","method":"notify","notifyType":"warning","message":"Please check the configuration"}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("session-notice").is_some());
    let id = open(&desktop, "/demo/other-metrics", None, &mut cx);
    let b = tab(&desktop, id, &cx);
    b.controller
        .read_with(&cx, |c, _| assert!(c.model().turn_metrics.is_none()));
    select(&desktop, 0, &mut cx);
    a.controller.read_with(&cx, |c, _| {
        assert_eq!(c.model().turn_metrics.as_deref(), Some(METRICS))
    });
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.run_until_parked();
    let footer = cx.debug_bounds("status-turn-metrics").unwrap();
    assert!(footer.origin.x + footer.size.width <= px(1000.));
    assert!(footer.origin.y >= px(656.));
    assert!(footer.size.width > px(50.));
    receive(&a, json!({"type":"agent_start"}), &mut cx);
    cx.run_until_parked();
    a.controller
        .read_with(&cx, |c, _| assert!(c.model().turn_metrics.is_none()));
}

#[gpui::test]
fn tool_inventory_uses_current_metadata_and_preserves_original_details(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    show_inspector(&desktop, &mut cx);
    let id = open(&desktop, "/demo/tool-metadata", None, &mut cx);
    let a = tab(&desktop, id, &cx);
    let description = format!(
        "Original description\n{}",
        "Long reported tool description. ".repeat(12)
    );
    receive(
        &a,
        json!({"type":"response","command":"get_active_tools","success":true,"data":{"activeTools":[{"name":"read","description":"Read a file"},{"name":"fixture_probe","description":description,"sourceInfo":{"path":"/offline/extensions/probe.ts","source":"local","scope":"project","origin":"top-level"}}]}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-inspector").is_some());
    let inventory = cx.debug_bounds("inspector-active-tools").unwrap();
    for selector in ["active-tool-0", "active-tool-1"] {
        let row = cx.debug_bounds(selector).unwrap();
        assert!(row.origin.x >= inventory.origin.x);
        assert!(row.origin.x + row.size.width <= inventory.origin.x + inventory.size.width);
        assert_eq!(
            row.size.height,
            px(20.),
            "Only the name is visible; original metadata stays in the tooltip"
        );
    }
    a.controller.read_with(&cx, |c, _| {
        assert!(
            c.model().tools.is_empty(),
            "No historical calls were used to build the list"
        );
        let tools = c.model().state.active_tools.as_ref().unwrap();
        assert_eq!(tools[1].description.as_deref(), Some(description.as_str()));
        assert_eq!(
            tools[1].source_info.as_ref().unwrap().path,
            "/offline/extensions/probe.ts"
        );
    });
    let other = open(&desktop, "/demo/no-tool-metadata", None, &mut cx);
    assert!(cx.debug_bounds("active-tool-0").is_none());
    select(&desktop, id, &mut cx);
    assert!(cx.debug_bounds("active-tool-1").is_some());
    // A reported empty loadout is not an unavailable inventory.
    receive(
        &a,
        json!({"type":"response","command":"get_active_tools","success":true,"data":{"activeTools":[]}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("active-tools-empty").is_some());
    assert!(cx.debug_bounds("active-tools-unreported").is_none());
    assert!(cx.debug_bounds("active-tool-0").is_none());
    select(&desktop, other, &mut cx);
    assert!(cx.debug_bounds("active-tools-unreported").is_some());
    assert!(cx.debug_bounds("active-tools-empty").is_none());
}
