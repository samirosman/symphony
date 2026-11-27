//! Rig controls: a Control node holds poses that add offsets to the
//! values of the nodes under it (Harmony's Attribute Controller with a
//! Slider Interpolator). The animator keys one value, the control
//! position; the rig's own keys stay as they are.
//!
//! The rest pose sits at position 0 and holds no offsets. Between two
//! poses an offset changes linearly; past the outermost pose it holds.

use serde::{Deserialize, Serialize};

use crate::anim::Value;
use crate::graph::{NodeId, NodeKind};
use crate::rig::BoneColour;

/// One animated value of a node that a pose can offset. `Rotation`
/// means a bone's turn on a bone and a peg's rotation on a peg.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Channel {
    X,
    Y,
    Z,
    Rotation,
    ScaleX,
    ScaleY,
    /// A bone's length as a multiple of its rest length.
    Stretch,
}

/// A change to one value of one node.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Offset {
    pub node: NodeId,
    pub channel: Channel,
    pub amount: f32,
}

/// The offsets that apply when the control is at `at`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub at: f32,
    pub offsets: Vec<Offset>,
}

impl Pose {
    /// What this pose adds to `channel` of `node`.
    pub fn amount(&self, node: NodeId, channel: Channel) -> f32 {
        self.offsets
            .iter()
            .filter(|o| o.node == node && o.channel == channel)
            .map(|o| o.amount)
            .sum()
    }
}

/// How the control picks between its poses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlType {
    /// One axis: the poses sit on a line (Harmony's Slider).
    #[default]
    Slider,
}

/// How the control shows in the Camera view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    /// Where the widget sits, in the space of `follow`.
    pub position: [f32; 2],
    /// The peg or bone the widget moves with; None: the Control's own
    /// parent.
    pub follow: Option<NodeId>,
    pub vertical: bool,
    pub colour: BoneColour,
    /// Length of the slider track, in stage units.
    pub size: f32,
    pub show_label: bool,
}

impl Default for Widget {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0],
            follow: None,
            vertical: false,
            colour: BoneColour::default(),
            size: 120.0,
            show_label: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Control {
    #[serde(default)]
    pub kind: ControlType,
    /// The control position (animatable).
    pub value: Value,
    /// Least and most the control position may be.
    pub range: [f32; 2],
    pub poses: Vec<Pose>,
    #[serde(default)]
    pub widget: Widget,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            kind: ControlType::Slider,
            value: Value::Constant(0.0),
            range: [-1.0, 1.0],
            poses: Vec::new(),
            widget: Widget::default(),
        }
    }
}

impl Control {
    /// `position` kept inside the range.
    pub fn clamp(&self, position: f32) -> f32 {
        let [a, b] = self.range;
        if position.is_nan() {
            return 0.0f32.clamp(a.min(b), a.max(b));
        }
        position.clamp(a.min(b), a.max(b))
    }

    /// The control position on `frame`, inside the range.
    pub fn position_at(&self, frame: u32) -> f32 {
        self.clamp(self.value.at(frame))
    }

    /// What the control adds to `channel` of `node` on `frame`.
    pub fn offset_at(&self, frame: u32, node: NodeId, channel: Channel) -> f32 {
        self.offset_for(self.position_at(frame), node, channel)
    }

    /// What the control adds to `channel` of `node` when it is at
    /// `position`: linear between the two nearest poses, held past the
    /// outermost one. The rest pose at 0 adds nothing; a pose stored at
    /// 0 is ignored.
    pub fn offset_for(&self, position: f32, node: NodeId, channel: Channel) -> f32 {
        let points = std::iter::once((0.0, 0.0)).chain(
            self.poses
                .iter()
                .filter(|p| p.at != 0.0 && p.at.is_finite())
                .map(|p| (p.at, p.amount(node, channel))),
        );
        let mut below: Option<(f32, f32)> = None;
        let mut above: Option<(f32, f32)> = None;
        for (at, amount) in points {
            if at <= position && below.is_none_or(|(b, _)| at > b) {
                below = Some((at, amount));
            }
            if at >= position && above.is_none_or(|(a, _)| at < a) {
                above = Some((at, amount));
            }
        }
        match (below, above) {
            (Some((b, low)), Some((a, high))) if a > b => {
                low + (high - low) * (position - b) / (a - b)
            }
            (Some((_, amount)), _) | (None, Some((_, amount))) => amount,
            (None, None) => 0.0,
        }
    }

    pub fn has_key(&self, frame: u32) -> bool {
        self.value.has_key(frame)
    }

