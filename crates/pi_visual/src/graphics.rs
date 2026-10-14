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
    Path {
        commands: Vec<PathCommand>,
        /// SVG strokes are in source units; legacy paths retain dp widths.
        #[serde(default)]
        scale_stroke: bool,
        #[serde(default)]
        color: Option<Paint>,
        #[serde(default = "stroke_width")]
        width: f32,
        #[serde(default)]
        fill: Option<Paint>,
        #[serde(default)]
        even_odd: bool,
    },
    Group {
        transform: [f32; 6],
        children: Scene,
    },
    Text {
        origin: [f32; 2],
        text: String,
        size: f32,
        color: Paint,
        #[serde(default)]
        align: TextAlign,
        #[serde(default)]
        mono: bool,
    },
    Polyline {
        points: Vec<[f32; 2]>,
        color: Paint,
        width: f32,
    },
    Line {
        from: [f32; 2],
        to: [f32; 2],
        color: Paint,
        width: f32,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        color: Paint,
        width: f32,
        #[serde(default)]
        fill: Option<Paint>,
    },
    Rect {
        origin: [f32; 2],
        size: [f32; 2],
        color: Paint,
        width: f32,
        #[serde(default)]
        fill: Option<Paint>,
    },
    /// Projected natively into `viewport` (viewBox units), or the whole viewBox.
    Scene3d {
        #[serde(default)]
        viewport: Option<[f32; 4]>,
        camera: crate::scene3d::Camera,
        /// Direction toward the light, in scene space.
        #[serde(default)]
        light: Option<[f32; 3]>,
        objects: Vec<crate::scene3d::Object>,
    },
}

fn stroke_width() -> f32 {
    1.5
}

