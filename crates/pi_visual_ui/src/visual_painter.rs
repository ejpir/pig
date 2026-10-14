//! Bounded logical geometry cache, separate from placement and paint.
use crate::visual_mesh::Builder as MeshBuilder;
use crate::visual_tessellation::{self as tessellation, Budget};
use gpui::{
    App, Background, Bounds, ContentMask, FillOptions, FillRule, Hsla, PathStyle, Pixels, Point,
    ShapedLine, StrokeOptions, TextRun, Window, point, px, size,
};
use pi_visual::{
    Scene, Shape,
    graphics::{Paint, PathCommand, SvgLinearGradient, TextAlign, multiply},
};
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
pub struct SceneCache {
    pub painted: Rc<Cell<bool>>,
    scale: Option<f32>,
    geometry: Vec<CachedGeometry>,
    text: Vec<(TextKey, ShapedLine, bool)>,
    pub builds: u64,
    pub hits: u64,
}
#[derive(Clone, PartialEq)]
struct Geometry {
    commands: Vec<PathCommand>,
    steps: Vec<u16>,
    width: f32,
    fill: bool,
    stroke: bool,
    even_odd: bool,
}
struct CachedGeometry {
    key: Geometry,
    paths: Vec<gpui::Path<Pixels>>,
    used: bool,
}
#[derive(PartialEq)]
struct TextKey {
    text: String,
    size: f32,
    mono: bool,
    color: Hsla,
}

fn color(hex: &str) -> Hsla {
    let n = u32::from_str_radix(&hex[1..], 16).unwrap_or(0);
    gpui::rgba(if hex.len() == 7 { (n << 8) | 255 } else { n }).into()
}
// The WGPU path shader derives gradient coordinates from *clipped* bounds.
// Compensate its angle and stop positions locally, so clipping doesn't stretch
// or rotate the gradient. No renderer/dependency changes are necessary.
fn gradient_axis(angle: f32, bounds: Bounds<Pixels>) -> ([f32; 2], f32) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let r = (angle.rem_euclid(360.) - 90.).to_radians();
    let mut n = [r.cos() * w / w.max(h), r.sin() * h / w.max(h)];
    let length = n[0].hypot(n[1]);
    n[0] /= length;
    n[1] /= length;
    let d = if n[0].abs() > n[1].abs() { w } else { h };
    (n, d)
}
fn gradient_mapping(
    angle: f32,
    original: Bounds<Pixels>,
    clipped: Bounds<Pixels>,
) -> (f32, f32, f32) {
    let (n, full) = gradient_axis(angle, original);
    let w = f32::from(clipped.size.width);
    let h = f32::from(clipped.size.height);
    let visible = if n[0].abs() > n[1].abs() { w } else { h };
    let adjusted = (n[1] / h).atan2(n[0] / w).to_degrees() + 90.;
    let shift = clipped.center() - original.center();
    let a = visible / full;
    let b = 0.5 - 0.5 * a + (f32::from(shift.x) * n[0] + f32::from(shift.y) * n[1]) / full;
    (adjusted, -b / a, (1. - b) / a)
}
fn svg_projection(g: &SvgLinearGradient, scale: f32, matrix: [f32; 6]) -> Option<[f64; 3]> {
    let mut m = matrix.map(f64::from);
    for n in &mut m[..4] {
        *n *= f64::from(scale);
    }
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0. || !det.is_finite() {
        return None;
    }
    let p = g.projection;
    let x = (m[3] * p[0] - m[1] * p[1]) / det;
    let y = (-m[2] * p[0] + m[0] * p[1]) / det;
    Some([x, y, p[2] - x * m[4] - y * m[5]])
}
fn svg_gradient_mapping(p: [f64; 3], clip: Bounds<Pixels>) -> anyhow::Result<(f32, f32, f32)> {
    let w = f64::from(f32::from(clip.size.width));
    let h = f64::from(f32::from(clip.size.height));
    let length = p[0].hypot(p[1]);
    let d = if p[0].abs() > p[1].abs() { w } else { h };
    let center = clip.center();
    let t = p[0] * f64::from(f32::from(center.x)) + p[1] * f64::from(f32::from(center.y)) + p[2];
    let angle = ((p[1] / h).atan2(p[0] / w).to_degrees() + 90.) as f32;
    let from = (0.5 - t / (d * length)) as f32;
    let to = (0.5 + (1. - t) / (d * length)) as f32;
    anyhow::ensure!(
        [angle, from, to].iter().all(|v| v.is_finite()) && to - from > 1e-7,
        "SVG gradient is too narrow or ill-conditioned at this scale"
    );
    Ok((angle, from, to))
}
fn paint(
    paint: &Paint,
    bounds: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    scale: f32,
    matrix: [f32; 6],
) -> anyhow::Result<Background> {
    Ok(match paint {
        Paint::Solid(c) => color(c).into(),
        Paint::Linear(g) => {
            let (angle, from, to) = gradient_mapping(g.angle, bounds, clip);
            gpui::linear_gradient(
                angle.rem_euclid(360.),
                gpui::linear_color_stop(color(&g.from), from),
                gpui::linear_color_stop(color(&g.to), to),
            )
        }
        Paint::SvgLinear(g) => {
            let Some(p) = svg_projection(g, scale, matrix) else {
                return Ok(color(&g.to).into());
            };
            let center = clip.center();
            let t = p[0] * f64::from(f32::from(center.x))
                + p[1] * f64::from(f32::from(center.y))
                + p[2];
            let half = (p[0].abs() * f64::from(f32::from(clip.size.width))
                + p[1].abs() * f64::from(f32::from(clip.size.height)))
                / 2.;
            anyhow::ensure!(
                t.is_finite() && half.is_finite(),
                "Invalid transformed SVG gradient"
            );
            if t + half <= 0. {
                return Ok(color(&g.from).into());
            }
            if t - half >= 1. || half == 0. {
                return Ok(color(&g.to).into());
            }
            let (angle, from, to) = svg_gradient_mapping(p, clip)?;
            gpui::linear_gradient(
                angle.rem_euclid(360.),
                gpui::linear_color_stop(color(&g.from), from),
                gpui::linear_color_stop(color(&g.to), to),
            )
        }
    })
}

