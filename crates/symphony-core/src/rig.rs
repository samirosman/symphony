//! Rest-pose bone bindings and analytic two-bone inverse kinematics.

use serde::{Deserialize, Serialize};

use crate::{anim::Value, math::Affine};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    pub length: f32,
    pub origin: [f32; 2],
    pub rest_angle: f32,
    pub rotation: Value,
    pub bend_radius: f32,
    #[serde(default)]
    pub spring_stiffness: f32,
    pub smart: Option<SmartJoint>,
    /// How the bone may change length.
    #[serde(default)]
    pub stretch_mode: Stretch,
    /// Length as a multiple of the rest length (animatable).
    #[serde(default = "one")]
    pub stretch: Value,
    /// Shortest and longest the bone may get (multiples of its rest
    /// length); None: no limit.
    #[serde(default)]
    pub stretch_limits: Option<[f32; 2]>,
    /// An anchor on the bone's tip (smart rigs).
    /// (Boxed: most bones have none, and nodes stay small.)
    #[serde(default)]
    pub anchor: Option<Box<Anchor>>,
}

fn one() -> Value {
    Value::Constant(1.0)
}

/// How a bone changes length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stretch {
    /// Always its rest length.
    #[default]
    Rigid,
    /// Longer or shorter, the artwork keeping its width.
    Simple,
    /// Longer and thinner, or shorter and fatter (keeps its volume).
    SquashStretch,
}

/// A pinned point of a smart rig.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub kind: AnchorKind,
    /// Where it's pinned, in the skeleton's space.
    pub target: [Value; 2],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnchorKind {
    /// The joint stays on its target.
    #[default]
    Locked,
    /// The target guides inverse kinematics without pinning the joint.
    Aim,
}

impl Default for Bone {
    fn default() -> Self {
        Self {
            length: 160.0,
            origin: [0.0; 2],
            rest_angle: 0.0,
            rotation: Value::default(),
            bend_radius: 28.0,
            spring_stiffness: 0.0,
            smart: None,
            stretch_mode: Stretch::Rigid,
            stretch: one(),
            stretch_limits: None,
            anchor: None,
        }
    }
}

impl Bone {
    /// Soften an intentional rotation relative to the bone's rest angle.
    pub fn spring_rotation(&self, wanted: f32) -> f32 {
        let from_rest = (wanted + 180.0).rem_euclid(360.0) - 180.0;
        wanted - from_rest * self.spring_stiffness.clamp(0.0, 1.0) * 0.9
    }

