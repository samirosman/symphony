//! Building the Mesh Warp's points mesh over artwork (Harmony 24's
//! Shape-Aware Deformation builds its mesh from the drawing the same
//! way: only where there's artwork, none over empty space).
//!
//! The artwork's silhouette (lines widened a little, paint zones) is
//! filled with evenly spaced points and triangulated, keeping its edge as
//! the mesh's edge, so separate parts get separate pieces of mesh. The
//! placed points become mesh vertices (snapped onto the artwork when
//! placed just off it).

use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use symphony_core::math::Affine;
use symphony_core::puppet::PuppetMesh;
use symphony_core::stroke::Drawing;

use crate::shape::Shape;

/// Most vertices a mesh gets (solving is dense: keep it quick).
const MAX_VERTICES: usize = 700;

/// The silhouette of `drawings` (each with its drawing -> warp space
/// transform): lines widened by `margin` each side, paint zones, and
/// imported pictures' boxes, merged.
pub fn silhouette(drawings: &[(&Drawing, Affine)], margin: f32) -> Vec<Shape> {
    let mut polygons: Vec<Vec<[f32; 2]>> = Vec::new();
    for (d, m) in drawings {
        let map = |p: [f32; 2]| {
            let (x, y) = m.apply(p[0], p[1]);
            [x, y]
        };
        for st in &d.strokes {
            let outline = if st.shape.is_empty() {
                let mut wide = st.clone();
                wide.min_width += margin * 2.0;
                wide.max_width += margin * 2.0;
                crate::shape::outline(&wide)
            } else {
                vec![st.shape.clone()]
            };
            for ring in outline.into_iter().flatten() {
                polygons.push(ring.into_iter().map(map).collect());
            }
        }
        for f in &d.fills {
            for ring in &f.contours {
                polygons.push(ring.iter().copied().map(map).collect());
            }
        }
        if let Some(img) = d.image {
            let [x, y, w, h] = img.rect;
            polygons.push(
                [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
                    .into_iter()
                    .map(map)
                    .collect(),
            );
        }
    }
    crate::shape::from_polygons(polygons, false)
}

fn inside(shapes: &[Shape], p: [f32; 2]) -> bool {
    let mut n = 0;
    for ring in shapes.iter().flatten() {
        if crate::inside_polygon(p[0], p[1], ring) {
            n += 1;
        }
    }
    n % 2 == 1
}

/// Points every `step` along a closed ring.
fn resample(ring: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
    let n = ring.len();
    let mut out = Vec::new();
    let mut carry = 0.0f32;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let mut t = carry;
        while t < len {
            let k = t / len.max(1e-9);
            out.push([a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k]);
            t += step;
        }
        carry = t - len;
    }
    out
}

/// A mesh over `shapes` with triangles about `density` across, and the
/// `pins` as vertices. Returns the mesh and where each pin ended up
/// (moved onto the artwork if it was off it). None when there's no
/// artwork to build on.
pub fn build(
    shapes: &[Shape],
    pins: &[[f32; 2]],
    density: f32,
) -> Option<(PuppetMesh, Vec<[f32; 2]>)> {
    if shapes.is_empty() {
        return None;
    }
    // Coarser until it fits the vertex budget.
    let area: f32 = shapes
        .iter()
        .map(|s| crate::shape::shape_area(s).abs())
        .sum();
    let perimeter: f32 = shapes
        .iter()
        .flatten()
        .map(|r| {
            r.iter()
                .zip(r.iter().cycle().skip(1))
                .map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1]))
                .sum::<f32>()
        })
        .sum();
    let mut step = density.max(1.0);
    while area / (step * step * 0.87) + perimeter / step > MAX_VERTICES as f32 {
        step *= 1.25;
    }
    let mut cdt: ConstrainedDelaunayTriangulation<Point2<f64>> =
        ConstrainedDelaunayTriangulation::new();
    let add = |cdt: &mut ConstrainedDelaunayTriangulation<Point2<f64>>, p: [f32; 2]| {
        cdt.insert(Point2::new(p[0] as f64, p[1] as f64)).ok()
    };
    // The edge, kept as the mesh's edge.
    for ring in shapes.iter().flatten() {
        let pts = resample(ring, step);
        if pts.len() < 3 {
            continue;
        }
        let handles: Vec<_> = pts.iter().filter_map(|&p| add(&mut cdt, p)).collect();
        for i in 0..handles.len() {
            let (a, b) = (handles[i], handles[(i + 1) % handles.len()]);
            if a != b && cdt.can_add_constraint(a, b) {
                cdt.add_constraint(a, b);
            }
        }
    }
    // Inside: a staggered grid (near-equilateral triangles), kept off the
    // edge a little so no sliver triangles form there.
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in shapes.iter().flatten().flatten() {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let row = step * 0.866;
    let mut y = lo[1] + row * 0.5;
    let mut odd = false;
    while y < hi[1] {
        let mut x = lo[0] + if odd { step * 0.5 } else { 0.0 } + step * 0.25;
        while x < hi[0] {
            let p = [x, y];
            if inside(shapes, p) && edge_distance(shapes, p) > step * 0.45 {
                add(&mut cdt, p);
            }
            x += step;
        }
        y += row;
        odd = !odd;
    }
    // The points: on the artwork, or moved to its nearest edge.
    let mut placed = Vec::with_capacity(pins.len());
    for &p in pins {
        let q = if inside(shapes, p) {
            p
        } else {
            nearest_edge_point(shapes, p)
        };
        add(&mut cdt, q);
        placed.push(q);
    }
    // Triangles inside the artwork only.
    let mut index = std::collections::HashMap::new();
    let mut vertices: Vec<[f32; 2]> = Vec::new();
    let mut triangles = Vec::new();
    for face in cdt.inner_faces() {
        let vs = face.vertices();
        let ps = vs.map(|v| {
            let p = v.position();
            [p.x as f32, p.y as f32]
        });
        let c = [
            (ps[0][0] + ps[1][0] + ps[2][0]) / 3.0,
            (ps[0][1] + ps[1][1] + ps[2][1]) / 3.0,
        ];
        if !inside(shapes, c) {
            continue;
        }
        let mut tri = [0u32; 3];
        for (k, v) in vs.iter().enumerate() {
            let i = *index.entry(v.fix().index()).or_insert_with(|| {
                vertices.push(ps[k]);
                (vertices.len() - 1) as u32
            });
            tri[k] = i;
        }
        triangles.push(tri);
    }
    if triangles.is_empty() {
        return None;
    }
    // Each point's vertex (the nearest; a point is a vertex, so exact).
    let pin_vertices = placed
        .iter()
        .map(|p| {
            vertices
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    let d = |q: &[f32; 2]| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                    d(a.1).total_cmp(&d(b.1))
                })
                .map_or(0, |(i, _)| i as u32)
        })
        .collect();
    Some((
        PuppetMesh {
            vertices,
            triangles,
            pins: pin_vertices,
        },
        placed,
    ))
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> ([f32; 2], f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = [a[0] + dx * t, a[1] + dy * t];
    (q, (q[0] - p[0]).hypot(q[1] - p[1]))
}