fn at(p: [f32; 2], m: [f32; 6]) -> Point<Pixels> {
    point(
        px(m[0] * p[0] + m[2] * p[1] + m[4]),
        px(m[1] * p[0] + m[3] * p[1] + m[5]),
    )
}
fn place(mut path: gpui::Path<Pixels>, m: [f32; 6]) -> gpui::Path<Pixels> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for v in &mut path.vertices {
        v.xy_position = at([v.xy_position.x.into(), v.xy_position.y.into()], m);
        let p = [f32::from(v.xy_position.x), f32::from(v.xy_position.y)];
        for i in 0..2 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    if !path.vertices.is_empty() {
        path.bounds = Bounds::new(
            point(px(min[0]), px(min[1])),
            size(px(max[0] - min[0]), px(max[1] - min[1])),
        );
    }
    path
}

fn stroke_builder(width: f32) -> MeshBuilder {
    // Lyon's default 0.1dp tolerance merges nearby polyline vertices, undoing
    // our HiDPI sampling on tiny, thick circles. These contours contain ONLY
    // lines, with default miter joins/butt caps: lowering this merge tolerance
    // does not trigger unbounded native curve/round-join subdivision.
    MeshBuilder::new(PathStyle::Stroke(
        StrokeOptions::default()
            .with_line_width(width)
            .with_tolerance(1e-12),
    ))
}

