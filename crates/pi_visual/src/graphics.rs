//! Bounded drawing data. These are values, not native drawing capabilities.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Animation {
    /// Omit to follow the display; an optional cap is useful for slower simulations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    /// A boolean state field. False is a script-controlled pause.
    pub running: String,
}

impl Animation {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(fps) = self.fps {
            ensure!(
                (1..=240).contains(&fps),
                "Animation fps must be 1..240 or omitted for display rate"
            );
        }
        ensure!(
            !self.running.is_empty() && self.running.len() <= 64,
            "Invalid animation binding"
        );
        Ok(())
    }
}

/// All positions are in the canvas's view_box coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Shape {
    Polyline {
        points: Vec<[f32; 2]>,
        color: String,
        width: f32,
    },
    Line {
        from: [f32; 2],
        to: [f32; 2],
        color: String,
        width: f32,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        color: String,
        width: f32,
        #[serde(default)]
        fill: Option<String>,
    },
    Rect {
        origin: [f32; 2],
        size: [f32; 2],
        color: String,
        width: f32,
        #[serde(default)]
        fill: Option<String>,
    },
}

pub type Scene = Vec<Shape>;

pub fn valid_color(color: &str) -> bool {
    matches!(color.len(), 7 | 9)
        && color.starts_with('#')
        && color[1..].bytes().all(|c| c.is_ascii_hexdigit())
}

fn pair(value: &[f32; 2]) -> bool {
    value
        .iter()
        .all(|n| n.is_finite() && n.abs() <= 1_000_000.0)
}

pub(crate) fn validate_scene(scene: &Scene) -> Result<()> {
    ensure!(scene.len() <= 64, "Scene has too many shapes");
    let mut vertices = 0;
    for shape in scene {
        let (color, width, fill) = match shape {
            Shape::Polyline {
                points,
                color,
                width,
            } => {
                ensure!(
                    (2..=64).contains(&points.len()) && points.iter().all(pair),
                    "Invalid polyline"
                );
                vertices += points.len();
                (color, width, None)
            }
            Shape::Line {
                from,
                to,
                color,
                width,
            } => {
                ensure!(pair(from) && pair(to), "Invalid line");
                vertices += 2;
                (color, width, None)
            }
            Shape::Circle {
                center,
                radius,
                color,
                width,
                fill,
            } => {
                ensure!(
                    pair(center) && radius.is_finite() && (0.0..=1_000_000.0).contains(radius),
                    "Invalid circle"
                );
                vertices += 64;
                (color, width, fill.as_ref())
            }
            Shape::Rect {
                origin,
                size,
                color,
                width,
                fill,
            } => {
                ensure!(
                    pair(origin) && pair(size) && size.iter().all(|n| *n >= 0.0),
                    "Invalid rectangle"
                );
                vertices += 4;
                (color, width, fill.as_ref())
            }
        };
        ensure!(
            valid_color(color) && fill.is_none_or(|c| valid_color(c)),
            "Invalid shape color"
        );
        ensure!(
            width.is_finite() && (0.5..=8.0).contains(width),
            "Invalid stroke width"
        );
        ensure!(vertices <= 2048, "Scene has too many vertices");
    }
    Ok(())
}
