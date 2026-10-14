//! Repository-local bounded Lyon output. GPUI receives only admitted triangles.
//! Internal tessellator scratch work is still cooperative, not time-preempted.
use anyhow::{Result, ensure};
use gpui::{Path, PathBuilder, PathStyle, Pixels, Point};
use lyon::{
    math,
    tessellation::{
        FillTessellator, FillVertex, StrokeTessellator, StrokeVertex, VertexBuffers, VertexId,
        geometry_builder::{
            FillGeometryBuilder, GeometryBuilder, GeometryBuilderError, StrokeGeometryBuilder,
        },
    },
};

pub(super) struct Builder {
    raw: lyon::path::builder::WithSvg<lyon::path::BuilderImpl>,
    style: PathStyle,
    invalid: bool,
}
impl Builder {
    pub(super) fn new(style: PathStyle) -> Self {
        Self {
            raw: lyon::path::Path::builder().with_svg(),
            style,
            invalid: false,
        }
    }
    fn checked(&mut self, p: Point<Pixels>) -> Option<math::Point> {
        let x = f32::from(p.x);
        let y = f32::from(p.y);
        if !x.is_finite() || !y.is_finite() || x.abs() > 1e12 || y.abs() > 1e12 {
            self.invalid = true;
            None
        } else {
            Some(math::point(x, y))
        }
    }
    pub(super) fn move_to(&mut self, p: Point<Pixels>) {
        if let Some(p) = self.checked(p) {
            self.raw.move_to(p);
        }
    }
    pub(super) fn line_to(&mut self, p: Point<Pixels>) {
        if let Some(p) = self.checked(p) {
            self.raw.line_to(p);
        }
    }
    pub(super) fn close(&mut self) {
        self.raw.close();
    }
    pub(super) fn build(self, limit: usize) -> Result<Path<Pixels>> {
        ensure!(!self.invalid, "Native contour coordinates out of range");
        let raw = self.raw.build();
        let mut output = Output::new(limit);
        let result = match self.style {
            PathStyle::Fill(options) => {
                FillTessellator::new().tessellate_path(&raw, &options, &mut output)
            }
            PathStyle::Stroke(options) => {
                StrokeTessellator::new().tessellate_path(&raw, &options, &mut output)
            }
        };
        ensure!(
            !output.overflow,
            "Native mesh exceeds remaining scene vertex budget"
        );
        result?;
        // Indices already obey the *expanded* mesh limit; GPUI cannot expand
        // an oversized triangle list and only then discover the limit.
        Ok(PathBuilder::build_path(output.buffers))
    }
}
struct Output {
    buffers: VertexBuffers<math::Point, u16>,
    limit: usize,
    overflow: bool,
}
impl Output {
    fn new(limit: usize) -> Self {
        Self {
            buffers: VertexBuffers {
                vertices: Vec::new(),
                indices: Vec::new(),
            },
            limit,
            overflow: false,
        }
    }
    fn vertex(&mut self, p: math::Point) -> std::result::Result<VertexId, GeometryBuilderError> {
        if self.overflow || self.buffers.vertices.len() >= self.limit.min(u16::MAX as usize) {
            self.overflow = true;
            return Err(GeometryBuilderError::TooManyVertices);
        }
        if !p.x.is_finite() || !p.y.is_finite() {
            return Err(GeometryBuilderError::InvalidVertex);
        }
        let id = VertexId(self.buffers.vertices.len() as u32);
        self.buffers.vertices.push(p);
        Ok(id)
    }
}
impl GeometryBuilder for Output {
    fn add_triangle(&mut self, a: VertexId, b: VertexId, c: VertexId) {
        // Lyon's triangle callback cannot return an error. Stop allocation now,
        // fail the next vertex callback (or finalization), never emit a prefix.
        if self.overflow || self.buffers.indices.len() + 3 > self.limit {
            self.overflow = true;
            return;
        }
        self.buffers
            .indices
            .extend([a.0 as u16, b.0 as u16, c.0 as u16]);
    }
    fn abort_geometry(&mut self) {
        self.buffers.vertices.clear();
        self.buffers.indices.clear();
    }
}
impl FillGeometryBuilder for Output {
    fn add_fill_vertex(
        &mut self,
        v: FillVertex<'_>,
    ) -> std::result::Result<VertexId, GeometryBuilderError> {
        self.vertex(v.position())
    }
}
impl StrokeGeometryBuilder for Output {
    fn add_stroke_vertex(
        &mut self,
        v: StrokeVertex<'_, '_>,
    ) -> std::result::Result<VertexId, GeometryBuilderError> {
        self.vertex(v.position())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn triangle_output_is_limited_before_expansion() {
        let mut o = Output::new(3);
        let a = o.vertex(math::point(0., 0.)).unwrap();
        let b = o.vertex(math::point(1., 0.)).unwrap();
        let c = o.vertex(math::point(0., 1.)).unwrap();
        o.add_triangle(a, b, c);
        o.add_triangle(a, b, c);
        assert!(o.overflow);
        assert_eq!(o.buffers.indices.len(), 3);
        assert!(o.vertex(math::point(2., 2.)).is_err());
        assert_eq!(o.buffers.vertices.len(), 3);
    }
    #[test]
    fn self_intersecting_star_cannot_expand_past_the_sink_limit() {
        let mut b = Builder::new(PathStyle::Fill(gpui::FillOptions::default()));
        for i in 0..255 {
            let angle = (i * 127 % 255) as f32 / 255. * std::f32::consts::TAU;
            let p = gpui::point(gpui::px(angle.cos() * 100.), gpui::px(angle.sin() * 100.));
            if i == 0 { b.move_to(p) } else { b.line_to(p) }
        }
        b.close();
        assert!(b.build(96).is_err());
    }
    #[test]
    fn nonfinite_contours_never_enter_native_tessellation() {
        let mut b = Builder::new(PathStyle::Fill(gpui::FillOptions::default()));
        b.move_to(gpui::point(gpui::px(f32::INFINITY), gpui::px(0.)));
        assert!(b.build(96).is_err());
    }
}
