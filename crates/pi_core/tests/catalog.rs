use pi_core::{protocol::Command, session::Session};
use serde_json::json;

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