    pub fn length(&self) -> f32 {
        self.length.clamp(1.0, 100_000.0)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        std::iter::once(&self.rotation)
            .chain(std::iter::once(&self.stretch))
            .chain(self.smart.iter().flat_map(|joint| joint.target.iter()))
            .chain(self.anchor.iter().flat_map(|a| a.target.iter()))
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Value> {
        std::iter::once(&mut self.rotation)
            .chain(std::iter::once(&mut self.stretch))
            .chain(
                self.smart
                    .iter_mut()
                    .flat_map(|joint| joint.target.iter_mut()),
            )
            .chain(self.anchor.iter_mut().flat_map(|a| a.target.iter_mut()))
    }

    /// The stretch range: (shortest, longest), as multiples of the rest
    /// length.
    pub fn stretch_range(&self) -> (f32, f32) {
        match self.stretch_mode {
            Stretch::Rigid => (1.0, 1.0),
            _ => match self.stretch_limits {
                Some([lo, hi]) => (lo.clamp(0.05, 1.0), hi.max(1.0)),
                None => (0.05, 100.0),
            },
        }
    }

    /// How much the bone is stretched on `frame` (1 when it can't be),
    /// within its limits.
    pub fn stretch_at(&self, frame: u32) -> f32 {
        let (lo, hi) = self.stretch_range();
        self.stretch.at(frame).clamp(lo, hi)
    }

    /// The artwork's scale along and across the bone for `stretch`.
    pub fn stretch_scale(&self, stretch: f32) -> [f32; 2] {
        match self.stretch_mode {
            Stretch::Rigid => [1.0, 1.0],
            Stretch::Simple => [stretch, 1.0],
            Stretch::SquashStretch => [stretch, 1.0 / stretch.max(0.05)],
        }
    }

    /// A pulled length as a stretch, with resistance near the limits: the
    /// pull is followed freely in the middle of the range and less and
    /// less as it nears either end, never passing it.
    pub fn resist(&self, wanted: f32) -> f32 {
        let (lo, hi) = self.stretch_range();
        if (hi - lo).abs() < 1e-6 {
            return lo;
        }
        // Free within the inner part of the range; beyond, an easing that
        // only reaches the limit as the pull goes to infinity.
        let soft = |over: f32, room: f32| room * (1.0 - (-over / room.max(1e-6)).exp());
        let (inner_lo, inner_hi) = (lo + (1.0 - lo) * 0.25, 1.0 + (hi - 1.0) * 0.75);
        if wanted > inner_hi {
            inner_hi + soft(wanted - inner_hi, hi - inner_hi)
        } else if wanted < inner_lo {
            inner_lo - soft(inner_lo - wanted, inner_lo - lo)
        } else {
            wanted
        }
    }

    pub fn has_key(&self, frame: u32) -> bool {
        self.values().any(|value| value.has_key(frame))
    }
}

/// Stored on the lower bone. The target is relative to the upper bone's
/// start, in the upper bone's rest axes before either solved rotation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartJoint {
    pub target: [Value; 2],
    pub flip: bool,
}