// Counts are chosen from physical-pixel error bounds before native tessellation.
fn contour(builder: &mut MeshBuilder, commands: &[PathCommand], steps: &[u16], scale: f32) {
    let mut from = [0., 0.];
    let p = |v: [f32; 2]| point(px(v[0] * scale), px(v[1] * scale));
    for (c, steps) in commands.iter().zip(steps) {
        match c {
            PathCommand::Move { to } => {
                builder.move_to(p(*to));
                from = *to;
            }
            PathCommand::Line { to } => {
                builder.line_to(p(*to));
                from = *to;
            }
            PathCommand::Quad { control, to } => {
                for i in 1..=*steps {
                    let t = f32::from(i) / f32::from(*steps);
                    let u = 1. - t;
                    builder.line_to(p([
                        u * u * from[0] + 2. * u * t * control[0] + t * t * to[0],
                        u * u * from[1] + 2. * u * t * control[1] + t * t * to[1],
                    ]));
                }
                from = *to;
            }
            PathCommand::Cubic {
                control1: a,
                control2: b,
                to,
            } => {
                for i in 1..=*steps {
                    let t = f32::from(i) / f32::from(*steps);
                    let u = 1. - t;
                    builder.line_to(p([
                        u * u * u * from[0]
                            + 3. * u * u * t * a[0]
                            + 3. * u * t * t * b[0]
                            + t * t * t * to[0],
                        u * u * u * from[1]
                            + 3. * u * u * t * a[1]
                            + 3. * u * t * t * b[1]
                            + t * t * t * to[1],
                    ]));
                }
                from = *to;
            }
            PathCommand::Close => builder.close(),
        }
    }
}
fn geometry(
    shape: &Shape,
    physical_scale: f64,
    stroke_scale: f64,
    view_scale: f32,
) -> (Geometry, Option<&Paint>, Option<&Paint>) {
    let (commands, stroke, width, fill, even_odd) = match shape {
        Shape::Path {
            commands,
            scale_stroke,
            color,
            width,
            fill,
            even_odd,
        } => (
            commands.clone(),
            color.as_ref(),
            if *scale_stroke {
                *width * view_scale
            } else {
                *width
            },
            fill.as_ref(),
            *even_odd,
        ),
        Shape::Polyline {
            points,
            color,
            width,
        } => (polyline(points), Some(color), *width, None, false),
        Shape::Line {
            from,
            to,
            color,
            width,
        } => (polyline(&[*from, *to]), Some(color), *width, None, false),
        Shape::Rect {
            origin: [x, y],
            size: [w, h],
            color,
            width,
            fill,
        } => {
            let mut commands = polyline(&[[*x, *y], [x + w, *y], [x + w, y + h], [*x, y + h]]);
            commands.push(PathCommand::Close);
            (commands, Some(color), *width, fill.as_ref(), false)
        }
        Shape::Circle {
            center: [x, y],
            radius,
            color,
            width,
            fill,
        } => {
            let segments = tessellation::circle_steps(
                *radius,
                physical_scale,
                f64::from(*width) * stroke_scale / 2.,
            );
            let points: Vec<_> = (0..segments)
                .map(|i| {
                    let a = f32::from(i) / f32::from(segments) * std::f32::consts::TAU;
                    [x + radius * a.cos(), y + radius * a.sin()]
                })
                .collect();
            let mut commands = polyline(&points);
            commands.push(PathCommand::Close);
            (commands, Some(color), *width, fill.as_ref(), false)
        }
        _ => unreachable!(),
    };
    let mut steps = tessellation::curve_steps(&commands, physical_scale);
    if stroke.is_some() {
        // Preserve the legacy floor for stroked curves, and account for the
        // stroke envelope when SVG ellipses/arcs lower to tiny Béziers. A small
        // centerline error alone does not keep a thick outline looking round.
        let floor = tessellation::circle_steps(0., 0., f64::from(width) * stroke_scale / 2.)
            .clamp(32, tessellation::MAX_CURVE_SEGMENTS as u16);
        for (command, steps) in commands.iter().zip(&mut steps) {
            if matches!(
                command,
                PathCommand::Quad { .. } | PathCommand::Cubic { .. }
            ) {
                *steps = (*steps).max(floor);
            }
        }
    }
    (
        Geometry {
            steps,
            commands,
            width,
            fill: fill.is_some(),
            stroke: stroke.is_some(),
            even_odd,
        },
        fill,
        stroke,
    )
}
fn polyline(points: &[[f32; 2]]) -> Vec<PathCommand> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i == 0 {
                PathCommand::Move { to: *p }
            } else {
                PathCommand::Line { to: *p }
            }
        })
        .collect()
}
fn shapes(
    scene: &Scene,
    matrix: [f32; 6],
    scale: f32,
    cache: &mut SceneCache,
    budget: &mut Budget,
    window: &mut Window,
    cx: &mut App,
) -> anyhow::Result<()> {
    for shape in scene {
        match shape {
            Shape::Group {
                transform,
                children,
            } => {
                let mut t = *transform;
                t[4] *= scale;
                t[5] *= scale;
                shapes(
                    children,
                    multiply(matrix, t),
                    scale,
                    cache,
                    budget,
                    window,
                    cx,
                )?;
            }
            Shape::Text {
                origin,
                text,
                size,
                color: ink,
                align,
                mono,
            } => {
                let Paint::Solid(ink) = ink else {
                    anyhow::bail!("Canvas text requires solid paint");
                };
                let font_size = size * matrix[3];
                let key = TextKey {
                    text: text.clone(),
                    size: font_size,
                    mono: *mono,
                    color: color(ink),
                };
                let index = cache
                    .text
                    .iter()
                    .position(|(k, _, _)| *k == key)
                    .unwrap_or_else(|| {
                        let line = window.text_system().shape_line(
                            text.clone().into(),
                            px(font_size),
                            &[TextRun {
                                len: text.len(),
                                font: gpui::font(if *mono {
                                    crate::theme::MONO
                                } else {
                                    crate::theme::SANS
                                }),
                                color: key.color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        );
                        cache.text.push((key, line, false));
                        cache.text.len() - 1
                    });
                let (_, line, used) = &mut cache.text[index];
                *used = true;
                let mut origin = at([origin[0] * scale, origin[1] * scale], matrix);
                origin.x -= match align {
                    TextAlign::Left => px(0.),
                    TextAlign::Center => line.width / 2.,
                    TextAlign::Right => line.width,
                };
                line.paint(
                    origin,
                    px(font_size * 1.25),
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                )?;
            }
            _ => {
                let quality = tessellation::physical_scale(scale, window.scale_factor(), matrix);
                let stroke_scale = tessellation::physical_scale(1., window.scale_factor(), matrix);
                let (key, fill, stroke) = geometry(shape, quality, stroke_scale, scale);
                anyhow::ensure!(
                    key.width.is_finite() && key.width > 0. && key.width <= 1e6,
                    "Native scaled stroke width out of range"
                );
                // Charge instances, not just misses, so cache hits cannot evade limits.
                budget.points(key.steps.iter().map(|n| usize::from(*n)).sum())?;
                let index = if let Some(index) = cache.geometry.iter().position(|c| c.key == key) {
                    budget.mesh(
                        cache.geometry[index]
                            .paths
                            .iter()
                            .map(|p| p.vertices.len())
                            .sum(),
                    )?;
                    cache.hits += 1;
                    index
                } else {
                    let mut paths = Vec::new();
                    if key.fill {
                        let mut b = MeshBuilder::new(PathStyle::Fill(
                            FillOptions::default().with_fill_rule(if key.even_odd {
                                FillRule::EvenOdd
                            } else {
                                FillRule::NonZero
                            }),
                        ));
                        contour(&mut b, &key.commands, &key.steps, scale);
                        let path = b.build(budget.remaining_mesh())?;
                        budget.mesh(path.vertices.len())?;
                        paths.push(path);
                    }
                    if key.stroke {
                        let mut b = stroke_builder(key.width);
                        contour(&mut b, &key.commands, &key.steps, scale);
                        let path = b.build(budget.remaining_mesh())?;
                        budget.mesh(path.vertices.len())?;
                        paths.push(path);
                    }
                    cache.geometry.push(CachedGeometry {
                        key,
                        paths,
                        used: false,
                    });
                    cache.builds += 1;
                    cache.geometry.len() - 1
                };
                let entry = &mut cache.geometry[index];
                entry.used = true;
                for (path, ink) in entry.paths.iter().zip(fill.into_iter().chain(stroke)) {
                    let path = place(path.clone(), matrix);
                    let clip = path.bounds.intersect(&window.content_mask().bounds);
                    if clip.size.width > px(0.) && clip.size.height > px(0.) {
                        let ink = paint(ink, path.bounds, clip, scale, matrix)?;
                        window.paint_path(path, ink);
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn paint_scene(
    scene: &Scene,
    view_box: [f32; 4],
    bounds: Bounds<Pixels>,
    cache: &mut SceneCache,
    window: &mut Window,
    cx: &mut App,
) -> anyhow::Result<()> {
    let (origin, scale) = crate::visual_controls::transform(bounds, view_box);
    if cache.scale != Some(scale) {
        cache.geometry.clear();
        cache.scale = Some(scale);
    }
    for c in &mut cache.geometry {
        c.used = false;
    }
    for (_, _, used) in &mut cache.text {
        *used = false;
    }
    let mut budget = Budget::default();
    let result = window.with_content_mask(Some(ContentMask { bounds }), |window| {
        shapes(
            scene,
            [1., 0., 0., 1., origin.x.into(), origin.y.into()],
            scale,
            cache,
            &mut budget,
            window,
            cx,
        )
    });
    cache.geometry.retain(|c| c.used);
    cache.text.retain(|(_, _, used)| *used);
    result
}

/// Native failures stay visible rather than silently dropping geometry.
pub fn paint_error(message: &str, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let message = message.replace(['\n', '\r'], " ");
    let line = window.text_system().shape_line(
        message.clone().into(),
        px(14.),
        &[TextRun {
            len: message.len(),
            font: gpui::font(crate::theme::SANS),
            color: gpui::rgb(0xff7777).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let _ = line.paint(
            bounds.origin,
            px(20.),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
    });
}

pub fn paint_button_label(
    text: &str,
    color: Hsla,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let line = window.text_system().shape_line(
        text.to_owned().into(),
        px(18.),
        &[TextRun {
            len: text.len(),
            font: gpui::font(crate::theme::SANS),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let _ = line.paint(
            bounds.center() - point(line.width / 2., px(12.)),
            px(24.),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual_controls::{ProjectionBarrier, Surface};
    use gpui::{Context, IntoElement, Render, div, prelude::*};

    #[test]
    fn clipped_gradient_keeps_original_colors_at_every_angle() {
        let original = Bounds::new(point(px(-100.), px(-50.)), size(px(320.), px(180.)));
        let clip = Bounds::new(point(px(0.), px(0.)), size(px(110.), px(100.)));
        for angle in [0., 30., 45., 90., 135., 180., 225., 270., 315.] {
            let (adjusted, from, to) = gradient_mapping(angle, original, clip);
            let (n, d) = gradient_axis(angle, original);
            let (nc, dc) = gradient_axis(adjusted, clip);
            for point in [clip.origin, clip.center(), clip.bottom_right()] {
                let delta = point - original.center();
                let clipped = point - clip.center();
                let expected = 0.5 + (f32::from(delta.x) * n[0] + f32::from(delta.y) * n[1]) / d;
                let actual = (0.5
                    + (f32::from(clipped.x) * nc[0] + f32::from(clipped.y) * nc[1]) / dc
                    - from)
                    / (to - from);
                assert!(
                    (expected - actual).abs() < 0.0001,
                    "angle {angle}: {expected} vs {actual}"
                );
            }
        }
    }
    struct Fixture {
        scene: Scene,
        counts: Rc<Cell<(u64, u64)>>,
        points: Rc<Cell<usize>>,
    }
    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let scene = self.scene.clone();
            let counts = self.counts.clone();
            let points = self.points.clone();
            div().w_full().child(Surface {
                id: "geometry-cache-test".into(),
                label: "Drawing".into(),
                probe: "geometry-cache-test".into(),
                height: 200.,
                input: None,
                events: None,
                held_button: false,
                slider: None,
                barrier: ProjectionBarrier::default(),
                coordinates: Rc::new(|_, _| [0., 0.]),
                painter: Rc::new(move |bounds, cache, window, cx| {
                    paint_scene(&scene, [0., 0., 100., 100.], bounds, cache, window, cx).unwrap();
                    assert!(cache.geometry.len() <= 64 && cache.text.len() <= 64);
                    counts.set((cache.builds, cache.hits));
                    points.set(
                        cache
                            .geometry
                            .iter()
                            .flat_map(|c| &c.key.steps)
                            .map(|n| usize::from(*n))
                            .sum(),
                    );
                    assert!(cache.text.iter().all(|(key, _, _)| key.size == 16.));
                }),
            })
        }
    }
    fn scene(x: f32, ink: &str) -> Scene {
        pi_visual::Runtime::from_bundle(&format!("const p=ui.path([['M',0,0],['Q',10,0,10,10],['L',0,10],['Z']]);app({{title:'Mesh',state:{{}},view:ui.canvas({{id:'s',draw(g){{g.group({{translate:[{x},20],rotate:20}},g=>g.path(p,{{fill:'{ink}'}}));g.text('HUD',0,0);}}}})}});")).unwrap().scene("s").unwrap().as_ref().clone()
    }
    #[gpui::test]
    fn mesh_and_text_cache_survive_placement_and_paint_changes(cx: &mut gpui::TestAppContext) {
        let counts = Rc::new(Cell::new((0, 0)));
        let (app, cx) = cx.add_window_view(|_, _| Fixture {
            scene: scene(20., "#70B8FF"),
            counts: counts.clone(),
            points: Rc::default(),
        });
        cx.simulate_resize(size(px(320.), px(240.)));
        cx.run_until_parked();
        let before = counts.get();
        assert!(before.0 > 0);
        app.update(cx, |app, cx| {
            app.scene = scene(40., "#FFFFFF");
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(counts.get().0, before.0);
        assert!(counts.get().1 > before.1);
    }
    #[gpui::test]
    fn density_and_group_zoom_upgrade_meshes_but_not_logical_text_size(
        cx: &mut gpui::TestAppContext,
    ) {
        let counts = Rc::new(Cell::new((0, 0)));
        let points = Rc::new(Cell::new(0));
        let (app, cx) = cx.add_window_view(|_, _| Fixture {
            scene: scene(20., "#70B8FF"),
            counts: counts.clone(),
            points: points.clone(),
        });
        cx.simulate_resize(size(px(320.), px(240.)));
        cx.run_until_parked();
        let low = points.get();
        let low_builds = counts.get().0;
        let bounds = cx.debug_bounds("geometry-cache-test").unwrap();
        cx.simulate_scale_factor_change(4.);
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("geometry-cache-test").unwrap(), bounds);
        assert!(points.get() > low && counts.get().0 > low_builds);
        let high = points.get();
        let builds = counts.get().0;
        cx.simulate_scale_factor_change(4.125);
        cx.run_until_parked();
        assert_eq!(counts.get().0, builds); // Same quality bucket: no pointless rebuild.
        app.update(cx, |app, cx| {
            let Shape::Group { transform, .. } = &mut app.scene[0] else {
                panic!()
            };
            for n in &mut transform[..4] {
                *n *= 4.;
            }
            cx.notify();
        });
        cx.run_until_parked();
        assert!(points.get() > high);
        cx.simulate_scale_factor_change(1.);
        cx.run_until_parked();
        assert_eq!(points.get(), high); // 4x group zoom at 1x DPR == 1x zoom at 4x DPR.
    }
    #[test]
    fn circle_resolution_and_curve_plans_are_part_of_geometry_identity() {
        let circle = Shape::Circle {
            center: [10., 10.],
            radius: 100.,
            color: Paint::Solid("#FFFFFF".into()),
            width: 1.5,
            fill: None,
        };
        let low = geometry(&circle, 1., 1., 1.).0;
        let high = geometry(&circle, 4., 4., 1.).0;
        assert!(high.commands.len() > low.commands.len());
        assert_ne!(low.steps.len(), high.steps.len());
        let mut budget = Budget::default();
        let mut enormous = circle;
        if let Shape::Circle { radius, .. } = &mut enormous {
            *radius = 1e6;
        }
        let key = geometry(&enormous, 1e6, 1., 1.).0;
        for _ in 0..32 {
            budget
                .points(key.steps.iter().map(|n| usize::from(*n)).sum())
                .unwrap();
        }
        assert!(budget.points(1).is_err());
    }
    #[test]
    fn tiny_thick_circle_native_stroke_stays_round_at_four_x_density() {
        let circle = Shape::Circle {
            center: [10., 10.],
            radius: 0.1,
            color: Paint::Solid("#FFFFFF".into()),
            width: 8.,
            fill: None,
        };
        let key = geometry(&circle, 4., 4., 1.).0;
        assert!(key.commands.len() > 9); // Eight polygon edges previously made this visibly octagonal.
        let mut builder = stroke_builder(8.);
        contour(&mut builder, &key.commands, &key.steps, 1.);
        let path = builder.build(tessellation::MAX_MESH_VERTICES).unwrap();
        let outer = path
            .vertices
            .iter()
            .map(|v| {
                (f64::from(f32::from(v.xy_position.x)) - 10.)
                    .hypot(f64::from(f32::from(v.xy_position.y)) - 10.)
                    * 4.
            })
            .fold(0_f64, f64::max);
        assert!(
            (outer - (0.1 + 4.) * 4.).abs() <= tessellation::PIXEL_ERROR,
            "outer stroke radius: {outer}"
        );
    }
    #[test]
    fn svg_gradients_follow_composed_transforms_and_clipping() {
        let g = SvgLinearGradient {
            projection: [0.02, -0.03, 0.4],
            from: "#000000".into(),
            to: "#FFFFFF".into(),
        };
        let clip = Bounds::new(point(px(90.), px(190.)), size(px(100.), px(60.)));
        for m in [
            [1., 0., 0., 1., 100., 200.],
            [0.6, 0.8, -1.6, 1.2, 100., 200.],
            [-2., 0.4, 0.3, 1., 100., 200.],
        ] {
            let p = svg_projection(&g, 2.5, m).unwrap();
            for local in [[0., 0.], [10., 20.], [-15., 80.]] {
                let screen = at([local[0] * 2.5, local[1] * 2.5], m);
                let actual = p[0] * f64::from(f32::from(screen.x))
                    + p[1] * f64::from(f32::from(screen.y))
                    + p[2];
                let expected = g.projection[0] * f64::from(local[0])
                    + g.projection[1] * f64::from(local[1])
                    + g.projection[2];
                assert!((actual - expected).abs() < 1e-5);
            }
            let (angle, from, to) = svg_gradient_mapping(p, clip).unwrap();
            let (axis, d) = gradient_axis(angle, clip);
            for point in [clip.origin, clip.center(), clip.bottom_right()] {
                let delta = point - clip.center();
                let t = 0.5 + (f32::from(delta.x) * axis[0] + f32::from(delta.y) * axis[1]) / d;
                let actual = f64::from((t - from) / (to - from));
                let expected = p[0] * f64::from(f32::from(point.x))
                    + p[1] * f64::from(f32::from(point.y))
                    + p[2];
                assert!((actual - expected).abs() < 1e-5, "{actual} vs {expected}");
            }
        }
        assert!(svg_projection(&g, 1., [0.; 6]).is_none());
        assert!(svg_gradient_mapping([1e20, 0., 0.], clip).is_err());
    }
    #[test]
    fn svg_source_strokes_and_legacy_dp_strokes_have_distinct_mesh_identity() {
        let mut p = Shape::Path {
            commands: vec![
                PathCommand::Move { to: [0., 0.] },
                PathCommand::Line { to: [10., 0.] },
            ],
            scale_stroke: false,
            color: Some(Paint::Solid("#FFFFFF".into())),
            width: 2.,
            fill: None,
            even_odd: false,
        };
        let legacy = geometry(&p, 6., 2., 3.).0;
        if let Shape::Path { scale_stroke, .. } = &mut p {
            *scale_stroke = true;
        }
        let svg = geometry(&p, 6., 2., 3.).0;
        assert_eq!(legacy.width, 2.);
        assert_eq!(svg.width, 6.);
        assert!(legacy != svg);
    }
    #[gpui::test]
    fn imported_svg_renders_native_gradients_and_reuses_placed_geometry(
        cx: &mut gpui::TestAppContext,
    ) {
        fn scene(x: u8) -> Scene {
            pi_visual::Runtime::from_bundle(&format!(r##"const icon=ui.svg('<svg viewBox="0 0 100 100"><defs><linearGradient id="g"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><circle cx="30" cy="30" r="20" fill="url(#g)" stroke="white" stroke-width="2"/></svg>');app({{title:'SVG',state:{{}},view:ui.canvas({{id:'s',draw(g){{g.svg(icon,{{x:{x}}});}}}})}});"##)).unwrap().scene("s").unwrap().as_ref().clone()
        }
        let counts = Rc::new(Cell::new((0, 0)));
        let (app, cx) = cx.add_window_view(|_, _| Fixture {
            scene: scene(0),
            counts: counts.clone(),
            points: Rc::default(),
        });
        cx.simulate_resize(size(px(320.), px(240.)));
        cx.run_until_parked();
        let before = counts.get();
        assert!(before.0 > 0);
        app.update(cx, |app, cx| {
            app.scene = scene(10);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(counts.get().0, before.0);
        assert!(counts.get().1 > before.1);
    }
}
