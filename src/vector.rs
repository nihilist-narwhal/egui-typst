//! Native vector backend: a Typst page becomes filled polygons, which become egui meshes.
//!
//! Typst lays a snippet out as a [`Frame`](typst::layout::Frame) of glyph runs and shapes. Here
//! the frame is walked once into *contours* (closed polygons in points: glyph outlines with their
//! curves flattened, rectangles, stroked lines as quads), independent of colour and scale.
//! [`Vector::mesh`] then turns contours into triangles for one scale and colour:
//!
//! - epaint fills convex shapes only and glyphs are not convex, so the interior is triangulated
//!   with `lyon` (non-zero winding, as fonts define their outlines);
//! - nothing in egui anti-aliases a raw mesh, so edges are feathered by hand: the opaque interior
//!   is *inset* by half a device pixel and a one-pixel ring ramps alpha from 1 to 0 across the true
//!   edge, which gives the usual coverage ramp without making glyphs heavier.
//!
//! Not drawn (and counted in [`Vector::skipped`]): images, gradients and tilings, colour glyphs
//! (emoji), clipped groups, dashed strokes and strokes on curves other than their straight
//! segments. Math does not need them.

use egui::epaint::Vertex;
use egui::{Color32, Mesh, Pos2};
use lyon_tessellation::math::{Point, point};
use lyon_tessellation::path::Path;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};
use ttf_parser::{GlyphId, OutlineBuilder};
use typst::layout::{Frame, FrameItem, GroupItem, Point as TPoint, Transform};
use typst::text::TextItem;
use typst::text::color::should_outline;
use typst::visualize::{CurveItem, FillRule, FixedStroke, Geometry, Paint, Shape};

use crate::SENTINEL;

/// Distance a flattened curve may stray from the true curve, in points.
const FLATTEN_TOLERANCE_PT: f32 = 0.01;
/// Most line segments one curve is split into.
const MAX_SEGMENTS: usize = 64;
/// How much of the feather lies inside the true edge (the rest is outside).
const INSET_FRACTION: f32 = 0.5;
/// The longest miter at a corner, as a multiple of the offset (beyond it the corner is bevelled).
const MITER_LIMIT: f32 = 2.5;
/// Most contours and points kept per snippet (a hostile snippet cannot ask for more).
const MAX_CONTOURS: usize = 100_000;
const MAX_POINTS: usize = 2_000_000;

/// Whose colour a path is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    /// The text colour the caller picks (the snippet used the placeholder colour).
    Text,
    /// A colour the snippet chose itself (RGBA, not premultiplied).
    Solid([u8; 4]),
}

/// Contours sharing one ink; filled together with the non-zero rule.
#[derive(Clone, Debug)]
struct Fill {
    contours: Vec<Vec<[f32; 2]>>,
    ink: Ink,
}

/// A typeset snippet as polygons, in points from the page's top-left corner.
#[derive(Clone, Debug)]
pub struct Vector {
    /// The page size in points.
    pub size_pt: [f32; 2],
    /// Items that could not be drawn (see the module docs).
    pub skipped: usize,
    fills: Vec<Fill>,
}

/// An affine map `(x, y) -> (a x + c y + e, b x + d y + f)`.
#[derive(Clone, Copy)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn translate(self, x: f32, y: f32) -> Self {
        Self {
            e: self.a * x + self.c * y + self.e,
            f: self.b * x + self.d * y + self.f,
            ..self
        }
    }

    /// `self` applied after `other` (points go through `other` first).
    fn then_after(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    fn apply(self, x: f32, y: f32) -> [f32; 2] {
        [
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        ]
    }
}

fn affine(t: &Transform) -> Affine {
    Affine {
        a: t.sx.get() as f32,
        b: t.ky.get() as f32,
        c: t.kx.get() as f32,
        d: t.sy.get() as f32,
        e: t.tx.to_pt() as f32,
        f: t.ty.to_pt() as f32,
    }
}

