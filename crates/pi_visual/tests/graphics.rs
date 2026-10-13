use pi_visual::Runtime;
use serde_json::json;
use std::sync::Arc;

fn source(event: &str, draw: &str) -> String {
    format!(
        r##"let secret=0;
app({{title:'Motion',state:{{x:0,playing:false}},view:ui.column([
 ui.canvas({{id:'scene',label:'Dot',viewBox:[0,0,100,100],draw(g,s){{{draw}}},pointerId:'move',onPointer(s,x,y){{{event}}}}}),
 ui.slider('Position','x',{{min:0,max:100,step:0.5,onChange(s){{{event}}}}}),
 ui.button('Play',s=>{{s.playing=!s.playing;}},{{id:'play'}}),
 ui.button('Test',s=>{{{event}}},{{id:'test'}})
]),animation:{{running:'playing',tick(s,dt){{s.x+=dt*10;}}}}}});"##
    )
}
const DRAW: &str = "g.circle(s.x,50,5,{fill:'#71BBFF'});";
#[test]
fn sliders_ticks_pointers_and_scene_cache() {
    let mut app = Runtime::from_bundle(&source("s.x=Math.min(100,s.x);", DRAW)).unwrap();
    let original = app.scene("scene").unwrap();
    assert!(Arc::ptr_eq(&original, &app.scene("scene").unwrap()));
    app.slide("x", 12.3).unwrap();
    assert_eq!(app.number("x"), Some(12.5));
    assert!(!Arc::ptr_eq(&original, &app.scene("scene").unwrap()));
    app.tick(0.1).unwrap();
    assert_eq!(app.number("x"), Some(12.5));
    app.dispatch("play").unwrap();
    app.tick(0.05).unwrap();
    assert_eq!(app.number("x"), Some(13.));
    app.slide("x", 500.).unwrap();
    assert_eq!(app.number("x"), Some(100.));
    assert!(app.slide("x", f64::NAN).is_err());
    assert!(app.tick(0.2).is_err());
    assert!(app.scene("other").is_err());
}
#[test]
fn failures_retain_state_and_global_mutation_cannot_escape_a_callback() {
    for event in [
        "s.x=true;",
        "s.extra=1;",
        "throw Error('bad');",
        "while(true){}",
        "Object.defineProperty(s,'x',{get(){return 1;}});",
    ] {
        let mut app = Runtime::from_bundle(&source(event, DRAW)).unwrap();
        assert!(app.slide("x", 22.).is_err(), "{event}");
        assert_eq!(app.number("x"), Some(0.));
    }
    let mut app = Runtime::from_bundle(&source("s.x=++secret;", DRAW)).unwrap();
    app.dispatch("test").unwrap();
    app.dispatch("test").unwrap();
    assert_eq!(app.number("x"), Some(1.));
}
#[test]
fn pure_draw_retains_last_valid_scene_on_failure() {
    let mut app = Runtime::from_bundle(&source(
        "s.x=1;",
        "if(s.x)throw Error('draw failed');g.circle(0,0,5);",
    ))
    .unwrap();
    let last = app.scene("scene").unwrap();
    app.dispatch("test").unwrap();
    assert!(app.scene("scene").is_err());
    assert!(app.drawing_failed());
    assert!(Arc::ptr_eq(&last, &app.last_scene("scene").unwrap()));
    assert_eq!(app.number("x"), Some(1.));
}
#[test]
fn geometry_safety_and_automatic_polyline_segmentation() {
    for draw in [
        "g.circle(0,0,-1);",
        "g.line(0,0,1,1,{width:99});",
        "g.circle(Infinity,0,2);",
        "for(let i=0;i<33;i++)g.circle(0,0,2);",
        "g.line(0,0,1,1,{color:'red'});",
    ] {
        let app = Runtime::from_bundle(&source("", draw)).unwrap();
        assert!(app.scene("scene").is_err(), "{draw}");
    }
    let app = Runtime::from_bundle(&source(
        "",
        "g.polyline(Array.from({length:512},(_,i)=>[i,i]));",
    ))
    .unwrap();
    assert_eq!(app.scene("scene").unwrap().len(), 9);
}
#[test]
fn sync_only_no_host_capabilities_and_bounded_initialization() {
    for code in [
        "while(true){}",
        "app({title:'Bad',state:{x:NaN},view:ui.column([])});",
        "import('os');",
        "Promise.resolve().then(()=>{});app({title:'X',state:{},view:ui.column([])});",
    ] {
        assert!(Runtime::from_bundle(code).is_err(), "{code}");
    }
    let app=Runtime::from_bundle(&source("","if(typeof fetch!=='undefined'||typeof document!=='undefined'||typeof require!=='undefined')throw Error('host exposed');g.circle(0,0,2);")).unwrap();
    app.scene("scene").unwrap();
    let mut app =
        Runtime::from_bundle(&source("Promise.resolve().then(()=>{s.x=2;});", DRAW)).unwrap();
    assert!(app.dispatch("test").is_err());
    assert_eq!(app.number("x"), Some(0.));
}
#[test]
fn saved_schema_one_numeric_type_and_unchanged_cache() {
    let mut app = Runtime::from_bundle(&source("", DRAW)).unwrap();
    let scene = app.scene("scene").unwrap();
    app.dispatch("test").unwrap();
    assert!(Arc::ptr_eq(&scene, &app.scene("scene").unwrap()));
    app.restore_state(&json!({"x":0.0,"playing":false}))
        .unwrap();
    assert!(Arc::ptr_eq(&scene, &app.scene("scene").unwrap()));
    assert!(
        app.restore_state(&json!({"x":true,"playing":false}))
            .is_err()
    );
    assert!(
        app.restore_state(&json!({"x":0,"playing":false,"bad":1}))
            .is_err()
    );
}

