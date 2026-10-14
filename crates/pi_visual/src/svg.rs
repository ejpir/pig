//! A data-only SVG importer. No URL loaders, scripting, CSS, fonts or entities.
use crate::{
    Scene, Shape,
    graphics::{self, Paint, PathCommand as P, SvgLinearGradient},
    svg_path,
};
use anyhow::{Result, bail, ensure};
use roxmltree::Node;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::Instant,
};

const MAX_SOURCE: usize = 16 * 1024;
const IDENTITY: [f32; 6] = [1., 0., 0., 1., 0., 0.];
#[derive(Default)]
pub(super) struct Cache {
    entries: VecDeque<(String, String, String)>,
    bytes: usize,
}
impl Cache {
    fn parse(&mut self, kind: &str, source: &str) -> Result<String> {
        if let Some((_, _, json)) = self
            .entries
            .iter()
            .find(|(k, s, _)| k == kind && s == source)
        {
            return Ok(json.clone());
        }
        let json = match kind {
            "path" => serde_json::to_string(&svg_path::parse(source)?)?,
            "svg" => serde_json::to_string(&parse(source)?)?,
            _ => bail!("Unknown vector parser"),
        };
        ensure!(json.len() <= 128 * 1024, "Compiled SVG exceeds 128KiB");
        let bytes = kind.len() + source.len() + json.len();
        while self.entries.len() >= 8 || self.bytes + bytes > 512 * 1024 {
            if let Some((k, s, j)) = self.entries.pop_front() {
                self.bytes -= k.len() + s.len() + j.len();
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.entries
            .push_back((kind.into(), source.into(), json.clone()));
        Ok(json)
    }
}
pub(super) fn install<'js>(
    ctx: &rquickjs::Ctx<'js>,
    cache: Arc<Mutex<Cache>>,
    deadline: Instant,
) -> rquickjs::Result<()> {
    let calls = std::cell::Cell::new(0usize);
    let bytes = std::cell::Cell::new(0usize);
    let parser = rquickjs::Function::new(
        ctx.clone(),
        move |ctx: rquickjs::Ctx<'js>, kind: String, source: String| -> rquickjs::Result<String> {
            // Count failed requests and cache hits too. Shared by registration and drawing.
            calls.set(calls.get().saturating_add(1));
            bytes.set(bytes.get().saturating_add(source.len()));
            let result = (|| -> Result<String> {
                ensure!(Instant::now() < deadline, "SVG parser deadline exceeded");
                ensure!(
                    calls.get() <= 32 && bytes.get() <= 64 * 1024,
                    "SVG parser invocation budget exceeded"
                );
                ensure!(source.len() <= MAX_SOURCE, "SVG source exceeds 16KiB");
                let json = cache
                    .lock()
                    .map_err(|_| anyhow::anyhow!("SVG cache unavailable"))?
                    .parse(&kind, &source)?;
                ensure!(Instant::now() < deadline, "SVG parser deadline exceeded");
                Ok(json)
            })();
            result.map_err(|e| rquickjs::Exception::throw_message(&ctx, &format!("{e:#}")))
        },
    )?;
    ctx.globals().set("__visualParseVector", parser)
}

