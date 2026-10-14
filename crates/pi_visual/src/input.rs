//! Transient, host-owned input. Never serialized as saved application state.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputEvent {
    Pointer {
        id: u64,
        phase: Phase,
        x: f64,
        y: f64,
    },
    Key {
        key: String,
        down: bool,
        repeat: bool,
    },
    Cancel,
    Blur,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pointer {
    pub target: String,
    pub id: u64,
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InputState {
    pub pointers: Vec<Pointer>,
    pub keys: Vec<String>,
    #[serde(skip)]
    pub(crate) key_target: Option<String>,
}
impl InputState {
    pub(crate) fn apply(&mut self, target: &str, event: &InputEvent) -> Result<()> {
        match event {
            InputEvent::Pointer { id, phase, x, y } => {
                ensure!(
                    *id <= 9_007_199_254_740_991
                        && x.is_finite()
                        && y.is_finite()
                        && x.abs() <= 2e6
                        && y.abs() <= 2e6,
                    "Invalid pointer"
                );
                let index = self
                    .pointers
                    .iter()
                    .position(|p| p.id == *id && p.target == target);
                match phase {
                    Phase::Down => {
                        ensure!(
                            index.is_none() && self.pointers.len() < 10,
                            "Duplicate or excess contact"
                        );
                        self.pointers.push(Pointer {
                            target: target.into(),
                            id: *id,
                            x: *x,
                            y: *y,
                        });
                    }
                    Phase::Move => {
                        let p = index
                            .and_then(|i| self.pointers.get_mut(i))
                            .ok_or_else(|| anyhow::anyhow!("Unknown contact"))?;
                        p.x = *x;
                        p.y = *y;
                    }
                    Phase::Up | Phase::Cancel => {
                        if let Some(i) = index {
                            self.pointers.remove(i);
                        }
                    }
                }
            }
            InputEvent::Key { key, down, .. } => {
                ensure!(!key.is_empty() && key.len() <= 64, "Invalid key");
                if *down && self.key_target.as_deref() != Some(target) {
                    self.keys.clear();
                    self.key_target = Some(target.into());
                }
                if *down && !self.keys.contains(key) {
                    ensure!(self.keys.len() < 32, "Too many held keys");
                    self.keys.push(key.clone());
                } else if !down && self.key_target.as_deref() == Some(target) {
                    self.keys.retain(|k| k != key);
                }
            }
            InputEvent::Blur => {
                if self.key_target.as_deref() == Some(target) {
                    self.keys.clear();
                    self.key_target = None;
                }
            }
            InputEvent::Cancel => {
                self.pointers.retain(|p| p.target != target);
                if self.key_target.as_deref() == Some(target) {
                    self.keys.clear();
                    self.key_target = None;
                }
            }
        }
        Ok(())
    }
}