#[test]
fn review_regressions_text_errors_ids_prototypes_and_promises() {
    let app =
        Runtime::from_bundle("app({title:'X',state:{text0:'saved'},view:ui.text(s=>'computed')});")
            .unwrap();
    let pi_visual::Node::Text { bind: Some(id), .. } = &app.definition.view else {
        panic!()
    };
    assert_eq!(app.text(id), "computed");
    let app=Runtime::from_bundle("app({title:'X',state:{},view:ui.column([ui.text(s=>{throw Error('bad') }),ui.text(s=>'ok')])});").unwrap();
    let pi_visual::Node::Column { children, .. } = &app.definition.view else {
        panic!()
    };
    let ids: Vec<_> = children
        .iter()
        .map(|n| {
            if let pi_visual::Node::Text { bind: Some(id), .. } = n {
                id
            } else {
                panic!()
            }
        })
        .collect();
    assert!(app.text(ids[0]).contains("Text failed"));
    assert!(app.drawing_failed());
    assert_eq!(app.text(ids[0]), app.text(ids[1]));
    assert!(
        Runtime::from_bundle(
            "app({title:'X',state:{x:0},view:ui.slider('X','x',{min:0,max:1,step:1e-310})});"
        )
        .is_err()
    );
    assert!(
        Runtime::from_bundle("app({title:'X',state:{},view:ui.button('X',s=>{},{id:'__tick'})});")
            .is_err()
    );
    for event in [
        "s.x=7;const p=(async()=>{throw Error('bad')})();Object.setPrototypeOf(p,null);return p;",
        "s.x=7;(async()=>{throw Error('bad')})();",
    ] {
        let mut app = Runtime::from_bundle(&source(event, DRAW)).unwrap();
        assert!(app.dispatch("test").is_err());
        assert_eq!(app.number("x"), Some(0.));
    }
    let app = Runtime::from_bundle(&source(
        "",
        "if(this!==undefined)throw Error('registry receiver');g.circle(s.x,0,2);",
    ))
    .unwrap();
    app.scene("scene").unwrap();
}

// A separate process protects the suite if the native-operation guard regresses.
#[test]
fn sparse_native_operations_cannot_stall_the_host() {
    const CHILD: &str = "PI_QUICKJS_SPARSE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        for code in [
            "Array.prototype.join.call({length:9007199254740991},'');",
            "Array.prototype.sort.call({length:9007199254740991});",
            "Array.prototype.reverse.call({length:9007199254740991});",
            "Array.prototype.join.call({get length(){return 9007199254740991;}},'');",
        ] {
            assert!(Runtime::from_bundle(code).is_err());
        }
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sparse_native_operations_cannot_stall_the_host",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Native operation exceeded guard");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