fn edge_distance(shapes: &[Shape], p: [f32; 2]) -> f32 {
    nearest(shapes, p).1
}

fn nearest_edge_point(shapes: &[Shape], p: [f32; 2]) -> [f32; 2] {
    nearest(shapes, p).0
}

fn nearest(shapes: &[Shape], p: [f32; 2]) -> ([f32; 2], f32) {
    let mut best = (p, f32::MAX);
    for ring in shapes.iter().flatten() {
        for i in 0..ring.len() {
            let (q, d) = segment_distance(p, ring[i], ring[(i + 1) % ring.len()]);
            if d < best.1 {
                best = (q, d);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Shape {
        vec![vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]]]
    }

    #[test]
    fn a_mesh_covers_the_artwork_only_and_holds_the_points() {
        // Two separate bars.
        let shapes = vec![rect(0.0, 0.0, 200.0, 20.0), rect(0.0, 100.0, 200.0, 20.0)];
        let pins = [[10.0, 10.0], [190.0, 10.0], [100.0, 300.0]];
        let (mesh, placed) = build(&shapes, &pins, 10.0).unwrap();
        // The area is covered, and nothing between the bars.
        let area: f32 = mesh
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.vertices[i as usize]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum();
        assert!((area - 8000.0).abs() < 80.0, "{area}");
        assert!(mesh.vertices.iter().all(|v| v[1] <= 20.5 || v[1] >= 99.5));
        // Points are vertices; the one off the art moved onto it.
        assert_eq!(mesh.pins.len(), 3);
        assert_eq!(mesh.vertices[mesh.pins[0] as usize], [10.0, 10.0]);
        assert!((placed[2][1] - 120.0).abs() < 1e-3, "{:?}", placed[2]);
        assert!(mesh.vertices.len() <= MAX_VERTICES + 3);
    }

    /// Bending a bar with points at both ends and one moved: the other bar
    /// (no points on it) doesn't move: only connected art follows.
    #[test]
    fn only_connected_artwork_follows_a_point() {
        let shapes = vec![rect(0.0, 0.0, 200.0, 20.0), rect(0.0, 100.0, 200.0, 20.0)];
        let (mesh, _) = build(&shapes, &[[10.0, 10.0], [190.0, 10.0]], 10.0).unwrap();
        let now = symphony_core::puppet::deform(&mesh, &[[10.0, 10.0], [190.0, -50.0]]);
        for (v, n) in mesh.vertices.iter().zip(&now) {
            if v[1] > 50.0 {
                assert!((v[0] - n[0]).abs() < 0.5 && (v[1] - n[1]).abs() < 0.5);
            }
        }
        let tip = mesh.pins[1] as usize;
        assert!((now[tip][1] + 50.0).abs() < 1.0);
    }
}
