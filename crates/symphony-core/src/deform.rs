//! Bone deformation: a chain of solid bones whose joints bend the art
//! around them, like Harmony's Bone deformer.
//!
//! A [`BoneChain`] keeps a rest pose and animated parameters for each
//! bone. [`BoneChain::pose_at`] solves both poses for one frame into a
//! [`ChainPose`], which moves points of the drawing from the rest pose to
//! the animated pose ([`ChainPose::map`]). Points are in the drawing's own
//! space, before any peg: the pegs sit above the chain.
//!
//! How a point moves (our design; the published behaviour gives the
//! parameters, not the formula):
//!
//! - The bisector at each joint splits the plane between the two bones.
//!   A point belongs to the bone on its side and moves with that bone. A
//!   longer or shorter bone stretches the art along the bone, not across.
//! - Within the joint's radius of the bisector, the point is blended
//!   between the two bones. The blend is in polar form around the joint,
//!   so every point keeps its distance to the joint and the fold stays
//!   round instead of thinning.
//! - The weights follow an envelope falloff: both bones weigh 0.5 near the
//!   joint (the core), then the far bone fades out to the edge of the
//!   zone. Bias sets the core: a small bias gives a smooth fold, a large
//!   bias a sharp one. The default radius and core match the weights of
//!   Blender's Grease Pencil rigging, measured from its output.

use serde::{Deserialize, Serialize};

use crate::anim::Value;
use crate::math::Affine;

/// Default joint radius as a share of the shorter bone at the joint.
pub const DEFAULT_RADIUS_SHARE: f32 = 0.3;
/// Default bias: the core is 0.125 of a bone length in a 0.3 zone.
pub const DEFAULT_BIAS: f32 = 0.125 / 0.3;
/// The largest bias. A core of the whole zone would be a hard hinge.
pub const MAX_BIAS: f32 = 0.95;

/// How the art in a joint's zone gets from one bone to the next. No one
/// method suits every drawing, so each joint chooses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Fold {
    /// The zone follows a circular arc that touches both bones, and every
    /// cross-section stays straight (Barr's bend, SIGGRAPH 1984). The limb
    /// keeps its width. Bias shortens the arc for a sharper elbow.
    #[default]
    Arc,
    /// Each point turns about the joint by a share of the bend that rises
    /// smoothly across the zone (what dual quaternion skinning does in
    /// 2D). The outer side bulges.
    Rotation,
    /// The same turn with envelope weights: both bones weigh the same in
    /// the core that Bias sets, as in Blender's Grease Pencil rigging.
    RotationEnvelope,
    /// A straight blend of where each bone puts the point (linear blend
    /// skinning). The joint thins as it bends.
    Linear,
    /// Rigid moving least squares with the bones as handles (Schaefer,
    /// McPhail and Warren, SIGGRAPH 2006). Soft all over: the bones are
    /// not solid, and the whole chain uses it when one bone asks for it.
    MovingLeastSquares,
}

impl Fold {
    pub const ALL: [Fold; 5] = [
        Fold::Arc,
        Fold::Rotation,
        Fold::RotationEnvelope,
        Fold::Linear,
        Fold::MovingLeastSquares,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Fold::Arc => "Circular arc",
            Fold::Rotation => "Rotation blend",
            Fold::RotationEnvelope => "Rotation blend, flat core",
            Fold::Linear => "Linear blend",
            Fold::MovingLeastSquares => "Moving least squares",
        }
    }

    /// What the method does to the art, for a hint.
    pub fn hint(self) -> &'static str {
        match self {
            Fold::Arc => "The joint follows an arc and the limb keeps its width.",
            Fold::Rotation => "Art turns about the joint. The outer side bulges.",
            Fold::RotationEnvelope => "Art turns about the joint; Bias sets a flat core.",
            Fold::Linear => "A straight blend. The joint thins as it bends.",
            Fold::MovingLeastSquares => "Soft all over. The whole chain bends as one.",
        }
    }
}

/// One bone's parameters at one moment: the rest pose, or the animated
/// pose at a frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BonePose {
    /// Start of the chain, in drawing space. Only the first bone uses it;
    /// every other bone starts at the end of its parent.
    pub x: f32,
    pub y: f32,
    /// Degrees, relative to the parent bone (the first bone: to the x
    /// axis). Positive turns from +x towards +y.
    pub orientation: f32,
    pub length: f32,
    /// Radius of the joint at this bone's start. The first bone has no
    /// joint, so its radius is unused.
    pub radius: f32,
    /// Shape of that joint's fold, 0 (smooth) to [`MAX_BIAS`] (sharp).
    pub bias: f32,
    /// How that joint bends the art.
    #[serde(default)]
    pub fold: Fold,
}

