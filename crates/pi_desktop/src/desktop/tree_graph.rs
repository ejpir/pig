//! Native vector rails and nodes for the conversation tree. Graph topology comes
//! from stable parent IDs; filtering does not turn it into decorative indenting.
use super::*;
use gpui::{Bounds, ContentMask, PathBuilder, canvas, point};
use pi_core::history::{Connection, Row};
use std::sync::Arc;
pub const PITCH: f32 = 36.;
pub const CENTER: f32 = 16.;
fn lane(depth: usize) -> f32 {
    36. + depth.min(6) as f32 * 26.
}
#[derive(Clone)]
pub struct Graph {
    rows: Arc<Vec<Row>>,
    edges: Arc<Vec<Connection>>,
    pub text_left: f32,
}
impl Graph {
    pub fn new(rows: Vec<Row>, edges: Vec<Connection>) -> Self {
        let depth = rows
            .iter()
            .map(|r| r.depth.min(6))
            .max()
            .unwrap_or(0)
            .max(1);
        Self {
            rows: Arc::new(rows),
            edges: Arc::new(edges),
            text_left: lane(depth) + 22.,
        }
    }
    pub fn rail_x(&self, index: usize) -> f32 {
        lane(self.rows[index].depth)
    }
    pub fn element(&self, index: usize, theme: Theme) -> impl IntoElement {
        let graph = self.clone();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    for edge in graph
                        .edges
                        .iter()
                        .filter(|e| e.parent <= index && e.child >= index)
                    {
                        let x0 = bounds.left() + px(graph.rail_x(edge.parent));
                        let x1 = bounds.left() + px(graph.rail_x(edge.child));
                        let y0 =
                            bounds.top() + px((edge.parent as f32 - index as f32) * PITCH + CENTER);
                        let y1 =
                            bounds.top() + px((edge.child as f32 - index as f32) * PITCH + CENTER);
                        let mut path = PathBuilder::stroke(px(1.6));
                        path.move_to(point(x0, y0));
                        if x0 == x1 {
                            path.line_to(point(x1, y1));
                        } else {
                            // Leave the parent smoothly, then follow the branch's rail.
                            let bend = y0 + px(PITCH).min(y1 - y0);
                            path.cubic_bezier_to(
                                point(x1, bend),
                                point(x0, y0 + px(18.)),
                                point(x1, bend - px(18.)),
                            );
                            path.line_to(point(x1, y1));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(
                                path,
                                if edge.active {
                                    theme.accent.opacity(0.8)
                                } else {
                                    theme.line_strong
                                },
                            );
                        }
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}
/// A dot, ring, rounded square, or diamond, not a font-dependent text glyph.
pub fn node(kind: &str, active: bool, theme: Theme) -> AnyElement {
    let color = match kind {
        "you" => {
            if active {
                theme.steel
            } else {
                theme.faint
            }
        }
        "pi" => {
            if active {
                theme.secondary
            } else {
                theme.faint
            }
        }
        "tool" => {
            if active {
                theme.muted
            } else {
                theme.line_strong
            }
        }
        "compaction" => theme.amber,
        _ => theme.faint,
    };
    if kind == "compaction" {
        return canvas(
            |_, _, _| (),
            move |b: Bounds<gpui::Pixels>, _, w, _| {
                let mut path = PathBuilder::fill();
                let c = b.center();
                path.move_to(point(c.x, b.top()));
                path.line_to(point(b.right(), c.y));
                path.line_to(point(c.x, b.bottom()));
                path.line_to(point(b.left(), c.y));
                path.close();
                if let Ok(path) = path.build() {
                    w.paint_path(path, color);
                }
            },
        )
        .size(px(10.))
        .into_any_element();
    }
    div()
        .size(px(if kind == "tool" { 7. } else { 9. }))
        .rounded(px(if kind == "tool" { 1.5 } else { 9. }))
        .when(kind == "pi", |d| {
            d.border(px(1.5)).border_color(color).bg(theme.canvas)
        })
        .when(kind != "pi", |d| d.bg(color))
        .into_any_element()
}