/// Collects flattened contours from curve segments.
struct Contours {
    done: Vec<Vec<[f32; 2]>>,
    current: Vec<[f32; 2]>,
    points: usize,
    full: bool,
}

impl Contours {
    fn new() -> Self {
        Self {
            done: Vec::new(),
            current: Vec::new(),
            points: 0,
            full: false,
        }
    }

    fn push(&mut self, p: [f32; 2]) {
        if self.points >= MAX_POINTS {
            self.full = true;
            return;
        }
        if p[0].is_finite() && p[1].is_finite() {
            self.points += 1;
            self.current.push(p);
        }
    }

    fn move_to(&mut self, p: [f32; 2]) {
        self.close();
        self.push(p);
    }

    fn close(&mut self) {
        let contour = std::mem::take(&mut self.current);
        if contour.len() >= 3 && self.done.len() < MAX_CONTOURS {
            self.done.push(contour);
        }
    }

    /// Flattens the cubic from the current point through `c1`, `c2` to `p`, into `n` equal steps in
    /// the parameter. Wang's bound: the curve strays from its polyline by at most
    /// `(3 / 4) * M / n^2` where `M` is the larger second difference of the control points,
    /// `|P0 - 2 P1 + P2|` and `|P1 - 2 P2 + P3|`; so `n = ceil(sqrt(0.75 M / tolerance))`.
    fn cubic_to(&mut self, c1: [f32; 2], c2: [f32; 2], p: [f32; 2]) {
        let Some(&p0) = self.current.last() else {
            return;
        };
        let second = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
            let (dx, dy) = (a[0] - 2.0 * b[0] + c[0], a[1] - 2.0 * b[1] + c[1]);
            (dx * dx + dy * dy).sqrt()
        };
        let m = second(p0, c1, c2).max(second(c1, c2, p));
        let n = ((0.75 * m / FLATTEN_TOLERANCE_PT).sqrt().ceil() as usize).clamp(1, MAX_SEGMENTS);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push([
                w0 * p0[0] + w1 * c1[0] + w2 * c2[0] + w3 * p[0],
                w0 * p0[1] + w1 * c1[1] + w2 * c2[1] + w3 * p[1],
            ]);
        }
    }
}

/// A `ttf-parser` outline sink that flattens into [`Contours`] through a transform.
struct Outline<'a> {
    out: &'a mut Contours,
    to_page: Affine,
    /// Font units to points.
    scale: f32,
}

impl Outline<'_> {
    fn page(&self, x: f32, y: f32) -> [f32; 2] {
        // glyph space is y-up
        self.to_page.apply(x * self.scale, -y * self.scale)
    }
}

impl OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.page(x, y);
        self.out.move_to(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.page(x, y);
        self.out.push(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // raise the quadratic to a cubic: c1 = p0 + 2/3 (q - p0), c2 = p + 2/3 (q - p)
        let Some(&p0) = self.out.current.last() else {
            return;
        };
        let (q, p) = (self.page(x1, y1), self.page(x, y));
        let c1 = [
            p0[0] + 2.0 / 3.0 * (q[0] - p0[0]),
            p0[1] + 2.0 / 3.0 * (q[1] - p0[1]),
        ];
        let c2 = [
            p[0] + 2.0 / 3.0 * (q[0] - p[0]),
            p[1] + 2.0 / 3.0 * (q[1] - p[1]),
        ];
        self.out.cubic_to(c1, c2, p);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1, c2, p) = (self.page(x1, y1), self.page(x2, y2), self.page(x, y));
        self.out.cubic_to(c1, c2, p);
    }

    fn close(&mut self) {
        self.out.close();
    }
}

struct Walker {
    fills: Vec<Fill>,
    skipped: usize,
    points: usize,
}