impl BonePose {
    /// A bone with the default joint for a parent of `parent_length`.
    pub fn new(orientation: f32, length: f32, parent_length: f32) -> Self {
        BonePose {
            x: 0.0,
            y: 0.0,
            orientation,
            length,
            radius: DEFAULT_RADIUS_SHARE * length.min(parent_length),
            bias: DEFAULT_BIAS,
            fold: Fold::default(),
        }
    }
}

/// The animated parameters of a bone, as functions of the frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoneParams {
    pub x: Value,
    pub y: Value,
    pub orientation: Value,
    pub length: Value,
    pub radius: Value,
    pub bias: Value,
}

impl BoneParams {
    /// Constant parameters equal to `pose`.
    pub fn constant(pose: BonePose) -> Self {
        BoneParams {
            x: Value::Constant(pose.x),
            y: Value::Constant(pose.y),
            orientation: Value::Constant(pose.orientation),
            length: Value::Constant(pose.length),
            radius: Value::Constant(pose.radius),
            bias: Value::Constant(pose.bias),
        }
    }

    pub fn at(&self, frame: u32) -> BonePose {
        BonePose {
            x: self.x.at(frame),
            y: self.y.at(frame),
            orientation: self.orientation.at(frame),
            length: self.length.at(frame),
            radius: self.radius.at(frame),
            bias: self.bias.at(frame),
            fold: Fold::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    pub rest: BonePose,
    pub anim: BoneParams,
}

impl Bone {
    /// A bone whose animated pose starts at its rest pose.
    pub fn new(rest: BonePose) -> Self {
        Bone {
            rest,
            anim: BoneParams::constant(rest),
        }
    }
}

/// Which art the chain moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Influence {
    /// All of it, however far from the chain.
    #[default]
    Infinite,
    /// None of it.
    Zero,
}

/// How finely an imported image is cut into triangles to be bent.
///
/// Cells start at `largest` image pixels a side and split in four while
/// the bend moves their inside more than `tolerance` output pixels away
/// from a flat cell, down to `smallest`. Art away from the joints moves
/// rigidly, so it keeps large cells.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshDetail {
    /// Largest cell side, image pixels.
    pub largest: f32,
    /// Smallest cell side, image pixels.
    pub smallest: f32,
    /// Largest error allowed, output pixels.
    pub tolerance: f32,
}

impl Default for MeshDetail {
    fn default() -> Self {
        MeshDetail {
            largest: 64.0,
            smallest: 2.0,
            tolerance: 0.25,
        }
    }
}

/// A chain of bones, root first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BoneChain {
    pub bones: Vec<Bone>,
    #[serde(default)]
    pub influence: Influence,
}

impl BoneChain {
    /// Both poses solved for `frame`, ready to move points.
    pub fn pose_at(&self, frame: u32) -> ChainPose {
        let rest: Vec<BonePose> = self.bones.iter().map(|b| b.rest).collect();
        let anim: Vec<BonePose> = self
            .bones
            .iter()
            .map(|b| BonePose {
                fold: b.rest.fold,
                ..b.anim.at(frame)
            })
            .collect();
        ChainPose::new(&rest, &anim, self.influence)
    }
}

type V = [f32; 2];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: V, k: f32) -> V {
    [a[0] * k, a[1] * k]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: V) -> f32 {
    dot(a, a).sqrt()
}
fn perp(a: V) -> V {
    [-a[1], a[0]]
}
fn dir(deg: f32) -> V {
    let (s, c) = deg.to_radians().sin_cos();
    [c, s]
}

/// A solved bone: start, unit direction, length and absolute angle.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    p: V,
    u: V,
    len: f32,
    angle: f32,
}

impl Frame {
    fn end(&self) -> V {
        add(self.p, mul(self.u, self.len))
    }
}

