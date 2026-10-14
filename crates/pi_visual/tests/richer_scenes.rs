use pi_visual::{InputEvent, Runtime, Shape, input::Phase, profile::Stage};
use serde_json::json;
use std::sync::Arc;

fn drawing(draw: &str) -> String {
    format!(
        "app({{title:'Shapes',state:{{}},view:ui.canvas({{id:'scene',draw(g,s){{{draw}}}}})}});"
    )
}
#[test]
fn compound_beziers_gradients_groups_and_canvas_text() {
    let app=Runtime::from_bundle(&drawing("const p=ui.path([['M',0,0],['Q',20,0,20,20],['C',20,30,0,30,0,20],['Z']]);g.group({translate:[25,25],rotate:30,scale:2},g=>g.path(p,{fill:ui.linearGradient(45,'#000000','#FFFFFF'),color:'#70B8FF',fillRule:'evenodd'}));g.text('Score 42',50,80,{align:'center',font:'mono'});")).unwrap();
    let scene = app.scene("scene").unwrap();
    let Shape::Group { children, .. } = &scene[0] else {
        panic!()
    };
    assert!(matches!(&children[0], Shape::Path { even_odd: true, .. }));
    assert!(matches!(&scene[1],Shape::Text{text,..} if text=="Score 42"));
}
#[test]
fn bounded_rich_geometry_rejects_invalid_values_before_native_paint() {
    for draw in [
        "g.path(ui.path([['L',0,0]]),{fill:'#FFFFFF'});",
        "g.path(ui.path([['M',0,0],['Z'],['L',2,3]]),{fill:'#FFFFFF'});",
        "g.path(ui.path([['M',0,0],['L',2,3]]));",
        "g.path(ui.path([['M',0,0],['Q',1,2,3,4]]),{fillRule:'bogus',fill:'#FFFFFF'});",
        "g.rect(0,0,2,2,{fill:ui.linearGradient(NaN,'#000000','#FFFFFF')});",
        "g.rect(0,0,2,2,{fill:ui.linearGradient(0,'red','#FFFFFF')});",
        "g.text('a\\nb',0,0);",
        "g.text('x',0,0,{font:'remote'});",
        "g.text('x',0,0,{color:ui.linearGradient(0,'#000000','#FFFFFF')});",
        "g.group({rotate:20},g=>g.text('x',0,0));",
        "g.group({scale:[1,2]},g=>g.text('x',0,0));",
        "g.group({scale:100},g=>g.text('x',0,0));",
        "function nest(n){if(n)g.group({},()=>nest(n-1));}nest(9);",
        "const p=ui.path([['M',0,0],['C',1,2,3,4,5,6]]);for(let i=0;i<63;i++)g.path(p,{fill:'#FFFFFF'});",
    ] {
        let app = Runtime::from_bundle(&drawing(draw)).unwrap();
        assert!(app.scene("scene").is_err(), "accepted {draw}");
    }
}
#[test]
fn layers_are_dependency_scoped_isolated_and_atomic() {
    let source = r##"app({title:'Layers',state:{x:1,unused:0,fail:false},view:ui.canvas({id:'scene',layers:[
      {deps:[],draw(g,s){if(Object.keys(s).length)throw Error('static snapshot');g.rect(0,0,10,10);}},
      {deps:['x','fail'],draw(g,s){if('unused' in s)throw Error('dependency leak');if(s.fail)throw Error('failed layer');g.circle(s.x,0,2);}}
    ]})});"##;
    let mut app = Runtime::from_bundle(source).unwrap();
    app.enable_profiling(true);
    let scene = app.scene("scene").unwrap();
    assert_eq!(
        app.take_profile()
            .times
            .iter()
            .filter(|(s, _)| *s == Stage::DrawScript)
            .count(),
        2
    );
    app.restore_state(&json!({"x":1,"unused":2,"fail":false}))
        .unwrap();
    assert_eq!(*app.scene("scene").unwrap(), *scene);
    assert!(
        !app.take_profile()
            .times
            .iter()
            .any(|(s, _)| *s == Stage::DrawScript)
    );
    app.restore_state(&json!({"x":2,"unused":2,"fail":false}))
        .unwrap();
    let good = app.scene("scene").unwrap();
    assert_ne!(*good, *scene);
    assert_eq!(
        app.take_profile()
            .times
            .iter()
            .filter(|(s, _)| *s == Stage::DrawScript)
            .count(),
        1
    );
    app.restore_state(&json!({"x":3,"unused":2,"fail":true}))
        .unwrap();
    assert!(app.scene("scene").is_err());
    assert!(Arc::ptr_eq(&good, &app.last_scene("scene").unwrap()));
    app.restore_state(&json!({"x":2,"unused":2,"fail":false}))
        .unwrap();
    app.take_profile();
    assert_eq!(*app.scene("scene").unwrap(), *good);
    assert!(
        !app.take_profile()
            .times
            .iter()
            .any(|(s, _)| *s == Stage::DrawScript)
    );
}
#[test]
fn layer_limits_apply_to_the_composed_scene() {
    let source = "app({title:'Budget',state:{},view:ui.canvas({id:'scene',layers:[{deps:[],draw(g){for(let i=0;i<33;i++)g.rect(0,0,1,1);}},{deps:[],draw(g){for(let i=0;i<33;i++)g.rect(0,0,1,1);}}]})});";
    assert!(
        Runtime::from_bundle(source)
            .unwrap()
            .scene("scene")
            .is_err()
    );
    for canvas in [
        "{layers:[{deps:['missing'],draw(g){}}]}",
        "{layers:[{deps:['x','x'],draw(g){}}]}",
        "{layers:[{deps:[],draw(g){}}],draw(g){}}",
        "{draw(g){},onInput(){},onPointer(){}}",
    ] {
        assert!(
            Runtime::from_bundle(&format!(
                "app({{title:'Bad',state:{{x:0}},view:ui.canvas({canvas})}})"
            ))
            .is_err()
        );
    }
}
fn input_app() -> Runtime {
    Runtime::from_bundle(r#"app({title:'Input',state:{playing:true,count:0,phase:'',held:0,buttons:0,keys:0},view:ui.column([
        ui.canvas({id:'scene',draw(g){},inputId:'input',onInput(s,e,i){s.count++;s.phase=e.kind==='pointer'?e.phase:e.kind;s.held=i.pointers.length;s.keys=i.keys.length;if(e.kind==='key'&&e.key==='bad')throw Error('bad key');}}),
        ui.holdButton('Left','left',{id:'left'})
    ]),animation:{running:'playing',tick(s,dt,i){s.held=i.pointers.length;s.buttons=i.buttons.length;s.keys=i.keys.length;}}});"#).unwrap()
}
fn pointer(id: u64, phase: Phase) -> InputEvent {
    InputEvent::Pointer {
        id,
        phase,
        x: 10.,
        y: 20.,
    }
}
#[test]
fn contacts_release_without_motion_and_do_not_enter_saved_state() {
    let mut app = input_app();
    app.input("input", pointer(1, Phase::Down)).unwrap();
    app.input("input", pointer(2, Phase::Down)).unwrap();
    assert_eq!(app.number("held"), Some(2.));
    app.input("input", pointer(1, Phase::Up)).unwrap();
    assert_eq!(app.number("held"), Some(1.));
    assert_eq!(app.state_json().unwrap()["phase"], "up");
    app.input("input", pointer(2, Phase::Cancel)).unwrap();
    assert!(app.input_state().pointers.is_empty());
    assert_eq!(app.state_json().unwrap().as_object().unwrap().len(), 6);
    assert!(app.input("missing", pointer(3, Phase::Down)).is_err());
    assert!(app.input("input", pointer(3, Phase::Move)).is_err());
}
#[test]
fn held_buttons_keyboard_failure_and_lifecycle_cleanup() {
    let mut app = input_app();
    app.input("left", pointer(1, Phase::Down)).unwrap();
    app.tick(0.01).unwrap();
    assert_eq!(app.number("buttons"), Some(1.));
    app.input("left", pointer(1, Phase::Up)).unwrap();
    app.tick(0.01).unwrap();
    assert_eq!(app.number("buttons"), Some(0.));
    app.input(
        "input",
        InputEvent::Key {
            key: "a".into(),
            down: true,
            repeat: false,
        },
    )
    .unwrap();
    app.input(
        "input",
        InputEvent::Key {
            key: "a".into(),
            down: true,
            repeat: true,
        },
    )
    .unwrap();
    assert_eq!(app.input_state().keys.len(), 1);
    let state = app.state_json().unwrap();
    assert!(
        app.input(
            "input",
            InputEvent::Key {
                key: "bad".into(),
                down: true,
                repeat: false
            }
        )
        .is_err()
    );
    assert_eq!(app.state_json().unwrap(), state); // callback rollback, physical input stays truthful
    app.cancel_input();
    assert!(app.input_state().keys.is_empty());
    app.tick(0.01).unwrap();
    assert_eq!(app.number("keys"), Some(0.));
    app.input(
        "left",
        InputEvent::Key {
            key: "space".into(),
            down: true,
            repeat: false,
        },
    )
    .unwrap();
    app.tick(0.01).unwrap();
    assert_eq!(app.number("buttons"), Some(1.));
    app.input("left", InputEvent::Blur).unwrap();
    app.tick(0.01).unwrap();
    assert_eq!(app.number("buttons"), Some(0.));
}
#[test]
fn snapshots_are_frozen_and_contact_count_is_bounded() {
    let mut app = input_app();
    for id in 0..10 {
        app.input("input", pointer(id, Phase::Down)).unwrap();
    }
    assert!(app.input("input", pointer(11, Phase::Down)).is_err());
    assert!(app.input("input", pointer(0, Phase::Down)).is_err());
    let mut frozen=Runtime::from_bundle("app({title:'Frozen',state:{playing:true},view:ui.canvas({draw(g){}}),animation:{running:'playing',tick(s,dt,i){i.keys.push('x');}}})").unwrap();
    assert!(frozen.tick(0.01).is_err());
    assert!(frozen.input_state().keys.is_empty());
}
#[test]
fn rich_scene_sample_runs_without_images_or_external_capabilities() {
    let mut app = Runtime::from_bundle(include_str!(
        "../../pi_visual/assets/samples/rich-scenes.pi-visual.js"
    ))
    .unwrap();
    app.scene("flight").unwrap();
    app.input("flight-input", pointer(1, Phase::Down)).unwrap();
    app.scene("flight").unwrap();
    app.cancel_input();
    app.scene("flight").unwrap();
    assert_eq!(app.number("contacts"), Some(0.));
}
#[test]
fn layer_globals_cannot_make_output_depend_on_cache_history() {
    let source = "let scratch=0;app({title:'Isolation',state:{x:0},view:ui.canvas({id:'scene',layers:[{deps:[],draw(g){scratch=20;g.rect(0,0,1,1);}},{deps:['x'],draw(g,s){g.circle(scratch+s.x,0,1);}}]})});";
    let mut warm = Runtime::from_bundle(source).unwrap();
    warm.scene("scene").unwrap();
    warm.restore_state(&json!({"x":1})).unwrap();
    let mut cold = Runtime::from_bundle(source).unwrap();
    cold.restore_state(&json!({"x":1})).unwrap();
    assert_eq!(*warm.scene("scene").unwrap(), *cold.scene("scene").unwrap());
    assert!(matches!(&warm.scene("scene").unwrap()[1],Shape::Circle{center,..} if center[0]==1.));
}
#[test]
fn late_key_release_cannot_clear_another_targets_hold() {
    let mut app = input_app();
    app.input(
        "input",
        InputEvent::Key {
            key: "space".into(),
            down: true,
            repeat: false,
        },
    )
    .unwrap();
    app.input(
        "left",
        InputEvent::Key {
            key: "space".into(),
            down: true,
            repeat: false,
        },
    )
    .unwrap();
    app.input(
        "input",
        InputEvent::Key {
            key: "space".into(),
            down: false,
            repeat: false,
        },
    )
    .unwrap();
    assert!(app.button_held("left"));
}
#[test]
fn shared_callback_with_different_dependencies_has_distinct_cache_entries() {
    // UI descriptions are data: aliasing a registered callback must not alias
    // its differently scoped caches, even if a guest edits those descriptions.
    let source = "const canvas=ui.canvas({id:'scene',layers:[{id:'shared',deps:['x'],draw(g,s){g.circle(('x' in s)?s.x:s.y,0,1);}}]});canvas.layers.push({draw:'shared',deps:['y']});app({title:'Aliases',state:{x:1,y:2},view:canvas});";
    let mut app = Runtime::from_bundle(source).unwrap();
    let scene = app.scene("scene").unwrap();
    assert!(matches!(&scene[0],Shape::Circle{center,..} if center[0]==1.));
    assert!(matches!(&scene[1],Shape::Circle{center,..} if center[0]==2.));
    app.restore_state(&json!({"x":3,"y":2})).unwrap();
    let scene = app.scene("scene").unwrap();
    assert!(matches!(&scene[0],Shape::Circle{center,..} if center[0]==3.));
    assert!(matches!(&scene[1],Shape::Circle{center,..} if center[0]==2.));
}
