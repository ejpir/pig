use pi_core::{protocol::Command, session::Session};
use serde_json::{Value, json};

#[test]
fn catalogs_preserve_metadata_without_inventing_missing_values() {
    let mut session = Session::default();
    assert!(!session.models_loaded);
    assert!(session.auth_providers.is_none());
    for (command, data) in [
        (
            "get_available_models",
            json!({"models":[{"provider":"a","id":"one","name":"One","reasoning":true,"maxTokens":32000,"cost":{"input":0,"output":1.5}}, {"provider":"b","id":"one"}]}),
        ),
        (
            "get_auth_providers",
            json!({"providers":[{"id":"a","name":"A","authType":"oauth","status":{"type":"oauth","source":"stored"}}]}),
        ),
        (
            "list_packages",
            json!({"packages":[{"source":"./kit","scope":"project","filtered":true}]}),
        ),
        (
            "get_project_trust",
            json!({"cwd":"/repo","trusted":false,"hasProjectResources":true,"savedDecision":{"path":"/repo","decision":true}}),
        ),
    ] {
        session
            .apply(&json!({"type":"response","command":command,"success":true,"data":data}))
            .unwrap();
    }
    assert!(session.models_loaded);
    assert_eq!(
        session.available_models[0].cost.as_ref().unwrap().input,
        Some(0.)
    );
    assert_eq!(session.available_models[0].max_tokens, Some(32000));
    assert!(session.available_models[1].cost.is_none());
    assert!(session.available_models[1].reasoning.is_none());
    assert!(
        session.packages.as_ref().unwrap()[0]
            .installed_path
            .is_none()
    );
    let trust = session.project_trust.as_ref().unwrap();
    assert!(
        !trust.trusted,
        "saved trust does not change this process's loaded resources"
    );
    assert!(trust.saved_decision.as_ref().unwrap().decision);
}

fn catalog_model() -> Value {
    json!({
        "provider": "a",
        "id": "one",
        "name": "One Vision",
        "input": ["text", "image"],
        "reasoning": true,
        "contextWindow": 200000,
        "maxTokens": 32000,
        "cost": {"input": 0.0, "output": 1.5, "cacheRead": 0.1, "cacheWrite": 0.2}
    })
}

fn apply_response(session: &mut Session, command: &str, data: Value) {
    session
        .apply(&json!({"type":"response", "command":command, "success":true, "data":data}))
        .unwrap();
}

fn selected_model(session: &Session) -> Value {
    serde_json::to_value(session.state.model.as_ref().unwrap()).unwrap()
}

#[test]
fn catalog_hydrates_selected_stub_with_exact_provider_and_id_metadata() {
    let mut session = Session::default();
    apply_response(
        &mut session,
        "get_state",
        json!({"model":{"provider":"a", "id":"one"}}),
    );
    let selected = session.state.model.as_ref().unwrap();
    assert!(selected.input.is_empty());
    assert!(selected.name.is_none());
    assert_eq!(selected.context_window, 0);
    assert!(!session.models_loaded);

    let mut other_provider = catalog_model();
    other_provider["provider"] = json!("b");
    other_provider["name"] = json!("Wrong provider");
    let mut other_id = catalog_model();
    other_id["id"] = json!("two");
    apply_response(
        &mut session,
        "get_available_models",
        json!({"models":[other_provider, other_id, catalog_model()]}),
    );

    assert!(session.models_loaded);
    assert_eq!(selected_model(&session), catalog_model());
}

#[test]
fn catalog_does_not_substitute_same_id_from_another_provider() {
    let mut session = Session::default();
    apply_response(
        &mut session,
        "get_state",
        json!({"model":{"provider":"b", "id":"one"}}),
    );
    let stub = selected_model(&session);
    apply_response(
        &mut session,
        "get_available_models",
        json!({"models":[catalog_model()]}),
    );

    assert_eq!(selected_model(&session), stub);
    assert!(session.state.model.as_ref().unwrap().input.is_empty());
}

#[test]
fn catalog_does_not_invent_a_selected_model() {
    let mut session = Session::default();
    apply_response(
        &mut session,
        "get_available_models",
        json!({"models":[catalog_model()]}),
    );
    assert!(session.models_loaded);
    assert!(session.state.model.is_none());

    apply_response(&mut session, "get_state", json!({"model":null}));
    assert!(session.state.model.is_none());
}

#[test]
fn stale_catalog_does_not_clobber_selected_model_with_populated_input() {
    let mut session = Session::default();
    let mut current = catalog_model();
    current["input"] = json!(["text"]);
    current["name"] = json!("Current text-only model");
    current["reasoning"] = json!(false);
    current["contextWindow"] = json!(64000);
    current["maxTokens"] = json!(8000);
    current["cost"] = json!(null);
    apply_response(&mut session, "get_state", json!({"model":current}));
    apply_response(
        &mut session,
        "get_available_models",
        json!({"models":[catalog_model()]}),
    );

    assert_eq!(selected_model(&session), current);
    // The same guard must hold when state arrives after the stale catalog.
    apply_response(&mut session, "get_state", json!({"model":current}));
    assert_eq!(selected_model(&session), current);
}

#[test]
fn loaded_catalog_hydrates_stub_from_later_get_state() {
    let mut session = Session::default();
    apply_response(
        &mut session,
        "get_available_models",
        json!({"models":[catalog_model()]}),
    );
    assert!(session.state.model.is_none());
    apply_response(
        &mut session,
        "get_state",
        json!({"model":{"provider":"a", "id":"one"}}),
    );

    assert_eq!(selected_model(&session), catalog_model());
}

#[test]
fn remote_snapshot_hydrates_stub_from_its_loaded_catalog() {
    let mut snapshot = serde_json::to_value(Session::default()).unwrap();
    snapshot["state"]["model"] = json!({"provider":"a", "id":"one"});
    snapshot["available_models"] = json!([catalog_model()]);
    snapshot["models_loaded"] = json!(true);
    let mut session = Session::default();
    assert!(session.available_models.is_empty());
    session
        .apply(&json!({"type":"remote_snapshot", "data":snapshot}))
        .unwrap();

    assert!(session.models_loaded);
    assert_eq!(selected_model(&session), catalog_model());
}

#[test]
fn catalog_mutations_use_rpc_fields_and_never_prompt() {
    let record = Command::SetModel {
        provider: "a".into(),
        model_id: "one".into(),
        persist: true,
    }
    .record("x")
    .unwrap();
    assert_eq!(
        record,
        json!({"type":"set_model","id":"x","provider":"a","modelId":"one","persist":true})
    );
    let install = Command::InstallPackage {
        source: "npm:@example/kit@1".into(),
        local: true,
    };
    assert!(install.replies_when_finished());
    assert_eq!(
        install.record("y").unwrap(),
        json!({"type":"install_package","id":"y","source":"npm:@example/kit@1","local":true})
    );
    assert_eq!(
        Command::SetScopedModels {
            patterns: None,
            persist: true
        }
        .record("z")
        .unwrap()["patterns"],
        json!(null)
    );
}
