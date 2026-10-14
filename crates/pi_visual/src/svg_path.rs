//! SVG path syntax normalized into the existing bounded M/L/Q/C/Z values.
use crate::graphics::PathCommand as P;
use anyhow::{Result, ensure};
use svgtypes::PathSegment as S;

pub(super) fn number(n: f64) -> Result<f32> {
    ensure!(
        n.is_finite() && n.abs() <= 1e6,
        "SVG number must be finite with abs<=1e6"
    );
    Ok(n as f32)
}
fn point(p: [f64; 2]) -> Result<[f32; 2]> {
    Ok([number(p[0])?, number(p[1])?])
}
fn push(out: &mut Vec<P>, p: P) -> Result<()> {
    ensure!(out.len() < 256, "SVG path exceeds 256 lowered commands");
    out.push(p);
    Ok(())
}

pub(super) fn parse(source: &str) -> Result<Vec<P>> {
    ensure!(source.len() <= 16 * 1024, "SVG path exceeds 16KiB");
    let mut out = Vec::new();
    let mut current = [0., 0.];
    let mut start = current;
    let mut cubic = None;
    let mut quadratic = None;
    let mut closed = false;
    for (index, segment) in svgtypes::PathParser::from(source).enumerate() {
        ensure!(index < 256, "SVG path exceeds 256 input commands");
        let segment = segment?;
        ensure!(
            index != 0 || matches!(segment, S::MoveTo { .. }),
            "SVG path must start with M/m"
        );
        let mut next_cubic = None;
        let mut next_quadratic = None;
        let pos = |abs: bool, x: f64, y: f64| -> Result<[f64; 2]> {
            number(x)?;
            number(y)?;
            let p = if abs {
                [x, y]
            } else {
                [current[0] + x, current[1] + y]
            };
            point(p)?;
            Ok(p)
        };
        if closed && !matches!(segment, S::MoveTo { .. } | S::ClosePath { .. }) {
            push(
                &mut out,
                P::Move {
                    to: point(current)?,
                },
            )?;
            closed = false;
        }
        match segment {
            S::MoveTo { abs, x, y } => {
                current = pos(abs, x, y)?;
                start = current;
                closed = false;
                push(
                    &mut out,
                    P::Move {
                        to: point(current)?,
                    },
                )?;
            }
            S::LineTo { abs, x, y } => {
                current = pos(abs, x, y)?;
                push(
                    &mut out,
                    P::Line {
                        to: point(current)?,
                    },
                )?;
            }
            S::HorizontalLineTo { abs, x } => {
                number(x)?;
                current[0] = if abs { x } else { current[0] + x };
                push(
                    &mut out,
                    P::Line {
                        to: point(current)?,
                    },
                )?;
            }
            S::VerticalLineTo { abs, y } => {
                number(y)?;
                current[1] = if abs { y } else { current[1] + y };
                push(
                    &mut out,
                    P::Line {
                        to: point(current)?,
                    },
                )?;
            }
            S::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let a = pos(abs, x1, y1)?;
                let b = pos(abs, x2, y2)?;
                let end = pos(abs, x, y)?;
                push(
                    &mut out,
                    P::Cubic {
                        control1: point(a)?,
                        control2: point(b)?,
                        to: point(end)?,
                    },
                )?;
                current = end;
                next_cubic = Some(b);
            }
            S::SmoothCurveTo { abs, x2, y2, x, y } => {
                let a = cubic.map_or(current, |p: [f64; 2]| {
                    [2. * current[0] - p[0], 2. * current[1] - p[1]]
                });
                let b = pos(abs, x2, y2)?;
                let end = pos(abs, x, y)?;
                push(
                    &mut out,
                    P::Cubic {
                        control1: point(a)?,
                        control2: point(b)?,
                        to: point(end)?,
                    },
                )?;
                current = end;
                next_cubic = Some(b);
            }
            S::Quadratic { abs, x1, y1, x, y } => {
                let a = pos(abs, x1, y1)?;
                let end = pos(abs, x, y)?;
                push(
                    &mut out,
                    P::Quad {
                        control: point(a)?,
                        to: point(end)?,
                    },
                )?;
                current = end;
                next_quadratic = Some(a);
            }
            S::SmoothQuadratic { abs, x, y } => {
                let a = quadratic.map_or(current, |p: [f64; 2]| {
                    [2. * current[0] - p[0], 2. * current[1] - p[1]]
                });
                let end = pos(abs, x, y)?;
                push(
                    &mut out,
                    P::Quad {
                        control: point(a)?,
                        to: point(end)?,
                    },
                )?;
                current = end;
                next_quadratic = Some(a);
            }
            S::EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                number(rx)?;
                number(ry)?;
                number(x_axis_rotation)?;
                let end = pos(abs, x, y)?;
                arc(
                    &mut out,
                    current,
                    end,
                    [rx.abs(), ry.abs()],
                    x_axis_rotation,
                    large_arc,
                    sweep,
                )?;
                current = end;
            }
            S::ClosePath { .. } => {
                if !closed {
                    push(&mut out, P::Close)?;
                }
                current = start;
                closed = true;
            }
        }
        cubic = next_cubic;
        quadratic = next_quadratic;
    }
    Ok(out)
}

