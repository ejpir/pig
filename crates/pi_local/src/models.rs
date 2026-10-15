//! Pinned public provider metadata. No credentials or inference.
use crate::auth::Provider;
use serde_json::Value;
use std::sync::OnceLock;

pub fn models(provider: Provider) -> &'static [(String, String)] {
    static CLAUDE: OnceLock<Vec<(String, String)>> = OnceLock::new();
    static CODEX: OnceLock<Vec<(String, String)>> = OnceLock::new();
    let cell = match provider {
        Provider::Claude => &CLAUDE,
        Provider::Codex => &CODEX,
    };
    cell.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!("../generated/chat-models.json"))
            .expect("bundled catalog");
        let mut models: Vec<_> = data[provider.id()]
            .as_array()
            .expect("provider catalog")
            .iter()
            .map(|m| {
                (
                    m["id"].as_str().unwrap().to_owned(),
                    m["name"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let default = match provider {
            Provider::Claude => "claude-sonnet-4-6",
            Provider::Codex => "gpt-5.5",
        };
        models.sort_by_key(|m| m.0 != default);
        models
    })
}

/// Pinned upstream model metadata for native pickers; no account lookup or network.
pub fn catalog(provider: Provider) -> Value {
    let data: Value = serde_json::from_str(include_str!("../generated/chat-models.json"))
        .expect("bundled catalog");
    data[provider.id()].clone()
}
