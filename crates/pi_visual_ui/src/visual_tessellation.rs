//! Bounded physical-pixel quality for native vector contours.
//!
//! Source geometry and script budgets remain density independent. All subdivision
//! choices are made before entering the native tessellator; there is no unbounded
//! recursion or tolerance-driven native curve expansion.
use pi_visual::graphics::PathCommand;

pub(super) const PIXEL_ERROR: f64 = 0.25;
pub(super) const MAX_CURVE_SEGMENTS: usize = 128;
pub(super) const MAX_CIRCLE_SEGMENTS: usize = 256;
// At most 4x the existing 2048 source-vertex budget (Q/C cost32, circles64).
pub(super) const MAX_POINTS: usize = 8192;
pub(super) const MAX_MESH_VERTICES: usize = 131_072;

/// Maximum stretching of an arbitrary composed affine transform, including
/// nonuniform scale, shear and reflection. Translation never changes quality.
pub(super) fn physical_scale(view_scale: f32, density: f32, matrix: [f32; 6]) -> f64 {
    let [a, b, c, d, _, _] = matrix.map(f64::from);
    let u = a * a + b * b;
    let v = c * c + d * d;
    let cross = a * c + b * d;
    let stretch = ((u + v + (u - v).hypot(2. * cross)) / 2.).sqrt();
    f64::from(view_scale).abs() * f64::from(density) * stretch
}

