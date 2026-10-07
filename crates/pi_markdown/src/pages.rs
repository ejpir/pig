//! Pages Pi made: an HTML file it wrote, shown as a card under the turn. The
//! app has the page from Pi's own tool calls, without reading the file: a
//! `write` carries the whole file, and an `edit` is applied to the last
//! version the app saw. That works the same for a session on another computer.

use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// From the project's root: `demo/aurora.html`.
    pub path: String,
    /// The page as the turn left it; `None` once Pi changed it in a way the
    /// app can't follow, such as an edit that matched loosely.
    pub html: Option<String>,
}

impl Page {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The page's `<title>`, or its file name.
    pub fn title(&self) -> String {
        self.html
            .as_deref()
            .and_then(title_of)
            .unwrap_or_else(|| self.file_name().to_owned())
    }
}

pub fn is_page(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.ends_with(".html") || path.ends_with(".htm")
}

/// What the app knows of each page so far in a session.
#[derive(Default)]
pub struct Pages(HashMap<String, Option<String>>);

impl Pages {
    /// Follows a tool call that finished without an error; returns the page it
    /// touched. A page the app never saw written stays unknown: an edit alone
    /// doesn't carry the file.
    pub fn follow(&mut self, tool: &str, args: &Value, path: &str) -> Option<Page> {
        if !is_page(path) {
            return None;
        }
        let html = match tool {
            "write" => Some(args["content"].as_str()?.to_owned()),
            "edit" => {
                let known = self.0.get(path)?;
                known.as_deref().and_then(|html| apply_edits(html, args))
            }
            _ => return None,
        };
        self.0.insert(path.to_owned(), html.clone());
        Some(Page {
            path: path.to_owned(),
            html,
        })
    }
}

/// Pi's `edit`: replacements matched against the file as it was, each found
/// exactly once. Pi also accepts loose matches; those give `None`.
fn apply_edits(html: &str, args: &Value) -> Option<String> {
    let parsed;
    let edits = match &args["edits"] {
        // Some models send the list as a JSON string, or a single edit.
        Value::String(text) => {
            parsed = serde_json::from_str::<Value>(text).ok()?;
            &parsed
        }
        Value::Null => args,
        edits => edits,
    };
    let edits: Vec<&Value> = match edits {
        Value::Array(edits) => edits.iter().collect(),
        edit => vec![edit],
    };
    let html = html.replace("\r\n", "\n");
    let mut matches = Vec::new();
    for edit in edits {
        let old = edit["oldText"].as_str()?.replace("\r\n", "\n");
        let new = edit["newText"].as_str()?.replace("\r\n", "\n");
        if old.is_empty() || html.matches(old.as_str()).count() != 1 {
            return None;
        }
        let start = html.find(&old)?;
        matches.push((start, start + old.len(), new));
    }
    matches.sort_by_key(|(start, _, _)| *start);
    let mut result = String::with_capacity(html.len());
    let mut at = 0;
    for (start, end, new) in matches {
        if start < at {
            return None;
        }
        result.push_str(&html[at..start]);
        result.push_str(&new);
        at = end;
    }
    result.push_str(&html[at..]);
    Some(result)
}

fn title_of(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let title = html[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PAGE: &str = "<html><head><title>Aurora\n  sky</title></head><body>blue</body></html>";

    #[test]
    fn a_written_page_is_shown_with_its_title() {
        let mut pages = Pages::default();
        let page = pages
            .follow("write", &json!({"content": PAGE}), "demo/aurora.html")
            .unwrap();
        assert_eq!(page.html.as_deref(), Some(PAGE));
        assert_eq!(page.title(), "Aurora sky");
        assert_eq!(page.file_name(), "aurora.html");
        assert!(
            pages
                .follow("write", &json!({"content": "x"}), "src/main.rs")
                .is_none()
        );
    }

    #[test]
    fn edits_apply_to_the_page_as_it_was() {
        let mut pages = Pages::default();
        pages.follow("write", &json!({"content": PAGE}), "a.html");
        let edits = json!({"edits": [
            {"oldText": "blue", "newText": "green"},
            {"oldText": "<html>", "newText": "<!doctype html><html>"},
        ]});
        let page = pages.follow("edit", &edits, "a.html").unwrap();
        assert_eq!(
            page.html.as_deref(),
            Some(
                "<!doctype html><html><head><title>Aurora\n  sky</title></head><body>green</body></html>"
            )
        );
        // The older single-edit shape, and the list sent as a string.
        let page = pages
            .follow(
                "edit",
                &json!({"oldText": "green", "newText": "red"}),
                "a.html",
            )
            .unwrap();
        assert!(page.html.unwrap().contains("red"));
        let text = json!({"edits": "[{\"oldText\":\"red\",\"newText\":\"gold\"}]"});
        assert!(
            pages
                .follow("edit", &text, "a.html")
                .unwrap()
                .html
                .unwrap()
                .contains("gold")
        );
    }

    #[test]
    fn edits_the_app_cant_follow_leave_the_page_unknown() {
        let mut pages = Pages::default();
        // An edit to a page that was never written here is not shown.
        let edit = json!({"oldText": "a", "newText": "b"});
        assert!(pages.follow("edit", &edit, "old.html").is_none());
        pages.follow("write", &json!({"content": "<p>a a</p>"}), "p.html");
        // Not unique: Pi would refuse it or match loosely.
        assert_eq!(pages.follow("edit", &edit, "p.html").unwrap().html, None);
        // And it stays unknown until Pi writes the page again.
        let next = json!({"oldText": "<p>", "newText": "<div>"});
        assert_eq!(pages.follow("edit", &next, "p.html").unwrap().html, None);
        let page = pages.follow("write", &json!({"content": "<p>c</p>"}), "p.html");
        assert_eq!(page.unwrap().html.as_deref(), Some("<p>c</p>"));
    }
}
