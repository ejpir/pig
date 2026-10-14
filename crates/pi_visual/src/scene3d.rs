//! Bounded 3D scenes: meshes and lines a host projects natively. Guests send
//! small descriptors and transforms; primitive geometry is generated here, so a
//! spinning sphere costs a few numbers per frame, not its triangles.
//!
//! Projection depth-sorts triangles back to front (a painter's algorithm, no
//! depth buffer): closed primitives look right, intersecting faces may not.
use crate::graphics::valid_color;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

/// Triangles one canvas scene may project, primitives and custom meshes alike.
pub const MAX_TRIANGLES: usize = 8192;
pub const MAX_OBJECTS: usize = 64;
/// Custom mesh data travels from the script every frame, so it stays small.
pub const MAX_CUSTOM_VERTICES: usize = 2048;
pub const MAX_CUSTOM_FACES: usize = 4096;
pub const MAX_LINE_POINTS: usize = 2048;
const MAX_COORDINATE: f32 = 10_000.;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    /// Vertical field of view, degrees.
    pub fov: f32,
    /// From the target, in scene units.
    pub distance: f32,
    /// Degrees around the vertical axis; 0 looks from +z toward the target.
    pub yaw: f32,
    /// Degrees above the horizon.
    pub pitch: f32,
    pub target: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Object {
    Mesh {
        mesh: Mesh,
        position: [f32; 3],
        /// Degrees about x, then y, then z.
        rotate: [f32; 3],
        scale: [f32; 3],
        color: String,
    },
    Line {
        points: Vec<[f32; 3]>,
        color: String,
        width: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mesh {
    Box {
        size: [f32; 3],
    },
    Sphere {
        radius: f32,
        segments: u32,
    },
    Cylinder {
        radius: f32,
        height: f32,
        segments: u32,
    },
    Cone {
        radius: f32,
        height: f32,
        segments: u32,
    },
    Torus {
        radius: f32,
        tube: f32,
        segments: u32,
    },
    /// Drawn from both sides: a script's winding cannot be trusted.
    Custom {
        vertices: Vec<[f32; 3]>,
        faces: Vec<[u32; 3]>,
    },
}

/// What one canvas scene has used of the 3D limits.
#[derive(Default)]
pub(crate) struct Budget {
    triangles: usize,
    objects: usize,
    custom_vertices: usize,
    custom_faces: usize,
    line_points: usize,
}

fn coordinate(v: f32) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}
fn vector(v: &[f32; 3]) -> bool {
    v.iter().copied().all(coordinate)
}
fn length(v: f32) -> bool {
    coordinate(v) && v > 0.
}
fn segments(n: u32) -> bool {
    (3..=64).contains(&n)
}
fn rings(segments: u32) -> u32 {
    (segments / 2).max(2)
}

impl Mesh {
    pub fn triangles(&self) -> usize {
        match self {
            Self::Box { .. } => 12,
            Self::Sphere { segments, .. } => (2 * segments * (rings(*segments) - 1)) as usize,
            Self::Cylinder { segments, .. } => (4 * segments) as usize,
            Self::Cone { segments, .. } => (2 * segments) as usize,
            Self::Torus { segments, .. } => (2 * segments * rings(*segments).max(3)) as usize,
            Self::Custom { faces, .. } => faces.len(),
        }
    }

    fn validate(&self, budget: &mut Budget) -> Result<()> {
        let valid = match self {
            Self::Box { size } => size.iter().copied().all(length),
            Self::Sphere {
                radius,
                segments: n,
            } => length(*radius) && segments(*n),
            Self::Cylinder {
                radius,
                height,
                segments: n,
            }
            | Self::Cone {
                radius,
                height,
                segments: n,
            } => length(*radius) && length(*height) && segments(*n),
            Self::Torus {
                radius,
                tube,
                segments: n,
            } => length(*radius) && length(*tube) && segments(*n),
            Self::Custom { vertices, faces } => {
                budget.custom_vertices += vertices.len();
                budget.custom_faces += faces.len();
                ensure!(
                    budget.custom_vertices <= MAX_CUSTOM_VERTICES
                        && budget.custom_faces <= MAX_CUSTOM_FACES,
                    "Custom meshes exceed {MAX_CUSTOM_VERTICES} vertices or {MAX_CUSTOM_FACES} faces per scene"
                );
                !vertices.is_empty()
                    && !faces.is_empty()
                    && vertices.iter().all(vector)
                    && faces
                        .iter()
                        .all(|f| f.iter().all(|i| (*i as usize) < vertices.len()))
            }
        };
        ensure!(valid, "Invalid 3D mesh");
        budget.triangles += self.triangles();
        ensure!(
            budget.triangles <= MAX_TRIANGLES,
            "3D scene exceeds {MAX_TRIANGLES} triangles"
        );
        Ok(())
    }

    /// Vertices and triangles, wound counter-clockwise seen from outside.
    pub fn build(&self) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        match self {
            Self::Box { size } => {
                let [x, y, z] = size.map(|s| s / 2.);
                let v = (0..8)
                    .map(|i| {
                        [
                            if i & 1 == 0 { -x } else { x },
                            if i & 2 == 0 { -y } else { y },
                            if i & 4 == 0 { -z } else { z },
                        ]
                    })
                    .collect();
                let quads = [
                    [1, 3, 7, 5],
                    [0, 4, 6, 2],
                    [2, 6, 7, 3],
                    [0, 1, 5, 4],
                    [4, 5, 7, 6],
                    [0, 2, 3, 1],
                ];
                let faces = quads
                    .iter()
                    .flat_map(|[a, b, c, d]| [[*a, *b, *c], [*a, *c, *d]])
                    .collect();
                orient(v, faces, |_| [0.; 3])
            }
            Self::Sphere { radius, segments } => {
                let (lon, lat) = (*segments, rings(*segments));
                let mut v = vec![[0., *radius, 0.]];
                for ring in 1..lat {
                    let theta = PI * ring as f32 / lat as f32;
                    for i in 0..lon {
                        let phi = TAU * i as f32 / lon as f32;
                        v.push([
                            radius * theta.sin() * phi.cos(),
                            radius * theta.cos(),
                            radius * theta.sin() * phi.sin(),
                        ]);
                    }
                }
                v.push([0., -radius, 0.]);
                let bottom = v.len() as u32 - 1;
                let at = |ring: u32, i: u32| 1 + (ring - 1) * lon + i % lon;
                let mut faces = Vec::new();
                for i in 0..lon {
                    faces.push([0, at(1, i), at(1, i + 1)]);
                    faces.push([bottom, at(lat - 1, i), at(lat - 1, i + 1)]);
                    for ring in 1..lat - 1 {
                        let (a, b) = (at(ring, i), at(ring, i + 1));
                        let (c, d) = (at(ring + 1, i), at(ring + 1, i + 1));
                        faces.push([a, c, d]);
                        faces.push([a, d, b]);
                    }
                }
                orient(v, faces, |_| [0.; 3])
            }
            Self::Cylinder {
                radius,
                height,
                segments,
            } => {
                let n = *segments;
                let h = height / 2.;
                let mut v = vec![[0., h, 0.], [0., -h, 0.]];
                for i in 0..n {
                    let a = TAU * i as f32 / n as f32;
                    v.push([radius * a.cos(), h, radius * a.sin()]);
                    v.push([radius * a.cos(), -h, radius * a.sin()]);
                }
                let top = |i: u32| 2 + 2 * (i % n);
                let low = |i: u32| 3 + 2 * (i % n);
                let mut faces = Vec::new();
                for i in 0..n {
                    faces.push([0, top(i), top(i + 1)]);
                    faces.push([1, low(i), low(i + 1)]);
                    faces.push([top(i), low(i), low(i + 1)]);
                    faces.push([top(i), low(i + 1), top(i + 1)]);
                }
                orient(v, faces, |_| [0.; 3])
            }
            Self::Cone {
                radius,
                height,
                segments,
            } => {
                let n = *segments;
                let h = height / 2.;
                let mut v = vec![[0., h, 0.], [0., -h, 0.]];
                for i in 0..n {
                    let a = TAU * i as f32 / n as f32;
                    v.push([radius * a.cos(), -h, radius * a.sin()]);
                }
                let rim = |i: u32| 2 + i % n;
                let mut faces = Vec::new();
                for i in 0..n {
                    faces.push([0, rim(i), rim(i + 1)]);
                    faces.push([1, rim(i), rim(i + 1)]);
                }
                orient(v, faces, |_| [0.; 3])
            }
            Self::Torus {
                radius,
                tube,
                segments,
            } => {
                let (n, m) = (*segments, rings(*segments).max(3));
                let mut v = Vec::new();
                for i in 0..n {
                    let a = TAU * i as f32 / n as f32;
                    for j in 0..m {
                        let b = TAU * j as f32 / m as f32;
                        let r = radius + tube * b.cos();
                        v.push([r * a.cos(), tube * b.sin(), r * a.sin()]);
                    }
                }
                let at = |i: u32, j: u32| (i % n) * m + j % m;
                let mut faces = Vec::new();
                for i in 0..n {
                    for j in 0..m {
                        faces.push([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                        faces.push([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
                    }
                }
                let radius = *radius;
                // Outward from the tube's centre line, not the torus centre.
                orient(v, faces, move |c| {
                    let r = c[0].hypot(c[2]).max(f32::EPSILON);
                    [c[0] / r * radius, 0., c[2] / r * radius]
                })
            }
            Self::Custom { vertices, faces } => (vertices.clone(), faces.clone()),
        }
    }
}

/// Winds each triangle so its normal points away from `inside(centroid)`.
fn orient(
    vertices: Vec<[f32; 3]>,
    mut faces: Vec<[u32; 3]>,
    inside: impl Fn([f32; 3]) -> [f32; 3],
) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    for face in &mut faces {
        let [a, b, c] = face.map(|i| vertices[i as usize]);
        let centroid = scale(add(add(a, b), c), 1. / 3.);
        if dot(normal(a, b, c), sub(centroid, inside(centroid))) < 0. {
            face.swap(1, 2);
        }
    }
    (vertices, faces)
}

pub(crate) fn validate(
    viewport: &Option<[f32; 4]>,
    camera: &Camera,
    light: &Option<[f32; 3]>,
    objects: &[Object],
    budget: &mut Budget,
) -> Result<()> {
    ensure!(
        viewport.is_none_or(|v| v.iter().all(|n| n.is_finite() && n.abs() <= 1e6)
            && v[2] > 0.
            && v[3] > 0.),
        "Invalid 3D viewport"
    );
    ensure!(
        (10.0..=120.0).contains(&camera.fov)
            && length(camera.distance)
            && camera.yaw.is_finite()
            && camera.yaw.abs() <= 1e6
            && (-89.9..=89.9).contains(&camera.pitch)
            && vector(&camera.target),
        "Invalid 3D camera: fov 10..120, distance > 0, pitch -89.9..89.9"
    );
    ensure!(
        light.is_none_or(|l| vector(&l) && l.iter().any(|n| *n != 0.)),
        "Invalid 3D light direction"
    );
    budget.objects += objects.len();
    ensure!(
        budget.objects <= MAX_OBJECTS,
        "3D scene exceeds {MAX_OBJECTS} objects"
    );
    for object in objects {
        match object {
            Object::Mesh {
                mesh,
                position,
                rotate,
                scale,
                color,
            } => {
                ensure!(
                    vector(position)
                        && rotate.iter().all(|r| r.is_finite() && r.abs() <= 1e6)
                        && scale.iter().copied().all(length)
                        && valid_color(color),
                    "Invalid 3D mesh placement or color"
                );
                mesh.validate(budget)?;
            }
            Object::Line {
                points,
                color,
                width,
            } => {
                budget.line_points += points.len();
                ensure!(
                    points.len() >= 2
                        && budget.line_points <= MAX_LINE_POINTS
                        && points.iter().all(vector)
                        && valid_color(color)
                        && (0.5..=8.0).contains(width),
                    "Invalid 3D line: 2+ points, <= {MAX_LINE_POINTS} per scene, width 0.5..8"
                );
                // Each segment is drawn as two triangles.
                budget.triangles += 2 * (points.len() - 1);
                ensure!(
                    budget.triangles <= MAX_TRIANGLES,
                    "3D scene exceeds {MAX_TRIANGLES} triangles"
                );
            }
        }
    }
    Ok(())
}

/// A projected, shaded piece of the scene, in viewBox units.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Triangle {
        points: [[f32; 2]; 3],
        rgba: [f32; 4],
    },
    /// A line segment the host strokes `width` dp wide.
    Segment {
        from: [f32; 2],
        to: [f32; 2],
        rgba: [f32; 4],
        width: f32,
    },
}

const AMBIENT: f32 = 0.38;

/// Projects a validated scene into `viewport`, back to front.
pub fn project(
    viewport: [f32; 4],
    camera: &Camera,
    light: Option<[f32; 3]>,
    objects: &[Object],
) -> Vec<Item> {
    let view = View::new(camera, viewport);
    // Toward the light: the camera's upper left unless the scene says otherwise.
    let light = normalize(match light {
        Some(l) => view.rotate(l),
        None => [-0.4, 0.55, -0.75],
    });
    let mut items: Vec<(f32, Item)> = Vec::new();
    for object in objects {
        match object {
            Object::Mesh {
                mesh,
                position,
                rotate,
                scale: size,
                color,
            } => {
                let base = rgba(color);
                let both_sides = matches!(mesh, Mesh::Custom { .. });
                let model = Model::new(*position, *rotate, *size);
                let (vertices, faces) = mesh.build();
                let eye: Vec<[f32; 3]> = vertices
                    .iter()
                    .map(|v| view.to_eye(model.apply(*v)))
                    .collect();
                for face in faces {
                    let [a, b, c] = face.map(|i| eye[i as usize]);
                    if [a, b, c].iter().any(|p| p[2] < view.near) {
                        continue;
                    }
                    // Eye space (right, up, away from the camera) has the
                    // opposite handedness to the scene, so cross products
                    // flip: negate to keep normals pointing outward.
                    let mut n = scale(normal(a, b, c), -1.);
                    // The camera looks down +z in eye space.
                    let facing = dot(n, a) < 0.;
                    if !facing {
                        if !both_sides {
                            continue;
                        }
                        n = scale(n, -1.);
                    }
                    let n = normalize(n);
                    let shade = AMBIENT + (1. - AMBIENT) * dot(n, light).max(0.);
                    let depth = (a[2] + b[2] + c[2]) / 3.;
                    items.push((
                        depth,
                        Item::Triangle {
                            points: [a, b, c].map(|p| view.screen(p)),
                            rgba: [base[0] * shade, base[1] * shade, base[2] * shade, base[3]],
                        },
                    ));
                }
            }
            Object::Line {
                points,
                color,
                width,
            } => {
                let base = rgba(color);
                for pair in points.windows(2) {
                    let [a, b] = [pair[0], pair[1]].map(|p| view.to_eye(p));
                    if a[2] < view.near || b[2] < view.near {
                        continue;
                    }
                    items.push((
                        (a[2] + b[2]) / 2.,
                        Item::Segment {
                            from: view.screen(a),
                            to: view.screen(b),
                            rgba: base,
                            width: *width,
                        },
                    ));
                }
            }
        }
    }
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    items.into_iter().map(|(_, item)| item).collect()
}

fn rgba(hex: &str) -> [f32; 4] {
    let n = u32::from_str_radix(&hex[1..], 16).unwrap_or(0);
    let n = if hex.len() == 7 { (n << 8) | 255 } else { n };
    [24, 16, 8, 0].map(|shift| ((n >> shift) & 255) as f32 / 255.)
}

struct Model {
    position: [f32; 3],
    rotation: [[f32; 3]; 3],
    scale: [f32; 3],
}
impl Model {
    fn new(position: [f32; 3], rotate: [f32; 3], scale: [f32; 3]) -> Self {
        let [x, y, z] = rotate.map(f32::to_radians);
        let rx = [
            [1., 0., 0.],
            [0., x.cos(), -x.sin()],
            [0., x.sin(), x.cos()],
        ];
        let ry = [
            [y.cos(), 0., y.sin()],
            [0., 1., 0.],
            [-y.sin(), 0., y.cos()],
        ];
        let rz = [
            [z.cos(), -z.sin(), 0.],
            [z.sin(), z.cos(), 0.],
            [0., 0., 1.],
        ];
        Self {
            position,
            rotation: matmul(rz, matmul(ry, rx)),
            scale,
        }
    }
    fn apply(&self, v: [f32; 3]) -> [f32; 3] {
        let s = [
            v[0] * self.scale[0],
            v[1] * self.scale[1],
            v[2] * self.scale[2],
        ];
        add(mat_vec(self.rotation, s), self.position)
    }
}

struct View {
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    near: f32,
    focal: f32,
    center: [f32; 2],
}
impl View {
    fn new(camera: &Camera, viewport: [f32; 4]) -> Self {
        let (yaw, pitch) = (camera.yaw.to_radians(), camera.pitch.to_radians());
        let offset = [
            pitch.cos() * yaw.sin(),
            pitch.sin(),
            pitch.cos() * yaw.cos(),
        ];
        let eye = add(camera.target, scale(offset, camera.distance));
        let forward = scale(offset, -1.);
        let right = normalize(cross(forward, [0., 1., 0.]));
        let up = cross(right, forward);
        Self {
            eye,
            right,
            up,
            forward,
            near: camera.distance * 0.01,
            focal: viewport[3] / 2. / (camera.fov.to_radians() / 2.).tan(),
            center: [
                viewport[0] + viewport[2] / 2.,
                viewport[1] + viewport[3] / 2.,
            ],
        }
    }
    /// Eye space: x right, y up, z away from the camera.
    fn to_eye(&self, p: [f32; 3]) -> [f32; 3] {
        self.rotate(sub(p, self.eye))
    }
    fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
        [dot(v, self.right), dot(v, self.up), dot(v, self.forward)]
    }
    fn screen(&self, p: [f32; 3]) -> [f32; 2] {
        [
            self.center[0] + p[0] / p[2] * self.focal,
            self.center[1] - p[1] / p[2] * self.focal,
        ]
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    a.map(|v| v * s)
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    cross(sub(b, a), sub(c, a))
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 0. { scale(a, 1. / l) } else { a }
}
fn matmul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut m = [[0.; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}
fn mat_vec(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primitives() -> Vec<Mesh> {
        vec![
            Mesh::Box { size: [1., 2., 3.] },
            Mesh::Sphere {
                radius: 1.,
                segments: 24,
            },
            Mesh::Cylinder {
                radius: 1.,
                height: 2.,
                segments: 16,
            },
            Mesh::Cone {
                radius: 1.,
                height: 2.,
                segments: 16,
            },
            Mesh::Torus {
                radius: 2.,
                tube: 0.5,
                segments: 24,
            },
        ]
    }

    #[test]
    fn primitive_counts_match_what_is_built_and_faces_point_outward() {
        for mesh in primitives() {
            let (v, faces) = mesh.build();
            assert_eq!(faces.len(), mesh.triangles(), "{mesh:?}");
            for face in faces {
                let [a, b, c] = face.map(|i| v[i as usize]);
                let centroid = scale(add(add(a, b), c), 1. / 3.);
                let inside = match mesh {
                    Mesh::Torus { radius, .. } => {
                        let r = centroid[0].hypot(centroid[2]);
                        [centroid[0] / r * radius, 0., centroid[2] / r * radius]
                    }
                    _ => [0.; 3],
                };
                assert!(
                    dot(normal(a, b, c), sub(centroid, inside)) > 0.,
                    "{mesh:?} {face:?}"
                );
            }
        }
    }

    fn camera() -> Camera {
        Camera {
            fov: 50.,
            distance: 5.,
            yaw: 30.,
            pitch: 20.,
            target: [0.; 3],
        }
    }
    fn cube(position: [f32; 3], color: &str) -> Object {
        Object::Mesh {
            mesh: Mesh::Box { size: [1.; 3] },
            position,
            rotate: [0.; 3],
            scale: [1.; 3],
            color: color.into(),
        }
    }
    fn triangles(items: &[Item]) -> Vec<([[f32; 2]; 3], [f32; 4])> {
        items
            .iter()
            .filter_map(|i| match i {
                Item::Triangle { points, rgba } => Some((*points, *rgba)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_cube_shows_only_its_front_faces_centred_in_the_viewport() {
        let items = project(
            [0., 0., 100., 100.],
            &camera(),
            None,
            &[cube([0.; 3], "#FF0000")],
        );
        let tris = triangles(&items);
        // From above and to the side, three faces (six triangles) face the camera.
        assert_eq!(tris.len(), 6);
        let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
        for (points, rgba) in &tris {
            assert_eq!(rgba[1], 0.);
            assert!(rgba[0] > 0. && rgba[0] <= 1.);
            for p in points {
                for k in 0..2 {
                    min[k] = min[k].min(p[k]);
                    max[k] = max[k].max(p[k]);
                }
            }
        }
        assert!(
            (((min[0] + max[0]) / 2.) - 50.).abs() < 5.,
            "{min:?} {max:?}"
        );
        assert!(min[0] > 0. && max[0] < 100. && min[1] > 0. && max[1] < 100.);
    }

    #[test]
    fn an_unrotated_cube_seen_head_on_is_a_centred_square() {
        let camera = Camera {
            yaw: 0.,
            pitch: 0.,
            ..camera()
        };
        let items = project(
            [0., 0., 160., 100.],
            &camera,
            None,
            &[cube([0.; 3], "#FFFFFF")],
        );
        let tris = triangles(&items);
        // Only the face toward the camera is visible.
        assert_eq!(tris.len(), 2);
        let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
        for (points, _) in &tris {
            for p in points {
                for k in 0..2 {
                    min[k] = min[k].min(p[k]);
                    max[k] = max[k].max(p[k]);
                }
            }
        }
        let (w, h) = (max[0] - min[0], max[1] - min[1]);
        assert!((w - h).abs() < 1e-3, "{w} x {h}");
        // The default light is toward the camera's upper left: the front face is lit.
        assert!(tris.iter().all(|(_, c)| c[0] > AMBIENT + 0.2), "{tris:?}");
        assert!(((min[0] + max[0]) / 2. - 80.).abs() < 1e-3);
        assert!(((min[1] + max[1]) / 2. - 50.).abs() < 1e-3);
    }

    #[test]
    fn nearer_objects_paint_after_farther_ones() {
        // The camera at yaw 0 looks from +z: the cube at z=1 is nearer.
        let camera = Camera {
            yaw: 0.,
            pitch: 0.,
            ..camera()
        };
        let items = project(
            [0., 0., 100., 100.],
            &camera,
            None,
            &[
                cube([0., 0., 1.], "#00FF00"),
                cube([0., 0., -1.], "#0000FF"),
            ],
        );
        let tris = triangles(&items);
        let last_far = tris.iter().rposition(|(_, c)| c[2] > 0.).unwrap();
        let first_near = tris.iter().position(|(_, c)| c[1] > 0.).unwrap();
        assert!(last_far < first_near);
    }

    #[test]
    fn custom_meshes_draw_from_both_sides_and_behind_the_camera_is_skipped() {
        let quad = Mesh::Custom {
            vertices: vec![[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.]],
            faces: vec![[0, 1, 2]],
        };
        for yaw in [0., 180.] {
            let camera = Camera {
                yaw,
                pitch: 0.,
                ..camera()
            };
            let object = Object::Mesh {
                mesh: quad.clone(),
                position: [0.; 3],
                rotate: [0.; 3],
                scale: [1.; 3],
                color: "#FFFFFF".into(),
            };
            assert_eq!(
                project([0., 0., 100., 100.], &camera, None, &[object]).len(),
                1
            );
        }
        let behind = cube([0., 0., 10.], "#FFFFFF");
        let camera = Camera {
            yaw: 0.,
            pitch: 0.,
            ..camera()
        };
        assert!(project([0., 0., 100., 100.], &camera, None, &[behind]).is_empty());
    }

    #[test]
    fn limits_cover_triangles_objects_custom_data_and_lines() {
        let sphere = |segments| Object::Mesh {
            mesh: Mesh::Sphere {
                radius: 1.,
                segments,
            },
            position: [0.; 3],
            rotate: [0.; 3],
            scale: [1.; 3],
            color: "#FFFFFF".into(),
        };
        let check =
            |objects: &[Object]| validate(&None, &camera(), &None, objects, &mut Budget::default());
        assert!(check(&[sphere(64)]).is_ok());
        assert!(check(&vec![sphere(64); 3]).is_err());
        assert!(check(&[sphere(65)]).is_err());
        assert!(check(&vec![cube([0.; 3], "#FFFFFF"); 65]).is_err());
        let line = |n: usize| Object::Line {
            points: vec![[0.; 3]; n],
            color: "#FFFFFF".into(),
            width: 2.,
        };
        assert!(check(&[line(2048)]).is_ok());
        assert!(check(&[line(2049)]).is_err());
        assert!(check(&[line(1)]).is_err());
        let bad_index = Object::Mesh {
            mesh: Mesh::Custom {
                vertices: vec![[0.; 3]],
                faces: vec![[0, 0, 1]],
            },
            position: [0.; 3],
            rotate: [0.; 3],
            scale: [1.; 3],
            color: "#FFFFFF".into(),
        };
        assert!(check(&[bad_index]).is_err());
        let camera = Camera {
            pitch: 90.,
            ..camera()
        };
        assert!(validate(&None, &camera, &None, &[], &mut Budget::default()).is_err());
    }
}