impl Walker {
    fn ink(&mut self, paint: &Paint) -> Option<Ink> {
        match paint {
            Paint::Solid(color) => {
                let rgba = color.to_vec4_u8();
                if rgba == [SENTINEL.r(), SENTINEL.g(), SENTINEL.b(), 255] {
                    Some(Ink::Text)
                } else {
                    Some(Ink::Solid(rgba))
                }
            }
            _ => {
                self.skipped += 1;
                None
            }
        }
    }

    fn add(&mut self, contours: Vec<Vec<[f32; 2]>>, ink: Ink) {
        let n: usize = contours.iter().map(Vec::len).sum();
        if contours.is_empty() || self.points + n > MAX_POINTS {
            return;
        }
        self.points += n;
        self.fills.push(Fill { contours, ink });
    }

    fn frame(&mut self, frame: &Frame, to_page: Affine) {
        for (pos, item) in frame.items() {
            let here = to_page.translate(pos.x.to_pt() as f32, pos.y.to_pt() as f32);
            match item {
                FrameItem::Group(group) => self.group(group, here),
                FrameItem::Text(text) => self.text(text, here),
                FrameItem::Shape(shape, _) => self.shape(shape, here),
                FrameItem::Image(..) => self.skipped += 1,
                FrameItem::Link(..) | FrameItem::Tag(..) => {}
            }
        }
    }

    fn group(&mut self, group: &GroupItem, here: Affine) {
        if group.clip.is_some() {
            self.skipped += 1;
        }
        self.frame(&group.frame, here.then_after(affine(&group.transform)));
    }

    fn text(&mut self, text: &TextItem, here: Affine) {
        let Some(ink) = self.ink(&text.fill) else {
            return;
        };
        if text.stroke.is_some() {
            self.skipped += 1;
        }
        let scale = text.size.to_pt() as f32 / text.font.units_per_em() as f32;
        let (mut x, mut y) = (0.0_f32, 0.0_f32);
        let mut contours = Vec::new();
        for glyph in &text.glyphs {
            let id = GlyphId(glyph.id);
            let (gx, gy) = (
                x + glyph.x_offset.at(text.size).to_pt() as f32,
                y + glyph.y_offset.at(text.size).to_pt() as f32,
            );
            if should_outline(&text.font, id) {
                let mut out = Contours::new();
                let mut builder = Outline {
                    out: &mut out,
                    to_page: here.translate(gx, -gy),
                    scale,
                };
                if text.font.ttf().outline_glyph(id, &mut builder).is_some() {
                    out.close();
                    contours.append(&mut out.done);
                }
            } else {
                self.skipped += 1;
            }
            x += glyph.x_advance.at(text.size).to_pt() as f32;
            y += glyph.y_advance.at(text.size).to_pt() as f32;
        }
        self.add(contours, ink);
    }

    fn shape(&mut self, shape: &Shape, here: Affine) {
        let at = |p: TPoint| here.apply(p.x.to_pt() as f32, p.y.to_pt() as f32);
        // the geometry as subpaths: (points, closed). A fill closes every subpath; a stroke
        // follows the closed flag.
        let mut subpaths: Vec<(Vec<[f32; 2]>, bool)> = Vec::new();
        match &shape.geometry {
            Geometry::Line(target) => {
                subpaths.push((vec![here.apply(0.0, 0.0), at(*target)], false))
            }
            Geometry::Rect(size) => {
                let (w, h) = (size.x.to_pt() as f32, size.y.to_pt() as f32);
                let ring = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
                subpaths.push((ring.iter().map(|&(x, y)| here.apply(x, y)).collect(), true));
            }
            Geometry::Curve(curve) => {
                let mut current: Vec<[f32; 2]> = Vec::new();
                for item in &curve.0 {
                    match item {
                        CurveItem::Move(p) => {
                            if current.len() >= 2 {
                                subpaths.push((std::mem::take(&mut current), false));
                            }
                            current = vec![at(*p)];
                        }
                        CurveItem::Line(p) => current.push(at(*p)),
                        CurveItem::Cubic(c1, c2, p) => {
                            let Some(&start) = current.last() else {
                                continue;
                            };
                            let mut flat = Contours::new();
                            flat.current.push(start);
                            flat.cubic_to(at(*c1), at(*c2), at(*p));
                            current.extend(flat.current.drain(1..));
                        }
                        CurveItem::Close => {
                            if current.len() >= 2 {
                                subpaths.push((std::mem::take(&mut current), true));
                            }
                        }
                    }
                }
                if current.len() >= 2 {
                    subpaths.push((current, false));
                }
            }
        }
        if let Some(paint) = &shape.fill
            && let Some(ink) = self.ink(paint)
        {
            if shape.fill_rule == FillRule::EvenOdd {
                self.skipped += 1;
            }
            let polygons: Vec<Vec<[f32; 2]>> = subpaths
                .iter()
                .filter(|(points, _)| points.len() >= 3)
                .map(|(points, _)| points.clone())
                .collect();
            self.add(polygons, ink);
        }
        if let Some(stroke) = &shape.stroke {
            let polylines: Vec<Vec<[f32; 2]>> = subpaths
                .into_iter()
                .map(|(mut points, closed)| {
                    if closed && let Some(&first) = points.first() {
                        points.push(first);
                    }
                    points
                })
                .collect();
            self.stroke(stroke, &polylines);
        }
    }

