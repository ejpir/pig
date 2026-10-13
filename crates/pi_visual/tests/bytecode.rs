//! Bytecode caches parsing only; strict execution and guest state remain fresh.
use pi_visual::Runtime;
use serde_json::json;

fn source(body: &str) -> String {
    format!(
        "var globalCount=0; let closureCount=0; app({{title:'Cached',state:{{x:0,fail:false}},view:ui.button('Run',function(s){{{body}}},{{id:'run'}})}});"
    )
}
#[test]
fn strict_global_execution_and_callback_receiver_survive_round_trips() {
    let mut app = Runtime::from_bundle(&source(
        "if(this!==undefined)throw Error('receiver');s.x=++globalCount+ ++closureCount;",
    ))
    .unwrap();
    app.dispatch("run").unwrap();
    assert_eq!(app.number("x"), Some(2.));
    app.dispatch("run").unwrap();
    assert_eq!(app.number("x"), Some(2.));
    let mut app = Runtime::from_bundle(&source("accidentallyGlobal=1;s.x=7;")).unwrap();
    assert!(app.dispatch("run").is_err());
    assert_eq!(app.number("x"), Some(0.));
    for declaration in ["var ui=1;", "var app=1;"] {
        assert!(Runtime::from_bundle(&(declaration.to_owned() + &source(""))).is_err());
    }
}
#[test]
fn warmed_bootstrap_cannot_share_closures_globals_or_failed_state() {
    let code = source(
        "globalThis.leak=(globalThis.leak||0)+1;s.x=++closureCount+globalThis.leak;if(s.fail)throw Error('rollback');",
    );
    let mut first = Runtime::from_bundle(&code).unwrap();
    let mut second = Runtime::from_bundle(&code).unwrap();
    for app in [&mut first, &mut second] {
        app.dispatch("run").unwrap();
        assert_eq!(app.number("x"), Some(2.));
        app.restore_state(&json!({"x":5,"fail":true})).unwrap();
        assert!(app.dispatch("run").is_err());
        assert_eq!(app.number("x"), Some(5.));
        app.restore_state(&json!({"x":5,"fail":false})).unwrap();
        app.dispatch("run").unwrap();
        assert_eq!(app.number("x"), Some(2.));
    }
}
#[test]
fn cached_registration_and_callbacks_reject_unreturned_promises() {
    for prefix in [
        "new Promise(()=>{});",
        "(async()=>{throw Error('unhandled');})();",
    ] {
        assert!(Runtime::from_bundle(&(prefix.to_owned() + &source(""))).is_err());
    }
    let mut app = Runtime::from_bundle(&source("s.x=9;new Promise(()=>{});")).unwrap();
    for _ in 0..3 {
        assert!(app.dispatch("run").is_err());
        assert_eq!(app.number("x"), Some(0.));
    }
}
