//! Turning pictures into vector shapes (vectorizing on import).
//!
//! [`contours`] finds the outlines where a value field crosses a level
//! (marching squares): with the darkness of anti-aliased line art as the
//! field, the outlines fall between pixels exactly where the soft edge is
//! half way, so traced lines are smooth, not staircased. [`shapes`] turns
//! those outlines into clean shapes (outsides with their holes) and
//! simplifies them.

use std::collections::HashMap;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use crate::shape::Shape;

/// Closed outlines where `field` (`w` x `h`, row by row, one value per
/// pixel centre) crosses `level`, in pixel coordinates (pixel `(x, y)`
/// covers `x..x+1`, `y..y+1`). Values at or above `level` are inside.
/// Outsides and holes wind opposite ways; outside the picture counts as
/// below `level`, so every outline closes.
pub fn contours(field: &[f32], w: usize, h: usize, level: f32) -> Vec<Vec<[f32; 2]>> {
    if w == 0 || h == 0 || field.len() < w * h {
        return Vec::new();
    }
    // A border of "outside" around the picture: grid point (gx, gy) is
    // pixel (gx - 1, gy - 1).
    let (gw, gh) = (w + 2, h + 2);
    let at = |gx: usize, gy: usize| -> f32 {
        if gx == 0 || gy == 0 || gx > w || gy > h {
            f32::NEG_INFINITY
        } else {
            field[(gy - 1) * w + (gx - 1)]
        }
    };
    // Edges between grid points: horizontal (gx,gy)-(gx+1,gy) and
    // vertical (gx,gy)-(gx,gy+1).
    let h_edge = |gx: usize, gy: usize| (gy * gw + gx) * 2;
    let v_edge = |gx: usize, gy: usize| (gy * gw + gx) * 2 + 1;
    let point = |a: (usize, usize), b: (usize, usize)| -> [f32; 2] {
        let (va, vb) = (at(a.0, a.1), at(b.0, b.1));
        let t = if va.is_finite() && vb.is_finite() && (vb - va).abs() > 1e-9 {
            ((level - va) / (vb - va)).clamp(0.0, 1.0)
        } else {
            // Against the border: the outline runs along the picture's side.
            0.5
        };
        let x = a.0 as f32 + (b.0 as f32 - a.0 as f32) * t;
        let y = a.1 as f32 + (b.1 as f32 - a.1 as f32) * t;
        // Grid point (1, 1) is pixel (0, 0)'s centre, at (0.5, 0.5).
        [x - 0.5, y - 0.5]
    };
    // Each crossing, by edge, and the segment leaving it.
    let mut next: HashMap<usize, usize> = HashMap::new();
    let mut points: HashMap<usize, [f32; 2]> = HashMap::new();
    for gy in 0..gh - 1 {
        for gx in 0..gw - 1 {
            // Corners clockwise (y down): top-left, top-right,
            // bottom-right, bottom-left.
            let corners = [(gx, gy), (gx + 1, gy), (gx + 1, gy + 1), (gx, gy + 1)];
            let inside = corners.map(|(x, y)| at(x, y) >= level);
            if inside.iter().all(|&i| i) || inside.iter().all(|&i| !i) {
                continue;
            }
            let edges = [
                h_edge(gx, gy),
                v_edge(gx + 1, gy),
                h_edge(gx, gy + 1),
                v_edge(gx, gy),
            ];
            // Walking clockwise, a crossing either leaves the inside
            // (exit) or enters it (entry). Segments run exit -> entry, so
            // the inside is always on the same side and neighbouring cells
            // (which walk a shared edge the other way) chain up.
            let mut crossings: Vec<(usize, bool)> = Vec::with_capacity(4);
            for k in 0..4 {
                let (a, b) = (inside[k], inside[(k + 1) % 4]);
                if a != b {
                    let (pa, pb) = (corners[k], corners[(k + 1) % 4]);
                    points.entry(edges[k]).or_insert_with(|| point(pa, pb));
                    crossings.push((edges[k], a));
                }
            }
            let exit_then_entry = |from: usize| {
                // From the exit at `from`, the next crossing clockwise.
                let e = crossings[from].0;
                let n = crossings[(from + 1) % crossings.len()].0;
                (e, n)
            };
            if crossings.len() == 2 {
                let exit = if crossings[0].1 { 0 } else { 1 };
                let entry = 1 - exit;
                next.insert(crossings[exit].0, crossings[entry].0);
            } else {
                // A saddle: the centre decides whether the two inside
                // corners join.
                let centre = corners.iter().map(|&(x, y)| at(x, y)).sum::<f32>() / 4.0;
                let joined = centre >= level;
                for k in 0..4 {
                    let is_exit = crossings[k].1;
                    if joined && is_exit {
                        // Inside joined: each exit pairs with the next
                        // entry clockwise (cutting off an outside corner).
                        let (e, n) = exit_then_entry(k);
                        next.insert(e, n);
                    } else if !joined && !is_exit {
                        // Separate inside corners: each entry's inside arc
                        // ends at the next exit; that exit joins back to
                        // this entry.
                        let entry = crossings[k].0;
                        let exit = crossings[(k + 1) % 4].0;
                        next.insert(exit, entry);
                    }
                }
            }
        }
    }
    // Follow the chains into closed outlines.
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut starts: Vec<usize> = next.keys().copied().collect();
    starts.sort_unstable();
    for start in starts {
        if seen.contains(&start) {
            continue;
        }
        let mut ring = Vec::new();
        let mut e = start;
        while seen.insert(e) {
            if let Some(p) = points.get(&e) {
                ring.push(*p);
            }
            match next.get(&e) {
                Some(&n) => e = n,
                None => break,
            }
        }
        if ring.len() >= 3 {
            out.push(ring);
        }
    }
    out
}

