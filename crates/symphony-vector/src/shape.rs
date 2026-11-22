//! Brush strokes as outlines, so the Eraser can cut its own shape out of
//! them (Harmony's brush strokes are contours). A stroke keeps its
//! centreline until first erased; from then on its `shape` (outer contour
//! first, then holes) is what is drawn, hit and painted against. Paint
//! zones are cut the same way.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use symphony_core::math::Affine;
use symphony_core::stroke::Stroke;

use crate::cut::Disc;
use crate::tessellate;

/// Contours of one shape: the outer edge first, then any holes.
pub type Shape = Vec<Vec<[f32; 2]>>;

/// Sides of the polygon standing in for a round eraser stamp.
const DISC_SIDES: usize = 24;

fn area(c: &[[f32; 2]]) -> f32 {
    let n = c.len();
    (0..n)
        .map(|i| {
            let (a, b) = (c[i], c[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        * 0.5
}

/// Area of a shape (outer minus holes).
pub fn shape_area(s: &Shape) -> f32 {
    s.iter().map(|c| area(c)).sum::<f32>().abs()
}

fn bbox(contours: &[Vec<[f32; 2]>]) -> Option<([f32; 2], [f32; 2])> {
    let mut it = contours.iter().flatten();
    let f = it.next()?;
    let (mut lo, mut hi) = (*f, *f);
    for p in it {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    Some((lo, hi))
}

fn to_f32(shapes: Vec<Vec<Vec<[f32; 2]>>>) -> Vec<Shape> {
    shapes
        .into_iter()
        .filter(|s| shape_area(s) > 1e-3)
        .collect()
}

/// The outline of a centreline stroke: its triangles merged into one
/// shape (or several, if it crosses itself into separate parts).
pub fn outline(stroke: &Stroke) -> Vec<Shape> {
    if !stroke.shape.is_empty() {
        return vec![stroke.shape.clone()];
    }
    let mesh = tessellate(stroke, Affine::IDENTITY, 0.05);
    let tris: Vec<Vec<[f32; 2]>> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let (a, b, c) = (
                mesh.vertices[t[0] as usize],
                mesh.vertices[t[1] as usize],
                mesh.vertices[t[2] as usize],
            );
            // One winding for all, so non-zero filling unions them.
            if area(&[a, b, c]) < 0.0 {
                vec![a, c, b]
            } else {
                vec![a, b, c]
            }
        })
        .filter(|t| area(t).abs() > 1e-6)
        .collect();
    if tris.is_empty() {
        return Vec::new();
    }
    let empty: Vec<[f32; 2]> = Vec::new();
    to_f32(tris.overlay(&empty, OverlayRule::Subject, FillRule::NonZero))
}

/// The eraser's path as polygons. `discs` are the stamps in path order;
/// each pair of neighbours is joined into a capsule (both round ends and
/// the straight sides between), so a drag leaves a clean edge rather than
/// a row of scallops.
pub fn eraser_shape(discs: &[Disc]) -> Vec<Vec<[f32; 2]>> {
    let ok: Vec<Disc> = discs
        .iter()
        .copied()
        .filter(|(c, r)| c[0].is_finite() && c[1].is_finite() && r.is_finite() && *r > 0.0)
        .collect();
    let circle = |c: [f32; 2], r: f32, from: f32, sweep: f32, n: usize| -> Vec<[f32; 2]> {
        (0..=n)
            .map(|i| {
                let a = from + sweep * i as f32 / n as f32;
                [c[0] + r * a.cos(), c[1] + r * a.sin()]
            })
            .collect()
    };
    match ok.as_slice() {
        [] => Vec::new(),
        [(c, r)] => {
            let mut d = circle(*c, *r, 0.0, std::f32::consts::TAU, DISC_SIDES);
            d.pop();
            vec![d]
        }
        _ => ok
            .windows(2)
            .map(|w| {
                let ((a, ra), (b, rb)) = (w[0], w[1]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                if dx.hypot(dy) < 1e-4 {
                    let mut d = circle(a, ra.max(rb), 0.0, std::f32::consts::TAU, DISC_SIDES);
                    d.pop();
                    return d;
                }
                // Counter-clockwise: the far half of b, then the back half of a.
                let heading = dy.atan2(dx);
                let half = DISC_SIDES / 2;
                let pi = std::f32::consts::PI;
                let mut poly = circle(b, rb, heading - pi / 2.0, pi, half);
                poly.extend(circle(a, ra, heading + pi / 2.0, pi, half));
                poly
            })
            .collect(),
    }
}

/// `shape` minus the eraser. `None` when the eraser misses it; otherwise
/// what's left as separate shapes (empty if it's all gone).
pub fn erase(shape: &Shape, eraser: &[Vec<[f32; 2]>]) -> Option<Vec<Shape>> {
    let (lo, hi) = bbox(shape)?;
    let (elo, ehi) = bbox(eraser)?;
    if elo[0] > hi[0] || elo[1] > hi[1] || ehi[0] < lo[0] || ehi[1] < lo[1] {
        return None;
    }
    let before = shape_area(shape);
    let clip: Vec<Vec<[f32; 2]>> = eraser.to_vec();
    let left = to_f32(shape.overlay(&clip, OverlayRule::Difference, FillRule::NonZero));
    let after: f32 = left.iter().map(shape_area).sum();
    // The eraser crossed the box but not the shape itself.
    if (before - after).abs() <= before * 1e-4 + 1e-4 {
        return None;
    }
    Some(left)
}

/// Is (`x`, `y`) inside the shape, or within `r` of its edge?
pub fn hits_shape(shape: &Shape, x: f32, y: f32, r: f32) -> bool {
    let mut inside = false;
    for c in shape {
        let n = c.len();
        for i in 0..n {
            let (a, b) = (c[i], c[(i + n - 1) % n]);
            if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
        }
    }
    if inside {
        return true;
    }
    shape.iter().any(|c| {
        let n = c.len();
        (0..n).any(|i| {
            let (a, b) = (c[i], c[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 {
                (((x - a[0]) * dx + (y - a[1]) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (a[0] + dx * t - x).hypot(a[1] + dy * t - y) <= r
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphony_core::id::Id;
    use symphony_core::stroke::{Point, StrokeKind};

    fn bar() -> Stroke {
        Stroke {
            id: Id(1),
            kind: StrokeKind::Brush,
            art: Default::default(),
            swatch: Id(0),
            min_width: 10.0,
            max_width: 10.0,
            points: [(0.0, 0.0), (100.0, 0.0)]
                .iter()
                .map(|&(x, y)| Point {
                    x,
                    y,
                    pressure: 1.0,
                })
                .collect(),
            tips: Default::default(),
            texture: None,
            shape: Vec::new(),
        }
    }

    #[test]
    fn outline_matches_the_drawn_stroke() {
        let o = outline(&bar());
        assert_eq!(o.len(), 1);
        // 100 x 10 body plus two round caps of radius 5.
        let want = 100.0 * 10.0 + std::f32::consts::PI * 25.0;
        assert!(
            (shape_area(&o[0]) - want).abs() < want * 0.02,
            "{}",
            shape_area(&o[0])
        );
    }

    #[test]
    fn erasing_the_middle_splits_with_round_bites() {
        let o = outline(&bar()).remove(0);
        let left = erase(&o, &eraser_shape(&[([50.0, 0.0], 8.0)])).unwrap();
        assert_eq!(left.len(), 2, "two pieces");
        // The cut follows the eraser's circle: the edge point nearest the
        // centre is about 8 away, on the curve, not a flat end at x=42.
        for piece in &left {
            let nearest = piece[0]
                .iter()
                .map(|p| (p[0] - 50.0).hypot(p[1]))
                .fold(f32::MAX, f32::min);
            assert!((nearest - 8.0).abs() < 0.3, "{nearest}");
            assert!(!hits_shape(piece, 50.0, 0.0, 0.0));
        }
    }

    #[test]
    fn shaving_an_edge_keeps_one_thinner_piece() {
        let o = outline(&bar()).remove(0);
        let left = erase(&o, &eraser_shape(&[([50.0, 9.0], 6.0)])).unwrap();
        assert_eq!(left.len(), 1, "a bite, not a cut");
        assert!(hits_shape(&left[0], 50.0, -3.0, 0.0));
        assert!(!hits_shape(&left[0], 50.0, 4.0, 0.0));
    }

    #[test]
    fn a_dragged_eraser_leaves_a_straight_clean_edge() {
        // Dragging straight down through the bar: the cut edge is the
        // capsule's straight side at x = 50 - 6, with no scallops.
        let o = outline(&bar()).remove(0);
        let path: Vec<Disc> = (0..=10)
            .map(|i| ([50.0, -20.0 + i as f32 * 4.0], 6.0))
            .collect();
        let left = erase(&o, &eraser_shape(&path)).unwrap();
        assert_eq!(left.len(), 2);
        let edge_x: Vec<f32> = left[0]
            .iter()
            .chain(&left[1])
            .flatten()
            .filter(|p| p[1].abs() <= 5.01 && (p[0] - 50.0).abs() < 10.0)
            .map(|p| (p[0] - 50.0).abs())
            .collect();
        assert!(!edge_x.is_empty());
        assert!(edge_x.iter().all(|d| (d - 6.0).abs() < 0.05), "{edge_x:?}");
    }

    #[test]
    fn misses_and_whole_erases() {
        let o = outline(&bar()).remove(0);
        assert!(erase(&o, &eraser_shape(&[([50.0, 40.0], 5.0)])).is_none());
        // Box overlaps but the shape doesn't.
        assert!(erase(&o, &eraser_shape(&[([-12.0, 12.0], 5.0)])).is_none());
        let all: Vec<Disc> = (0..=20).map(|i| ([i as f32 * 5.0, 0.0], 12.0)).collect();
        assert!(erase(&o, &eraser_shape(&all)).unwrap().is_empty());
    }
}