    /// A stroke as one quad per straight segment (butt caps; joins are left open, which is
    /// invisible at the thicknesses math uses).
    fn stroke(&mut self, stroke: &FixedStroke, polylines: &[Vec<[f32; 2]>]) {
        let width = stroke.thickness.to_pt() as f32;
        if !width.is_finite() || width <= 0.0 {
            return;
        }
        if stroke.dash.is_some() {
            self.skipped += 1;
        }
        let Some(ink) = self.ink(&stroke.paint) else {
            return;
        };
        let half = width / 2.0;
        let mut quads = Vec::new();
        for line in polylines {
            for pair in line.windows(2) {
                let (p, q) = (pair[0], pair[1]);
                let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1e-6 {
                    continue;
                }
                // the same winding for every quad, so overlaps union under the non-zero rule
                let (nx, ny) = (-dy / len * half, dx / len * half);
                quads.push(vec![
                    [p[0] + nx, p[1] + ny],
                    [q[0] + nx, q[1] + ny],
                    [q[0] - nx, q[1] - ny],
                    [p[0] - nx, p[1] - ny],
                ]);
            }
        }
        self.add(quads, ink);
    }
}

impl Vector {
    /// Walks a typeset page into contours.
    pub(crate) fn from_frame(frame: &Frame) -> Self {
        let mut walker = Walker {
            fills: Vec::new(),
            skipped: 0,
            points: 0,
        };
        walker.frame(frame, Affine::IDENTITY);
        Self {
            size_pt: [frame.width().to_pt() as f32, frame.height().to_pt() as f32],
            skipped: walker.skipped,
            fills: walker.fills,
        }
    }

    /// How many contours the snippet has (glyph parts and shapes).
    pub fn contour_count(&self) -> usize {
        self.fills.iter().map(|f| f.contours.len()).sum()
    }

    /// Triangles for one device scale and text colour, positions in points from the page's
    /// top-left corner. Edges are feathered over one device pixel (`1 / pixels_per_point`).
    pub fn mesh(&self, pixels_per_point: f32, text: Color32) -> Mesh {
        self.mesh_with_feather(1.0 / pixels_per_point.clamp(0.25, 16.0), text)
    }

    /// Like [`Vector::mesh`] with the feather width given in points (`0.0`: hard, aliased edges).
    /// The opaque interior is inset by half the feather and the ring spans the true edge.
    pub fn mesh_with_feather(&self, feather_pt: f32, text: Color32) -> Mesh {
        let feather = if feather_pt.is_finite() {
            feather_pt.clamp(0.0, 4.0)
        } else {
            0.0
        };
        let mut mesh = Mesh::default();
        for fill in &self.fills {
            let color = match fill.ink {
                Ink::Text => text,
                Ink::Solid([r, g, b, a]) => Color32::from_rgba_unmultiplied(r, g, b, a),
            };
            fill_mesh(&mut mesh, &fill.contours, color, feather);
        }
        mesh
    }
}