/// Two-stop, bounds-relative linear paint. No asset or shader capabilities.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Paint {
    Solid(String),
    Linear(LinearGradient),
    SvgLinear(SvgLinearGradient),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearGradient {
    pub angle: f32,
    pub from: String,
    pub to: String,
}
/// A source-local linear color parameter t = ax + by + c. The host maps this
/// covector through the inverse composed transform, including viewBox fitting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SvgLinearGradient {
    pub projection: [f64; 3],
    pub from: String,
    pub to: String,
}
impl Paint {
    pub fn valid(&self) -> bool {
        match self {
            Self::Solid(c) => valid_color(c),
            Self::SvgLinear(g) => {
                g.projection
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1e12)
                    && valid_color(&g.from)
                    && valid_color(&g.to)
            }
            Self::Linear(g) => {
                g.angle.is_finite()
                    && g.angle.abs() <= 36000.
                    && valid_color(&g.from)
                    && valid_color(&g.to)
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PathCommand {
    Move {
        to: [f32; 2],
    },
    Line {
        to: [f32; 2],
    },
    Quad {
        control: [f32; 2],
        to: [f32; 2],
    },
    Cubic {
        control1: [f32; 2],
        control2: [f32; 2],
        to: [f32; 2],
    },
    Close,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
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
    let mut budget = Budget::default();
    validate_shapes(scene, 0, [1., 0., 0., 1., 0., 0.], &mut budget)
}
#[derive(Default)]
struct Budget {
    shapes: usize,
    vertices: usize,
    glyphs: usize,
    scenes3d: usize,
    space: crate::scene3d::Budget,
}

pub fn multiply(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
fn validate_shapes(
    scene: &Scene,
    depth: usize,
    matrix: [f32; 6],
    budget: &mut Budget,
) -> Result<()> {
    ensure!(depth <= 8, "Drawing groups too deep");
    for shape in scene {
        budget.shapes += 1;
        ensure!(budget.shapes <= 64, "Scene has too many shapes");
        let (color, width, fill) = match shape {
            Shape::Group {
                transform,
                children,
            } => {
                let next = multiply(matrix, *transform);
                ensure!(
                    transform
                        .iter()
                        .chain(next.iter())
                        .all(|v| v.is_finite() && v.abs() <= 1e6),
                    "Invalid transform"
                );
                validate_shapes(children, depth + 1, next, budget)?;
                continue;
            }
            Shape::Scene3d {
                viewport,
                camera,
                light,
                objects,
            } => {
                budget.scenes3d += 1;
                ensure!(budget.scenes3d <= 4, "Scene has more than 4 3D scenes");
                crate::scene3d::validate(viewport, camera, light, objects, &mut budget.space)?;
                continue;
            }
            Shape::Text {
                origin,
                text,
                size,
                color,
                ..
            } => {
                ensure!(
                    pair(origin) && size.is_finite() && (4.0..=128.0).contains(size),
                    "Invalid canvas text geometry"
                );
                ensure!(
                    matrix[1] == 0. && matrix[2] == 0. && matrix[0] > 0. && matrix[0] == matrix[3],
                    "Canvas text supports translation and positive uniform scale only"
                );
                ensure!(
                    text.len() <= 2048 && !text.contains(['\n', '\r']),
                    "Canvas text must be a bounded single line"
                );
                budget.glyphs += text.chars().count();
                ensure!(
                    budget.glyphs <= 2048
                        && color.valid()
                        && matches!(color, Paint::Solid(_))
                        && (4.0..=256.0).contains(&(size * matrix[3])),
                    "Invalid canvas text"
                );
                continue;
            }
            Shape::Path {
                commands,
                scale_stroke,
                color,
                width,
                fill,
                ..
            } => {
                ensure!(
                    !commands.is_empty()
                        && commands.len() <= 256
                        && matches!(commands[0], PathCommand::Move { .. }),
                    "Path must start with move and contain <=256 commands"
                );
                let mut open = false;
                for command in commands {
                    let valid = match command {
                        PathCommand::Move { to } => {
                            open = true;
                            pair(to)
                        }
                        PathCommand::Line { to } => open && pair(to),
                        PathCommand::Quad { control, to } => open && pair(control) && pair(to),
                        PathCommand::Cubic {
                            control1,
                            control2,
                            to,
                        } => open && pair(control1) && pair(control2) && pair(to),
                        PathCommand::Close => {
                            let was_open = open;
                            open = false;
                            was_open
                        }
                    };
                    ensure!(valid, "Invalid path command");
                    // Stable source-complexity weights, independent of display density.
                    // Hosts enforce separate bounded native tessellation/mesh budgets.
                    budget.vertices += match command {
                        PathCommand::Quad { .. } | PathCommand::Cubic { .. } => 32,
                        _ => 1,
                    };
                }
                ensure!(
                    color.is_some() || fill.is_some(),
                    "Path needs a stroke or fill"
                );
                ensure!(
                    color.as_ref().is_none_or(Paint::valid)
                        && fill.as_ref().is_none_or(Paint::valid),
                    "Invalid path paint"
                );
                ensure!(
                    width.is_finite()
                        && if *scale_stroke {
                            *width > 0. && *width <= 1e6
                        } else {
                            (0.5..=8.0).contains(width)
                        },
                    "Invalid stroke width"
                );
                ensure!(budget.vertices <= 2048, "Scene has too many vertices");
                continue;
            }
            Shape::Polyline {
                points,
                color,
                width,
            } => {
                ensure!(
                    (2..=64).contains(&points.len()) && points.iter().all(pair),
                    "Invalid polyline"
                );
                budget.vertices += points.len();
                (color, width, None)
            }
            Shape::Line {
                from,
                to,
                color,
                width,
            } => {
                ensure!(pair(from) && pair(to), "Invalid line");
                budget.vertices += 2;
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
                budget.vertices += 64;
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
                budget.vertices += 4;
                (color, width, fill.as_ref())
            }
        };
        ensure!(
            color.valid() && fill.is_none_or(Paint::valid),
            "Invalid shape color"
        );
        ensure!(
            width.is_finite() && (0.5..=8.0).contains(width),
            "Invalid stroke width"
        );
        ensure!(budget.vertices <= 2048, "Scene has too many vertices");
    }
    Ok(())
}