fn solve(poses: &[BonePose]) -> Vec<Frame> {
    let mut out: Vec<Frame> = Vec::with_capacity(poses.len());
    for (i, b) in poses.iter().enumerate() {
        let (p, angle) = match out.last() {
            None => ([b.x, b.y], b.orientation),
            Some(parent) => (parent.end(), parent.angle + b.orientation),
        };
        let len = if b.length.is_finite() {
            b.length.max(0.0)
        } else {
            0.0
        };
        let angle = if angle.is_finite() { angle } else { 0.0 };
        let p = if i == 0 && !(p[0].is_finite() && p[1].is_finite()) {
            [0.0, 0.0]
        } else {
            p
        };
        out.push(Frame {
            p,
            u: dir(angle),
            len,
            angle,
        });
    }
    out
}

/// The joint at the start of bone `j` (j >= 1), in the rest pose.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Joint {
    at: V,
    /// Bisector normal: points towards bone `j`.
    m: V,
    radius: f32,
    core: f32,
    fold: Fold,
}

/// A smooth step from 0 at the parent's edge of a joint zone to 1 at the
/// child's edge.
fn smooth_weight(sigma: f32, radius: f32) -> f32 {
    let t = (0.5 + sigma / (2.0 * radius)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn turn(v: V, radians: f32) -> V {
    let (s, c) = radians.sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1]]
}

/// Handles along each bone for [`Fold::MovingLeastSquares`].
const MLS_SAMPLES: usize = 16;

/// Weight of the far bone at distance `d` from the joint's bisector, as an
/// envelope falloff: 1 inside the core, then down to 0 at `radius`.
fn falloff(d: f32, radius: f32, core: f32) -> f32 {
    let inner = core * radius;
    if d <= inner {
        return 1.0;
    }
    let span = radius - inner;
    if span <= 0.0 || d >= radius {
        return 0.0;
    }
    let x = (d - inner) / span;
    1.0 - x * x
}

/// Weight of the child bone for a point `sigma` from the bisector of a
/// joint (negative on the parent's side), with the joint's `radius` and
/// `bias`. Both bones' weights add up to 1.
pub fn child_weight(sigma: f32, radius: f32, bias: f32) -> f32 {
    let core = bias.clamp(0.0, MAX_BIAS);
    let f = falloff(sigma.abs(), radius.max(0.0), core);
    if sigma < 0.0 {
        f / (1.0 + f)
    } else {
        1.0 / (1.0 + f)
    }
}

/// A chain solved for one frame. Building it allocates; [`Self::map`]
/// does not.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainPose {
    rest: Vec<Frame>,
    anim: Vec<Frame>,
    joints: Vec<Joint>,
    influence: Influence,
    /// Rest and animated handle points, when the chain bends by moving
    /// least squares.
    handles: Vec<(V, V)>,
}