fn arc(
    out: &mut Vec<P>,
    from: [f64; 2],
    to: [f64; 2],
    radii: [f64; 2],
    rotation: f64,
    large: bool,
    sweep: bool,
) -> Result<()> {
    if from == to {
        return Ok(());
    }
    if radii[0] == 0. || radii[1] == 0. {
        return push(out, P::Line { to: point(to)? });
    }
    // SVG endpoint-to-center conversion, normalized before squaring. This
    // avoids underflow assertions and absolute-small-radius shortcuts in
    // general-purpose arc helpers. Nonzero tiny radii still get SVG correction.
    let rotation = rotation.to_radians();
    let (sin, cos) = rotation.sin_cos();
    let dx = (from[0] - to[0]) / 2.;
    let dy = (from[1] - to[1]) / 2.;
    let x = cos * dx + sin * dy;
    let y = -sin * dx + cos * dy;
    let mut rx = radii[0];
    let mut ry = radii[1];
    let correction = (x / rx).hypot(y / ry);
    ensure!(
        correction.is_finite(),
        "SVG arc radii are numerically ill-conditioned"
    );
    if correction > 1. {
        rx *= correction;
        ry *= correction;
    }
    number(rx)?;
    number(ry)?;
    let u = x / rx;
    let v = y / ry;
    let norm = u.hypot(v);
    if norm == 0. {
        return push(out, P::Line { to: point(to)? });
    }
    let sign = if large == sweep { -1. } else { 1. };
    let height = (1. - norm.min(1.).powi(2)).max(0.).sqrt() * sign;
    let cx = rx * height * (v / norm);
    let cy = -ry * height * (u / norm);
    let start_angle = ((y - cy) / ry).atan2((x - cx) / rx);
    let end_angle = ((-y - cy) / ry).atan2((-x - cx) / rx);
    let mut sweep_angle = (end_angle - start_angle).rem_euclid(std::f64::consts::TAU);
    if sweep_angle == 0. && large {
        sweep_angle = std::f64::consts::TAU;
    }
    if !sweep && sweep_angle > 0. {
        sweep_angle -= std::f64::consts::TAU;
        if sweep_angle == 0. && large {
            sweep_angle = -std::f64::consts::TAU;
        }
    }
    if sweep_angle == 0. {
        return push(out, P::Line { to: point(to)? });
    }
    let arc = kurbo::Arc {
        center: kurbo::Point::new(
            cos * cx - sin * cy + (from[0] + to[0]) / 2.,
            sin * cx + cos * cy + (from[1] + to[1]) / 2.,
        ),
        radii: kurbo::Vec2::new(rx, ry),
        start_angle,
        sweep_angle,
        x_rotation: rotation,
    };
    ensure!(
        arc.start_angle.is_finite()
            && arc.sweep_angle.is_finite()
            && arc.sweep_angle.abs() <= std::f64::consts::TAU + 1e-9,
        "Invalid SVG arc"
    );
    number(arc.radii.x)?;
    number(arc.radii.y)?;
    number(arc.center.x)?;
    number(arc.center.y)?;
    // At most eight cubics per arc, each spanning <=45 degrees. Never call
    // a tolerance-driven arc expander on untrusted radii/eccentricities.
    let n = (arc.sweep_angle.abs() / std::f64::consts::FRAC_PI_4)
        .ceil()
        .clamp(1., 8.) as usize;
    let step = arc.sweep_angle / n as f64;
    let (sin, cos) = arc.x_rotation.sin_cos();
    let sample = |t: f64| {
        let (s, c) = t.sin_cos();
        (
            [
                arc.center.x + cos * arc.radii.x * c - sin * arc.radii.y * s,
                arc.center.y + sin * arc.radii.x * c + cos * arc.radii.y * s,
            ],
            [
                -cos * arc.radii.x * s - sin * arc.radii.y * c,
                -sin * arc.radii.x * s + cos * arc.radii.y * c,
            ],
        )
    };
    for i in 0..n {
        let (p, d) = sample(arc.start_angle + i as f64 * step);
        let (q, e) = sample(arc.start_angle + (i + 1) as f64 * step);
        let alpha = 4. / 3. * (step / 4.).tan();
        push(
            out,
            P::Cubic {
                control1: point([p[0] + alpha * d[0], p[1] + alpha * d[1]])?,
                control2: point([q[0] - alpha * e[0], q[1] - alpha * e[1]])?,
                to: point(if i + 1 == n { to } else { q })?,
            },
        )?;
    }
    Ok(())
}

pub(super) fn bounds(commands: &[P]) -> kurbo::Rect {
    use kurbo::Shape;
    let mut path = kurbo::BezPath::new();
    let p = |a: [f32; 2]| kurbo::Point::new(f64::from(a[0]), f64::from(a[1]));
    for c in commands {
        match c {
            P::Move { to } => path.move_to(p(*to)),
            P::Line { to } => path.line_to(p(*to)),
            P::Quad { control, to } => path.quad_to(p(*control), p(*to)),
            P::Cubic {
                control1,
                control2,
                to,
            } => path.curve_to(p(*control1), p(*control2), p(*to)),
            P::Close => path.close_path(),
        }
    }
    path.bounding_box()
}
