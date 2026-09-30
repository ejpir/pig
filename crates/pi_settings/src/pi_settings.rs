//! Settings files and their schemas, GPUI-free: pi's `settings.json` with the
//! settings pi documents, read and written for the user (`<agent-dir>/settings.json`)
//! and project (`.pi/settings.json`). [`Setting`] and [`SettingsFile`] know no pi
//! keys, so the desktop's own preferences use them with a schema of their own.
//!
//! pi's schema is generated from pi's settings reference by `gen_schema.py`. Files
//! keep keys the schema does not know and their order; a file that is not valid
//! JSON is shown but never written, so a hand edit is not lost.
mod schema;

use anyhow::{Context as _, Result, bail};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub use schema::SETTINGS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Number,
    Text,
    /// JSON literals, such as `"medium"`, `0` or `false`.
    Choice(&'static [&'static str]),
    /// A list of strings.
    List,
    /// An object or array, edited in the file.
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Any,
    /// Only the user (agent-directory) file; pi ignores it in project files.
    UserOnly,
}

#[derive(Debug)]
pub struct Setting {
    /// Dotted for nested objects: `compaction.enabled`.
    pub key: &'static str,
    pub category: &'static str,
    pub title: &'static str,
    pub kind: Kind,
    /// A JSON literal, when pi's docs give a concrete default.
    pub default: Option<&'static str>,
    /// The default as the docs word it: `"medium"`, `Automatic`, `All available models`.
    pub default_text: &'static str,
    pub description: &'static str,
    pub scope: Scope,
}

impl Setting {
    pub fn default_value(&self) -> Option<Value> {
        serde_json::from_str(self.default?).ok()
    }

    pub fn choices(&self) -> Vec<Value> {
        match self.kind {
            Kind::Choice(options) => options
                .iter()
                .filter_map(|option| serde_json::from_str(option).ok())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether a value from a file has this setting's type. A hand edit of the
    /// wrong type then falls back to the default instead of being misread.
    pub fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            Kind::Bool => value.is_boolean(),
            Kind::Number => value.is_number(),
            Kind::Text => value.is_string(),
            Kind::Choice(_) => self.choices().contains(value),
            Kind::List => value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
            Kind::Json => true,
        }
    }

    /// What values are allowed, for the inspector.
    pub fn allowed(&self) -> String {
        match self.kind {
            Kind::Bool => "true, false".into(),
            Kind::Number => "a number".into(),
            Kind::Text => "text".into(),
            Kind::Choice(_) => self
                .choices()
                .iter()
                .map(display)
                .collect::<Vec<_>>()
                .join(", "),
            Kind::List => "a list of text".into(),
            Kind::Json => "JSON, edited in the file".into(),
        }
    }

    /// Parses what was typed for a text, number or list setting.
    pub fn parse(&self, input: &str) -> Result<Value> {
        let input = input.trim();
        Ok(match self.kind {
            Kind::Text => Value::String(input.to_owned()),
            Kind::Number => {
                let number: f64 = input.parse().context("Enter a number")?;
                serde_json::Number::from_f64(number)
                    .map(|n| {
                        if number.fract() == 0.0 && number.abs() < 9e15 {
                            Value::from(number as i64)
                        } else {
                            Value::Number(n)
                        }
                    })
                    .context("Enter a finite number")?
            }
            Kind::List => Value::Array(
                input
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| Value::String(item.to_owned()))
                    .collect(),
            ),
            _ => bail!("{} is not typed in", self.key),
        })
    }
}

pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|setting| setting.key == key)
}

/// Categories in the order the docs give them.
pub fn categories() -> Vec<&'static str> {
    let mut categories: Vec<&'static str> = Vec::new();
    for setting in SETTINGS {
        if !categories.contains(&setting.category) {
            categories.push(setting.category);
        }
    }
    categories
}

/// A value as a person reads it: strings without quotes, lists with commas.
pub fn display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

/// `<agent-dir>/settings.json`, where `PI_CODING_AGENT_DIR` moves the agent directory.
pub fn user_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("PI_CODING_AGENT_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => dirs::home_dir()?.join(".pi").join("agent"),
    };
    Some(dir.join("settings.json"))
}

pub fn project_path(project: &Path) -> PathBuf {
    project.join(".pi").join("settings.json")
}

/// One settings file. Missing is empty; unreadable is an error and read-only.
#[derive(Clone, Debug, Default)]
pub struct SettingsFile {
    pub path: PathBuf,
    pub values: Map<String, Value>,
    pub error: Option<String>,
}

