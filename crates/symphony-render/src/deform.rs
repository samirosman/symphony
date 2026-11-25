//! Bone deformation of vector art: a drawing bent by a bone chain, ready
//! to be painted like any other drawing.
//!
//! [`deform_drawing`] returns a copy of a drawing in which every stroke's
//! centre line, every erased stroke's outline and every paint zone has
//! been moved by [`ChainPose::map`]. Strokes and paint use the same map,
//! so the paint stays inside the lines. Line widths stay as drawn: a
//! longer bone does not make its lines thicker.
//!
//! The map bends space smoothly near each joint, so straight segments
//! there must become curves. Each segment is split until its mapped
//! middle is within [`TOLERANCE`] of the straight line between its mapped
//! ends, and until it is shorter than a quarter of the smallest joint
//! radius before and after the map. Segments away from the joints move
//! rigidly and stay as they are.

use symphony_core::deform::ChainPose;
use symphony_core::stroke::{Drawing, Fill, Point, Stroke};
use symphony_vector::curve;

/// How far a mapped segment may stray from a straight line, in drawing
/// units (a twentieth of a pixel at 100%).
pub const TOLERANCE: f32 = 0.05;
/// Longest segment near a joint, as a share of the smallest joint radius.
const STEP_SHARE: f32 = 0.25;
/// At most 2^12 pieces from one segment, so a tear (a hard hinge) or a
/// wild pose costs a bounded amount of work.
const MAX_DEPTH: u32 = 12;

/// The drawing `d`, bent by `pose`.
pub fn deform_drawing(d: &Drawing, pose: &ChainPose) -> Drawing {
    if pose.moves_nothing() {
        return d.clone();
    }
    let step = step_for(pose);
    Drawing {
        id: d.id,
        column: d.column,
        name: d.name.clone(),
        // The image stays as imported; the renderer bends its pixels.
        image: d.image,
        strokes: d
            .strokes
            .iter()
            .map(|s| deform_stroke(s, pose, step))
            .collect(),
        fills: d
            .fills
            .iter()
            .map(|f| Fill {
                id: f.id,
                swatch: f.swatch,
                art: f.art,
                linked: f.linked.clone(),
                contours: f
                    .contours
                    .iter()
                    .map(|c| deform_ring(c, pose, step))
                    .collect(),
            })
            .collect(),
        // Only the shapes are drawn; the text source rides along as it is.
        text: d.text.clone(),
    }
}

/// The longest segment allowed near a joint.
fn step_for(pose: &ChainPose) -> f32 {
    pose.smallest_radius()
        .map_or(f32::INFINITY, |r| r * STEP_SHARE)
}

fn deform_stroke(st: &Stroke, pose: &ChainPose, step: f32) -> Stroke {
    Stroke {
        id: st.id,
        kind: st.kind,
        art: st.art,
        swatch: st.swatch,
        min_width: st.min_width,
        max_width: st.max_width,
        points: deform_line(&st.points, pose, step),
        tips: st.tips,
        texture: st.texture,
        shape: st
            .shape
            .iter()
            .map(|c| deform_ring(c, pose, step))
            .collect(),
    }
}

/// A stroke's centre line. The drawn curve is followed first (the map is
/// not affine, so bending the stored points and then smoothing them would
/// give a different curve), then each piece is split and mapped.
fn deform_line(points: &[Point], pose: &ChainPose, step: f32) -> Vec<Point> {
    let curve = if step.is_finite() {
        curve::spline(points, step)
    } else {
        points.to_vec()
    };
    let Some(first) = curve.first() else {
        return Vec::new();
    };
    let at = |p: &Point| pose.map([p.x, p.y]);
    let mut out = Vec::with_capacity(curve.len() * 2);
    let a0 = at(first);
    out.push(Point {
        x: a0[0],
        y: a0[1],
        pressure: first.pressure,
        pen: first.pen,
    });
    for w in curve.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = Seg {
            a: [a.x, a.y],
            b: [b.x, b.y],
            ma: pose.map([a.x, a.y]),
            mb: pose.map([b.x, b.y]),
        };
        refine(pose, step, seg, MAX_DEPTH, &mut |t, m| {
            out.push(Point {
                x: m[0],
                y: m[1],
                pressure: a.pressure + (b.pressure - a.pressure) * t,
                pen: if t < 0.5 { a.pen } else { b.pen },
            })
        });
    }
    out
}

/// A closed outline (paint zone or erased stroke). The segment back to
/// the first point is split too; the first point is not repeated.
fn deform_ring(ring: &[[f32; 2]], pose: &ChainPose, step: f32) -> Vec<[f32; 2]> {
    let n = ring.len();
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (ma, mb) = (pose.map(a), pose.map(b));
        out.push(ma);
        if n < 2 {
            break;
        }
        let seg = Seg { a, b, ma, mb };
        refine(pose, step, seg, MAX_DEPTH, &mut |t, m| {
            // The segment's end is the next point, pushed on its own turn.
            if t < 1.0 {
                out.push(m);
            }
        });
    }
    out
}