impl ChainPose {
    /// `rest` and `anim` must have the same length; extra bones on either
    /// side are left out.
    pub fn new(rest: &[BonePose], anim: &[BonePose], influence: Influence) -> Self {
        let n = rest.len().min(anim.len());
        let rest_f = solve(&rest[..n]);
        let anim_f = solve(&anim[..n]);
        let joints = (1..n)
            .map(|j| {
                let (a, b) = (rest_f[j - 1], rest_f[j]);
                let s = add(a.u, b.u);
                // A full fold in the rest pose has no bisector; split
                // across the parent bone instead.
                let m = if len(s) > 1e-6 {
                    mul(s, 1.0 / len(s))
                } else {
                    b.u
                };
                // Two joint zones on one bone never overlap.
                let room = 0.5 * a.len.min(b.len);
                let r = anim[j].radius;
                let radius = if r.is_finite() {
                    r.clamp(0.0, room)
                } else {
                    0.0
                };
                let bias = anim[j].bias;
                let core = if bias.is_finite() {
                    bias.clamp(0.0, MAX_BIAS)
                } else {
                    DEFAULT_BIAS
                };
                Joint {
                    at: b.p,
                    m,
                    radius,
                    core,
                    fold: anim[j].fold,
                }
            })
            .collect();
        let soft = anim[..n].iter().any(|b| b.fold == Fold::MovingLeastSquares);
        let handles = if soft {
            rest_f
                .iter()
                .zip(&anim_f)
                .flat_map(|(r, a)| {
                    (0..=MLS_SAMPLES).map(move |k| {
                        let t = k as f32 / MLS_SAMPLES as f32;
                        (add(r.p, mul(r.u, r.len * t)), add(a.p, mul(a.u, a.len * t)))
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        ChainPose {
            rest: rest_f,
            anim: anim_f,
            joints,
            influence,
            handles,
        }
    }

    /// Move a point of the drawing from the rest pose to the animated pose.
    pub fn map(&self, p: V) -> V {
        if self.moves_nothing() {
            return p;
        }
        if !self.handles.is_empty() {
            return self.least_squares(p);
        }
        // The bone that owns `p`: walk the joints from the root while the
        // point is past each bisector.
        let mut i = 0;
        for (k, j) in self.joints.iter().enumerate() {
            if dot(sub(p, j.at), j.m) >= 0.0 {
                i = k + 1;
            } else {
                break;
            }
        }
        // The joint zones at both ends of that bone.
        for j in [i, i + 1] {
            if j == 0 || j > self.joints.len() {
                continue;
            }
            let joint = &self.joints[j - 1];
            let sigma = dot(sub(p, joint.at), joint.m);
            if sigma.abs() < joint.radius {
                return match joint.fold {
                    Fold::Arc | Fold::MovingLeastSquares => self.arc(p, j, sigma),
                    Fold::Rotation => self.blend(p, j, smooth_weight(sigma, joint.radius)),
                    Fold::RotationEnvelope => {
                        self.blend(p, j, child_weight(sigma, joint.radius, joint.core))
                    }
                    Fold::Linear => {
                        let w = smooth_weight(sigma, joint.radius);
                        let (a, b) = (self.bone_map(j - 1, p), self.bone_map(j, p));
                        add(a, mul(sub(b, a), w))
                    }
                };
            }
        }
        self.bone_map(i, p)
    }

    /// Bend the zone of joint `j` along a circular arc that touches both
    /// bones. `sigma` is the point's distance from the joint's bisector.
    ///
    /// Seen from the parent bone, the child turns by `delta` about the
    /// joint. The art from `-d` to `d` along the bisector's normal goes
    /// onto an arc of radius `d / tan(delta / 2)`: it leaves the parent's
    /// side straight on and meets the child's side straight on. A
    /// cross-section stays a straight line through the arc's centre. Art
    /// further inside than the centre comes out on its far side: it
    /// overlaps, as a sleeve does in a deep bend. (Stopping it at the
    /// centre would tear it from the rigid art at the zone's edges.)
    /// Bias shortens `d`, for a tighter arc.
    fn arc(&self, p: V, j: usize, sigma: f32) -> V {
        let joint = &self.joints[j - 1];
        let (r0, r1) = (self.rest[j - 1], self.rest[j]);
        let (a0, a1) = (self.anim[j - 1], self.anim[j]);
        let pivot = a1.p;
        // Rigid about the joint with each bone, without their stretch.
        let with_parent = (a0.angle - r0.angle).to_radians();
        let with_child = (a1.angle - r1.angle).to_radians();
        let local = sub(p, joint.at);
        let on_parent = add(pivot, turn(local, with_parent));
        let on_child = add(pivot, turn(local, with_child));
        let mut delta = with_child - with_parent;
        delta =
            (delta + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let d = joint.radius * (1.0 - joint.core);
        let bent = if sigma <= -d {
            on_parent
        } else if sigma >= d {
            on_child
        } else if delta.abs() < 1e-5 || d < 1e-6 {
            on_parent
        } else {
            let side = delta.signum();
            let radius = d / (delta.abs() / 2.0).tan();
            // Across the limb, positive towards the inside of the bend.
            let across = dot(local, perp(joint.m)) * side;
            let reach = radius - across;
            let phi = side * (0.5 + sigma / (2.0 * d)) * delta.abs();
            // In the joint's axes: x along the bisector's normal.
            let x = -d + reach * phi.sin() * side;
            let y = (radius - reach * phi.cos()) * side;
            let m = turn(joint.m, with_parent);
            add(pivot, add(mul(m, x), mul(perp(m), y)))
        };
        // A stretched bone moves its own side of the zone; fade that in
        // towards each edge so the map stays whole there.
        let t = (0.5 + sigma / (2.0 * joint.radius)).clamp(0.0, 1.0);
        let pull_parent = sub(self.bone_map(j - 1, p), on_parent);
        let pull_child = sub(self.bone_map(j, p), on_child);
        add(bent, add(mul(pull_parent, 1.0 - t), mul(pull_child, t)))
    }

    /// Rigid moving least squares: the rotation and shift that best take
    /// the rest handles to the animated ones, weighted towards the
    /// handles near `p`.
    fn least_squares(&self, p: V) -> V {
        let weight = |h: &(V, V)| {
            let d = sub(h.0, p);
            1.0 / (dot(d, d) + 1e-6)
        };
        let (mut total, mut from, mut to) = (0.0, [0.0; 2], [0.0; 2]);
        for h in &self.handles {
            let w = weight(h);
            total += w;
            from = add(from, mul(h.0, w));
            to = add(to, mul(h.1, w));
        }
        let (from, to) = (mul(from, 1.0 / total), mul(to, 1.0 / total));
        let (mut cross, mut along) = (0.0, 0.0);
        for h in &self.handles {
            let w = weight(h);
            let (a, b) = (sub(h.0, from), sub(h.1, to));
            cross += w * (a[0] * b[1] - a[1] * b[0]);
            along += w * dot(a, b);
        }
        add(to, turn(sub(p, from), cross.atan2(along)))
    }

    /// Blend the maps of bones `j - 1` and `j` in polar form around the
    /// animated joint, `w` being the weight of bone `j`.
    fn blend(&self, p: V, j: usize, w: f32) -> V {
        let a = self.bone_map(j - 1, p);
        let b = self.bone_map(j, p);
        let c = self.anim[j].p;
        let (va, vb) = (sub(a, c), sub(b, c));
        let (la, lb) = (len(va), len(vb));
        if la < 1e-6 || lb < 1e-6 {
            return add(a, mul(sub(b, a), w));
        }
        let ta = va[1].atan2(va[0]);
        let mut delta = vb[1].atan2(vb[0]) - ta;
        if delta > std::f32::consts::PI {
            delta -= std::f32::consts::TAU;
        } else if delta < -std::f32::consts::PI {
            delta += std::f32::consts::TAU;
        }
        let (s, co) = (ta + w * delta).sin_cos();
        add(c, mul([co, s], la + w * (lb - la)))
    }

    /// Move `p` rigidly with bone `i`, stretching along the bone. Before
    /// the root and past the last bone's end the art does not stretch.
    fn bone_map(&self, i: usize, p: V) -> V {
        let (r, a) = (self.rest[i], self.anim[i]);
        let d = sub(p, r.p);
        let t = dot(d, r.u);
        let s = dot(d, perp(r.u));
        let last = i + 1 == self.rest.len();
        let t2 = if i == 0 && t < 0.0 {
            t
        } else if last && t > r.len {
            a.len + (t - r.len)
        } else if r.len > 1e-6 {
            t * a.len / r.len
        } else {
            t
        };
        add(add(a.p, mul(a.u, t2)), mul(perp(a.u), s))
    }

    /// Does [`Self::map`] leave every point where it is? True for Zero
    /// influence and for an empty chain, not for an unmoved chain.
    pub fn moves_nothing(&self) -> bool {
        self.influence == Influence::Zero || self.rest.is_empty()
    }

    /// Is the animated pose the rest pose, so that nothing bends?
    pub fn at_rest(&self) -> bool {
        self.moves_nothing() || self.rest == self.anim
    }

    /// Everything that shapes the bend, for a cache key.
    pub fn hash_into(&self, h: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        (self.influence as u8).hash(h);
        for f in self.rest.iter().chain(&self.anim) {
            for v in [f.p[0], f.p[1], f.len, f.angle] {
                v.to_bits().hash(h);
            }
        }
        for j in &self.joints {
            j.radius.to_bits().hash(h);
            j.core.to_bits().hash(h);
            j.fold.hash(h);
        }
        self.handles.len().hash(h);
    }

    /// The smallest joint radius above zero, after clamping. Art must be
    /// sampled finer than this to bend smoothly at the joints. None when
    /// no joint blends (one bone, or hard hinges only).
    pub fn smallest_radius(&self) -> Option<f32> {
        if !self.handles.is_empty() {
            // Soft everywhere: sample as for a joint a quarter of the
            // shortest bone wide.
            return self
                .rest
                .iter()
                .map(|f| f.len / 4.0)
                .filter(|r| *r > 0.0)
                .reduce(f32::min);
        }
        self.joints
            .iter()
            .map(|j| {
                j.radius
                    * if j.fold == Fold::Arc {
                        1.0 - j.core
                    } else {
                        1.0
                    }
            })
            .filter(|r| *r > 0.0)
            .reduce(f32::min)
    }

    /// Kinematic Output: how the end of bone `i` moves from the rest pose
    /// to the animated pose, for a drawing that follows it without being
    /// deformed. The identity for a bone that doesn't exist.
    pub fn kinematic(&self, i: usize) -> Affine {
        let (Some(r), Some(a)) = (self.rest.get(i), self.anim.get(i)) else {
            return Affine::IDENTITY;
        };
        let (e, e2) = (r.end(), a.end());
        Affine::translate(e2[0], e2[1])
            .compose(Affine::rotate_deg(a.angle - r.angle))
            .compose(Affine::translate(-e[0], -e[1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V, b: V, eps: f32) -> bool {
        (a[0] - b[0]).abs() <= eps && (a[1] - b[1]).abs() <= eps
    }

    /// Two bones of length `l` along +x from the origin.
    fn arm(l: f32) -> Vec<BonePose> {
        vec![BonePose::new(0.0, l, l), BonePose::new(0.0, l, l)]
    }

    fn bent(l: f32, deg: f32) -> ChainPose {
        bent_by(l, deg, Fold::default())
    }

    fn bent_by(l: f32, deg: f32, fold: Fold) -> ChainPose {
        let rest = arm(l);
        let mut anim = rest.clone();
        anim[1].orientation = deg;
        anim[1].fold = fold;
        ChainPose::new(&rest, &anim, Influence::Infinite)
    }

    fn grid() -> impl Iterator<Item = V> {
        (-8..=24).flat_map(|i| (-8..=8).map(move |k| [i as f32 * 0.125, k as f32 * 0.125]))
    }

    #[test]
    fn the_rest_pose_moves_nothing() {
        let mut rest = arm(1.0);
        rest[0].x = 0.3;
        rest[0].orientation = 20.0;
        rest[1].orientation = -35.0;
        let pose = ChainPose::new(&rest, &rest, Influence::Infinite);
        for p in grid() {
            assert!(close(pose.map(p), p, 1e-4), "{p:?} -> {:?}", pose.map(p));
        }
    }

    #[test]
    fn one_bone_turns_the_art_rigidly_about_the_root() {
        let rest = vec![BonePose::new(0.0, 1.0, 1.0)];
        let mut anim = rest.clone();
        anim[0].orientation = 90.0;
        let pose = ChainPose::new(&rest, &anim, Influence::Infinite);
        let turn = Affine::rotate_deg(90.0);
        for p in grid() {
            let (x, y) = turn.apply(p[0], p[1]);
            assert!(close(pose.map(p), [x, y], 1e-4));
        }
    }

    #[test]
    fn a_bent_joint_keeps_both_bones_solid() {
        let pose = bent(1.0, 90.0);
        // Far on the parent side: unchanged.
        for x in [-0.4, 0.0, 0.3, 0.6] {
            assert!(close(pose.map([x, 0.1]), [x, 0.1], 1e-5));
        }
        // Far on the child side: turned about the joint at (1, 0).
        for x in [1.4, 1.8, 2.3] {
            assert!(close(pose.map([x, 0.1]), [0.9, x - 1.0], 1e-5));
        }
    }

    #[test]
    fn the_rotation_fold_is_continuous_and_keeps_distance_to_the_joint() {
        let pose = bent_by(1.0, 90.0, Fold::RotationEnvelope);
        let mut last = pose.map([0.5, 0.0]);
        for i in 1..=400 {
            let x = 0.5 + i as f32 / 400.0;
            let q = pose.map([x, 0.0]);
            // Neighbours 0.0025 apart stay close: no tear, no jump. The
            // weight is steepest at the zone edge (as in Blender), where
            // one step moves a point about 0.013.
            assert!(len(sub(q, last)) < 0.02, "jump at x = {x}");
            last = q;
            // Polar blend: the distance to the joint is kept, as with
            // Blender's Preserve Volume.
            let d = (x - 1.0).abs();
            assert!((len(sub(q, [1.0, 0.0])) - d).abs() < 1e-4, "x = {x}");
        }
    }

    #[test]
    fn no_fold_tears_the_art_off_the_bone_axis() {
        // Continuous means neighbours move apart by at most a fixed
        // multiple of the step. The steepest is the envelope's zone edge:
        // weight slope 2 / 0.175 = 11.4, times 150 degrees (2.62 rad),
        // times the largest distance to the joint here (0.58), is about 17.
        let h = 0.0025;
        for (deg, fold) in [30.0, 90.0, 150.0, -120.0]
            .into_iter()
            .flat_map(|deg| Fold::ALL.map(|fold| (deg, fold)))
        {
            let pose = bent_by(1.0, deg, fold);
            let at = |i: i32, k: i32| pose.map([0.5 + i as f32 * h, k as f32 * h]);
            for i in 0..400 {
                for k in -120..120 {
                    let q = at(i, k);
                    for n in [at(i + 1, k), at(i, k + 1)] {
                        assert!(
                            len(sub(n, q)) < 20.0 * h,
                            "{fold:?}, {deg} deg at ({i}, {k})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_arc_keeps_the_limb_as_wide_as_drawn_and_meets_both_bones() {
        for deg in [20.0_f32, 90.0, -135.0] {
            let pose = bent(1.0, deg);
            // Default joint: radius 0.3, less the default bias.
            let d = 0.3 * (1.0 - DEFAULT_BIAS);
            let radius = d / (deg.abs().to_radians() / 2.0).tan();
            for i in 0..=40 {
                let x = 1.0 - d + 2.0 * d * i as f32 / 40.0;
                let (a, b) = (pose.map([x, -0.04]), pose.map([x, 0.04]));
                assert!((len(sub(a, b)) - 0.08).abs() < 1e-4, "{deg} deg, x = {x}");
                // The centre line is on the arc.
                let centre = [1.0 - d, radius * deg.signum()];
                let on_axis = pose.map([x, 0.0]);
                assert!((len(sub(on_axis, centre)) - radius).abs() < 1e-3);
            }
            // Art outside the zone is solid on its bone.
            assert!(close(pose.map([0.8, 0.1]), [0.8, 0.1], 1e-5));
            let (s, c) = deg.to_radians().sin_cos();
            let far = [1.0 + 0.3 * c - 0.1 * s, 0.3 * s + 0.1 * c];
            assert!(close(pose.map([1.3, 0.1]), far, 1e-4));
        }
    }

    #[test]
    fn bias_tightens_the_arc() {
        let turn = |bias: f32| {
            let rest = arm(1.0);
            let mut anim = rest.clone();
            anim[1].orientation = 90.0;
            anim[1].bias = bias;
            let pose = ChainPose::new(&rest, &anim, Influence::Infinite);
            // How far the joint itself moves: the arc cuts the corner.
            len(sub(pose.map([1.0, 0.0]), [1.0, 0.0]))
        };
        assert!(turn(0.0) > turn(0.5) && turn(0.5) > turn(0.9));
        assert!(turn(0.9) < 0.02);
    }

    #[test]
    fn moving_least_squares_follows_the_bones_and_is_soft_between() {
        let pose = bent_by(1.0, 90.0, Fold::MovingLeastSquares);
        // Points on the bones go where the bones go.
        assert!(close(pose.map([0.5, 0.0]), [0.5, 0.0], 0.01));
        assert!(close(pose.map([1.5, 0.0]), [1.0, 0.5], 0.01));
        // Off the bones the art gives a little everywhere.
        let off = pose.map([0.3, 0.2]);
        assert!(len(sub(off, [0.3, 0.2])) > 1e-3);
        assert!(off[0].is_finite() && off[1].is_finite());
    }

    #[test]
    fn a_longer_bone_stretches_along_itself() {
        let rest = vec![BonePose::new(0.0, 1.0, 1.0)];
        let mut anim = rest.clone();
        anim[0].length = 1.5;
        let pose = ChainPose::new(&rest, &anim, Influence::Infinite);
        assert!(close(pose.map([0.5, 0.2]), [0.75, 0.2], 1e-5));
        // Past the end and before the root, the art only moves.
        assert!(close(pose.map([1.4, 0.0]), [1.9, 0.0], 1e-5));
        assert!(close(pose.map([-0.3, 0.0]), [-0.3, 0.0], 1e-5));
    }

    #[test]
    fn weights_match_blender_grease_pencil_automatic_weights() {
        // Child bone weight along two bones of length 1 (joint at x = 1),
        // read from Blender 5.3.0's Automatic Weights. Blender's zone
        // ends a little later (about 0.31), so allow 0.1.
        let blender = [
            (0.725, 0.2152),
            (0.75, 0.2967),
            (0.80, 0.4030),
            (0.85, 0.4658),
            (0.90, 0.5),
            (1.0, 0.5),
            (1.10, 0.5),
            (1.15, 0.5342),
            (1.20, 0.5970),
            (1.25, 0.7033),
            (1.275, 0.7848),
        ];
        for (x, w) in blender {
            let ours = child_weight(x - 1.0, 0.3, DEFAULT_BIAS);
            assert!((ours - w).abs() <= 0.1, "x = {x}: {ours} vs {w}");
        }
        assert_eq!(child_weight(-0.3, 0.3, DEFAULT_BIAS), 0.0);
        assert_eq!(child_weight(0.3, 0.3, DEFAULT_BIAS), 1.0);
    }

    #[test]
    fn a_larger_bias_widens_the_core() {
        let (r, sigma) = (0.3, -0.15);
        assert!(child_weight(sigma, r, 0.1) < child_weight(sigma, r, 0.7));
        assert_eq!(child_weight(sigma, r, 0.7), 0.5);
        // Weights rise monotonically across the zone.
        for bias in [0.0, DEFAULT_BIAS, MAX_BIAS] {
            let mut last = 0.0;
            for i in -30..=30 {
                let w = child_weight(i as f32 * 0.01, r, bias);
                assert!(w >= last - 1e-6);
                last = w;
            }
        }
    }

    #[test]
    fn zero_influence_moves_nothing() {
        let rest = arm(1.0);
        let mut anim = rest.clone();
        anim[0].orientation = 45.0;
        let pose = ChainPose::new(&rest, &anim, Influence::Zero);
        for p in grid() {
            assert_eq!(pose.map(p), p);
        }
    }

    #[test]
    fn kinematic_output_follows_the_end_of_a_bone() {
        assert_eq!(bent(1.0, 0.0).kinematic(1), Affine::IDENTITY);
        let k = bent(1.0, 90.0).kinematic(1);
        // The rest end (2, 0) goes to the animated end (1, 1), and a point
        // past it turns with the bone.
        let (x, y) = k.apply(2.0, 0.0);
        assert!(close([x, y], [1.0, 1.0], 1e-5));
        let (x, y) = k.apply(2.5, 0.0);
        assert!(close([x, y], [1.0, 1.5], 1e-5));
        assert_eq!(bent(1.0, 90.0).kinematic(7), Affine::IDENTITY);
    }

    #[test]
    fn the_chain_follows_its_keyframes() {
        let mut chain = BoneChain {
            bones: arm(1.0).into_iter().map(Bone::new).collect(),
            influence: Influence::Infinite,
        };
        let mut v = Value::Constant(0.0);
        v.set_key(1, 0.0);
        v.set_key(11, 90.0);
        chain.bones[1].anim.orientation = v;
        assert!(close(chain.pose_at(1).map([2.0, 0.0]), [2.0, 0.0], 1e-5));
        assert!(close(chain.pose_at(11).map([2.0, 0.0]), [1.0, 1.0], 1e-5));
        let half = chain.pose_at(6).map([2.0, 0.0]);
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(half, [1.0 + s, s], 1e-4));
    }

    #[test]
    fn odd_chains_give_finite_points() {
        let mut cases: Vec<(Vec<BonePose>, Vec<BonePose>)> = Vec::new();
        // A zero-length bone.
        let mut zero = arm(1.0);
        zero[1].length = 0.0;
        cases.push((zero.clone(), arm(1.0)));
        cases.push((arm(1.0), zero));
        // No joint radius.
        let mut hinge = arm(1.0);
        hinge[1].radius = 0.0;
        let mut hinge_anim = hinge.clone();
        hinge_anim[1].orientation = 60.0;
        cases.push((hinge, hinge_anim));
        // A full fold, in the animated pose and in the rest pose.
        cases.push((arm(1.0), {
            let mut a = arm(1.0);
            a[1].orientation = 180.0;
            a
        }));
        let mut folded = arm(1.0);
        folded[1].orientation = 180.0;
        cases.push((folded, arm(1.0)));
        // Bad numbers.
        let mut nan = arm(1.0);
        nan[1].radius = f32::NAN;
        nan[1].bias = f32::INFINITY;
        nan[0].length = f32::NAN;
        cases.push((arm(1.0), nan));
        for (rest, anim) in cases {
            let pose = ChainPose::new(&rest, &anim, Influence::Infinite);
            for p in grid() {
                let q = pose.map(p);
                assert!(q[0].is_finite() && q[1].is_finite(), "{p:?} -> {q:?}");
            }
        }
        // No bones at all.
        let empty = ChainPose::new(&[], &[], Influence::Infinite);
        assert_eq!(empty.map([1.0, 2.0]), [1.0, 2.0]);
    }
}
