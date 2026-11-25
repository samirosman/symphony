//! Curve and Envelope deformers (Harmony 27's Curve and Envelope
//! deformations): a chain of Bezier segments over the artwork. Moving its
//! points and handles bends the art with it, without changing the
//! drawings themselves.
//!
//! One node holds the whole chain. Each point has a rest pose (where it
//! was placed over the art) and an animated pose; the art bends by how
//! the animated chain differs from the rest chain.
//!
//! - **Curve** (open): the art follows the chain along its length, like a
//!   spine. Each art point keeps its place along the chain and its
//!   distance to the side of it. Past the ends, the art follows the end
//!   tangents. The first point is the root: an offset and a turn move the
//!   whole chain.
//! - **Envelope** (closed): the last point joins the first. Every art
//!   point is a blend of the loop's points by mean value coordinates
//!   (Floater, "Mean value coordinates", CAGD 2003), so the loop maps
//!   exactly onto its new shape and the inside follows smoothly. All
//!   points move on their own; there is no root.

use serde::{Deserialize, Serialize};

use crate::anim::Value;
use crate::warp::WarpQuality;

/// A point of the chain at rest: where it is, and its two Bezier handles
/// as an angle (degrees, the node's space) and a length. `in` points
/// back toward the previous point, `out` on toward the next.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointPose {
    pub pos: [f32; 2],
    pub in_angle: f32,
    pub in_length: f32,
    pub out_angle: f32,
    pub out_length: f32,
}

impl PointPose {
    /// A smooth point: the `in` handle opposite the `out` one, as long.
    pub fn smooth(pos: [f32; 2], out: [f32; 2]) -> Self {
        let angle = out[1].atan2(out[0]).to_degrees();
        let length = out[0].hypot(out[1]);
        PointPose {
            pos,
            in_angle: angle + 180.0,
            in_length: length,
            out_angle: angle,
            out_length: length,
        }
    }

    /// The `in` and `out` handles' ends.
    pub fn handles(&self) -> [[f32; 2]; 2] {
        let end = |angle: f32, length: f32| {
            let (s, c) = angle.to_radians().sin_cos();
            [self.pos[0] + c * length, self.pos[1] + s * length]
        };
        [
            end(self.in_angle, self.in_length),
            end(self.out_angle, self.out_length),
        ]
    }
}

/// A point's animated pose: [`PointPose`]'s fields, animatable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointParams {
    pub x: Value,
    pub y: Value,
    pub in_angle: Value,
    pub in_length: Value,
    pub out_angle: Value,
    pub out_length: Value,
}

impl PointParams {
    /// Constant values equal to `pose`.
    pub fn constant(pose: PointPose) -> Self {
        PointParams {
            x: Value::Constant(pose.pos[0]),
            y: Value::Constant(pose.pos[1]),
            in_angle: Value::Constant(pose.in_angle),
            in_length: Value::Constant(pose.in_length),
            out_angle: Value::Constant(pose.out_angle),
            out_length: Value::Constant(pose.out_length),
        }
    }

    pub fn at(&self, frame: u32) -> PointPose {
        PointPose {
            pos: [self.x.at(frame), self.y.at(frame)],
            in_angle: self.in_angle.at(frame),
            in_length: self.in_length.at(frame),
            out_angle: self.out_angle.at(frame),
            out_length: self.out_length.at(frame),
        }
    }

    pub fn values(&self) -> [&Value; 6] {
        [
            &self.x,
            &self.y,
            &self.in_angle,
            &self.in_length,
            &self.out_angle,
            &self.out_length,
        ]
    }

    pub fn values_mut(&mut self) -> [&mut Value; 6] {
        [
            &mut self.x,
            &mut self.y,
            &mut self.in_angle,
            &mut self.in_length,
            &mut self.out_angle,
            &mut self.out_length,
        ]
    }
}

/// One point of the chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub rest: PointPose,
    pub anim: PointParams,
}