/// A segment in the rest pose and its two mapped ends.
#[derive(Clone, Copy)]
struct Seg {
    a: [f32; 2],
    b: [f32; 2],
    ma: [f32; 2],
    mb: [f32; 2],
}

/// Emit the mapped points after `seg.a` up to and including `seg.b`, each
/// with its parameter along the segment (0 at `a`, 1 at `b`).
fn refine(pose: &ChainPose, step: f32, seg: Seg, depth: u32, emit: &mut impl FnMut(f32, [f32; 2])) {
    refine_span(pose, step, seg, 0.0, 1.0, depth, emit);
}

fn refine_span(
    pose: &ChainPose,
    step: f32,
    s: Seg,
    t0: f32,
    t1: f32,
    depth: u32,
    emit: &mut impl FnMut(f32, [f32; 2]),
) {
    let mid = [(s.a[0] + s.b[0]) * 0.5, (s.a[1] + s.b[1]) * 0.5];
    let mm = pose.map(mid);
    let chord = [(s.ma[0] + s.mb[0]) * 0.5, (s.ma[1] + s.mb[1]) * 0.5];
    let off = (mm[0] - chord[0]).hypot(mm[1] - chord[1]);
    let long = dist(s.a, s.b).max(dist(s.ma, s.mb)) > step;
    if depth == 0 || !(off > TOLERANCE || long) {
        emit(t1, s.mb);
        return;
    }
    let tm = (t0 + t1) * 0.5;
    let left = Seg {
        a: s.a,
        b: mid,
        ma: s.ma,
        mb: mm,
    };
    let right = Seg {
        a: mid,
        b: s.b,
        ma: mm,
        mb: s.mb,
    };
    refine_span(pose, step, left, t0, tm, depth - 1, emit);
    refine_span(pose, step, right, tm, t1, depth - 1, emit);
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphony_core::deform::{BonePose, Influence};
    use symphony_core::id::Id;
    use symphony_core::stroke::{StrokeKind, Tips};

    const L: f32 = 100.0;

    /// Two bones along +x from (-L, 0), the joint at the origin.
    fn arm(radius: f32) -> Vec<BonePose> {
        let mut root = BonePose::new(0.0, L, L);
        root.x = -L;
        let mut child = BonePose::new(0.0, L, L);
        child.radius = radius;
        vec![root, child]
    }

    fn bent(radius: f32, deg: f32) -> ChainPose {
        let rest = arm(radius);
        let mut anim = rest.clone();
        anim[1].orientation = deg;
        ChainPose::new(&rest, &anim, Influence::Infinite)
    }

    fn line(points: &[[f32; 2]]) -> Stroke {
        Stroke {
            id: Id(1),
            kind: StrokeKind::Brush,
            art: Default::default(),
            swatch: Id(1),
            min_width: 4.0,
            max_width: 4.0,
            points: points
                .iter()
                .map(|p| Point {
                    x: p[0],
                    y: p[1],
                    pressure: 1.0,
                    ..Point::default()
                })
                .collect(),
            tips: Tips::default(),
            texture: None,
            shape: Vec::new(),
        }
    }

    fn drawing(strokes: Vec<Stroke>, fills: Vec<Fill>) -> Drawing {
        Drawing {
            id: Id(10),
            column: Id(11),
            name: "arm".into(),
            image: None,
            strokes,
            fills,
            text: Vec::new(),
        }
    }

    fn max_gap(points: &[Point]) -> f32 {
        points
            .windows(2)
            .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
            .fold(0.0, f32::max)
    }

    #[test]
    fn zero_influence_leaves_the_drawing_alone() {
        let rest = arm(30.0);
        let mut anim = rest.clone();
        anim[1].orientation = 90.0;
        let pose = ChainPose::new(&rest, &anim, Influence::Zero);
        let d = drawing(vec![line(&[[-L, 0.0], [L, 0.0]])], Vec::new());
        assert_eq!(deform_drawing(&d, &pose), d);
    }

    #[test]
    fn the_rest_pose_keeps_every_point() {
        let pose = bent(30.0, 0.0);
        let d = drawing(
            vec![line(&[[-L, 10.0], [0.0, -5.0], [L, 10.0]])],
            Vec::new(),
        );
        let out = deform_drawing(&d, &pose);
        // Unmoved art is flat everywhere, so only the length limit splits.
        let pts = &out.strokes[0].points;
        assert!(pts.len() > 3);
        assert!(max_gap(pts) <= step_for(&pose) + 1e-3);
        let (a, z) = (pts[0], pts[pts.len() - 1]);
        assert!((a.x + L).abs() < 1e-4 && (a.y - 10.0).abs() < 1e-4);
        assert!((z.x - L).abs() < 1e-4 && (z.y - 10.0).abs() < 1e-4);
    }

    #[test]
    fn a_line_inside_the_arm_bends_without_a_tear() {
        // A straight line parallel to the bones, off the axis: on the axis
        // even a hard hinge is continuous, so it would prove nothing.
        let st = line(&[[-L, 15.0], [L, 15.0]]);
        let blend = bent(30.0, 90.0);
        let step = step_for(&blend);
        let pts = deform_line(&st.points, &blend, step);
        assert!(
            max_gap(&pts) <= step + 1e-3,
            "blended fold: gap {}",
            max_gap(&pts)
        );
        // The same test on a hard hinge finds the tear.
        let hinge = bent(0.0, 90.0);
        let torn = deform_line(&st.points, &hinge, step);
        assert!(max_gap(&torn) > 2.0 * step, "hinge gap {}", max_gap(&torn));
        // The line ends where the child bone carries it: turned 90 degrees
        // about the joint, (L, 15) goes to (-15, L).
        let z = pts[pts.len() - 1];
        assert!((z.x + 15.0).abs() < 1e-3 && (z.y - L).abs() < 1e-3, "{z:?}");
    }

    #[test]
    fn curves_stay_within_the_tolerance_of_the_map() {
        let pose = bent(30.0, 120.0);
        let st = line(&[[-L, 15.0], [L, 15.0]]);
        let pts = deform_line(&st.points, &pose, step_for(&pose));
        // Every point of the true bent line is close to the polyline.
        let mut worst = 0.0f32;
        for x in (-100..100).map(|i| i as f32) {
            let m = pose.map([x, 15.0]);
            let d = pts
                .windows(2)
                .map(|w| seg_dist(m, [w[0].x, w[0].y], [w[1].x, w[1].y]))
                .fold(f32::INFINITY, f32::min);
            worst = worst.max(d);
        }
        assert!(worst < 4.0 * TOLERANCE, "worst distance {worst}");
    }

    fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
        let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = vx * vx + vy * vy;
        let t = if l2 > 0.0 {
            (((p[0] - a[0]) * vx + (p[1] - a[1]) * vy) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (p[0] - a[0] - t * vx).hypot(p[1] - a[1] - t * vy)
    }

    #[test]
    fn paint_follows_the_same_map_as_the_lines() {
        // A gentle bend: past about 25 degrees the inner side of this arm
        // folds over itself, and folded paint has no inside.
        let pose = bent(30.0, 20.0);
        let zone = vec![[-L, -20.0], [L, -20.0], [L, 20.0], [-L, 20.0]];
        let fill = Fill {
            id: Id(2),
            swatch: Id(3),
            art: Default::default(),
            linked: None,
            contours: vec![zone],
        };
        let out = deform_drawing(&drawing(Vec::new(), vec![fill.clone()]), &pose);
        let bent_fill = &out.fills[0];
        assert!(bent_fill.contours[0].len() > 4);
        // Points inside the zone stay inside it, points outside stay out,
        // including near the fold.
        for x in [-90.0, -30.0, -10.0, 0.0, 10.0, 30.0, 90.0] {
            for y in [-15.0, 0.0, 15.0] {
                let m = pose.map([x, y]);
                assert!(bent_fill.contains(m[0], m[1]), "({x}, {y}) left the paint");
            }
            for y in [-25.0, 25.0] {
                let m = pose.map([x, y]);
                assert!(
                    !bent_fill.contains(m[0], m[1]),
                    "({x}, {y}) entered the paint"
                );
            }
        }
    }

    #[test]
    fn erased_strokes_bend_their_outline() {
        let pose = bent(30.0, 90.0);
        let mut st = line(&[[-L, 0.0], [L, 0.0]]);
        st.shape = vec![vec![[-L, -2.0], [L, -2.0], [L, 2.0], [-L, 2.0]]];
        let out = deform_stroke(&st, &pose, step_for(&pose));
        let ring = &out.shape[0];
        assert!(ring.len() > 4);
        // (L, 2) is carried by the child bone to (-2, L).
        assert!(ring
            .iter()
            .any(|p| (p[0] + 2.0).abs() < 1e-3 && (p[1] - L).abs() < 1e-3));
    }

    #[test]
    fn line_widths_stay_as_drawn() {
        let rest = arm(30.0);
        let mut anim = rest.clone();
        anim[1].length = 2.0 * L;
        let pose = ChainPose::new(&rest, &anim, Influence::Infinite);
        let mut st = line(&[[-L, 0.0], [L, 0.0]]);
        st.min_width = 3.0;
        st.max_width = 7.0;
        let out = deform_stroke(&st, &pose, step_for(&pose));
        assert_eq!((out.min_width, out.max_width), (3.0, 7.0));
        assert!(out.points.iter().all(|p| p.pressure == 1.0));
        // The stretched bone carries the end twice as far.
        let z = out.points[out.points.len() - 1];
        assert!((z.x - 2.0 * L).abs() < 1e-3, "{z:?}");
    }

    #[test]
    fn rigid_art_is_not_split() {
        // One bone: the map is a rotation, so straight lines stay straight.
        let mut root = BonePose::new(0.0, L, L);
        root.x = -L;
        let mut turned = root;
        turned.orientation = 40.0;
        let pose = ChainPose::new(&[root], &[turned], Influence::Infinite);
        let st = line(&[[-L, 0.0], [L, 5.0]]);
        let out = deform_stroke(&st, &pose, step_for(&pose));
        assert_eq!(out.points.len(), 2);
    }
}