impl SmartJoint {
    pub fn solve(&self, frame: u32, upper: f32, lower: f32) -> [f32; 2] {
        let [x, y] = self.target.each_ref().map(|value| value.at(frame));
        let reach = x.hypot(y).clamp((upper - lower).abs(), upper + lower);
        let cosine = ((reach * reach - upper * upper - lower * lower) / (2.0 * upper * lower))
            .clamp(-1.0, 1.0);
        let bend = cosine.acos() * if self.flip { -1.0 } else { 1.0 };
        let aim = if x == 0.0 && y == 0.0 {
            0.0
        } else {
            y.atan2(x)
        };
        let correction = (lower * bend.sin()).atan2(upper + lower * bend.cos());
        [(aim - correction).to_degrees(), bend.to_degrees()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonePose {
    pub rest: Affine,
    /// The bone's frame (position and turn; never stretched).
    pub posed: Affine,
    pub parent_skin: Affine,
    /// Rest length.
    pub length: f32,
    /// The artwork's scale along and across the bone (stretch).
    pub scale: [f32; 2],
}

impl BonePose {
    /// The posed length (stretched).
    pub fn reach(self) -> f32 {
        self.length * self.scale[0]
    }

    pub fn tip(self) -> (f32, f32) {
        self.posed.apply(self.reach(), 0.0)
    }

    pub fn skin(self) -> Affine {
        if self.posed == self.rest && self.scale == [1.0, 1.0] {
            Affine::IDENTITY
        } else {
            self.posed
                .compose(Affine::scale(self.scale[0], self.scale[1]))
                .compose(self.rest.inverse().unwrap_or(Affine::IDENTITY))
        }
    }
}

/// One bone of a chain for [`solve_chain`]: its rest length and the
/// range it may stretch over.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub length: f32,
    pub stretch: (f32, f32),
}

/// FABRIK (Aristidou and Lasenby, 2011) over a chain of joints from a
/// fixed base to an end pulled toward `target`: returns the joints'
/// positions (base first). Bones keep their rest lengths where they can,
/// stretching (within their ranges) only when the target is out of reach.
pub fn solve_chain(joints: &[[f32; 2]], links: &[Link], target: [f32; 2]) -> Vec<[f32; 2]> {
    let n = joints.len();
    if n < 2 || links.len() != n - 1 {
        return joints.to_vec();
    }
    let base = joints[0];
    let dist = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    let toward = |from: [f32; 2], to: [f32; 2], len: f32| {
        let d = dist(from, to).max(1e-6);
        [
            from[0] + (to[0] - from[0]) * len / d,
            from[1] + (to[1] - from[1]) * len / d,
        ]
    };
    // Rest lengths, stretched evenly (within each range) if that's what
    // it takes to reach.
    let rest: f32 = links.iter().map(|l| l.length).sum();
    let need = dist(base, target);
    let k = if need > rest {
        need / rest.max(1e-6)
    } else {
        1.0
    };
    let lengths: Vec<f32> = links
        .iter()
        .map(|l| l.length * k.clamp(l.stretch.0, l.stretch.1))
        .collect();
    let reach: f32 = lengths.iter().sum();
    let mut p = joints.to_vec();
    if need >= reach {
        // Out of reach: the chain points straight at it.
        for i in 0..n - 1 {
            p[i + 1] = toward(p[i], target, lengths[i]);
        }
        return p;
    }
    // A perfectly straight chain can't decide which way to bend: give its
    // inner joints the slightest nudge to one side.
    let (dx, dy) = (target[0] - base[0], target[1] - base[1]);
    let span = dx.hypot(dy).max(1e-6);
    let straight = (1..n - 1).all(|i| {
        let (px, py) = (p[i][0] - base[0], p[i][1] - base[1]);
        let (ex, ey) = (p[n - 1][0] - base[0], p[n - 1][1] - base[1]);
        let e = ex.hypot(ey).max(1e-6);
        (px * ey - py * ex).abs() / e < 1e-3 * reach
    });
    if straight {
        for (i, q) in p.iter_mut().enumerate().take(n - 1).skip(1) {
            let nudge = 1e-2 * reach * (i as f32 / n as f32);
            q[0] += -dy / span * nudge;
            q[1] += dx / span * nudge;
        }
    }
    for _ in 0..24 {
        // Backward: end on the target.
        p[n - 1] = target;
        for i in (0..n - 1).rev() {
            p[i] = toward(p[i + 1], p[i], lengths[i]);
        }
        // Forward: base back on its place.
        p[0] = base;
        for i in 0..n - 1 {
            p[i + 1] = toward(p[i], p[i + 1], lengths[i]);
        }
        if dist(p[n - 1], target) < 1e-3 {
            break;
        }
    }
    p
}

/// Result of an anchored edit, including the permitted translation and
/// the accepted target when the edited tip already has an anchor.
#[derive(Default)]
pub struct RigSolution {
    pub changes: Vec<(crate::graph::NodeId, f32, f32)>,
    pub feasible: bool,
    pub translation: [f32; 2],
    pub target: Option<[f32; 2]>,
}

pub(crate) struct RigEdge {
    pub start: usize,
    pub end: usize,
    pub length: f32,
    pub range: [f32; 2],
    pub spring_stiffness: f32,
    pub rest_angle: f32,
}

/// Project the whole tree together. Roots, locked anchors, and the
/// requested drag are hard constraints. Aim targets pull the rig as far
/// as it can go while those constraints still hold.
pub(crate) fn solve_tree(
    rest: &[[f32; 2]],
    edges: &[RigEdge],
    hard: &[(usize, [f32; 2])],
    soft: &[(usize, [f32; 2])],
) -> Option<Vec<[f32; 2]>> {
    let mut pinned = vec![false; rest.len()];
    let mut active = vec![false; rest.len()];
    let mut initial = rest.to_vec();
    for &(i, p) in hard {
        pinned[i] = true;
        active[i] = true;
        initial[i] = p;
    }
    for &(i, _) in soft {
        active[i] = true;
    }
    for edge in edges.iter().rev() {
        active[edge.start] |= active[edge.end];
    }
    let distance = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let lengths_fit = |p: &[[f32; 2]], stretch: bool| {
        edges.iter().filter(|e| active[e.end]).all(|edge| {
            let len = distance(p[edge.start], p[edge.end]);
            let [lo, hi] = if stretch {
                edge.range
            } else {
                [edge.length; 2]
            };
            len >= lo - 0.008 && len <= hi + 0.008
        })
    };
    let mut parents = vec![None; rest.len()];
    for (index, edge) in edges.iter().enumerate() {
        parents[edge.end] = Some(index);
    }
    let springs = edges
        .iter()
        .any(|edge| edge.spring_stiffness > 0.0 && active[edge.end]);
    let relax = |stretch: bool, pull: bool| {
        let mut points = initial.clone();
        if !pull && lengths_fit(&points, stretch) {
            return Some(points);
        }
        // Break a perfectly straight chain's symmetry when a rigid chain
        // is shortened. Preserve the current bend whenever one exists.
        for edge in edges {
            if active[edge.end] && !pinned[edge.end] {
                let [ax, ay] = rest[edge.start];
                let [bx, by] = rest[edge.end];
                points[edge.end][0] -= (by - ay) * 0.001;
                points[edge.end][1] += (bx - ax) * 0.001;
            }
        }
        for step in 0..192 {
            if pull && step < 100 {
                for &(i, goal) in soft {
                    if !pinned[i] {
                        for (value, desired) in points[i].iter_mut().zip(goal) {
                            *value += (desired - *value) * 0.35;
                        }
                    }
                }
            }
            if springs && step < 100 {
                let angle = |points: &[[f32; 2]], edge: &RigEdge| {
                    let (a, b) = (points[edge.start], points[edge.end]);
                    (b[1] - a[1]).atan2(b[0] - a[0])
                };
                for edge in edges
                    .iter()
                    .filter(|edge| active[edge.end] && !pinned[edge.end])
                {
                    let strength = edge.spring_stiffness.clamp(0.0, 1.0);
                    if strength == 0.0 {
                        continue;
                    }
                    let current = angle(&points, edge);
                    let parent = parents[edge.start].map_or(0.0, |i| angle(&points, &edges[i]));
                    let preferred = parent + edge.rest_angle.to_radians();
                    let turn = (preferred - current + std::f32::consts::PI)
                        .rem_euclid(std::f32::consts::TAU)
                        - std::f32::consts::PI;
                    let next = current + turn * strength * 0.2;
                    let base = points[edge.start];
                    let length = distance(base, points[edge.end]);
                    points[edge.end] =
                        [base[0] + next.cos() * length, base[1] + next.sin() * length];
                }
            }
            let mut project = |edge: &RigEdge| {
                let RigEdge { start, end, .. } = *edge;
                if !active[end] || (pinned[start] && pinned[end]) {
                    return;
                }
                let a = points[start];
                let b = points[end];
                let len = distance(a, b);
                let [lo, hi] = if stretch {
                    edge.range
                } else {
                    [edge.length; 2]
                };
                let wanted = len.clamp(lo, hi);
                let (dx, dy) = if len > 0.00001 {
                    ((b[0] - a[0]) / len, (b[1] - a[1]) / len)
                } else {
                    (1.0, 0.0)
                };
                let weight = if pinned[start] || pinned[end] {
                    1.0
                } else {
                    0.5
                };
                let correction = (len - wanted) * weight;
                if !pinned[start] {
                    points[start] = [a[0] + dx * correction, a[1] + dy * correction];
                }
                if !pinned[end] {
                    points[end] = [b[0] - dx * correction, b[1] - dy * correction];
                }
            };
            edges.iter().rev().for_each(&mut project);
            edges.iter().for_each(&mut project);
            if (step >= 100
                || (!springs
                    && (!pull
                        || soft
                            .iter()
                            .all(|&(i, target)| distance(points[i], target) < 0.008))))
                && lengths_fit(&points, stretch)
            {
                return Some(points);
            }
        }
        None
    };
    let mut solved = relax(false, !soft.is_empty())
        .or_else(|| relax(true, !soft.is_empty()))
        .or_else(|| relax(true, false))?;
    // Free branches inherit their parent's change in angle and keep their
    // own local pose, even when the constraint solve moves the parent.
    let mut turns = vec![0.0; rest.len()];
    for edge in edges {
        let d = |p: &[[f32; 2]]| {
            (p[edge.end][1] - p[edge.start][1]).atan2(p[edge.end][0] - p[edge.start][0])
        };
        let old = d(rest);
        if !active[edge.end] {
            let angle = old + turns[edge.start];
            let [x, y] = solved[edge.start];
            solved[edge.end] = [x + edge.length * angle.cos(), y + edge.length * angle.sin()];
        }
        turns[edge.end] = d(&solved) - old;
    }
    Some(solved)
}

/// Two rigid transformations blended around the selected bone's rest joint.
/// Away from this band each side follows its own bone at full weight.
#[derive(Clone, Debug, PartialEq)]
pub struct BoneField {
    pub rest_inverse: Affine,
    pub parent: Affine,
    pub child: Affine,
    pub radius: f32,
}

impl BoneField {
    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let (x, _) = self.rest_inverse.apply(p[0], p[1]);
        let (cx, cy) = self.child.apply(p[0], p[1]);
        if self.radius <= 0.0 || x >= self.radius {
            return [cx, cy];
        }
        let t = (0.5 + x / (2.0 * self.radius)).clamp(0.0, 1.0);
        let w = t * t * (3.0 - 2.0 * t);
        let (px, py) = self.parent.apply(p[0], p[1]);
        [px + (cx - px) * w, py + (cy - py) * w]
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    fn links(n: usize, stretch: (f32, f32)) -> Vec<Link> {
        vec![
            Link {
                length: 10.0,
                stretch
            };
            n
        ]
    }

    #[test]
    fn a_chain_reaches_its_target_keeping_lengths() {
        let joints = [[0.0, 0.0], [10.0, 0.0], [20.0, 0.0], [30.0, 0.0]];
        let p = solve_chain(&joints, &links(3, (1.0, 1.0)), [15.0, 15.0]);
        assert!((p[3][0] - 15.0).abs() < 0.01 && (p[3][1] - 15.0).abs() < 0.01);
        assert_eq!(p[0], [0.0, 0.0], "the base stays");
        for w in p.windows(2) {
            let d = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            assert!((d - 10.0).abs() < 0.01, "{d}");
        }
    }

    #[test]
    fn out_of_reach_points_at_it_or_stretches_to_its_limit() {
        let joints = [[0.0, 0.0], [10.0, 0.0], [20.0, 0.0]];
        let rigid = solve_chain(&joints, &links(2, (1.0, 1.0)), [0.0, 50.0]);
        assert!((rigid[2][1] - 20.0).abs() < 0.01 && rigid[2][0].abs() < 0.01);
        let stretchy = solve_chain(&joints, &links(2, (1.0, 2.0)), [0.0, 50.0]);
        assert!(
            (stretchy[2][1] - 40.0).abs() < 0.01,
            "limited to twice: {:?}",
            stretchy[2]
        );
        let free = solve_chain(&joints, &links(2, (1.0, 100.0)), [0.0, 50.0]);
        assert!((free[2][1] - 50.0).abs() < 0.01);
    }

    #[test]
    fn stretch_resists_near_its_limits() {
        let bone = Bone {
            stretch_mode: Stretch::Simple,
            stretch_limits: Some([0.5, 2.0]),
            ..Bone::default()
        };
        assert_eq!(bone.resist(1.3), 1.3, "free in the middle");
        let near = bone.resist(1.9);
        assert!(near < 1.9 && near > 1.75, "{near}");
        assert!(
            bone.resist(50.0) <= 2.0 && bone.resist(50.0) > 1.99,
            "never past the limit"
        );
        assert!(bone.resist(0.0) > 0.5);
        assert_eq!(
            Bone::default().resist(3.0),
            1.0,
            "rigid bones don't stretch"
        );
        assert_eq!(
            Bone {
                stretch_mode: Stretch::SquashStretch,
                ..Bone::default()
            }
            .stretch_scale(2.0),
            [2.0, 0.5]
        );
    }
}