/// The signed area of a polygon (shoelace): positive when the interior is on the
/// `(-dy, dx)` side of each edge.
fn signed_area(contour: &[[f32; 2]]) -> f32 {
    let mut twice = 0.0;
    for (i, p) in contour.iter().enumerate() {
        let q = contour[(i + 1) % contour.len()];
        twice += p[0] * q[1] - q[0] * p[1];
    }
    twice / 2.0
}

fn unit(v: [f32; 2]) -> Option<[f32; 2]> {
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    (len > 1e-9).then(|| [v[0] / len, v[1] / len])
}

/// Drops repeated points (and the closing duplicate), which have no direction.
fn clean(contour: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(contour.len());
    for &p in contour {
        if out
            .last()
            .is_none_or(|q| (q[0] - p[0]).abs() > 1e-5 || (q[1] - p[1]).abs() > 1e-5)
        {
            out.push(p);
        }
    }
    while out.len() > 1
        && out
            .first()
            .zip(out.last())
            .is_some_and(|(a, b)| (a[0] - b[0]).abs() <= 1e-5 && (a[1] - b[1]).abs() <= 1e-5)
    {
        out.pop();
    }
    out
}

/// Offsets every vertex of a closed polygon along its (mitred) normal by `distance` points toward
/// the material side (`side` = +1 or -1), negative `distance` to go outward.
fn offset(contour: &[[f32; 2]], side: f32, distance: f32) -> Vec<[f32; 2]> {
    let n = contour.len();
    (0..n)
        .map(|i| {
            let (prev, p, next) = (contour[(i + n - 1) % n], contour[i], contour[(i + 1) % n]);
            let d0 = unit([p[0] - prev[0], p[1] - prev[1]]);
            let d1 = unit([next[0] - p[0], next[1] - p[1]]);
            let normal = |d: [f32; 2]| [-d[1] * side, d[0] * side];
            let (n0, n1) = match (d0, d1) {
                (Some(a), Some(b)) => (normal(a), normal(b)),
                (Some(a), None) | (None, Some(a)) => (normal(a), normal(a)),
                (None, None) => return p,
            };
            let sum = [n0[0] + n1[0], n0[1] + n1[1]];
            let Some(mid) = unit(sum) else {
                // a 180 degree reversal: no sensible mitre, stay put
                return p;
            };
            // the mitre length: 1 / cos(half the turning angle)
            let cos = (mid[0] * n0[0] + mid[1] * n0[1]).max(1.0 / MITER_LIMIT);
            let k = distance / cos;
            [p[0] + mid[0] * k, p[1] + mid[1] * k]
        })
        .collect()
}