impl SettingsFile {
    /// Blocking file IO; call off the UI thread.
    pub fn load(path: PathBuf) -> Self {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Self {
                    path,
                    ..Default::default()
                };
            }
            Err(error) => {
                return Self {
                    path,
                    error: Some(error.to_string()),
                    ..Default::default()
                };
            }
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(values)) => Self {
                path,
                values,
                error: None,
            },
            Ok(_) => Self {
                path,
                error: Some("settings.json is not a JSON object".into()),
                ..Default::default()
            },
            Err(error) => Self {
                path,
                error: Some(format!("settings.json is not valid JSON: {error}")),
                ..Default::default()
            },
        }
    }

    /// The value at a dotted key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        let mut parts = key.split('.');
        let mut value = self.values.get(parts.next()?)?;
        for part in parts {
            value = value.as_object()?.get(part)?;
        }
        Some(value)
    }

    pub fn set(&mut self, key: &str, value: Value) {
        let parts: Vec<&str> = key.split('.').collect();
        let mut object = &mut self.values;
        for part in &parts[..parts.len() - 1] {
            let entry = object
                .entry(*part)
                .or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            object = entry.as_object_mut().unwrap();
        }
        object.insert(parts[parts.len() - 1].to_owned(), value);
    }

    /// Removes a key, and parent objects it leaves empty: pi then uses its default.
    pub fn remove(&mut self, key: &str) {
        fn remove(object: &mut Map<String, Value>, parts: &[&str]) {
            match parts {
                [last] => {
                    object.shift_remove(*last);
                }
                [first, rest @ ..] => {
                    if let Some(Value::Object(child)) = object.get_mut(*first) {
                        remove(child, rest);
                        if child.is_empty() {
                            object.shift_remove(*first);
                        }
                    }
                }
                [] => {}
            }
        }
        let parts: Vec<&str> = key.split('.').collect();
        remove(&mut self.values, &parts);
    }

    /// The settings of `schema` this file sets.
    pub fn changed(&self, schema: &'static [Setting]) -> Vec<&'static Setting> {
        schema
            .iter()
            .filter(|setting| self.get(setting.key).is_some())
            .collect()
    }

    /// Blocking; writes the whole file through a temporary file, two-space indented.
    pub fn save(&self) -> Result<()> {
        if let Some(error) = &self.error {
            bail!("{error}; fix the file by hand first");
        }
        let dir = self.path.parent().context("settings path has no folder")?;
        std::fs::create_dir_all(dir)?;
        let mut text = serde_json::to_string_pretty(&self.values)?;
        text.push('\n');
        let name = self
            .path
            .file_name()
            .context("settings path has no file name")?;
        let temporary = dir.join(format!(".{}.pi-desktop", name.to_string_lossy()));
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_schema_covers_pi_settings_with_types_and_defaults() {
        let thinking = setting("defaultThinkingLevel").unwrap();
        assert_eq!(thinking.category, "Model & thinking");
        assert_eq!(thinking.title, "Default thinking level");
        assert_eq!(thinking.default_value(), Some(json!("medium")));
        assert_eq!(thinking.choices().len(), 7);
        assert_eq!(
            setting("defaultTools").unwrap().default_value(),
            Some(json!(["read", "bash", "edit", "write"]))
        );
        assert_eq!(setting("compaction.enabled").unwrap().kind, Kind::Bool);
        assert_eq!(setting("defaultModel").unwrap().default, None);
        assert_eq!(setting("defaultModel").unwrap().default_text, "Automatic");
        assert_eq!(
            setting("defaultProjectTrust").unwrap().scope,
            Scope::UserOnly
        );
        assert_eq!(
            setting("terminal.images").unwrap().choices().last(),
            Some(&json!(false))
        );
        assert_eq!(categories()[0], "Model & thinking");
        assert!(SETTINGS.len() > 60);
    }

    #[test]
    fn typed_values_parse_by_kind() {
        let tokens = setting("compaction.reserveTokens").unwrap();
        assert_eq!(tokens.parse("8192").unwrap(), json!(8192));
        assert!(tokens.parse("lots").is_err());
        let tools = setting("defaultTools").unwrap();
        assert_eq!(tools.parse("read, bash,").unwrap(), json!(["read", "bash"]));
        assert_eq!(display(&json!(["read", "bash"])), "read, bash");
        let thinking = setting("defaultThinkingLevel").unwrap();
        assert!(thinking.accepts(&json!("high")) && !thinking.accepts(&json!("loud")));
        assert!(!tokens.accepts(&json!("8192")) && tools.accepts(&json!(["read"])));
    }

    #[test]
    fn files_keep_unknown_keys_and_order_and_drop_empty_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent").join("settings.json");
        let mut file = SettingsFile::load(path.clone());
        assert!(
            file.values.is_empty() && file.error.is_none(),
            "a missing file is empty"
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"zeta": 1, "theme": "dark", "custom": {"x": true}}"#,
        )
        .unwrap();
        file = SettingsFile::load(path.clone());
        file.set("compaction.enabled", json!(false));
        file.set("defaultThinkingLevel", json!("high"));
        assert_eq!(file.get("compaction.enabled"), Some(&json!(false)));
        assert_eq!(
            file.changed(SETTINGS)
                .iter()
                .map(|s| s.key)
                .collect::<Vec<_>>(),
            ["defaultThinkingLevel", "compaction.enabled", "theme"]
        );
        file.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("{\n  \"zeta\": 1,\n  \"theme\": \"dark\",\n  \"custom\""),
            "{text}"
        );
        file.remove("compaction.enabled");
        assert!(
            !file.values.contains_key("compaction"),
            "empty parents go too"
        );

        std::fs::write(&path, "{ oops").unwrap();
        let broken = SettingsFile::load(path.clone());
        assert!(broken.error.is_some());
        assert!(
            broken.save().is_err(),
            "a hand edit that does not parse is never overwritten"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ oops");
    }
}