#[derive(Serialize)]
struct Svg {
    view_box: [f32; 4],
    width: f32,
    height: f32,
    stretch: bool,
    shapes: Scene,
}
fn scalar(s: &str) -> Result<f32> {
    svg_path::number(s.trim().strip_suffix("px").unwrap_or(s.trim()).parse()?)
}
fn value(node: Node<'_, '_>, name: &str, default: f32) -> Result<f32> {
    node.attribute(name).map_or(Ok(default), scalar)
}
fn numbers(s: &str) -> Result<Vec<f32>> {
    let mut out = Vec::new();
    for n in svgtypes::NumberListParser::from(s) {
        ensure!(out.len() < 512, "SVG number list too long");
        out.push(svg_path::number(n?)?);
    }
    Ok(out)
}
fn fraction(s: &str) -> Result<f64> {
    let s = s.trim();
    let n = if let Some(n) = s.strip_suffix('%') {
        n.parse::<f64>()? / 100.
    } else {
        s.parse()?
    };
    ensure!(
        n.is_finite() && (0. ..=1.).contains(&n),
        "SVG opacity/offset must be 0..1 or 0%..100%"
    );
    Ok(n)
}
fn rgba(s: &str, alpha: f64) -> Result<String> {
    if let Some(hex) = s.strip_prefix('#') {
        ensure!(
            hex.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid SVG hex color"
        );
    }
    let c: svgtypes::Color = s.parse()?;
    Ok(format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        c.red,
        c.green,
        c.blue,
        (f64::from(c.alpha) * alpha).round() as u8
    ))
}
fn transform(s: Option<&str>) -> Result<[f32; 6]> {
    use svgtypes::TransformListToken as T;
    let mut m = IDENTITY;
    for (i, t) in svgtypes::TransformListParser::from(s.unwrap_or("")).enumerate() {
        ensure!(i < 32, "Too many SVG transforms");
        let a = match t? {
            T::Matrix { a, b, c, d, e, f } => [a, b, c, d, e, f],
            T::Translate { tx, ty } => [1., 0., 0., 1., tx, ty],
            T::Scale { sx, sy } => [sx, 0., 0., sy, 0., 0.],
            T::Rotate { angle } => {
                svg_path::number(angle)?;
                let (s, c) = angle.to_radians().sin_cos();
                [c, s, -s, c, 0., 0.]
            }
            T::SkewX { angle } => {
                svg_path::number(angle)?;
                [1., 0., angle.to_radians().tan(), 1., 0., 0.]
            }
            T::SkewY { angle } => {
                svg_path::number(angle)?;
                [1., angle.to_radians().tan(), 0., 1., 0., 0.]
            }
        };
        let mut v = [0.; 6];
        for i in 0..6 {
            v[i] = svg_path::number(a[i])?;
        }
        m = graphics::multiply(m, v);
        ensure!(
            m.iter().all(|v| v.is_finite() && v.abs() <= 1e6),
            "SVG transform out of range"
        );
    }
    Ok(m)
}
fn check_tree(root: Node<'_, '_>) -> Result<()> {
    let mut ids = HashSet::new();
    for n in root.descendants() {
        ensure!(
            n.ancestors().count() <= 12,
            "SVG XML nesting exceeds 10 elements"
        );
        if n.is_pi() {
            bail!("SVG processing instructions are unsupported");
        }
        if n.is_text() {
            ensure!(
                n.text().unwrap_or("").trim().is_empty()
                    || n.parent()
                        .is_some_and(|p| matches!(p.tag_name().name(), "title" | "desc")),
                "SVG text elements are unsupported"
            );
            continue;
        }
        if !n.is_element() {
            continue;
        }
        ensure!(
            matches!(
                n.tag_name().namespace(),
                None | Some("http://www.w3.org/2000/svg")
            ),
            "Unsupported SVG namespace"
        );
        let tag = n.tag_name().name();
        let parent = n
            .parent_element()
            .map(|p| p.tag_name().name())
            .unwrap_or("");
        let specific: &[&str] = match tag {
            "svg" => {
                ensure!(n == root, "Nested SVG viewports are unsupported");
                &[
                    "width",
                    "height",
                    "viewBox",
                    "preserveAspectRatio",
                    "version",
                    "overflow",
                ]
            }
            "g" => &[],
            "path" => &["d"],
            "rect" => &["x", "y", "width", "height", "rx", "ry"],
            "circle" => &["cx", "cy", "r"],
            "ellipse" => &["cx", "cy", "rx", "ry"],
            "line" => &["x1", "y1", "x2", "y2"],
            "polyline" | "polygon" => &["points"],
            "defs" => &[],
            "linearGradient" => &[
                "x1",
                "y1",
                "x2",
                "y2",
                "gradientUnits",
                "gradientTransform",
                "spreadMethod",
            ],
            "stop" => &["offset", "stop-color", "stop-opacity"],
            "title" | "desc" => &[],
            _ => bail!("Unsupported SVG element: {tag}"),
        };
        match tag {
            "svg" => {}
            "linearGradient" => ensure!(parent == "defs", "SVG gradients must be inside defs"),
            "stop" => ensure!(parent == "linearGradient", "SVG stop outside gradient"),
            _ => ensure!(
                matches!(parent, "svg" | "g"),
                "Unsupported SVG child {tag} in {parent}"
            ),
        }
        let style = matches!(
            tag,
            "svg" | "g" | "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
        );
        ensure!(n.attributes().len() <= 32, "Too many SVG attributes");
        for a in n.attributes() {
            ensure!(
                a.namespace().is_none(),
                "Namespaced SVG attributes are unsupported"
            );
            ensure!(
                a.name() == "id"
                    || specific.contains(&a.name())
                    || (style
                        && [
                            "fill",
                            "stroke",
                            "stroke-width",
                            "fill-rule",
                            "fill-opacity",
                            "stroke-opacity",
                            "color",
                            "transform",
                            "stroke-linecap",
                            "stroke-linejoin",
                            "stroke-miterlimit"
                        ]
                        .contains(&a.name())),
                "Unsupported SVG attribute: {}",
                a.name()
            );
        }
        if let Some(id) = n.attribute("id") {
            ensure!(
                !id.is_empty()
                    && id.len() <= 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b)),
                "Invalid SVG id"
            );
            ensure!(ids.insert(id), "Duplicate SVG id: {id}");
        }
    }
    Ok(())
}
#[derive(Clone)]
struct Style {
    fill: String,
    stroke: String,
    color: String,
    width: f32,
    even_odd: bool,
    fill_alpha: f64,
    stroke_alpha: f64,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            fill: "black".into(),
            stroke: "none".into(),
            color: "black".into(),
            width: 1.,
            even_odd: false,
            fill_alpha: 1.,
            stroke_alpha: 1.,
        }
    }
}
impl Style {
    fn at(&self, n: Node<'_, '_>) -> Result<Self> {
        let mut s = self.clone();
        for (name, to) in [
            ("fill", &mut s.fill),
            ("stroke", &mut s.stroke),
            ("color", &mut s.color),
        ] {
            if let Some(v) = n.attribute(name) {
                *to = v.trim().into();
            }
        }
        rgba(&s.color, 1.)?;
        s.width = value(n, "stroke-width", s.width)?;
        ensure!(s.width >= 0., "Negative SVG stroke width");
        if let Some(v) = n.attribute("fill-rule") {
            s.even_odd = match v {
                "nonzero" => false,
                "evenodd" => true,
                _ => bail!("Unsupported SVG fill rule"),
            };
        }
        if let Some(v) = n.attribute("fill-opacity") {
            s.fill_alpha = fraction(v)?;
        }
        if let Some(v) = n.attribute("stroke-opacity") {
            s.stroke_alpha = fraction(v)?;
        }
        ensure!(
            n.attribute("stroke-linecap").is_none_or(|v| v == "butt"),
            "Only butt SVG stroke caps are supported"
        );
        ensure!(
            n.attribute("stroke-linejoin").is_none_or(|v| v == "miter"),
            "Only miter SVG stroke joins are supported"
        );
        ensure!(
            value(n, "stroke-miterlimit", 4.)? == 4.,
            "Only SVG miter limit 4 is supported"
        );
        Ok(s)
    }
}
struct Gradient {
    start: [String; 2],
    end: [String; 2],
    object: bool,
    matrix: [f32; 6],
    from: String,
    to: String,
}
fn gradients(root: Node<'_, '_>) -> Result<HashMap<String, Gradient>> {
    let mut result = HashMap::new();
    for n in root
        .descendants()
        .filter(|n| n.has_tag_name("linearGradient"))
    {
        ensure!(result.len() < 16, "Too many SVG gradients");
        let id = n
            .attribute("id")
            .ok_or_else(|| anyhow::anyhow!("SVG gradient requires id"))?;
        let object = match n.attribute("gradientUnits").unwrap_or("objectBoundingBox") {
            "objectBoundingBox" => true,
            "userSpaceOnUse" => false,
            _ => bail!("Unsupported SVG gradient units"),
        };
        ensure!(
            n.attribute("spreadMethod").is_none_or(|s| s == "pad"),
            "Only pad SVG gradients are supported"
        );
        let stops: Vec<_> = n.children().filter(Node::is_element).collect();
        ensure!(
            stops.len() == 2,
            "SVG gradients require exactly two stops at 0 and 1"
        );
        let stop = |n: Node<'_, '_>, offset: f64| -> Result<String> {
            ensure!(
                fraction(n.attribute("offset").unwrap_or("0"))? == offset,
                "SVG gradient stops must be at 0 and 1"
            );
            rgba(
                n.attribute("stop-color").unwrap_or("black"),
                fraction(n.attribute("stop-opacity").unwrap_or("1"))?,
            )
        };
        let g = Gradient {
            start: [
                n.attribute("x1").unwrap_or("0%").into(),
                n.attribute("y1").unwrap_or("0%").into(),
            ],
            end: [
                n.attribute("x2").unwrap_or("100%").into(),
                n.attribute("y2").unwrap_or("0%").into(),
            ],
            object,
            matrix: transform(n.attribute("gradientTransform"))?,
            from: stop(stops[0], 0.)?,
            to: stop(stops[1], 1.)?,
        };
        // Validate unused definitions too, with a nondegenerate reference box.
        gradient_paint(&g, kurbo::Rect::new(0., 0., 1., 1.), [0., 0., 1., 1.], 1.)?;
        result.insert(id.into(), g);
    }
    Ok(result)
}
fn gradient_paint(
    g: &Gradient,
    bounds: kurbo::Rect,
    view: [f32; 4],
    alpha: f64,
) -> Result<Option<Paint>> {
    let coord = |s: &str, axis: usize| -> Result<f64> {
        let s = s.trim();
        if let Some(p) = s.strip_suffix('%') {
            let f = p.parse::<f64>()? / 100.;
            svg_path::number(f)?;
            Ok(if g.object {
                f
            } else {
                f * f64::from(view[axis + 2])
            })
        } else {
            Ok(f64::from(scalar(s)?))
        }
    };
    let a = [coord(&g.start[0], 0)?, coord(&g.start[1], 1)?];
    let b = [coord(&g.end[0], 0)?, coord(&g.end[1], 1)?];
    let from = rgba(&g.from, alpha)?;
    let to = rgba(&g.to, alpha)?;
    let delta = [b[0] - a[0], b[1] - a[1]];
    let norm = delta[0] * delta[0] + delta[1] * delta[1];
    // Object-bbox paint servers are ignored on zero-area geometry (including
    // the stroke of a horizontal/vertical line). Coincident endpoints instead
    // use the final stop on a usable coordinate system.
    if g.object && (bounds.width() == 0. || bounds.height() == 0.) {
        return Ok(None);
    }
    if norm == 0. {
        return Ok(Some(Paint::Solid(to)));
    }
    let mut m = g.matrix.map(f64::from);
    if g.object {
        m = [
            m[0] * bounds.width(),
            m[1] * bounds.height(),
            m[2] * bounds.width(),
            m[3] * bounds.height(),
            m[4] * bounds.width() + bounds.x0,
            m[5] * bounds.height() + bounds.y0,
        ];
    }
    // Conditioning is relative to the linear transform, not its absolute
    // size: a tiny square can be well-conditioned and enlarged by g.svg.
    let scale = m[..4].iter().map(|n| n.abs()).fold(0_f64, f64::max);
    ensure!(
        scale > 0. && scale.is_finite(),
        "Singular SVG gradient transform"
    );
    let ma = m[0] / scale;
    let mb = m[1] / scale;
    let mc = m[2] / scale;
    let md = m[3] / scale;
    let det = ma * md - mb * mc;
    ensure!(
        det.is_finite() && det.abs() > 1e-12,
        "Singular/ill-conditioned SVG gradient transform"
    );
    let p = [delta[0] / norm, delta[1] / norm];
    let n = [
        (md * p[0] - mb * p[1]) / det / scale,
        (-mc * p[0] + ma * p[1]) / det / scale,
    ];
    let projection = [
        n[0],
        n[1],
        -p[0] * a[0] - p[1] * a[1] - n[0] * m[4] - n[1] * m[5],
    ];
    let paint = Paint::SvgLinear(SvgLinearGradient {
        projection,
        from,
        to,
    });
    ensure!(paint.valid(), "SVG gradient projection out of range");
    Ok(Some(paint))
}
fn ink(
    value: &str,
    color: &str,
    alpha: f64,
    commands: &[P],
    gradients: &HashMap<String, Gradient>,
    view: [f32; 4],
) -> Result<Option<Paint>> {
    if value == "none" {
        return Ok(None);
    }
    if let Some(id) = value
        .strip_prefix("url(#")
        .and_then(|s| s.strip_suffix(')'))
    {
        let g = gradients
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown local SVG gradient: {id}"))?;
        return gradient_paint(g, svg_path::bounds(commands), view, alpha);
    }
    Ok(Some(Paint::Solid(rgba(
        if value == "currentColor" {
            color
        } else {
            value
        },
        alpha,
    )?)))
}
fn geometry(n: Node<'_, '_>) -> Result<Vec<P>> {
    let get = |name| value(n, name, 0.);
    let source = match n.tag_name().name() {
        "path" => n.attribute("d").unwrap_or("").into(),
        "line" => format!(
            "M {} {} L {} {}",
            get("x1")?,
            get("y1")?,
            get("x2")?,
            get("y2")?
        ),
        "rect" => {
            let (x, y, w, h) = (get("x")?, get("y")?, get("width")?, get("height")?);
            ensure!(w >= 0. && h >= 0., "Negative SVG rect size");
            let mut rx = value(n, "rx", get("ry")?)?;
            let mut ry = value(n, "ry", rx)?;
            ensure!(rx >= 0. && ry >= 0., "Negative SVG rect radius");
            rx = rx.min(w / 2.);
            ry = ry.min(h / 2.);
            if w == 0. || h == 0. {
                return Ok(vec![]);
            }
            if rx == 0. || ry == 0. {
                format!("M {x} {y} h {w} v {h} h {} Z", -w)
            } else {
                format!(
                    "M {} {y} H {} A {rx} {ry} 0 0 1 {} {} V {} A {rx} {ry} 0 0 1 {} {} H {} A {rx} {ry} 0 0 1 {x} {} V {} A {rx} {ry} 0 0 1 {} {y} Z",
                    x + rx,
                    x + w - rx,
                    x + w,
                    y + ry,
                    y + h - ry,
                    x + w - rx,
                    y + h,
                    x + rx,
                    y + h - ry,
                    y + ry,
                    x + rx
                )
            }
        }
        "circle" | "ellipse" => {
            let (x, y) = (get("cx")?, get("cy")?);
            let (rx, ry) = if n.has_tag_name("circle") {
                let r = get("r")?;
                (r, r)
            } else {
                (get("rx")?, get("ry")?)
            };
            ensure!(rx >= 0. && ry >= 0., "Negative SVG ellipse radius");
            if rx == 0. || ry == 0. {
                return Ok(vec![]);
            }
            format!(
                "M {} {y} A {rx} {ry} 0 1 0 {} {y} A {rx} {ry} 0 1 0 {} {y} Z",
                x + rx,
                x - rx,
                x + rx
            )
        }
        "polyline" | "polygon" => {
            let pts = numbers(n.attribute("points").unwrap_or(""))?;
            ensure!(pts.len() % 2 == 0, "Odd SVG points coordinate count");
            if pts.is_empty() {
                return Ok(vec![]);
            }
            let mut out = vec![P::Move {
                to: [pts[0], pts[1]],
            }];
            for p in pts[2..].as_chunks::<2>().0 {
                out.push(P::Line { to: [p[0], p[1]] });
            }
            if n.has_tag_name("polygon") {
                out.push(P::Close);
            }
            ensure!(out.len() <= 256, "SVG path exceeds 256 lowered commands");
            return Ok(out);
        }
        _ => bail!("Unsupported SVG geometry"),
    };
    svg_path::parse(&source)
}
fn compile(
    n: Node<'_, '_>,
    parent: &Style,
    gradients: &HashMap<String, Gradient>,
    view: [f32; 4],
) -> Result<Scene> {
    if matches!(n.tag_name().name(), "defs" | "title" | "desc") {
        return Ok(vec![]);
    }
    let style = parent.at(n)?;
    let matrix = transform(n.attribute("transform"))?;
    let scene = if matches!(n.tag_name().name(), "svg" | "g") {
        // Resolve paints even on an empty group; do not hide unsupported URLs.
        ink(
            &style.fill,
            &style.color,
            style.fill_alpha,
            &[],
            gradients,
            view,
        )?;
        ink(
            &style.stroke,
            &style.color,
            style.stroke_alpha,
            &[],
            gradients,
            view,
        )?;
        let mut children = Vec::new();
        for c in n.children().filter(Node::is_element) {
            children.extend(compile(c, &style, gradients, view)?);
        }
        if n.has_tag_name("g") {
            vec![Shape::Group {
                transform: matrix,
                children,
            }]
        } else if matrix == IDENTITY {
            children
        } else {
            vec![Shape::Group {
                transform: matrix,
                children,
            }]
        }
    } else {
        let commands = geometry(n)?;
        let fill = ink(
            &style.fill,
            &style.color,
            style.fill_alpha,
            &commands,
            gradients,
            view,
        )?;
        let stroke = ink(
            &style.stroke,
            &style.color,
            style.stroke_alpha,
            &commands,
            gradients,
            view,
        )?;
        let color = if style.width == 0. { None } else { stroke };
        if commands.is_empty() || (fill.is_none() && color.is_none()) {
            return Ok(vec![]);
        }
        let path = Shape::Path {
            commands,
            scale_stroke: true,
            color,
            width: if style.width == 0. { 1. } else { style.width },
            fill,
            even_odd: style.even_odd,
        };
        if matrix == IDENTITY {
            vec![path]
        } else {
            vec![Shape::Group {
                transform: matrix,
                children: vec![path],
            }]
        }
    };
    Ok(scene)
}
fn parse(source: &str) -> Result<Svg> {
    ensure!(source.len() <= MAX_SOURCE, "SVG source exceeds 16KiB");
    ensure!(!source.contains("<!DOCTYPE"), "SVG DTDs are unsupported");
    let doc = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 512,
            entity_resolver: None,
        },
    )?;
    ensure!(
        !doc.descendants().any(|n| n.is_pi()),
        "SVG processing instructions are unsupported"
    );
    let root = doc.root_element();
    ensure!(root.has_tag_name("svg"), "Expected SVG root element");
    check_tree(root)?;
    ensure!(
        root.attribute("overflow").is_none_or(|s| s == "visible"),
        "SVG viewport clipping is unsupported; only canvas clipping is available"
    );
    ensure!(
        root.attribute("version")
            .is_none_or(|s| matches!(s, "1.0" | "1.1" | "2.0")),
        "Unsupported SVG version"
    );
    let view = if let Some(v) = root.attribute("viewBox") {
        let v = numbers(v)?;
        ensure!(v.len() == 4, "SVG viewBox needs four numbers");
        [v[0], v[1], v[2], v[3]]
    } else {
        [
            0.,
            0.,
            value(root, "width", 300.)?,
            value(root, "height", 150.)?,
        ]
    };
    ensure!(
        view[2] > 0. && view[3] > 0.,
        "SVG viewBox dimensions must be positive"
    );
    let width = value(root, "width", view[2])?;
    let height = value(root, "height", view[3])?;
    ensure!(width > 0. && height > 0., "SVG size must be positive");
    let stretch = match root
        .attribute("preserveAspectRatio")
        .unwrap_or("xMidYMid meet")
        .trim()
    {
        "none" => true,
        "xMidYMid" | "xMidYMid meet" => false,
        _ => bail!("SVG preserveAspectRatio supports xMidYMid meet or none"),
    };
    let gradients = gradients(root)?;
    let shapes = compile(root, &Style::default(), &gradients, view)?;
    // Include g.svg's wrapper in the ordinary scene/depth budgets.
    graphics::validate_scene(&vec![Shape::Group {
        transform: IDENTITY,
        children: shapes.clone(),
    }])?;
    Ok(Svg {
        view_box: view,
        width,
        height,
        stretch,
        shapes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pure_cache_is_bounded_by_bytes_and_entries_and_eviction_preserves_results() {
        let path = format!("<path d='M100000 100000{}'/>", "h1".repeat(255));
        let mut cache = Cache::default();
        let first = format!("<svg><!--0-->{}</svg>", path.repeat(8));
        let expected = cache.parse("svg", &first).unwrap();
        for i in 1..12 {
            let source = format!("<svg><!--{i}-->{}</svg>", path.repeat(8));
            cache.parse("svg", &source).unwrap();
            assert!(cache.entries.len() <= 8 && cache.bytes <= 512 * 1024);
        }
        assert!(!cache.entries.iter().any(|(_, s, _)| s == &first));
        assert_eq!(cache.parse("svg", &first).unwrap(), expected);
        assert!(cache.parse("path", &first).is_err());
    }
}