fn level(requested: f64, minimum: usize, maximum: usize) -> u16 {
    // NaN/infinite quality requests must not turn into unbounded allocations.
    if !requested.is_finite() {
        return maximum as u16;
    }
    // A tiny conservative margin avoids changes caused by f32 rotation roundoff
    // at an exact power-of-two boundary. Translation and paint are not key inputs.
    let steps = (requested * 1.000_001)
        .ceil()
        .clamp(minimum as f64, maximum as f64) as usize;
    steps.next_power_of_two().min(maximum) as u16
}
fn second_difference(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f64 {
    (f64::from(a[0]) - 2. * f64::from(b[0]) + f64::from(c[0]))
        .hypot(f64::from(a[1]) - 2. * f64::from(b[1]) + f64::from(c[1]))
}

/// Per-command segment counts are the cache's quality key: finer screens or
/// magnification rebuild only when the required level changes. A loop or cusp
/// cannot be mistaken for a straight line just because its endpoints coincide.
pub(super) fn curve_steps(commands: &[PathCommand], physical_scale: f64) -> Vec<u16> {
    let mut from = [0., 0.];
    commands
        .iter()
        .map(|command| {
            let (end, coefficient) = match command {
                PathCommand::Move { to } | PathCommand::Line { to } => {
                    from = *to;
                    return 1;
                }
                PathCommand::Close => return 0,
                PathCommand::Quad { control, to } => {
                    (*to, second_difference(from, *control, *to) / 4.)
                }
                PathCommand::Cubic {
                    control1,
                    control2,
                    to,
                } => (
                    *to,
                    0.75 * second_difference(from, *control1, *control2)
                        .max(second_difference(*control1, *control2, *to)),
                ),
            };
            from = end;
            // Linear interpolation error <= max |B''(t)| / (8 n²). Bounds use
            // double precision before scaling; source coordinates may be large.
            level(
                (coefficient * physical_scale / PIXEL_ERROR).sqrt(),
                1,
                MAX_CURVE_SEGMENTS,
            )
        })
        .collect()
}
pub(super) fn circle_steps(radius: f32, physical_scale: f64, half_stroke_pixels: f64) -> u16 {
    // The polygonal centerline has sagitta R(1-cos(pi/n)). Mitered outer
    // corners overshoot by half_width * (sec(pi/n)-1). Bound BOTH, including
    // a thick stroke on a tiny circle; width is dp, not viewBox units.
    // n>=8 ensures cos(pi/n)>=cos(pi/8). Also 1-cos(x)<=x²/2.
    let envelope =
        f64::from(radius) * physical_scale + half_stroke_pixels / (std::f64::consts::PI / 8.).cos();
    level(
        std::f64::consts::PI * (envelope / (2. * PIXEL_ERROR)).sqrt(),
        8,
        MAX_CIRCLE_SEGMENTS,
    )
}

#[derive(Default)]
pub(super) struct Budget {
    points: usize,
    mesh_vertices: usize,
}
impl Budget {
    pub(super) fn remaining_mesh(&self) -> usize {
        MAX_MESH_VERTICES.saturating_sub(self.mesh_vertices)
    }
    pub(super) fn points(&mut self, count: usize) -> anyhow::Result<()> {
        self.points = self
            .points
            .checked_add(count)
            .ok_or_else(|| anyhow::anyhow!("Native point budget overflow"))?;
        anyhow::ensure!(
            self.points <= MAX_POINTS,
            "Native drawing exceeds {MAX_POINTS} tessellation points"
        );
        Ok(())
    }
    pub(super) fn mesh(&mut self, count: usize) -> anyhow::Result<()> {
        self.mesh_vertices = self
            .mesh_vertices
            .checked_add(count)
            .ok_or_else(|| anyhow::anyhow!("Native mesh budget overflow"))?;
        anyhow::ensure!(
            self.mesh_vertices <= MAX_MESH_VERTICES,
            "Native drawing exceeds {MAX_MESH_VERTICES} mesh vertices"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn quad(a: [f64; 2], b: [f64; 2], c: [f64; 2], t: f64) -> [f64; 2] {
        let u = 1. - t;
        [
            u * u * a[0] + 2. * u * t * b[0] + t * t * c[0],
            u * u * a[1] + 2. * u * t * b[1] + t * t * c[1],
        ]
    }
    fn cubic(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], t: f64) -> [f64; 2] {
        let u = 1. - t;
        std::array::from_fn(|i| {
            u * u * u * a[i] + 3. * u * u * t * b[i] + 3. * u * t * t * c[i] + t * t * t * d[i]
        })
    }
    fn interpolation_error(steps: u16, scale: f64, at: impl Fn(f64) -> [f64; 2]) {
        for i in 0..steps {
            let a = at(f64::from(i) / f64::from(steps));
            let b = at(f64::from(i + 1) / f64::from(steps));
            for j in 1..8 {
                let t = f64::from(j) / 8.;
                let p = at((f64::from(i) + t) / f64::from(steps));
                let q = [a[0] * (1. - t) + b[0] * t, a[1] * (1. - t) + b[1] * t];
                assert!((p[0] - q[0]).hypot(p[1] - q[1]) * scale <= PIXEL_ERROR + 1e-9);
            }
        }
    }
    #[test]
    fn quadratic_and_cubic_error_tracks_physical_density_and_zoom() {
        let a = [0., 0.];
        let b = [120., 70.];
        let c = [-90., 60.];
        let d = [10., 20.];
        for density in [1., 1.25, 1.5, 2., 3., 4.] {
            for zoom in [0.25, 1., 2., 4.] {
                let scale = density * zoom;
                let q = curve_steps(
                    &[
                        PathCommand::Move { to: a },
                        PathCommand::Quad { control: b, to: d },
                    ],
                    scale,
                );
                let curve = curve_steps(
                    &[
                        PathCommand::Move { to: a },
                        PathCommand::Cubic {
                            control1: b,
                            control2: c,
                            to: d,
                        },
                    ],
                    scale,
                );
                // These examples meet the target within the cap (the finest one
                // uses exactly the highest permitted level).
                interpolation_error(q[1], scale, |t| {
                    quad(a.map(f64::from), b.map(f64::from), d.map(f64::from), t)
                });
                interpolation_error(curve[1], scale, |t| {
                    cubic(
                        a.map(f64::from),
                        b.map(f64::from),
                        c.map(f64::from),
                        d.map(f64::from),
                        t,
                    )
                });
            }
        }
    }
    #[test]
    fn inflections_closed_loops_and_collinear_backtracking_are_not_flattened_away() {
        for (a, b, end) in [
            ([100., 100.], [-100., -100.], [0., 0.]),
            ([200., 0.], [-100., 0.], [10., 0.]),
            ([100., 120.], [-100., 120.], [0., 0.]),
        ] {
            let steps = curve_steps(
                &[
                    PathCommand::Move { to: [0., 0.] },
                    PathCommand::Cubic {
                        control1: a,
                        control2: b,
                        to: end,
                    },
                ],
                3.,
            )[1];
            assert!(steps > 1);
            interpolation_error(steps, 3., |t| {
                cubic(
                    [0., 0.],
                    a.map(f64::from),
                    b.map(f64::from),
                    end.map(f64::from),
                    t,
                )
            });
        }
        assert_eq!(
            curve_steps(
                &[
                    PathCommand::Move { to: [0., 0.] },
                    PathCommand::Quad {
                        control: [10., 0.],
                        to: [20., 0.]
                    }
                ],
                4.
            ),
            vec![1, 1]
        );
    }
    #[test]
    fn circles_use_a_pixel_error_bound_and_saturate_safely() {
        for radius in [0., 0.1, 1., 20., 100.] {
            for scale in [0., 1., 1.5, 2., 3., 4., 8.] {
                let n = circle_steps(radius, scale, 0.);
                let error = 2.
                    * f64::from(radius)
                    * scale
                    * (std::f64::consts::PI / (2. * f64::from(n))).sin().powi(2);
                assert!(
                    error <= PIXEL_ERROR,
                    "r={radius}, scale={scale}, n={n}, error={error}"
                );
                assert!((8..=MAX_CIRCLE_SEGMENTS as u16).contains(&n) && n.is_power_of_two());
            }
        }
        assert_eq!(circle_steps(1e6, 1e12, 0.), MAX_CIRCLE_SEGMENTS as u16);
        assert_eq!(
            circle_steps(1., f64::INFINITY, 0.),
            MAX_CIRCLE_SEGMENTS as u16
        );
        assert_eq!(
            curve_steps(
                &[
                    PathCommand::Move { to: [0., 0.] },
                    PathCommand::Quad {
                        control: [1e6, 1e6],
                        to: [0., 0.]
                    }
                ],
                1e12
            )[1],
            MAX_CURVE_SEGMENTS as u16
        );
    }
    #[test]
    fn thick_circle_strokes_do_not_regress_into_octagons() {
        for fit in [0.1, 1., 4.] {
            for density in [1., 2., 4.] {
                for zoom in [1., 2., 4.] {
                    let half_width = 4. * density * zoom;
                    let radius = 0.1 * fit * density * zoom;
                    let n = circle_steps(0.1, fit * density * zoom, half_width);
                    let cosine = (std::f64::consts::PI / f64::from(n)).cos();
                    assert!(radius * (1. - cosine) <= PIXEL_ERROR);
                    assert!(half_width * (1. / cosine - 1.) <= PIXEL_ERROR);
                }
            }
        }
        assert!(circle_steps(0.1, 4., 16.) > 8);
    }
    #[test]
    fn composed_transforms_include_shear_reflection_and_only_one_view_scale() {
        use pi_visual::graphics::multiply;
        let turn = [0., 1., -1., 0., 50., 60.];
        let reflected = [-3., 0., 0., 2., 0., 0.];
        let m = multiply(turn, reflected);
        assert_eq!(physical_scale(2., 3., m), 18.);
        let cancelled = multiply([4., 0., 0., 0.25, 0., 0.], [0.25, 0., 0., 4., 0., 0.]);
        assert_eq!(physical_scale(2., 3., cancelled), 6.);
        let shear = [1., 0., 3., 1., 100., -100.];
        let maximum = physical_scale(1., 1., shear);
        for degrees in 0..360 {
            let t = f64::from(degrees).to_radians();
            let x = t.cos();
            let y = t.sin();
            assert!((x + 3. * y).hypot(y) <= maximum + 1e-12);
        }
        assert_eq!(physical_scale(1., 4., [0.; 6]), 0.);
    }
    #[test]
    fn native_work_budgets_cover_instances_and_fail_without_overflow() {
        let mut b = Budget::default();
        for _ in 0..32 {
            b.points(MAX_CIRCLE_SEGMENTS).unwrap();
        }
        assert_eq!(b.points, MAX_POINTS);
        assert!(b.points(1).is_err());
        let mut b = Budget::default();
        b.mesh(MAX_MESH_VERTICES).unwrap();
        assert!(b.mesh(1).is_err());
        let mut b = Budget::default();
        assert!(b.points(usize::MAX).is_err());
        assert!(b.points(1).is_err());
        let mut b = Budget::default();
        assert!(b.mesh(usize::MAX).is_err());
        assert!(b.mesh(1).is_err());
    }
}
