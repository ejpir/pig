use pi_visual::Runtime;

const SOURCE: &str = include_str!("../../pi_visual/assets/samples/spirograph.pi-visual.js");

#[test]
fn spirograph_runs_within_budget_and_all_controls_change_the_scene() {
    let mut app = Runtime::from_bundle(SOURCE).unwrap();
    let initial = app.scene("scene").unwrap();
    assert_eq!(initial.len(), 8);
    app.dispatch("play").unwrap();
    for _ in 0..10 {
        app.tick(1.0 / 30.0).unwrap();
        app.scene("scene").unwrap();
    }
    assert!(app.number("angle").unwrap() > 0.0);
    app.dispatch("play").unwrap();
    let paused = app.number("angle");
    app.tick(0.1).unwrap();
    assert_eq!(paused, app.number("angle"));
    for (bind, value) in [
        ("ring", 110.0),
        ("wheel", 36.0),
        ("pen", 84.0),
        ("speed", 2.5),
    ] {
        app.slide(bind, value).unwrap();
        app.scene("scene").unwrap();
    }
    assert_eq!(app.number("turns"), Some(18.0));
    assert_eq!(app.text("closure"), "Closes after 18 turns · ratio 110:36");
    for (bind, value) in [
        ("ring", 60.0),
        ("wheel", 58.0),
        ("pen", 0.0),
        ("speed", 0.2),
    ] {
        app.slide(bind, value).unwrap();
        app.scene("scene").unwrap();
    }
    app.dispatch("reset").unwrap();
    assert!(!app.animating());
    assert_eq!(app.number("ring"), Some(96.0));
}