    /// Add `amount` to the offset of `channel` of `node` in the pose at
    /// `at`. False when no pose is there (the rest pose takes none).
    pub fn add_offset(&mut self, at: f32, node: NodeId, channel: Channel, amount: f32) -> bool {
        let Some(pose) = self.poses.iter_mut().find(|p| p.at == at && at != 0.0) else {
            return false;
        };
        match pose
            .offsets
            .iter_mut()
            .find(|o| o.node == node && o.channel == channel)
        {
            Some(offset) => offset.amount += amount,
            None => pose.offsets.push(Offset {
                node,
                channel,
                amount,
            }),
        }
        true
    }
}

/// The values of a bone or peg that a pose can offset, on `frame`.
pub fn channel_values(kind: &NodeKind, frame: u32) -> Vec<(Channel, f32)> {
    match kind {
        NodeKind::Bone(bone) => vec![
            (Channel::Rotation, bone.rotation.at(frame)),
            (Channel::Stretch, bone.stretch.at(frame)),
        ],
        NodeKind::Peg(peg) | NodeKind::Skeleton(peg) => {
            let scale = |v: &Option<Value>| v.as_ref().map_or(1.0, |v| v.at(frame));
            vec![
                (Channel::X, peg.x.at(frame)),
                (Channel::Y, peg.y.at(frame)),
                (Channel::Z, peg.z.at(frame)),
                (Channel::Rotation, peg.rotation.at(frame)),
                (Channel::ScaleX, scale(&peg.scale_x)),
                (Channel::ScaleY, scale(&peg.scale_y)),
            ]
        }
        _ => Vec::new(),
    }
}

/// Put back into `kind` the values a pose can offset, as `from` holds
/// them; every other setting stays.
pub fn restore_channels(kind: &mut NodeKind, from: &NodeKind) {
    match (kind, from) {
        (NodeKind::Bone(bone), NodeKind::Bone(old)) => {
            bone.rotation = old.rotation.clone();
            bone.stretch = old.stretch.clone();
        }
        (NodeKind::Peg(peg), NodeKind::Peg(old))
        | (NodeKind::Skeleton(peg), NodeKind::Skeleton(old)) => {
            peg.x = old.x.clone();
            peg.y = old.y.clone();
            peg.z = old.z.clone();
            peg.rotation = old.rotation.clone();
            peg.scale_x = old.scale_x.clone();
            peg.scale_y = old.scale_y.clone();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BONE: NodeId = NodeId(3);

    fn slider(poses: &[(f32, f32)]) -> Control {
        Control {
            poses: poses
                .iter()
                .map(|&(at, amount)| Pose {
                    at,
                    offsets: vec![Offset {
                        node: BONE,
                        channel: Channel::Rotation,
                        amount,
                    }],
                })
                .collect(),
            ..Control::default()
        }
    }

    fn at(c: &Control, position: f32) -> f32 {
        c.offset_for(position, BONE, Channel::Rotation)
    }

    #[test]
    fn rest_pose_adds_nothing() {
        let c = slider(&[(1.0, 40.0)]);
        assert_eq!(at(&c, 0.0), 0.0);
        assert_eq!(at(&c, -1.0), 0.0, "no pose below the rest: holds 0");
    }

    #[test]
    fn blends_linearly_between_the_nearest_poses() {
        let c = slider(&[(1.0, 40.0), (-1.0, -20.0), (0.5, 10.0)]);
        assert_eq!(at(&c, 0.25), 5.0);
        assert_eq!(at(&c, 0.75), 25.0);
        assert_eq!(at(&c, -0.5), -10.0);
    }

    #[test]
    fn holds_past_the_outermost_pose() {
        let mut c = slider(&[(0.5, 10.0)]);
        c.range = [-2.0, 2.0];
        assert_eq!(at(&c, 1.5), 10.0);
    }

    #[test]
    fn a_pose_at_rest_is_ignored() {
        let c = slider(&[(0.0, 99.0), (1.0, 40.0)]);
        assert_eq!(at(&c, 0.0), 0.0);
        assert_eq!(at(&c, 0.5), 20.0);
    }

    #[test]
    fn other_values_are_untouched() {
        let c = slider(&[(1.0, 40.0)]);
        assert_eq!(c.offset_for(1.0, BONE, Channel::Stretch), 0.0);
        assert_eq!(c.offset_for(1.0, NodeId(4), Channel::Rotation), 0.0);
    }

    #[test]
    fn position_stays_in_range() {
        let mut c = slider(&[(1.0, 40.0)]);
        c.value = Value::Constant(5.0);
        assert_eq!(c.position_at(1), 1.0);
        c.range = [1.0, -1.0];
        assert_eq!(c.position_at(1), 1.0, "a reversed range still clamps");
    }
}
