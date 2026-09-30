use pi_core::history::{Filter, History};
use serde_json::json;

#[test]
fn metadata_is_not_a_conversation_branch_and_filtered_rails_keep_parents() {
    let h=History::parse(&json!({"leafId":"c","entries":[
        {"id":"a","type":"message","message":{"role":"user","content":"hi"}},
        {"id":"tool","parentId":"a","type":"message","message":{"role":"toolResult","content":"ok"}},
        {"id":"b","parentId":"tool","type":"message","message":{"role":"assistant","content":[]}},
        {"id":"c","parentId":"tool","type":"message","message":{"role":"assistant","content":[]}},
        {"id":"label","parentId":"tool","type":"label","targetId":"b","label":"abandoned"}
    ]})).unwrap();
    assert_eq!(h.branch_count(), 2);
    let rows = h.rows(Filter::NoTools);
    let edges = h.connections(&rows);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        (edges[0].parent, edges[0].child, edges[0].active),
        (0, 1, false)
    );
    assert_eq!(
        (edges[1].parent, edges[1].child, edges[1].active),
        (0, 2, true)
    );
    assert_eq!(rows[1].depth, 1);
    assert_eq!(rows[2].depth, 0);
}

#[test]
fn orphan_and_cycle_edges_never_point_backwards() {
    let h=History::parse(&json!({"leafId":"b","entries":[
        {"id":"a","parentId":"b","type":"message","message":{"role":"user","content":"a"}},
        {"id":"b","parentId":"a","type":"message","message":{"role":"assistant","content":[]}},
        {"id":"c","parentId":"missing","type":"message","message":{"role":"user","content":"orphan"}}
    ]})).unwrap();
    let rows = h.rows(Filter::All);
    assert_eq!(rows.len(), 3);
    assert!(
        h.connections(&rows)
            .iter()
            .all(|edge| edge.parent < edge.child)
    );
}