fn fill_mesh(mesh: &mut Mesh, contours: &[Vec<[f32; 2]>], color: Color32, feather: f32) {
    let contours: Vec<Vec<[f32; 2]>> = contours
        .iter()
        .map(|c| clean(c))
        .filter(|c| c.len() >= 3)
        .collect();
    // the material lies on the side the largest contour's winding says (outer contours and holes
    // wind oppositely, so one side is material for all of them)
    let Some(biggest) = contours
        .iter()
        .map(|c| signed_area(c))
        .max_by(|a, b| a.abs().total_cmp(&b.abs()))
    else {
        return;
    };
    let side = if biggest >= 0.0 { 1.0 } else { -1.0 };

    let inset_by = feather * INSET_FRACTION;
    let outset_by = feather - inset_by;
    let insets: Vec<Vec<[f32; 2]>> = contours.iter().map(|c| offset(c, side, inset_by)).collect();

    // interior: the inset polygons, triangulated (non-zero winding)
    let mut builder = Path::builder();
    for contour in &insets {
        let mut points = contour.iter().map(|p| point(p[0], p[1]));
        if let Some(first) = points.next() {
            builder.begin(first);
            for p in points {
                builder.line_to(p);
            }
            builder.end(true);
        }
    }
    let path = builder.build();
    let mut buffers: VertexBuffers<Point, u32> = VertexBuffers::new();
    let options = FillOptions::non_zero().with_tolerance(feather / 8.0);
    if FillTessellator::new()
        .tessellate_path(
            &path,
            &options,
            &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position()),
        )
        .is_ok()
    {
        let base = mesh.vertices.len() as u32;
        mesh.vertices
            .extend(buffers.vertices.iter().map(|p| Vertex {
                pos: Pos2::new(p.x, p.y),
                uv: egui::epaint::WHITE_UV,
                color,
            }));
        mesh.indices
            .extend(buffers.indices.iter().map(|i| base + i));
    }

    // the ring: opaque on the inset line, transparent on the outset line
    for (contour, inset) in contours.iter().zip(&insets) {
        let outset = offset(contour, side, -outset_by);
        let n = contour.len();
        let base = mesh.vertices.len() as u32;
        for i in 0..n {
            for (p, c) in [(inset[i], color), (outset[i], Color32::TRANSPARENT)] {
                mesh.vertices.push(Vertex {
                    pos: Pos2::new(p[0], p[1]),
                    uv: egui::epaint::WHITE_UV,
                    color: c,
                });
            }
        }
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            let (a, b, c, d) = (
                base + 2 * i,
                base + 2 * i + 1,
                base + 2 * j + 1,
                base + 2 * j,
            );
            mesh.indices.extend([a, b, c, a, c, d]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator (a 64-bit LCG), so the test needs no dependency.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 40) as f32) / (1u64 << 24) as f32
        }
    }

    fn distance_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
        let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
        let len2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if len2 > 0.0 {
            ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + t * ab[0], a[1] + t * ab[1]];
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
    }

    /// The largest distance from the true curve to its flattening, over `samples` points.
    fn flattening_error(p: [[f32; 2]; 4], samples: usize) -> (f32, usize) {
        let mut out = Contours::new();
        out.push(p[0]);
        out.cubic_to(p[1], p[2], p[3]);
        let polyline = &out.current;
        let mut worst = 0.0_f32;
        for i in 0..=samples {
            let t = i as f32 / samples as f32;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            let on_curve = [
                w0 * p[0][0] + w1 * p[1][0] + w2 * p[2][0] + w3 * p[3][0],
                w0 * p[0][1] + w1 * p[1][1] + w2 * p[2][1] + w3 * p[3][1],
            ];
            let d = polyline
                .windows(2)
                .map(|s| distance_to_segment(on_curve, s[0], s[1]))
                .fold(f32::MAX, f32::min);
            worst = worst.max(d);
        }
        (worst, polyline.len() - 1)
    }

    #[test]
    fn flattening_stays_within_the_tolerance() {
        let mut rng = Lcg(7);
        let (mut worst_ratio, mut capped) = (0.0_f32, 0_usize);
        for _ in 0..3_000 {
            let mut point = || [rng.next() * 60.0, rng.next() * 60.0];
            let p = [point(), point(), point(), point()];
            let (error, segments) = flattening_error(p, 400);
            let ratio = error / FLATTEN_TOLERANCE_PT;
            if segments == MAX_SEGMENTS {
                // a curve this wild hits the cap on segments (which bounds the work a hostile
                // snippet can ask for), so the tolerance is not promised
                capped += 1;
                continue;
            }
            worst_ratio = worst_ratio.max(ratio);
            assert!(
                ratio <= 1.0,
                "{p:?}: error {error} with {segments} segments is {ratio:.2}x the tolerance"
            );
        }
        println!(
            "worst flattening error / tolerance: {worst_ratio:.3} over {} random cubics ({capped} hit the cap)",
            3_000 - capped
        );
    }
}
