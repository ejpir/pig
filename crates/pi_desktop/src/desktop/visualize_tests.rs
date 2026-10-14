use super::*;
use crate::desktop::visualize;
const COUNTER: &str = "app({title:'Counter',state:{n:0},view:ui.column([ui.text(s=>String(s.n)),ui.button('Add',s=>{s.n++})])});";
fn load(tab: &Tab, source: &str, cx: &mut VisualTestContext) {
    cx.simulate_resize(size(px(1344.), px(1100.)));
    cx.update(|window, _| window.activate_window());
    for record in visualize::sample_records(source) {
        receive(tab, record, cx);
    }
    cx.run_until_parked();
}
fn number(tab: &Tab, title: &str, field: &str, cx: &VisualTestContext) -> f64 {
    tab.transcript.read_with(cx, |view, _| {
        view.visuals
            .borrow()
            .running
            .values()
            .filter_map(|i| i.runtime.as_ref().ok())
            .find(|r| r.definition.title == title)
            .unwrap()
            .number(field)
            .unwrap()
    })
}
#[gpui::test]
fn native_counter_uses_shared_controls_not_hidden_markdown_or_agent_calls(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let tab = tab(&desktop, 0, &cx);
    load(&tab, COUNTER, &mut cx);
    assert!(cx.debug_bounds("visual-app").is_some());
    tab.transcript.read_with(&cx, |v, _| {
        assert!(v.documents.get("1:visual-0-1").is_none())
    });
    click("visual-button-Add", &mut cx);
    assert_eq!(number(&tab, "Counter", "n", &cx), 1.);
    tab.controller
        .read_with(&cx, |c, _| assert_eq!(c.model().messages.len(), 2));
    // Prose changes cannot reset the program; scope/source changes do.
    let mut records = visualize::sample_records(COUNTER);
    let prose = format!(
        "Extra introduction.\n{}",
        records[1]["data"]["messages"][1]["content"][0]["text"]
            .as_str()
            .unwrap()
    );
    records[1]["data"]["messages"][1]["content"][0]["text"] = json!(prose);
    records[2]["data"]["entries"][1]["message"] = records[1]["data"]["messages"][1].clone();
    for record in records {
        receive(&tab, record, &mut cx);
    }
    cx.run_until_parked();
    assert_eq!(number(&tab, "Counter", "n", &cx), 1.);
    load(&tab, &COUNTER.replace("Counter", "Changed"), &mut cx);
    assert_eq!(number(&tab, "Changed", "n", &cx), 0.);
}
#[gpui::test]
fn completed_fence_is_inert_until_the_message_and_identity_are_complete(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let tab = tab(&desktop, 0, &cx);
    let records = visualize::sample_records(COUNTER);
    receive(&tab, records[0].clone(), &mut cx);
    receive(
        &tab,
        json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[records[1]["data"]["messages"][0].clone()]}}),
        &mut cx,
    );
    let assistant = records[1]["data"]["messages"][1].clone();
    receive(
        &tab,
        json!({"type":"message_start","message":assistant}),
        &mut cx,
    );
    receive(&tab, records[2].clone(), &mut cx);
    cx.run_until_parked();
    assert!(cx.debug_bounds("visual-app").is_none());
    receive(
        &tab,
        json!({"type":"message_end","message":assistant}),
        &mut cx,
    );
    receive(&tab, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    assert!(cx.debug_bounds("visual-app").is_some());
}
#[gpui::test]
fn native_slider_navigation_cancels_transaction_before_hidden_callbacks(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let first = tab(&desktop, 0, &cx);
    load(
        &first,
        "app({title:'Slider',state:{x:10},view:ui.slider('X','x',{min:0,max:100,step:1})});",
        &mut cx,
    );
    let bounds = cx.debug_bounds("visual-slider-x").unwrap();
    let position = point(bounds.right() - px(20.), bounds.center().y);
    cx.simulate_mouse_down(
        position,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(number(&first, "Slider", "x", &cx) > 50.);
    open(&desktop, "/demo/other", None, &mut cx);
    assert_eq!(number(&first, "Slider", "x", &cx), 10.);
    cx.simulate_mouse_up(
        position,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert_eq!(number(&first, "Slider", "x", &cx), 10.);
}
#[gpui::test]
fn reserved_mention_inserts_without_submitting_and_submission_expands_once(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let tab = tab(&desktop, 0, &cx);
    let before = tab
        .controller
        .read_with(&cx, |c, _| c.model().messages.len());
    set_draft(&tab, "@Vis", &mut cx);
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(draft(&tab, &cx), "@Visualize ");
    tab.controller
        .read_with(&cx, |c, _| assert_eq!(c.model().messages.len(), before));
    set_draft(&tab, "@Visualize a counter", &mut cx);
    cx.simulate_keystrokes("alt-enter");
    cx.run_until_parked();
    tab.controller.read_with(&cx, |c, _| {
        let sent = c.model().follow_up.last().unwrap();
        assert!(pi_visual::prompt::admitted(sent));
        assert_eq!(sent.matches(pi_visual::prompt::CONTRACT).count(), 1);
    });
}

#[gpui::test]
fn text_only_animation_has_a_mount_lease_and_stops_on_blur(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let tab = tab(&desktop, 0, &cx);
    load(
        &tab,
        "app({title:'Timer',state:{n:0,playing:true},view:ui.text(s=>String(s.n)),animation:{running:'playing',fps:1,tick(s){s.n++}}});",
        &mut cx,
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(number(&tab, "Timer", "n", &cx) > 0.);
    cx.deactivate_window();
    cx.run_until_parked();
    let before = number(&tab, "Timer", "n", &cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(number(&tab, "Timer", "n", &cx), before);
}
#[gpui::test]
fn full_screen_scales_the_drawing_and_escape_returns_to_the_thread(cx: &mut TestAppContext) {
    const SCENE: &str = "app({title:'Scene',state:{},view:ui.column([ui.canvas({id:'scene',label:'Scene',height:120,viewBox:[0,0,100,100],draw(g,s){}})])});";
    let (desktop, mut cx) = setup(cx);
    let tab = tab(&desktop, 0, &cx);
    load(&tab, SCENE, &mut cx);
    let inline = cx.debug_bounds("visual-canvas-root-0").unwrap();
    assert!(cx.debug_bounds("visual-full-screen").is_none());
    click("visual-expand", &mut cx);
    let full = cx.debug_bounds("visual-canvas-root-0").unwrap();
    assert!(cx.debug_bounds("visual-full-screen").is_some());
    // About two thirds of the 1100px window, not the card's 120px.
    assert!(full.size.height > inline.size.height * 4., "{full:?}");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("visual-full-screen").is_none());
    assert_eq!(
        cx.debug_bounds("visual-canvas-root-0").unwrap().size.height,
        inline.size.height
    );
}