/// Outlines into shapes (each an outside and its holes), with points
/// closer than `tolerance` to the line dropped and pieces smaller than
/// `min_area` left out. Overlaps and touching outlines merge.
pub fn shapes(outlines: Vec<Vec<[f32; 2]>>, tolerance: f32, min_area: f32) -> Vec<Shape> {
    let simplified: Vec<Vec<[f32; 2]>> = outlines
        .into_iter()
        .map(|r| simplify_ring(&r, tolerance))
        .filter(|r| r.len() >= 3)
        .collect();
    if simplified.is_empty() {
        return Vec::new();
    }
    let empty: Vec<[f32; 2]> = Vec::new();
    simplified
        .overlay(&empty, OverlayRule::Subject, FillRule::EvenOdd)
        .into_iter()
        .filter(|s| crate::shape::shape_area(s).abs() >= min_area)
        .collect()
}

/// Douglas-Peucker on a closed ring.
fn simplify_ring(ring: &[[f32; 2]], tolerance: f32) -> Vec<[f32; 2]> {
    let n = ring.len();
    if n < 4 || tolerance <= 0.0 {
        return ring.to_vec();
    }
    // Split at the point farthest from the first, so both halves are
    // open polylines.
    let d2 = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    let far = (1..n)
        .max_by(|&i, &j| d2(ring[0], ring[i]).total_cmp(&d2(ring[0], ring[j])))
        .unwrap_or(n / 2);
    let mut keep = vec![false; n + 1];
    keep[0] = true;
    keep[far] = true;
    keep[n] = true;
    let at = |i: usize| ring[i % n];
    let mut stack = vec![(0usize, far), (far, n)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (at(a), at(b));
        let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
        let len = dx.hypot(dy).max(1e-9);
        let (mut worst, mut idx) = (0.0f32, a);
        for i in a + 1..b {
            let p = at(i);
            let d = ((p[0] - pa[0]) * dy - (p[1] - pa[1]) * dx).abs() / len;
            if d > worst {
                worst = d;
                idx = i;
            }
        }
        if worst > tolerance {
            keep[idx] = true;
            stack.push((a, idx));
            stack.push((idx, b));
        }
    }
    (0..n).filter(|&i| keep[i]).map(at).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(shapes: &[Shape]) -> f32 {
        shapes
            .iter()
            .map(crate::shape::shape_area)
            .map(f32::abs)
            .sum()
    }

    /// A filled disc traces to one round shape of about its area.
    #[test]
    fn a_disc_traces_to_its_circle() {
        let (w, h) = (64, 64);
        let field: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                // Soft edge, like anti-aliasing.
                (20.0 - (x - 32.0).hypot(y - 32.0) + 0.5).clamp(0.0, 1.0)
            })
            .collect();
        let s = shapes(contours(&field, w, h, 0.5), 0.1, 1.0);
        assert_eq!(s.len(), 1);
        let want = std::f32::consts::PI * 400.0;
        assert!(
            (area(&s) - want).abs() < want * 0.02,
            "{} vs {want}",
            area(&s)
        );
    }

    /// A ring keeps its hole; two blobs stay two shapes; the picture's
    /// edge closes outlines that touch it.
    #[test]
    fn holes_separate_pieces_and_edges() {
        let (w, h) = (40, 20);
        let mut field = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let ring = (2..18).contains(&x)
                    && (2..18).contains(&y)
                    && !((6..14).contains(&x) && (6..14).contains(&y));
                // A block running off the right edge.
                let edge = x >= 30;
                if ring || edge {
                    field[y * w + x] = 1.0;
                }
            }
        }
        let s = shapes(contours(&field, w, h, 0.5), 0.1, 1.0);
        assert_eq!(s.len(), 2, "{s:?}");
        let ring = s
            .iter()
            .find(|sh| sh.len() == 2)
            .expect("the ring has a hole");
        assert!(crate::shape::shape_area(ring).abs() > 150.0);
    }

    /// Diagonal corners touching (a saddle) never leave a broken outline.
    #[test]
    fn checkerboard_saddles_close() {
        let (w, h) = (8, 8);
        let field: Vec<f32> = (0..w * h).map(|i| ((i % w + i / w) % 2) as f32).collect();
        for ring in contours(&field, w, h, 0.5) {
            assert!(ring.len() >= 3);
        }
        assert!(!shapes(contours(&field, w, h, 0.5), 0.0, 0.0).is_empty());
    }
}