impl CurvePoint {
    /// A point whose animated pose starts at its rest pose.
    pub fn new(rest: PointPose) -> Self {
        CurvePoint {
            rest,
            anim: PointParams::constant(rest),
        }
    }
}

/// The root of an open Curve (Harmony's Offset node): moves the whole
/// chain by (x, y) and turns it by `angle` degrees about the first
/// point's rest place. All zero at rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Root {
    pub x: Value,
    pub y: Value,
    pub angle: Value,
}

impl Default for Root {
    fn default() -> Self {
        Root {
            x: Value::Constant(0.0),
            y: Value::Constant(0.0),
            angle: Value::Constant(0.0),
        }
    }
}

/// A Curve or Envelope node's parameters.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CurveDeformer {
    /// An Envelope: the last point joins the first.
    pub closed: bool,
    pub points: Vec<CurvePoint>,
    /// Open curves only.
    #[serde(default)]
    pub root: Root,
    #[serde(default)]
    pub quality: WarpQuality,
}

/// Most points a chain can have.
pub const MAX_POINTS: usize = 64;

impl CurveDeformer {
    /// An empty Curve (`closed` false) or Envelope (true).
    pub fn new(closed: bool) -> Self {
        CurveDeformer {
            closed,
            ..CurveDeformer::default()
        }
    }

    /// Every animatable value.
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        [&self.root.x, &self.root.y, &self.root.angle]
            .into_iter()
            .chain(self.points.iter().flat_map(|p| p.anim.values()))
    }

    /// [`CurveDeformer::values`], to change.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Value> {
        [&mut self.root.x, &mut self.root.y, &mut self.root.angle]
            .into_iter()
            .chain(self.points.iter_mut().flat_map(|p| p.anim.values_mut()))
    }

    pub fn has_key(&self, frame: u32) -> bool {
        self.values().any(|v| v.has_key(frame))
    }

    /// Enough points to bend anything (two for a Curve, three for an
    /// Envelope)?
    pub fn is_complete(&self) -> bool {
        self.points.len() >= if self.closed { 3 } else { 2 }
    }

    /// The rest chain's segments.
    pub fn rest_segments(&self) -> Vec<Segment> {
        segments(
            &self.points.iter().map(|p| p.rest).collect::<Vec<_>>(),
            self.closed,
        )
    }

    /// The animated points on `frame`, the root's move and turn applied.
    pub fn poses_at(&self, frame: u32) -> Vec<PointPose> {
        let mut poses: Vec<PointPose> = self.points.iter().map(|p| p.anim.at(frame)).collect();
        if self.closed {
            return poses;
        }
        let (dx, dy, turn) = (
            self.root.x.at(frame),
            self.root.y.at(frame),
            self.root.angle.at(frame),
        );
        if (dx, dy, turn) == (0.0, 0.0, 0.0) {
            return poses;
        }
        let pivot = self.points.first().map_or([0.0, 0.0], |p| p.rest.pos);
        let (s, c) = turn.to_radians().sin_cos();
        for p in &mut poses {
            let [x, y] = [p.pos[0] - pivot[0], p.pos[1] - pivot[1]];
            p.pos = [pivot[0] + x * c - y * s + dx, pivot[1] + x * s + y * c + dy];
            p.in_angle += turn;
            p.out_angle += turn;
        }
        poses
    }

    /// What bends the art on `frame` (None: too few points yet).
    pub fn field_at(&self, frame: u32) -> Option<CurveField> {
        if !self.is_complete() {
            return None;
        }
        let now = segments(&self.poses_at(frame), self.closed);
        Some(CurveField::new(
            self.closed,
            &self.rest_segments(),
            &now,
            2 * self.quality.pieces() as usize,
        ))
    }
}

/// A cubic Bezier segment: start, two handles, end.
pub type Segment = [[f32; 2]; 4];

