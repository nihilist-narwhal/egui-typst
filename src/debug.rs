//! Helpers for looking at meshes without a GPU. Not part of the supported API.

use egui::Mesh;

/// Coverage (alpha, 0 to 255) of `mesh` on a `width` x `height` pixel grid at `scale` pixels per
/// point, drawn the way a GPU draws without multisampling: a pixel is painted when its centre is
/// inside a triangle, vertex colours are interpolated and blended premultiplied "over".
pub fn rasterise_alpha(mesh: &Mesh, scale: f32, width: usize, height: usize) -> Vec<f32> {
    let mut alpha = vec![0.0_f32; width * height];
    for tri in mesh.indices.chunks_exact(3) {
        let (Some(a), Some(b), Some(c)) = (
            mesh.vertices.get(tri[0] as usize),
            mesh.vertices.get(tri[1] as usize),
            mesh.vertices.get(tri[2] as usize),
        ) else {
            continue;
        };
        let v = [a, b, c];
        let p: Vec<(f32, f32)> = v
            .iter()
            .map(|v| (v.pos.x * scale, v.pos.y * scale))
            .collect();
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
        if area.abs() < 1e-9 || !area.is_finite() {
            continue;
        }
        let min_x = p.iter().map(|q| q.0).fold(f32::MAX, f32::min);
        let max_x = p.iter().map(|q| q.0).fold(f32::MIN, f32::max);
        let min_y = p.iter().map(|q| q.1).fold(f32::MAX, f32::min);
        let max_y = p.iter().map(|q| q.1).fold(f32::MIN, f32::max);
        let (x0, x1) = (
            min_x.floor().max(0.0) as usize,
            (max_x.ceil().max(0.0) as usize).min(width),
        );
        let (y0, y1) = (
            min_y.floor().max(0.0) as usize,
            (max_y.ceil().max(0.0) as usize).min(height),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p[1].0 - cx) * (p[2].1 - cy) - (p[2].0 - cx) * (p[1].1 - cy)) / area;
                let w1 = ((p[2].0 - cx) * (p[0].1 - cy) - (p[0].0 - cx) * (p[2].1 - cy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let src = w0 * f32::from(v[0].color.a())
                    + w1 * f32::from(v[1].color.a())
                    + w2 * f32::from(v[2].color.a());
                let dst = &mut alpha[y * width + x];
                *dst = src + *dst * (1.0 - src / 255.0);
            }
        }
    }
    alpha
}

/// The triangle edges of `mesh` in pixels at `scale`, for drawing a wireframe.
pub fn wire_edges(mesh: &Mesh, scale: f32) -> Vec<[(f32, f32); 2]> {
    let mut edges = Vec::new();
    for tri in mesh.indices.chunks_exact(3) {
        let points: Vec<(f32, f32)> = tri
            .iter()
            .filter_map(|&i| mesh.vertices.get(i as usize))
            .map(|v| (v.pos.x * scale, v.pos.y * scale))
            .collect();
        if let [a, b, c] = points[..] {
            edges.extend([[a, b], [b, c], [c, a]]);
        }
    }
    edges
}
