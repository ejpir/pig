//! Desktop admission and immutable history bindings. Runtime/UI live in pi_visual_ui.
use super::{session::SessionController, transcript::TranscriptView};
use gpui::{App, Context, EntityId, Global, Window};
use pi_core::{
    history::History,
    session::{Session, content_text},
};
use pi_visual::reply::{Parsed, Part};
use pi_visual_ui::{Host, VisualApps};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

type Apps = VisualApps<EntityId>;
#[derive(Clone)]
struct Store(Rc<RefCell<Apps>>);
impl Global for Store {}
pub(super) fn store(cx: &mut App) -> Rc<RefCell<Apps>> {
    if let Some(store) = cx.try_global::<Store>() {
        return store.0.clone();
    }
    let path = crate::prefs::user_path(cx);
    let store = Store(Rc::new(RefCell::new(Apps::load(path.as_deref()))));
    cx.set_global(store.clone());
    store.0
}

impl Host for TranscriptView {
    type Route = EntityId;
    type Target = usize;
    fn visual_apps(&self) -> &RefCell<Apps> {
        &self.visuals
    }
    fn visual_route(&self) -> EntityId {
        self.visual_route
    }
    fn visual_available(&self) -> bool {
        self.visuals.borrow().active_route == Some(self.visual_route)
    }
    fn visual_persist(&self) -> bool {
        self.visual_persist
    }
    fn visual_changed(&mut self, row: usize, _cx: &mut Context<Self>) {
        if row < self.list.item_count() {
            self.list.remeasure_items(row..row + 1);
        }
    }
    fn visual_refocus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.visual_focus, cx);
    }
}

pub(super) fn palette(c: crate::theme::Theme) -> pi_visual_ui::theme::Theme {
    pi_visual_ui::theme::Theme {
        canvas: c.canvas,
        panel: c.panel,
        line: c.line,
        line_strong: c.line_strong,
        selected: c.selected,
        text: c.text,
        secondary: c.secondary,
        muted: c.muted,
        accent: c.accent,
        on_accent: c.on_accent,
        coral: c.coral,
        tint: c.tint(c.accent),
    }
}

pub(super) struct Binding {
    pub texts: HashMap<usize, Arc<Parsed>>,
    pub keys: HashMap<(usize, usize), String>,
}
impl Binding {
    pub fn retained_bytes(&self) -> usize {
        self.texts.values().map(|p| p.source().len()).sum()
    }
    pub fn project(controller: &SessionController, row: usize, apps: &mut Apps) -> Option<Self> {
        let model = controller.model();
        let (user, assistant, prompt) = admission(model, row)?;
        let scope = scope(controller)?;
        let turn = serde_json::to_string(&(user, assistant)).ok()?;
        let mut texts = HashMap::new();
        let mut keys = HashMap::new();
        let mut candidates = 0;
        let mut retained = 0;
        for (block_index, block) in model.messages[row]["content"]
            .as_array()?
            .iter()
            .enumerate()
        {
            if block["type"] != "text" {
                continue;
            }
            let parsed = apps.parsed_reply(block["text"].as_str().unwrap_or(""));
            for (part_index, part) in parsed.parts().enumerate() {
                if let Part::App { source, .. } = part {
                    if candidates < 2 && retained + parsed.source().len() <= 256 * 1024 {
                        let key = pi_visual_ui::identity(
                            "desktop-v1",
                            &scope,
                            &turn,
                            0,
                            candidates,
                            &prompt,
                            source,
                        );
                        apps.ensure(&key, source, !controller.is_demo());
                        if apps.running.get(&key).is_some_and(|i| i.runtime.is_ok()) {
                            keys.insert((block_index, part_index), key);
                        }
                    }
                    candidates += 1;
                }
            }
            if keys.keys().any(|(b, _)| *b == block_index) {
                retained += parsed.source().len();
                texts.insert(block_index, parsed);
            }
        }
        (!keys.is_empty()).then_some(Self { texts, keys })
    }
}

fn scope(controller: &SessionController) -> Option<String> {
    let model = controller.model();
    let id = model
        .state
        .session_id
        .as_deref()
        .filter(|s| !s.is_empty())?;
    let file = model.state.session_file.as_deref();
    let value = if let Some(remote) = controller.remote_target() {
        serde_json::json!([
            "ssh",
            format!("{:?}", remote.backend),
            remote.host,
            remote.key,
            remote.cwd,
            id,
            file
        ])
    } else {
        serde_json::json!([
            "local",
            model.cwd.to_str()?,
            id,
            file.filter(|s| !s.is_empty())?
        ])
    };
    serde_json::to_string(&value).ok()
}