/// The chain's segments from its points (the last joins the first when
/// `closed`).
pub fn segments(points: &[PointPose], closed: bool) -> Vec<Segment> {
    let n = points.len();
    let count = match (closed, n) {
        (_, 0 | 1) => 0,
        (true, _) => n,
        (false, _) => n - 1,
    };
    (0..count)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            [a.pos, a.handles()[1], b.handles()[0], b.pos]
        })
        .collect()
}

/// A point on a Bezier segment at `t`.
pub fn bezier(s: &Segment, t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    [
        s.iter().zip(w).map(|(p, w)| p[0] * w).sum(),
        s.iter().zip(w).map(|(p, w)| p[1] * w).sum(),
    ]
}

/// Sample segments `per` times each (the end of a closed chain is not
/// repeated).
fn sample(segs: &[Segment], per: usize, closed: bool) -> Vec<[f32; 2]> {
    let per = per.max(1);
    let mut out: Vec<[f32; 2]> = segs
        .iter()
        .flat_map(|s| (0..per).map(move |k| bezier(s, k as f32 / per as f32)))
        .collect();
    if !closed {
        if let Some(last) = segs.last() {
            out.push(last[3]);
        }
    }
    out
}

/// One Curve or Envelope on one frame: its rest and animated chains,
/// sampled at the same parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveField {
    pub closed: bool,
    rest: Vec<[f32; 2]>,
    now: Vec<[f32; 2]>,
}

impl CurveField {
    pub fn new(closed: bool, rest: &[Segment], now: &[Segment], per: usize) -> Self {
        CurveField {
            closed,
            rest: sample(rest, per, closed),
            now: sample(now, per, closed),
        }
    }

    /// The animated chain, sampled (for display).
    pub fn now(&self) -> &[[f32; 2]] {
        &self.now
    }

    pub fn is_flat(&self) -> bool {
        self.rest == self.now
    }

    /// The longest straight piece of art that still bends smoothly.
    pub fn piece(&self) -> f32 {
        let n = self.rest.len();
        let edges = if self.closed { n } else { n.saturating_sub(1) };
        (0..edges)
            .map(|i| {
                let (a, b) = (self.rest[i], self.rest[(i + 1) % n]);
                (b[0] - a[0]).hypot(b[1] - a[1])
            })
            .fold(f32::MAX, f32::min)
            .max(0.5)
    }

    pub fn hash_into(&self, h: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        self.closed.hash(h);
        for v in self.rest.iter().chain(&self.now).flatten() {
            v.to_bits().hash(h);
        }
    }

    /// Bend a point (in the node's space).
    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        if self.closed {
            mean_value(&self.rest, &self.now, p)
        } else {
            spine(&self.rest, &self.now, p)
        }
    }
}

