//! Regression: work grows with trace length, rather than only testing startup frames.
use pi_visual::Runtime;
const SOURCE: &str = include_str!("../../pi_visual/assets/samples/spirograph-trace.pi-visual.js");

#[test]
fn growing_spirograph_draws_through_completion() {
    let mut app = Runtime::from_bundle(SOURCE).unwrap();
    app.scene("scene").unwrap();
    app.dispatch("play").unwrap();
    for step in 0..=30 {
        app.tick(0.1).unwrap();
        app.scene("scene")
            .unwrap_or_else(|error| panic!("frame {step}: {error:#}"));
    }
    assert_eq!(app.number("progress"), Some(1.0));
    assert!(!app.animating());
    let complete = app.scene("scene").unwrap();
    assert!(complete.len() > 8);
    // Finish-at-100% and late restore must not depend on earlier cached frames.
    let state = app.state_json().unwrap();
    let mut fresh = Runtime::from_bundle(SOURCE).unwrap();
    fresh.restore_state(&state).unwrap();
    fresh.scene("scene").unwrap();
    for (bind, value) in [("ring", 110.0), ("wheel", 36.0), ("pen", 84.0)] {
        app.slide(bind, value).unwrap();
    }
    let mut last = app.state_json().unwrap();
    last["progress"] = serde_json::json!(1.0);
    app.restore_state(&last).unwrap();
    app.scene("scene").unwrap();
}
