//! Read-only projections of Pi's append-only session tree. No UI or filesystem policy.
use crate::session::content_text;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub struct History {
    pub entries: Vec<Value>,
    pub leaf: Option<String>,
    pub labels: HashMap<String, String>,
    pub active: HashSet<String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    Default,
    NoTools,
    User,
    Labeled,
    All,
}
impl Filter {
    pub const ALL: [Self; 5] = [
        Self::Default,
        Self::NoTools,
        Self::User,
        Self::Labeled,
        Self::All,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::NoTools => "No tools",
            Self::User => "User only",
            Self::Labeled => "Labeled",
            Self::All => "All",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Row {
    pub index: usize,
    pub depth: usize,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    /// Indexes into the filtered rows, not the unfiltered entry store.
    pub parent: usize,
    pub child: usize,
    pub active: bool,
}
impl History {
    pub fn parse(data: &Value) -> Result<Self> {
        let entries = data["entries"]
            .as_array()
            .context("invalid session entries")?
            .clone();
        let mut ids = HashSet::new();
        for e in &entries {
            let id = e["id"].as_str().context("entry is missing its stable ID")?;
            if !ids.insert(id.to_string()) {
                bail!("duplicate session entry ID: {id}");
            }
        }
        let leaf = data["leafId"].as_str().map(str::to_owned);
        let mut labels = HashMap::new();
        for e in &entries {
            if e["type"] == "label"
                && let Some(id) = e["targetId"].as_str()
            {
                if let Some(label) = e["label"].as_str().filter(|s| !s.trim().is_empty()) {
                    labels.insert(id.to_string(), label.to_string());
                } else {
                    labels.remove(id);
                }
            }
        }
        let by_id: HashMap<_, _> = entries
            .iter()
            .map(|e| (e["id"].as_str().unwrap(), e))
            .collect();
        let mut active = HashSet::new();
        let mut cursor = leaf.as_deref();
        while let Some(id) = cursor {
            if !active.insert(id.to_owned()) {
                break;
            }
            cursor = by_id.get(id).and_then(|e| e["parentId"].as_str());
        }
        Ok(Self {
            entries,
            leaf,
            labels,
            active,
        })
    }
    pub fn entry(&self, id: &str) -> Option<&Value> {
        self.entries.iter().find(|e| e["id"] == id)
    }
    pub fn path(&self) -> impl Iterator<Item = &Value> {
        self.entries
            .iter()
            .filter(|e| e["id"].as_str().is_some_and(|id| self.active.contains(id)))
    }
    pub fn branch_count(&self) -> usize {
        // Labels/settings are metadata, not extra conversation branches.
        let rows = self.rows(Filter::Default);
        let parents: HashSet<_> = self
            .connections(&rows)
            .into_iter()
            .map(|e| e.parent)
            .collect();
        rows.len().saturating_sub(parents.len())
    }
    /// Connect each visible entry to its nearest visible ancestor. Hidden tool or
    /// metadata entries do not break the graph. Malformed cycles remain bounded.
    pub fn connections(&self, rows: &[Row]) -> Vec<Connection> {
        let entries: HashMap<_, _> = self
            .entries
            .iter()
            .filter_map(|e| e["id"].as_str().map(|id| (id, e)))
            .collect();
        let visible: HashMap<_, _> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| self.entries[r.index]["id"].as_str().map(|id| (id, i)))
            .collect();
        let mut result = Vec::new();
        for (child, row) in rows.iter().enumerate() {
            let mut cursor = self.entries[row.index]["parentId"].as_str();
            let mut seen = HashSet::new();
            while let Some(id) = cursor {
                if !seen.insert(id) {
                    break;
                }
                if let Some(&parent) = visible.get(id) {
                    if parent < child {
                        result.push(Connection {
                            parent,
                            child,
                            active: row.active && rows[parent].active,
                        });
                    }
                    break;
                }
                cursor = entries.get(id).and_then(|e| e["parentId"].as_str());
            }
        }
        result
    }
    /// Iterative traversal handles long histories and broken/cyclic parent chains safely.
    pub fn rows(&self, filter: Filter) -> Vec<Row> {
        let ids: HashSet<_> = self
            .entries
            .iter()
            .filter_map(|e| e["id"].as_str())
            .collect();
        let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut roots = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            match e["parentId"].as_str() {
                Some(p) if ids.contains(p) => children.entry(p).or_default().push(i),
                _ => roots.push(i),
            }
        }
        let mut stack: Vec<_> = roots.into_iter().rev().map(|i| (i, 0)).collect();
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for fallback in 0..self.entries.len() {
            if stack.is_empty() && !seen.contains(&fallback) {
                stack.push((fallback, 0));
            }
            while let Some((index, depth)) = stack.pop() {
                if !seen.insert(index) {
                    continue;
                }
                let e = &self.entries[index];
                let id = e["id"].as_str().unwrap_or("");
                let kind = kind(e);
                let visible = match filter {
                    Filter::All => true,
                    Filter::Labeled => self.labels.contains_key(id),
                    Filter::User => kind == "you",
                    Filter::NoTools => matches!(kind, "you" | "pi" | "compaction" | "summary"),
                    Filter::Default => matches!(
                        kind,
                        "you" | "pi" | "tool" | "compaction" | "summary" | "custom"
                    ),
                };
                if visible {
                    result.push(Row {
                        index,
                        depth,
                        active: self.active.contains(id),
                    });
                }
                if let Some(next) = children.get(id) {
                    for &child in next.iter().rev() {
                        let branch = next.len() > 1
                            && !self
                                .active
                                .contains(self.entries[child]["id"].as_str().unwrap_or(""));
                        stack.push((child, depth + usize::from(branch)));
                    }
                }
            }
        }
        result
    }
}
pub fn kind(e: &Value) -> &str {
    match e["type"].as_str().unwrap_or("") {
        "message" => match e["message"]["role"].as_str().unwrap_or("") {
            "user" => "you",
            "assistant" => "pi",
            "toolResult" | "bashExecution" => "tool",
            _ => "custom",
        },
        "compaction" => "compaction",
        "branch_summary" => "summary",
        "custom_message" => "custom",
        // The desktop's own records, such as turn links: shown only under All.
        "custom"
            if e["customType"]
                .as_str()
                .is_some_and(|t| t.starts_with("pi-desktop-")) =>
        {
            "desktop"
        }
        t => t,
    }
}
pub fn entry_text(e: &Value) -> String {
    if e["type"] == "message" {
        let m = &e["message"];
        let s = content_text(&m["content"]);
        if !s.is_empty() {
            return s;
        }
        if let Some(blocks) = m["content"].as_array() {
            return blocks
                .iter()
                .filter(|b| b["type"] == "toolCall")
                .map(|b| {
                    format!(
                        "{} {}",
                        b["name"].as_str().unwrap_or("tool"),
                        b["arguments"]
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
    e["summary"]
        .as_str()
        .or(e["content"].as_str())
        .or(e["label"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| kind(e).to_string())
}
pub fn preview(e: &Value) -> String {
    entry_text(e)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}
pub fn time(e: &Value) -> String {
    e["timestamp"]
        .as_str()
        .and_then(|s| s.get(11..16))
        .unwrap_or("—")
        .to_string()
}

#[derive(Clone, Debug)]
pub struct ResponseUsage {
    pub input: u64,
    pub output: u64,
    pub cache: u64,
    pub compaction_before: bool,
}
pub fn response_usage(history: &History) -> Vec<ResponseUsage> {
    let mut compacted = false;
    let mut result = Vec::new();
    for e in history.path() {
        if e["type"] == "compaction" {
            compacted = true;
        }
        let m = &e["message"];
        if m["role"] == "assistant" && m["usage"].is_object() {
            let u = &m["usage"];
            result.push(ResponseUsage {
                input: u["input"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(u["cacheWrite"].as_u64().unwrap_or(0)),
                output: u["output"].as_u64().unwrap_or(0),
                cache: u["cacheRead"].as_u64().unwrap_or(0),
                compaction_before: std::mem::take(&mut compacted),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn branching_labels_filters_and_usage() {
        let h = History::parse(&json!({"leafId":"c", "entries":[{"id":"a","parentId":null,"type":"message","message":{"role":"user","content":"hello"}}, {"id":"b","parentId":"a","type":"message","message":{"role":"assistant","content":[],"usage":{"input":9,"output":3,"cacheRead":8}}}, {"id":"c","parentId":"a","type":"message","message":{"role":"assistant","content":[],"usage":{"input":2,"output":1,"cacheRead":4}}}, {"id":"l","parentId":"c","type":"label","targetId":"b","label":"branch"}]})).unwrap();
        assert!(h.active.contains("a"));
        assert!(!h.active.contains("b"));
        assert_eq!(h.rows(Filter::Default).len(), 3);
        assert_eq!(h.rows(Filter::User).len(), 1);
        assert_eq!(h.rows(Filter::Labeled)[0].index, 1);
        assert_eq!(response_usage(&h)[0].input, 2);
    }
    #[test]
    fn the_desktops_records_show_only_under_all() {
        let h = History::parse(&json!({"leafId":"t", "entries":[{"id":"a","parentId":null,"type":"message","message":{"role":"user","content":"hello"}}, {"id":"t","parentId":"a","type":"custom","customType":"pi-desktop-turn","data":{}}, {"id":"x","parentId":"t","type":"custom","customType":"plan-mode","data":{}}]})).unwrap();
        assert_eq!(kind(&h.entries[1]), "desktop");
        assert_eq!(
            h.rows(Filter::Default).len(),
            2,
            "an extension's entry stays"
        );
        assert_eq!(h.rows(Filter::All).len(), 3);
    }
    #[test]
    fn long_and_broken_trees_are_bounded() {
        let mut entries: Vec<_> = (0..10000).map(|i| json!({"id":i.to_string(),"parentId":if i==0 {"9999".into()} else {(i-1).to_string()},"type":"message","message":{"role":"user","content":"hi"}})).collect();
        let h = History::parse(&json!({"entries":entries,"leafId":"9999"})).unwrap();
        assert_eq!(h.rows(Filter::All).len(), 10000);
        entries.push(entries[0].clone());
        assert!(History::parse(&json!({"entries":entries})).is_err());
    }
}