/// The open chain's map: find where `p` sits along the rest polyline
/// (which piece, how far along it, how far to its side) and put it at the
/// same place along the animated one.
fn spine(rest: &[[f32; 2]], now: &[[f32; 2]], p: [f32; 2]) -> [f32; 2] {
    let n = rest.len();
    if n < 2 {
        return p;
    }
    // The nearest piece; past the ends, the end pieces carry on straight.
    let mut best = (f32::MAX, 0usize, 0.0f32);
    for i in 0..n - 1 {
        let (a, b) = (rest[i], rest[i + 1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-12);
        let t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2;
        let lo = if i == 0 { f32::MIN } else { 0.0 };
        let hi = if i == n - 2 { f32::MAX } else { 1.0 };
        let t = t.clamp(lo, hi);
        let q = [a[0] + d[0] * t, a[1] + d[1] * t];
        let dist = (p[0] - q[0]).hypot(p[1] - q[1]);
        if dist < best.0 {
            best = (dist, i, t);
        }
    }
    let (_, i, t) = best;
    let frame = |pts: &[[f32; 2]]| {
        let (a, b) = (pts[i], pts[i + 1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let len = d[0].hypot(d[1]).max(1e-6);
        let along = [a[0] + d[0] * t, a[1] + d[1] * t];
        // Normals blended between the piece's ends, so the side of the
        // chain bends smoothly rather than in steps.
        let normal_at = |j: usize| {
            let (a, b) = (pts[j.saturating_sub(1)], pts[(j + 1).min(pts.len() - 1)]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let len = d[0].hypot(d[1]).max(1e-6);
            [-d[1] / len, d[0] / len]
        };
        let (na, nb) = (normal_at(i), normal_at(i + 1));
        let k = t.clamp(0.0, 1.0);
        let nm = [na[0] + (nb[0] - na[0]) * k, na[1] + (nb[1] - na[1]) * k];
        let nl = nm[0].hypot(nm[1]).max(1e-6);
        (along, [nm[0] / nl, nm[1] / nl], [d[0] / len, d[1] / len])
    };
    let (q, n0, _) = frame(rest);
    let (q1, n1, _) = frame(now);
    let side = (p[0] - q[0]) * n0[0] + (p[1] - q[1]) * n0[1];
    [q1[0] + n1[0] * side, q1[1] + n1[1] * side]
}

/// The closed chain's map: mean value coordinates of `p` against the rest
/// polygon, applied to the animated one.
fn mean_value(rest: &[[f32; 2]], now: &[[f32; 2]], p: [f32; 2]) -> [f32; 2] {
    let n = rest.len();
    if n < 3 {
        return p;
    }
    let s: Vec<[f32; 2]> = rest.iter().map(|v| [v[0] - p[0], v[1] - p[1]]).collect();
    let r: Vec<f32> = s.iter().map(|v| v[0].hypot(v[1])).collect();
    for i in 0..n {
        if r[i] < 1e-6 {
            return now[i];
        }
    }
    // tan(angle / 2) of each edge as seen from p.
    let mut half = vec![0.0f32; n];
    for i in 0..n {
        let j = (i + 1) % n;
        let cross = s[i][0] * s[j][1] - s[i][1] * s[j][0];
        let dot = s[i][0] * s[j][0] + s[i][1] * s[j][1];
        if cross.abs() < 1e-9 * r[i] * r[j] && dot < 0.0 {
            // On the edge: blend its two ends.
            let t = r[i] / (r[i] + r[j]);
            return [
                now[i][0] + (now[j][0] - now[i][0]) * t,
                now[i][1] + (now[j][1] - now[i][1]) * t,
            ];
        }
        half[i] = cross / (r[i] * r[j] + dot);
    }
    let (mut sum, mut q) = (0.0f32, [0.0f32; 2]);
    for i in 0..n {
        let w = (half[(i + n - 1) % n] + half[i]) / r[i];
        sum += w;
        q[0] += w * now[i][0];
        q[1] += w * now[i][1];
    }
    if sum.abs() < 1e-12 || !sum.is_finite() {
        return p;
    }
    [q[0] / sum, q[1] / sum]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 2], b: [f32; 2], tol: f32) -> bool {
        (a[0] - b[0]).abs() < tol && (a[1] - b[1]).abs() < tol
    }

    /// A straight open curve along the x axis, 0 to 300.
    fn line() -> CurveDeformer {
        let mut c = CurveDeformer::new(false);
        for x in [0.0, 100.0, 200.0, 300.0] {
            c.points
                .push(CurvePoint::new(PointPose::smooth([x, 0.0], [33.0, 0.0])));
        }
        c
    }

    /// A square-ish envelope around (0, 0), 200 wide.
    fn loop4() -> CurveDeformer {
        let mut c = CurveDeformer::new(true);
        for (pos, out) in [
            ([-100.0, -100.0], [66.0, 0.0]),
            ([100.0, -100.0], [0.0, 66.0]),
            ([100.0, 100.0], [-66.0, 0.0]),
            ([-100.0, 100.0], [0.0, -66.0]),
        ] {
            c.points.push(CurvePoint::new(PointPose::smooth(pos, out)));
        }
        c
    }

    #[test]
    fn at_rest_nothing_moves() {
        for c in [line(), loop4()] {
            let f = c.field_at(1).unwrap();
            assert!(f.is_flat());
            for p in [[0.0, 0.0], [50.0, 20.0], [-30.0, 70.0], [400.0, -10.0]] {
                assert!(close(f.apply(p), p, 1e-3), "{p:?} -> {:?}", f.apply(p));
            }
        }
    }

    #[test]
    fn too_few_points_bend_nothing() {
        let mut c = CurveDeformer::new(true);
        c.points
            .push(CurvePoint::new(PointPose::smooth([0.0, 0.0], [1.0, 0.0])));
        c.points
            .push(CurvePoint::new(PointPose::smooth([9.0, 0.0], [1.0, 0.0])));
        assert!(c.field_at(1).is_none());
        c.closed = false;
        assert!(c.field_at(1).is_some());
    }

    #[test]
    fn the_root_moves_and_turns_the_whole_curve() {
        let mut c = line();
        c.root.x = Value::Constant(10.0);
        c.root.angle = Value::Constant(90.0);
        let f = c.field_at(1).unwrap();
        // The end of the line, 300 along x, turns to 300 along y.
        assert!(close(f.apply([300.0, 0.0]), [10.0, 300.0], 1e-2));
        // Art beside the line keeps its side: 20 above (y + 20 is to
        // the line's left) stays 20 to its left after the turn.
        let q = f.apply([100.0, 20.0]);
        assert!(close(q, [-10.0, 100.0], 1e-2), "{q:?}");
    }

    #[test]
    fn a_curve_carries_the_art_beside_it() {
        let mut c = line();
        // Lift the last two points by 50.
        for p in &mut c.points[2..] {
            p.anim.y = Value::Constant(50.0);
        }
        let f = c.field_at(1).unwrap();
        assert!(!f.is_flat());
        // On the moved part, art moves with the curve.
        assert!(close(f.apply([300.0, 0.0]), [300.0, 50.0], 1e-2));
        assert!(
            close(f.apply([0.0, 10.0]), [0.0, 10.0], 1e-2),
            "the root end stays"
        );
        // Past the end, art follows the end tangent.
        assert!(close(f.apply([340.0, 0.0]), [340.0, 50.0], 1e-1));
    }

    #[test]
    fn an_envelope_maps_its_loop_onto_the_new_loop() {
        let mut c = loop4();
        // Pull the top right corner out.
        c.points[1].anim.x = Value::Constant(160.0);
        c.points[1].anim.y = Value::Constant(-160.0);
        let f = c.field_at(1).unwrap();
        assert!(close(f.apply([100.0, -100.0]), [160.0, -160.0], 1e-2));
        // The far corner stays; the middle moves toward the pull a little.
        assert!(close(f.apply([-100.0, 100.0]), [-100.0, 100.0], 1e-2));
        let mid = f.apply([0.0, 0.0]);
        assert!(mid[0] > 0.0 && mid[1] < 0.0 && mid[0] < 40.0, "{mid:?}");
        // Inside points near the pulled corner move more than far ones.
        let near = f.apply([80.0, -80.0]);
        assert!(near[0] - 80.0 > mid[0], "{near:?} {mid:?}");
    }

    #[test]
    fn an_envelope_moved_whole_moves_the_art_whole() {
        let mut c = loop4();
        for p in &mut c.points {
            p.anim.x = Value::Constant(p.rest.pos[0] + 25.0);
        }
        let f = c.field_at(1).unwrap();
        for p in [[0.0, 0.0], [70.0, -30.0], [-90.0, 95.0]] {
            assert!(close(f.apply(p), [p[0] + 25.0, p[1]], 1e-2));
        }
    }

    #[test]
    fn keys_are_found_and_files_round_trip() {
        let mut c = loop4();
        c.points[2].anim.out_angle.set_key(9, 45.0);
        assert!(c.has_key(9) && !c.has_key(8));
        let json = serde_json::to_string(&c).unwrap();
        let back: CurveDeformer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }
}