/// Fail closed on absent/stale/ambiguous history, malformed ancestry, streaming,
/// synthetic reports and a newer user turn which did not explicitly opt in.
fn admission(model: &Session, row: usize) -> Option<(String, String, String)> {
    let message = model.messages.get(row)?;
    if message["role"] != "assistant"
        || model.streaming_message_index() == Some(row)
        || matches!(message["stopReason"].as_str(), Some("aborted" | "error"))
    {
        return None;
    }
    let user_row = (0..row)
        .rev()
        .find(|&i| model.messages[i]["role"] == "user")?;
    let user = &model.messages[user_row];
    let prompt = content_text(&user["content"]);
    if pi_core::subagent::report(&prompt).is_some() || !pi_visual::prompt::admitted(&prompt) {
        return None;
    }
    let history = model.history.as_deref()?;
    let path = active_path(history)?;
    let unique = |wanted: &serde_json::Value| -> Option<&serde_json::Value> {
        if model.messages.iter().filter(|m| *m == wanted).count() != 1 {
            return None;
        }
        let mut matches = path
            .iter()
            .copied()
            .filter(|e| e["type"] == "message" && &e["message"] == wanted);
        let found = matches.next()?;
        matches.next().is_none().then_some(found)
    };
    let user_id = unique(user)?["id"].as_str()?;
    let assistant = unique(message)?;
    let assistant_id = assistant["id"].as_str()?;
    // Active ancestry is leaf-first, independent of the stored entries' ordering.
    let position = path.iter().position(|e| e["id"] == assistant_id)?;
    let ancestor = path[position + 1..]
        .iter()
        .find(|e| e["type"] == "message" && e["message"]["role"] == "user")?;
    if ancestor["id"] != user_id {
        return None;
    }
    Some((user_id.to_owned(), assistant_id.to_owned(), prompt))
}
fn active_path(history: &History) -> Option<Vec<&serde_json::Value>> {
    let mut by_id = HashMap::new();
    for entry in &history.entries {
        let id = entry["id"].as_str().filter(|s| !s.is_empty())?;
        if by_id.insert(id, entry).is_some() {
            return None;
        }
    }
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = history.leaf.as_deref();
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return None;
        }
        let entry = *by_id.get(id)?;
        path.push(entry);
        cursor = match &entry["parentId"] {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) => Some(s.as_str()),
            _ => return None,
        };
    }
    Some(path)
}

/// Opt-in demo/test records follow the same durable identity path as real sessions.
pub(super) fn sample_records(source: &str) -> Vec<serde_json::Value> {
    use serde_json::json;
    let user = json!({"role":"user","content":pi_visual::prompt::expand("@Visualize a native interactive app"),"timestamp":1});
    let assistant = json!({"role":"assistant","content":[{"type":"text","text":format!("A native app, running locally.\n\n```javascript pi-visual\n{source}\n```\n\nNo model calls are made by these controls.")}],"stopReason":"stop","timestamp":2});
    vec![
        json!({"type":"response","command":"get_state","success":true,"data":{"sessionId":"visual-demo","sessionFile":"/demo/visual-demo.jsonl"}}),
        json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[user.clone(),assistant.clone()]}}),
        json!({"type":"response","command":"get_entries","success":true,"data":{"entries":[{"id":"visual-user","type":"message","parentId":null,"message":user},{"id":"visual-assistant","type":"message","parentId":"visual-user","message":assistant}],"leafId":"visual-assistant"}}),
        json!({"type":"agent_settled"}),
        json!({"type":"queue_update","steering":[],"followUp":[]}),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const SOURCE: &str = "app({title:'Counter',state:{n:0},view:ui.button('Add',s=>{s.n++})});";
    fn model() -> Session {
        let mut m = Session::default();
        for r in sample_records(SOURCE) {
            m.apply(&r).unwrap();
        }
        m
    }
    #[test]
    fn durable_admission_is_turn_scoped_and_streams_fail_closed() {
        let mut m = model();
        assert!(admission(&m, 1).is_some());
        m.apply(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}))
            .unwrap();
        assert!(admission(&m, 2).is_none());
        m = model();
        m.messages
            .insert(1, json!({"role":"user","content":"Just explain it"}));
        assert!(admission(&m, 2).is_none());
        m = model();
        m.messages[1]["stopReason"] = json!("aborted");
        assert!(admission(&m, 1).is_none());
    }
    #[test]
    fn malformed_stale_or_ambiguous_history_never_admits() {
        for change in 0..5 {
            let mut m = model();
            let h = Arc::make_mut(m.history.as_mut().unwrap());
            match change {
                0 => h.entries[0]["id"] = json!(""),
                1 => h.entries[0]["parentId"] = json!("visual-assistant"),
                2 => h.entries[1]["parentId"] = json!("missing"),
                3 => h.entries.push(h.entries[1].clone()),
                _ => h.entries[1]["message"]["content"] = json!("changed"),
            };
            assert!(admission(&m, 1).is_none(), "case {change}");
        }
        let mut m = model();
        m.messages.push(m.messages[1].clone());
        assert!(admission(&m, 1).is_none());
    }
    #[test]
    fn ancestry_not_timestamps_or_array_order_identifies_the_turn() {
        let mut m = model();
        Arc::make_mut(m.history.as_mut().unwrap()).entries.reverse();
        assert_eq!(admission(&m, 1).unwrap().0, "visual-user");
        let original = m.messages[0].clone();
        m.messages.insert(1,json!({"role":"user","content":pi_visual::prompt::expand("@Visualize different"),"timestamp":1}));
        assert!(admission(&m, 2).is_none());
        m.messages[1] = original;
        assert!(admission(&m, 2).is_none());
    }
}
