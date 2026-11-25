//! Node graph. Data flows top to bottom like Harmony's Node view:
//! Peg -> Drawing -> Composite -> Display / Write.
//!
//! Node IDs are slots that are never reused, so removing a node leaves a
//! gap instead of renumbering the others (E2.2).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::anim::Value;
use crate::id::Id;
use crate::math::Affine;
use crate::rig::{Bone, BoneField, BonePose};
use crate::xsheet::Xsheet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub usize);

/// Transform parameters on a peg. Order: scale and rotate around the
/// pivot, then translate by position.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PegParams {
    pub x: Value,
    pub y: Value,
    /// Positive depth places artwork farther from the camera.
    #[serde(default)]
    pub z: Value,
    pub rotation: Value,
    pub scale_x: Option<Value>,
    pub scale_y: Option<Value>,
    pub pivot: (f32, f32),
}

impl PegParams {
    pub fn transform_at(&self, frame: u32) -> Affine {
        let sx = self.scale_x.as_ref().map_or(1.0, |v| v.at(frame));
        let sy = self.scale_y.as_ref().map_or(1.0, |v| v.at(frame));
        let (px, py) = self.pivot;
        Affine::translate(self.x.at(frame), self.y.at(frame))
            .compose(Affine::translate(px, py))
            .compose(Affine::rotate_deg(self.rotation.at(frame)))
            .compose(Affine::scale(sx, sy))
            .compose(Affine::translate(-px, -py))
    }

    /// Does any parameter have a key on `frame`?
    pub fn has_key(&self, frame: u32) -> bool {
        [
            Some(&self.x),
            Some(&self.y),
            Some(&self.z),
            Some(&self.rotation),
            self.scale_x.as_ref(),
            self.scale_y.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|v| v.has_key(frame))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Shows whatever drawing its xsheet column exposes on the current frame.
    Drawing {
        column: Id,
    },
    Peg(PegParams),
    Skeleton(PegParams),
    Bone(Bone),
    /// Bends the artwork under it with a grid of points (Harmony's Mesh
    /// Warp). A position node, like a peg.
    /// Boxed: it's much bigger than the other kinds.
    MeshWarp(Box<crate::warp::MeshWarp>),
    /// Stacks its inputs. Port 0 (leftmost) is in front.
    Composite,
    Effect(Effect),
    Gradient(Gradient),
    /// What the Camera view shows.
    Display,
    /// Render output.
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Mask {
        inverted: bool,
    },
    Blur {
        radius: f32,
        #[serde(default)]
        mode: BlurMode,
    },
    Grain {
        amount: f32,
        seed: u32,
    },
    Opacity {
        amount: f32,
    },
    /// Blends its input onto everything behind it with `mode`, at
    /// `amount` opacity (the Blend Modes node).
    Blend {
        mode: BlendMode,
        amount: f32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum BlurMode {
    #[default]
    Box,
    Gaussian,
    Motion {
        angle: f32,
    },
    RadialSpin,
    RadialZoom,
}

impl BlurMode {
    pub const ALL: [Self; 5] = [
        Self::Box,
        Self::Gaussian,
        Self::Motion { angle: 0.0 },
        Self::RadialSpin,
        Self::RadialZoom,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Gaussian => "Gaussian",
            Self::Motion { .. } => "Motion",
            Self::RadialSpin => "Radial Spin",
            Self::RadialZoom => "Radial Zoom",
        }
    }
}

/// How an image combines with what's behind it. Formulas follow the W3C
/// Compositing and Blending spec, plus Add, Subtract, Divide and Linear
/// Burn as painting apps define them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    Add,
    Subtract,
    Divide,
    Difference,
    Exclusion,
    HardLight,
    SoftLight,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub const ALL: [BlendMode; 20] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::Add,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    pub fn title(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::LinearBurn => "Linear Burn",
            BlendMode::Add => "Add",
            BlendMode::Subtract => "Subtract",
            BlendMode::Divide => "Divide",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::HardLight => "Hard Light",
            BlendMode::SoftLight => "Soft Light",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Luminosity => "Luminosity",
        }
    }

    /// Plain words people use for what this mode does, for search.
    pub fn words(self) -> &'static str {
        match self {
            BlendMode::Normal => "over regular default",
            BlendMode::Multiply => "shadow shade darker tint ink",
            BlendMode::Screen => "light glow brighter lighter",
            BlendMode::Overlay => "contrast texture light shadow",
            BlendMode::Darken => "darker min minimum",
            BlendMode::Lighten => "lighter max maximum",
            BlendMode::ColorDodge => "glow highlight bright light",
            BlendMode::ColorBurn => "dark deep shadow contrast",
            BlendMode::LinearBurn => "dark shadow burn",
            BlendMode::Add => "plus linear dodge glow light additive",
            BlendMode::Subtract => "minus remove take away",
            BlendMode::Divide => "divide brighten remove colour cast",
            BlendMode::Difference => "invert compare difference",
            BlendMode::Exclusion => "invert soft difference",
            BlendMode::HardLight => "contrast strong light",
            BlendMode::SoftLight => "gentle soft contrast light",
            BlendMode::Hue => "colour color hue tint recolour",
            BlendMode::Saturation => "colour color saturation vivid grey",
            BlendMode::Color => "colour color tint recolour colorize",
            BlendMode::Luminosity => "brightness light value luminance",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub start: [u8; 4],
    pub end: [u8; 4],
    pub angle: f32,
}

impl Default for Gradient {
    fn default() -> Self {
        Self {
            start: [35, 48, 77, 255],
            end: [173, 196, 211, 255],
            angle: 90.0,
        }
    }
}

pub enum RenderCommand {
    Drawing(RenderItem),
    Gradient(Gradient),
    BeginEffect,
    EndEffect(Effect),
    EndMask { inverted: bool },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub enabled: bool,
}

/// Animated properties shared by every node, with image parameters where applicable.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeKey {
    pub enabled: bool,
    pub effect: Option<Effect>,
    pub gradient: Option<Gradient>,
}

impl NodeKey {
    pub fn from_node(node: &Node) -> Self {
        Self {
            enabled: node.enabled,
            effect: match node.kind {
                NodeKind::Effect(value) => Some(value),
                _ => None,
            },
            gradient: match node.kind {
                NodeKind::Gradient(value) => Some(value),
                _ => None,
            },
        }
    }

    fn between(self, next: Self, t: f32) -> Self {
        let lerp = |from: f32, to: f32| from + (to - from) * t;
        let effect = match (self.effect, next.effect) {
            (
                Some(Effect::Blur { radius: a, mode }),
                Some(Effect::Blur {
                    radius: b,
                    mode: next,
                }),
            ) => {
                let mode = match (mode, next) {
                    (BlurMode::Motion { angle: a }, BlurMode::Motion { angle: b }) => {
                        let delta = (b - a + 180.0).rem_euclid(360.0) - 180.0;
                        BlurMode::Motion {
                            angle: (a + delta * t).rem_euclid(360.0),
                        }
                    }
                    _ if mode == next => mode,
                    _ => return self,
                };
                Some(Effect::Blur {
                    radius: lerp(a, b),
                    mode,
                })
            }
            (Some(Effect::Grain { amount: a, seed }), Some(Effect::Grain { amount: b, .. })) => {
                Some(Effect::Grain {
                    amount: lerp(a, b),
                    seed,
                })
            }
            (Some(Effect::Opacity { amount: a }), Some(Effect::Opacity { amount: b })) => {
                Some(Effect::Opacity { amount: lerp(a, b) })
            }
            // The mode holds until the next key; its opacity eases.
            (Some(Effect::Blend { mode, amount: a }), Some(Effect::Blend { amount: b, .. })) => {
                Some(Effect::Blend {
                    mode,
                    amount: lerp(a, b),
                })
            }
            _ => self.effect,
        };
        let gradient = self
            .gradient
            .zip(next.gradient)
            .map(|(a, b)| {
                let colour = |from: [u8; 4], to: [u8; 4]| {
                    std::array::from_fn(|i| lerp(from[i] as f32, to[i] as f32).round() as u8)
                };
                Gradient {
                    start: colour(a.start, b.start),
                    end: colour(a.end, b.end),
                    angle: lerp(a.angle, b.angle),
                }
            })
            .or(self.gradient);
        Self {
            enabled: self.enabled,
            effect,
            gradient,
        }
    }
}

/// Output of `from` feeds input port `port` of `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub from: NodeId,
    pub to: NodeId,
    pub port: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Graph {
    nodes: Vec<Option<Node>>,
    links: Vec<Link>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    keys: BTreeMap<NodeId, BTreeMap<u32, NodeKey>>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    matte_drawings: std::collections::BTreeSet<NodeId>,
}

/// Camera-facing artwork keeps a 2D transform and a separate depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StagePose {
    pub transform: Affine,
    pub depth: f32,
}

impl Default for StagePose {
    fn default() -> Self {
        Self {
            transform: Affine::IDENTITY,
            depth: 0.0,
        }
    }
}

/// One drawing to draw, with its accumulated transform.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderItem {
    pub node: NodeId,
    pub column: Id,
    pub drawing: Id,
    /// The pegs' transform (Mesh Warps count as no change here: tools
    /// work on the drawing as drawn).
    pub transform: Affine,
    pub depth: f32,
    pub matte_pass: bool,
    /// Mesh Warps above the drawing that bend it, if any.
    pub warp: Option<std::sync::Arc<crate::warp::Warp>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EvalError {
    /// The graph loops back on itself (or is absurdly deep).
    Cycle(NodeId),
    UnknownNode(NodeId),
}

const MAX_DEPTH: usize = 512;

impl Graph {
    pub fn add(&mut self, name: impl Into<String>, kind: NodeKind) -> NodeId {
        self.nodes.push(Some(Node {
            name: name.into(),
            kind,
            enabled: true,
        }));
        NodeId(self.nodes.len() - 1)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0).and_then(Option::as_ref)
    }

    pub fn keys(&self, id: NodeId) -> Option<&BTreeMap<u32, NodeKey>> {
        self.keys.get(&id)
    }

    pub fn has_key(&self, id: NodeId, frame: u32) -> bool {
        self.keys(id).is_some_and(|keys| keys.contains_key(&frame))
            || self.node(id).is_some_and(|node| match &node.kind {
                NodeKind::Peg(peg) | NodeKind::Skeleton(peg) => peg.has_key(frame),
                NodeKind::Bone(bone) => bone.has_key(frame),
                NodeKind::MeshWarp(warp) => warp.has_key(frame),
                _ => false,
            })
    }

    pub(crate) fn set_keys(&mut self, id: NodeId, keys: BTreeMap<u32, NodeKey>) {
        if keys.is_empty() {
            self.keys.remove(&id);
        } else {
            self.keys.insert(id, keys);
        }
    }

    fn sample_key(&self, id: NodeId, frame: u32) -> Option<NodeKey> {
        let keys = self.keys(id)?;
        let Some((&start, &value)) = keys.range(..=frame).next_back() else {
            return keys.first_key_value().map(|(_, key)| *key);
        };
        if start == frame {
            return Some(value);
        }
        Some(match keys.range(frame..).next() {
            Some((&end, &next)) => {
                value.between(next, (frame - start) as f32 / (end - start) as f32)
            }
            None => value,
        })
    }

    pub fn node_at_frame(&self, id: NodeId, frame: u32) -> Option<Node> {
        let mut node = self.node(id)?.clone();
        if let Some(key) = self.sample_key(id, frame) {
            node.enabled = key.enabled;
            match &mut node.kind {
                NodeKind::Effect(effect) => *effect = key.effect.unwrap_or(*effect),
                NodeKind::Gradient(gradient) => *gradient = key.gradient.unwrap_or(*gradient),
                _ => {}
            }
        }
        Some(node)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0).and_then(Option::as_mut)
    }

    /// Remove a node and its links. Returns them so the removal can be undone.
    pub fn remove(&mut self, id: NodeId) -> Option<(Node, Vec<Link>)> {
        let node = self.nodes.get_mut(id.0)?.take()?;
        let (gone, keep): (Vec<Link>, Vec<Link>) =
            self.links.iter().partition(|l| l.from == id || l.to == id);
        self.links = keep;
        Some((node, gone))
    }

    /// Put a removed node back in its original slot.
    pub fn restore(&mut self, id: NodeId, node: Node, links: &[Link]) {
        if self.nodes.len() <= id.0 {
            self.nodes.resize(id.0 + 1, None);
        }
        self.nodes[id.0] = Some(node);
        for l in links {
            self.connect(l.from, l.to, l.port);
        }
    }

    pub fn next_id(&self) -> NodeId {
        NodeId(self.nodes.len())
    }

    pub fn replace_links(&mut self, links: Vec<Link>) {
        self.links = links;
    }

    pub fn is_drawing_matte(&self, id: NodeId) -> bool {
        self.matte_drawings.contains(&id)
    }

    pub(crate) fn set_drawing_matte(&mut self, id: NodeId, matte: bool) {
        if matte {
            self.matte_drawings.insert(id);
        } else {
            self.matte_drawings.remove(&id);
        }
    }

    pub fn can_connect_to(&self, from: NodeId, to: NodeId, port: usize) -> bool {
        let Some(target) = self.node(to) else {
            return false;
        };
        let mask = matches!(target.kind, NodeKind::Effect(Effect::Mask { .. }));
        if self.is_drawing_matte(from) && !(mask && port == 1) {
            return false;
        }
        if !matches!(target.kind, NodeKind::Composite) && port > usize::from(mask) {
            return false;
        }
        self.can_connect(from, to)
    }

    /// Image and transform ports carry different data. Reject incompatible
    /// endpoints and cycles before committing an interactive connection.
    pub fn can_connect(&self, from: NodeId, to: NodeId) -> bool {
        let (Some(source), Some(target)) = (self.node(from), self.node(to)) else {
            return false;
        };
        let valid = match source.kind {
            NodeKind::Peg(_) | NodeKind::MeshWarp(_) => matches!(
                target.kind,
                NodeKind::Peg(_)
                    | NodeKind::MeshWarp(_)
                    | NodeKind::Skeleton(_)
                    | NodeKind::Drawing { .. }
            ),
            NodeKind::Skeleton(_) => matches!(target.kind, NodeKind::Bone(_)),
            NodeKind::Bone(_) => matches!(
                target.kind,
                NodeKind::Bone(_)
                    | NodeKind::Peg(_)
                    | NodeKind::MeshWarp(_)
                    | NodeKind::Drawing { .. }
            ),
            NodeKind::Composite
            | NodeKind::Drawing { .. }
            | NodeKind::Effect(_)
            | NodeKind::Gradient(_) => matches!(
                target.kind,
                NodeKind::Composite | NodeKind::Effect(_) | NodeKind::Display | NodeKind::Write
            ),
            NodeKind::Display | NodeKind::Write => false,
        };
        if !valid {
            return false;
        }
        if self.bone(to).is_some_and(|bone| bone.smart.is_some())
            && self.bone(from).is_some_and(|bone| {
                bone.smart.is_some() || self.smart_child(from).is_some_and(|other| other != to)
            })
        {
            return false;
        }
        let mut pending = vec![to];
        let mut visited = std::collections::BTreeSet::new();
        while let Some(node) = pending.pop() {
            if node == from {
                return false;
            }
            if visited.insert(node) {
                pending.extend(self.links.iter().filter(|l| l.from == node).map(|l| l.to));
            }
        }
        true
    }

    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.as_ref().map(|n| (NodeId(i), n)))
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Connect `from` into `to`'s input `port`, replacing whatever was there.
    pub fn connect(&mut self, from: NodeId, to: NodeId, port: usize) {
        self.links.retain(|l| !(l.to == to && l.port == port));
        self.links.push(Link { from, to, port });
    }

    pub fn disconnect_inputs(&mut self, to: NodeId) {
        self.links.retain(|l| l.to != to);
    }

    pub fn input(&self, to: NodeId, port: usize) -> Option<NodeId> {
        self.links
            .iter()
            .find(|l| l.to == to && l.port == port)
            .map(|l| l.from)
    }

    /// Inputs sorted by port.
    fn inputs(&self, to: NodeId) -> Vec<(usize, NodeId)> {
        let mut v: Vec<_> = self
            .links
            .iter()
            .filter(|l| l.to == to)
            .map(|l| (l.port, l.from))
            .collect();
        v.sort();
        v
    }

    /// Everything visible from `out` (a Display or Write) on `frame`, back to front.
    pub fn evaluate(
        &self,
        out: NodeId,
        xsheet: &Xsheet,
        frame: u32,
    ) -> Result<Vec<RenderItem>, EvalError> {
        Ok(self
            .render_commands(out, xsheet, frame)?
            .into_iter()
            .filter_map(|command| match command {
                RenderCommand::Drawing(item) if !item.matte_pass => Some(item),
                _ => None,
            })
            .collect())
    }

    pub fn render_commands(
        &self,
        out: NodeId,
        xsheet: &Xsheet,
        frame: u32,
    ) -> Result<Vec<RenderCommand>, EvalError> {
        let mut commands = Vec::new();
        self.eval_image(out, xsheet, frame, 0, false, &mut commands)?;
        Ok(commands)
    }

    /// The full transform a drawing node gets from its pegs on `frame`.
    pub fn world_transform(&self, drawing: NodeId, frame: u32) -> Result<Affine, EvalError> {
        self.world_pose(drawing, frame).map(|pose| pose.transform)
    }

    pub fn world_pose(&self, node: NodeId, frame: u32) -> Result<StagePose, EvalError> {
        match self.input(node, 0) {
            Some(p) => self.eval_transform(p, frame, 0),
            None => Ok(StagePose::default()),
        }
    }

    fn get(&self, id: NodeId, depth: usize) -> Result<&Node, EvalError> {
        if depth > MAX_DEPTH {
            return Err(EvalError::Cycle(id));
        }
        self.node(id).ok_or(EvalError::UnknownNode(id))
    }

    fn eval_image(
        &self,
        id: NodeId,
        xs: &Xsheet,
        frame: u32,
        depth: usize,
        matte_pass: bool,
        out: &mut Vec<RenderCommand>,
    ) -> Result<(), EvalError> {
        let node = self.get(id, depth)?;
        let key = self.sample_key(id, frame);
        let enabled = key.map_or(node.enabled, |key| key.enabled);
        if !enabled && !matches!(node.kind, NodeKind::Effect(_)) {
            return Ok(());
        }
        match &node.kind {
            NodeKind::Display | NodeKind::Write => {
                if let Some(src) = self.input(id, 0) {
                    self.eval_image(src, xs, frame, depth + 1, matte_pass, out)?;
                }
            }
            NodeKind::Composite => {
                // Highest port is furthest back, so it's emitted first.
                for (_, src) in self.inputs(id).into_iter().rev() {
                    self.eval_image(src, xs, frame, depth + 1, matte_pass, out)?;
                }
            }
            NodeKind::Gradient(gradient) => out.push(RenderCommand::Gradient(
                key.and_then(|key| key.gradient).unwrap_or(*gradient),
            )),
            NodeKind::Effect(Effect::Mask { inverted }) => {
                if let Some(image) = self.input(id, 0) {
                    if let Some(matte) = self.input(id, 1).filter(|_| enabled) {
                        let inverted = match key.and_then(|key| key.effect) {
                            Some(Effect::Mask { inverted }) => inverted,
                            _ => *inverted,
                        };
                        out.push(RenderCommand::BeginEffect);
                        self.eval_image(image, xs, frame, depth + 1, matte_pass, out)?;
                        out.push(RenderCommand::BeginEffect);
                        self.eval_image(matte, xs, frame, depth + 1, true, out)?;
                        out.push(RenderCommand::EndMask { inverted });
                    } else {
                        self.eval_image(image, xs, frame, depth + 1, matte_pass, out)?;
                    }
                }
            }
            NodeKind::Effect(effect) => {
                if let Some(src) = self.input(id, 0) {
                    if enabled {
                        out.push(RenderCommand::BeginEffect);
                    }
                    self.eval_image(src, xs, frame, depth + 1, matte_pass, out)?;
                    if enabled {
                        out.push(RenderCommand::EndEffect(
                            key.and_then(|key| key.effect).unwrap_or(*effect),
                        ));
                    }
                }
            }
            NodeKind::Drawing { column } => {
                if !matte_pass && self.is_drawing_matte(id) {
                    return Ok(());
                }
                let col = xs.column(*column);
                if let Some(drawing) = col.filter(|c| c.visible).and_then(|c| c.get(frame)) {
                    let pose = match self.input(id, 0) {
                        Some(p) => self.eval_transform(p, frame, depth + 1)?,
                        None => StagePose::default(),
                    };
                    let warp = match self.input(id, 0) {
                        Some(p) => self.warp_chain(p, frame, depth + 1)?,
                        None => None,
                    };
                    out.push(RenderCommand::Drawing(RenderItem {
                        node: id,
                        column: *column,
                        drawing,
                        transform: pose.transform,
                        depth: pose.depth,
                        matte_pass,
                        warp,
                    }));
                }
            }
            NodeKind::Peg(_)
            | NodeKind::MeshWarp(_)
            | NodeKind::Skeleton(_)
            | NodeKind::Bone(_) => {}
        }
        Ok(())
    }

    /// The Mesh Warps between a drawing and the stage, from the position
    /// node `id` above it upward; None when none of them bends anything.
    fn warp_chain(
        &self,
        id: NodeId,
        frame: u32,
        depth: usize,
    ) -> Result<Option<std::sync::Arc<crate::warp::Warp>>, EvalError> {
        // The chain from the nearest node up to the root.
        let mut chain = Vec::new();
        let mut at = Some(id);
        while let Some(n) = at {
            if chain.len() + depth > MAX_DEPTH {
                return Err(EvalError::Cycle(n));
            }
            chain.push(n);
            at = self.input(n, 0);
        }
        let bound_bone = chain
            .iter()
            .copied()
            .find(|&id| self.bone(id).is_some() && self.enabled_at(id, frame));
        let mut steps = Vec::new();
        let mut outer = Affine::IDENTITY;
        let mut lines = crate::warp::WarpLines::Paths;
        for &n in chain.iter().rev() {
            let node = self.get(n, depth)?;
            let enabled = self
                .sample_key(n, frame)
                .map_or(node.enabled, |key| key.enabled);
            if !enabled {
                continue;
            }
            match &node.kind {
                NodeKind::Peg(p) | NodeKind::Skeleton(p) => {
                    outer = outer.compose(p.transform_at(frame))
                }
                NodeKind::Bone(bone) if bound_bone == Some(n) => {
                    let pose = self.bone_pose_inner(n, frame, depth)?;
                    let radius = self
                        .parent_bone(n)
                        .map_or(0.0, |_| bone.bend_radius.clamp(0.0, pose.length));
                    let field = BoneField {
                        rest_inverse: pose.rest.inverse().unwrap_or(Affine::IDENTITY),
                        parent: pose.parent_skin,
                        child: pose.skin(),
                        radius,
                    };
                    steps.push(crate::warp::WarpStep {
                        outer,
                        field: crate::warp::WarpField::Bone(Box::new(field)),
                    });
                    outer = Affine::IDENTITY;
                }
                NodeKind::MeshWarp(w) => {
                    if let Some(field) = w.field_at(frame) {
                        steps.push(crate::warp::WarpStep { outer, field });
                        if w.lines == crate::warp::WarpLines::Shape {
                            lines = w.lines;
                        }
                        outer = Affine::IDENTITY;
                    }
                }
                _ => {}
            }
        }
        let warp = crate::warp::Warp {
            steps,
            inner: outer,
            lines,
        };
        Ok((!warp.steps.is_empty() && !warp.is_flat()).then(|| std::sync::Arc::new(warp)))
    }

    /// The nearest attached bone's rest endpoints in the drawing's local
    /// coordinates. Intervening pegs still offset the drawing relative to it.
    pub fn drawing_bone_guide(&self, drawing: NodeId, frame: u32) -> Option<[(f32, f32); 2]> {
        let mut id = drawing;
        let mut inner = Affine::IDENTITY;
        for _ in 0..MAX_DEPTH {
            id = self.input(id, 0)?;
            match &self.node(id)?.kind {
                NodeKind::Bone(_) => {
                    let pose = self.bone_pose(id, frame)?;
                    let guide = inner.inverse()?.compose(pose.rest);
                    return Some([guide.apply(0.0, 0.0), guide.apply(pose.length, 0.0)]);
                }
                NodeKind::Peg(peg) => {
                    if self.enabled_at(id, frame) {
                        inner = peg.transform_at(frame).compose(inner);
                    }
                }
                NodeKind::MeshWarp(_) => {}
                _ => return None,
            }
        }
        None
    }

    pub fn bone(&self, id: NodeId) -> Option<&Bone> {
        match &self.node(id)?.kind {
            NodeKind::Bone(bone) => Some(bone),
            _ => None,
        }
    }

    pub fn parent_bone(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.input(id, 0)?;
        self.bone(parent).map(|_| parent)
    }

    fn enabled_at(&self, id: NodeId, frame: u32) -> bool {
        self.node(id).is_some_and(|node| {
            self.sample_key(id, frame)
                .map_or(node.enabled, |key| key.enabled)
        })
    }

    fn smart_child(&self, upper: NodeId) -> Option<NodeId> {
        self.links
            .iter()
            .filter(|link| link.from == upper && link.port == 0)
            .map(|link| link.to)
            .filter(|&id| self.bone(id).is_some_and(|b| b.smart.is_some()))
            .min()
    }

    /// The lower bone storing the active IK target for either member of the pair.
    pub fn smart_controller(&self, id: NodeId, frame: u32) -> Option<NodeId> {
        if !self.enabled_at(id, frame) {
            return None;
        }
        if self.parent_bone(id).is_some_and(|parent| {
            self.enabled_at(parent, frame) && self.smart_child(parent) == Some(id)
        }) {
            return Some(id);
        }
        self.smart_child(id)
            .filter(|&child| self.enabled_at(child, frame))
    }

    /// A smart joint owns two bones, so another solver cannot drive either one.
    pub fn can_make_smart(&self, lower: NodeId) -> bool {
        let Some(upper) = self.parent_bone(lower) else {
            return false;
        };
        self.bone(lower).is_some_and(|b| b.smart.is_none())
            && self.bone(upper).is_some_and(|b| b.smart.is_none())
            && self.smart_child(upper).is_none()
            && self.smart_child(lower).is_none()
    }

    /// Position of the bone relative to its skeleton, before Camera projection.
    pub fn bone_pose(&self, id: NodeId, frame: u32) -> Option<BonePose> {
        self.bone(id)?;
        self.bone_pose_inner(id, frame, 0).ok()
    }

    fn bone_pose_inner(&self, id: NodeId, frame: u32, depth: usize) -> Result<BonePose, EvalError> {
        self.get(id, depth)?;
        let bone = self.bone(id).ok_or(EvalError::UnknownNode(id))?;
        let parent = self.parent_bone(id);
        let (rest_base, posed_base, parent_skin) = if let Some(parent) = parent {
            let parent = self.bone_pose_inner(parent, frame, depth + 1)?;
            // Children start at the stretched tip, without being stretched.
            (
                parent.rest.compose(Affine::translate(parent.length, 0.0)),
                parent.posed.compose(Affine::translate(parent.reach(), 0.0)),
                parent.skin(),
            )
        } else {
            let origin = Affine::translate(bone.origin[0], bone.origin[1]);
            (origin, origin, Affine::IDENTITY)
        };
        let rest_turn = Affine::rotate_deg(bone.rest_angle);
        let mut turn = bone.rest_angle;
        if self.enabled_at(id, frame) {
            turn += bone.rotation.at(frame);
            if let Some(lower) = self.smart_controller(id, frame) {
                if let Some((upper, lower_bone)) = self
                    .parent_bone(lower)
                    .and_then(|upper| self.bone(upper))
                    .zip(self.bone(lower))
                {
                    if let Some(joint) = &lower_bone.smart {
                        let [aim, bend] = joint.solve(frame, upper.length(), lower_bone.length());
                        turn = if lower == id {
                            bend
                        } else {
                            bone.rest_angle + aim
                        };
                    }
                }
            }
        }
        let stretch = if self.enabled_at(id, frame) {
            bone.stretch_at(frame)
        } else {
            1.0
        };
        Ok(BonePose {
            rest: rest_base.compose(rest_turn),
            posed: posed_base.compose(Affine::rotate_deg(turn)),
            parent_skin,
            length: bone.length(),
            scale: bone.stretch_scale(stretch),
        })
    }

    /// Every bone in the rig `root` (see [`Graph::rig_root`]), parents
    /// before their children.
    pub fn rig_bones(&self, root: NodeId) -> Vec<NodeId> {
        let mut bones: Vec<(usize, NodeId)> = self
            .nodes()
            .filter(|(id, _)| self.bone(*id).is_some() && self.rig_root(*id) == root)
            .map(|(id, _)| {
                let mut depth = 0;
                let mut at = id;
                while let Some(p) = self.parent_bone(at) {
                    depth += 1;
                    at = p;
                    if depth > MAX_DEPTH {
                        break;
                    }
                }
                (depth, id)
            })
            .collect();
        bones.sort();
        bones.into_iter().map(|(_, id)| id).collect()
    }

    /// Smart rigs: the bone values that put `dragged`'s tip at `target`
    /// (skeleton space) with the rig's anchors respected: the chain from
    /// the dragged bone up to the nearest locked joint (or the root) is
    /// solved, chains below it that end in a locked anchor are re-solved
    /// so they stay planted, aim anchors turn their bones to their
    /// targets, and every other bone keeps its pose relative to its
    /// parent. None when the rig has no anchors (posing is then bone by
    /// bone) or a smart joint is in the way. Returns (bone, rotation,
    /// stretch) to set.
    pub fn solve_rig(
        &self,
        dragged: NodeId,
        target: [f32; 2],
        frame: u32,
    ) -> Option<Vec<(NodeId, f32, f32)>> {
        use crate::rig::{solve_chain, AnchorKind, Link};
        use std::collections::{BTreeMap, BTreeSet};
        let root = self.rig_root(dragged);
        let bones = self.rig_bones(root);
        let anchored: Vec<NodeId> = bones
            .iter()
            .copied()
            .filter(|&b| self.bone(b).is_some_and(|x| x.anchor.is_some()))
            .collect();
        if anchored.is_empty() {
            return None;
        }
        if bones
            .iter()
            .any(|&b| self.smart_controller(b, frame).is_some())
        {
            return None;
        }
        let poses: BTreeMap<NodeId, crate::rig::BonePose> = bones
            .iter()
            .filter_map(|&b| self.bone_pose(b, frame).map(|p| (b, p)))
            .collect();
        let point = |m: Affine, x: f32| {
            let (px, py) = m.apply(x, 0.0);
            [px, py]
        };
        let angle_of = |m: Affine| m.b.atan2(m.a);
        let locked = |b: NodeId| {
            self.bone(b)
                .and_then(|x| x.anchor.as_ref())
                .filter(|a| a.kind == AnchorKind::Locked)
                .map(|a| [a.target[0].at(frame), a.target[1].at(frame)])
        };
        let link = |b: NodeId| {
            let bone = self.bone(b)?;
            Some(Link {
                length: bone.length(),
                stretch: bone.stretch_range(),
            })
        };
        // New absolute angle and stretch for bones the solve decides.
        let mut fixed: BTreeMap<NodeId, (f32, f32)> = BTreeMap::new();
        let set_from =
            |chain: &[NodeId], joints: &[[f32; 2]], fixed: &mut BTreeMap<NodeId, (f32, f32)>| {
                for (i, &b) in chain.iter().enumerate() {
                    let (a, z) = (joints[i], joints[i + 1]);
                    let len = (z[0] - a[0]).hypot(z[1] - a[1]);
                    let rest = self.bone(b).map_or(1.0, |x| x.length());
                    fixed.insert(b, ((z[1] - a[1]).atan2(z[0] - a[0]), len / rest));
                }
            };
        // The dragged chain: up to a locked joint or the root.
        let mut chain = vec![dragged];
        while let Some(p) = self.parent_bone(*chain.last()?) {
            if locked(p).is_some() {
                break;
            }
            chain.push(p);
        }
        chain.reverse();
        let base = point(poses.get(&chain[0])?.posed, 0.0);
        let mut joints = vec![base];
        for &b in &chain {
            let p = poses.get(&b)?;
            joints.push(point(p.posed, p.reach()));
        }
        let links: Vec<Link> = chain.iter().map(|&b| link(b)).collect::<Option<_>>()?;
        let solved = solve_chain(&joints, &links, target);
        set_from(&chain, &solved, &mut fixed);
        let moved: BTreeSet<NodeId> = chain.iter().copied().collect();
        // Where a decided bone's tip ends up.
        let new_tip = |b: NodeId, fixed: &BTreeMap<NodeId, (f32, f32)>| -> Option<[f32; 2]> {
            let i = chain.iter().position(|&c| c == b)?;
            let _ = fixed;
            Some(solved[i + 1])
        };
        // Locked anchors hanging off the moved chain stay planted.
        for &a in &anchored {
            let Some(goal) = locked(a) else { continue };
            if moved.contains(&a) {
                continue;
            }
            let mut sub = vec![a];
            let mut hook = None;
            while let Some(p) = self.parent_bone(*sub.last()?) {
                if moved.contains(&p) {
                    hook = Some(p);
                    break;
                }
                if locked(p).is_some() {
                    break;
                }
                sub.push(p);
            }
            let Some(hook) = hook else { continue };
            sub.reverse();
            let start = new_tip(hook, &fixed)?;
            let mut js = vec![start];
            for &b in &sub {
                let p = poses.get(&b)?;
                js.push(point(p.posed, p.reach()));
            }
            let ls: Vec<Link> = sub.iter().map(|&b| link(b)).collect::<Option<_>>()?;
            let done = solve_chain(&js, &ls, goal);
            set_from(&sub, &done, &mut fixed);
        }
        // Rebuild every bone's absolute angle (decided, or kept relative
        // to its parent), then aim anchors turn their bones.
        let mut abs: BTreeMap<NodeId, (f32, [f32; 2], f32)> = BTreeMap::new(); // angle, base, reach
        for &b in &bones {
            let p = poses.get(&b)?;
            let old = angle_of(p.posed);
            let bone = self.bone(b)?;
            let parent = self.parent_bone(b);
            let base = match parent.and_then(|q| abs.get(&q)) {
                Some(&(qa, qb, qr)) => [qb[0] + qa.cos() * qr, qb[1] + qa.sin() * qr],
                None => point(p.posed, 0.0),
            };
            let (mut angle, stretch) = match fixed.get(&b) {
                Some(&(a, st)) => (a, st),
                None => {
                    let rel = parent
                        .and_then(|q| poses.get(&q))
                        .map_or(old, |q| old - angle_of(q.posed));
                    let parent_abs = parent.and_then(|q| abs.get(&q)).map_or(0.0, |v| v.0);
                    (
                        if parent.is_some() {
                            parent_abs + rel
                        } else {
                            old
                        },
                        p.scale[0],
                    )
                }
            };
            if let Some(a) = bone.anchor.as_ref().filter(|a| a.kind == AnchorKind::Aim) {
                let t = [a.target[0].at(frame), a.target[1].at(frame)];
                angle = (t[1] - base[1]).atan2(t[0] - base[0]);
            }
            abs.insert(b, (angle, base, bone.length() * stretch));
        }
        // Back to each bone's own rotation value.
        let mut out = Vec::new();
        for &b in &bones {
            let bone = self.bone(b)?;
            let (angle, _, reach) = abs[&b];
            let parent_abs = self
                .parent_bone(b)
                .and_then(|q| abs.get(&q))
                .map_or(0.0, |v| v.0);
            let mut turn = (angle - parent_abs).to_degrees() - bone.rest_angle;
            // Nearest to the current value (no spins through 360).
            let now = bone.rotation.at(frame);
            turn = now + ((turn - now + 180.0).rem_euclid(360.0) - 180.0);
            let stretch = reach / bone.length();
            let changed = (turn - now).abs() > 1e-3
                || (bone.stretch_mode != crate::rig::Stretch::Rigid
                    && (stretch - bone.stretch_at(frame)).abs() > 1e-4);
            if changed {
                out.push((b, turn, stretch));
            }
        }
        Some(out)
    }

    /// The skeleton that owns this bone, or the disconnected chain's root bone.
    pub fn rig_root(&self, mut id: NodeId) -> NodeId {
        for _ in 0..MAX_DEPTH {
            if self.bone(id).is_none() {
                break;
            }
            let Some(parent) = self.input(id, 0) else {
                break;
            };
            id = parent;
        }
        id
    }

    /// Scene-space peg above the bone chain.
    pub fn rig_stage_pose(&self, mut id: NodeId, frame: u32) -> Option<StagePose> {
        for _ in 0..MAX_DEPTH {
            match self.parent_bone(id) {
                Some(parent) => id = parent,
                None => return self.world_pose(id, frame).ok(),
            }
        }
        None
    }

    /// The IK target axes relative to the skeleton peg.
    pub fn smart_target_basis(&self, lower: NodeId, frame: u32) -> Option<Affine> {
        let upper = self.parent_bone(lower)?;
        let bone = self.bone(upper)?;
        let base = if let Some(parent) = self.parent_bone(upper) {
            let pose = self.bone_pose(parent, frame)?;
            pose.posed.compose(Affine::translate(pose.reach(), 0.0))
        } else {
            Affine::translate(bone.origin[0], bone.origin[1])
        };
        Some(base.compose(Affine::rotate_deg(bone.rest_angle)))
    }

    fn eval_transform(&self, id: NodeId, frame: u32, depth: usize) -> Result<StagePose, EvalError> {
        let node = self.get(id, depth)?;
        let parent = match self.input(id, 0) {
            Some(p) => self.eval_transform(p, frame, depth + 1)?,
            None => StagePose::default(),
        };
        Ok(match &node.kind {
            NodeKind::Peg(p) | NodeKind::Skeleton(p)
                if self
                    .sample_key(id, frame)
                    .map_or(node.enabled, |key| key.enabled) =>
            {
                StagePose {
                    transform: parent.transform.compose(p.transform_at(frame)),
                    depth: parent.depth + p.z.at(frame),
                }
            }
            _ => parent,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xsheet::Column;

    #[test]
    fn blur_reads_legacy_box_settings_and_holds_modes_at_keys() {
        let effect = serde_json::from_str::<Effect>(r#"{"Blur":{"radius":12.0}}"#).unwrap();
        assert_eq!(
            effect,
            Effect::Blur {
                radius: 12.0,
                mode: BlurMode::Box
            }
        );
        let key = |radius, mode| NodeKey {
            enabled: true,
            effect: Some(Effect::Blur { radius, mode }),
            gradient: None,
        };
        let start = key(12.0, BlurMode::Box);
        assert_eq!(start.between(key(24.0, BlurMode::Gaussian), 0.5), start);
        assert_eq!(
            start.between(key(24.0, BlurMode::Box), 0.5),
            key(18.0, BlurMode::Box),
        );
    }

    /// Body (back) under a peg, Head (front); a Composite into a Display.
    fn scene() -> (Graph, Xsheet, NodeId, NodeId) {
        let (head_c, body_c) = (Id(1), Id(2));
        let mut xs = Xsheet::default();
        let mut head = Column::new(head_c, "Head");
        head.expose(Id(10), 1, 4);
        let mut body = Column::new(body_c, "Body");
        body.expose(Id(20), 1, 2);
        xs.columns = vec![head, body];

        let mut g = Graph::default();
        let mut peg = PegParams::default();
        peg.x.set_key(1, 0.0);
        peg.x.set_key(3, 20.0);
        let body_peg = g.add("Body-P", NodeKind::Peg(peg));
        let head_n = g.add("Head", NodeKind::Drawing { column: head_c });
        let body_n = g.add("Body", NodeKind::Drawing { column: body_c });
        let comp = g.add("Composite", NodeKind::Composite);
        let display = g.add("Display", NodeKind::Display);
        g.connect(body_peg, body_n, 0);
        g.connect(head_n, comp, 0);
        g.connect(body_n, comp, 1);
        g.connect(comp, display, 0);
        (g, xs, display, body_peg)
    }

    #[test]
    fn back_to_front_with_peg() {
        let (g, xs, display, _) = scene();
        let items = g.evaluate(display, &xs, 2).unwrap();
        let cols: Vec<_> = items.iter().map(|i| i.column).collect();
        assert_eq!(cols, [Id(2), Id(1)]);
        assert_eq!(items[0].transform.tx, 10.0);
        assert_eq!(items[1].transform, Affine::IDENTITY);
    }

    #[test]
    fn empty_cell_draws_nothing() {
        let (g, xs, display, _) = scene();
        let items = g.evaluate(display, &xs, 3).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].column, Id(1));
    }

    #[test]
    fn hidden_column_draws_nothing() {
        let (g, mut xs, display, _) = scene();
        xs.columns[0].visible = false;
        assert_eq!(g.evaluate(display, &xs, 1).unwrap().len(), 1);
    }

    #[test]
    fn cycle_is_an_error() {
        let (mut g, xs, display, body_peg) = scene();
        g.connect(body_peg, body_peg, 0);
        assert!(matches!(
            g.evaluate(display, &xs, 1),
            Err(EvalError::Cycle(_))
        ));
    }

    #[test]
    fn drawing_bone_guide_uses_rest_pose_and_accounts_for_drawing_peg() {
        let (mut graph, _, _, peg) = scene();
        let drawing = graph
            .links()
            .iter()
            .find(|link| link.from == peg)
            .unwrap()
            .to;
        assert_eq!(graph.drawing_bone_guide(drawing, 2), None);
        let bone = graph.add(
            "Bone",
            NodeKind::Bone(Bone {
                origin: [30.0, 40.0],
                length: 80.0,
                rotation: Value::Constant(90.0),
                ..Bone::default()
            }),
        );
        graph.connect(bone, peg, 0);
        assert_eq!(
            graph.drawing_bone_guide(drawing, 2),
            Some([(20.0, 40.0), (100.0, 40.0)])
        );
    }

    #[test]
    fn a_bone_deforms_multiple_drawings_and_emits_no_artwork_of_its_own() {
        let (mut graph, xs, display, _) = scene();
        let drawings: Vec<_> = graph
            .nodes()
            .filter_map(|(id, node)| matches!(node.kind, NodeKind::Drawing { .. }).then_some(id))
            .collect();
        let skeleton = graph.add(
            "Skeleton",
            NodeKind::Skeleton(PegParams {
                x: Value::Constant(30.0),
                ..PegParams::default()
            }),
        );
        let upper = graph.add(
            "Upper",
            NodeKind::Bone(Bone {
                length: 100.0,
                ..Bone::default()
            }),
        );
        let lower = graph.add(
            "Lower",
            NodeKind::Bone(Bone {
                rotation: Value::Constant(90.0),
                ..Bone::default()
            }),
        );
        assert!(graph.can_connect(skeleton, upper));
        assert!(graph.can_connect(upper, lower));
        assert!(!graph.can_connect(lower, display));
        graph.connect(skeleton, upper, 0);
        graph.connect(upper, lower, 0);
        for id in drawings {
            assert!(graph.can_connect(lower, id));
            graph.connect(lower, id, 0);
        }

        let commands = graph.render_commands(display, &xs, 1).unwrap();
        assert_eq!(commands.len(), 2);
        for command in commands {
            let RenderCommand::Drawing(item) = command else {
                panic!("expected drawing")
            };
            let warp = item.warp.unwrap();
            assert_eq!(warp.bend([50.0, 0.0]), [50.0, 0.0]);
            let near_joint = warp.bend([114.0, 0.0]);
            assert!(near_joint[0] > 100.0 && near_joint[0] < 114.0);
            assert!(near_joint[1] > 0.0 && near_joint[1] < 14.0);
            let tip = warp.bend([160.0, 0.0]);
            assert!((tip[0] - 100.0).abs() < 0.001 && (tip[1] - 60.0).abs() < 0.001);
            assert_eq!(warp.outer().tx, 30.0);
            let raster = warp.raster_transform().unwrap().apply(160.0, 0.0);
            assert!((raster.0 - 130.0).abs() < 0.001 && (raster.1 - 60.0).abs() < 0.001);
        }
        assert!(graph.render_commands(lower, &xs, 1).unwrap().is_empty());
    }

    #[test]
    fn smart_joint_reaches_an_animated_target_and_holds_bone_lengths() {
        use crate::rig::SmartJoint;
        let mut graph = Graph::default();
        let upper = graph.add(
            "Upper",
            NodeKind::Bone(Bone {
                length: 100.0,
                ..Bone::default()
            }),
        );
        let lower = graph.add(
            "Lower",
            NodeKind::Bone(Bone {
                length: 80.0,
                ..Bone::default()
            }),
        );
        graph.connect(upper, lower, 0);
        assert!(graph.can_make_smart(lower));
        let mut x = Value::Constant(100.0);
        x.set_key(1, 100.0);
        x.set_key(3, 140.0);
        let NodeKind::Bone(bone) = &mut graph.node_mut(lower).unwrap().kind else {
            unreachable!()
        };
        bone.smart = Some(SmartJoint {
            target: [x, Value::Constant(60.0)],
            flip: false,
        });
        let branch = graph.add("Branch", NodeKind::Bone(Bone::default()));
        graph.connect(upper, branch, 0);
        assert!(!graph.can_make_smart(branch));
        let below = graph.add("Below", NodeKind::Bone(Bone::default()));
        graph.connect(lower, below, 0);
        assert!(!graph.can_make_smart(below));
        assert!(graph.has_key(lower, 3));

        let pose = graph.bone_pose(lower, 2).unwrap();
        let (x, y) = pose.tip();
        assert!((x - 120.0).abs() < 0.001 && (y - 60.0).abs() < 0.001);
        assert!((pose.posed.tx.hypot(pose.posed.ty) - 100.0).abs() < 0.001);
        assert!(((x - pose.posed.tx).hypot(y - pose.posed.ty) - 80.0).abs() < 0.001);
        let saved = serde_json::to_vec(&graph).unwrap();
        let loaded: Graph = serde_json::from_slice(&saved).unwrap();
        assert_eq!(loaded.bone_pose(lower, 2), Some(pose));
    }

    #[test]
    fn remove_and_restore_keeps_ids() {
        let (mut g, xs, display, body_peg) = scene();
        let (node, links) = g.remove(body_peg).unwrap();
        assert!(g.node(body_peg).is_none());
        g.restore(body_peg, node, &links);
        assert_eq!(g.evaluate(display, &xs, 2).unwrap()[0].transform.tx, 10.0);
    }
    /// A leg: hip -> knee -> ankle -> toe, three bones of 100.
    fn leg() -> (Graph, [NodeId; 3]) {
        let mut graph = Graph::default();
        let skeleton = graph.add("Skeleton", NodeKind::Skeleton(PegParams::default()));
        let mut bones = [NodeId(0); 3];
        let mut up = skeleton;
        for (i, b) in bones.iter_mut().enumerate() {
            *b = graph.add(
                format!("Bone {i}"),
                NodeKind::Bone(Bone {
                    length: 100.0,
                    ..Bone::default()
                }),
            );
            graph.connect(up, *b, 0);
            up = *b;
        }
        (graph, bones)
    }

    fn apply(graph: &mut Graph, changes: &[(NodeId, f32, f32)]) {
        for &(id, rotation, stretch) in changes {
            if let Some(NodeKind::Bone(b)) = graph.node_mut(id).map(|n| &mut n.kind) {
                b.rotation = Value::Constant(rotation);
                b.stretch = Value::Constant(stretch);
            }
        }
    }

    fn near(a: (f32, f32), b: [f32; 2], within: f32) -> bool {
        (a.0 - b[0]).hypot(a.1 - b[1]) <= within
    }

    #[test]
    fn smart_rigs_solve_around_locked_anchors() {
        let (mut graph, [hip, knee, foot]) = leg();
        // No anchors: posing stays bone by bone.
        assert!(graph.solve_rig(knee, [150.0, 80.0], 1).is_none());
        // The foot's tip locked a little in from where it is.
        if let Some(NodeKind::Bone(b)) = graph.node_mut(foot).map(|n| &mut n.kind) {
            b.anchor = Some(Box::new(crate::rig::Anchor {
                kind: crate::rig::AnchorKind::Locked,
                target: [Value::Constant(280.0), Value::Constant(0.0)],
            }));
        }
        // Drag the knee in (the leg bends): it gets there, the foot stays
        // planted (the foot bone is 100 long, as far as the anchor is from
        // the knee), the hip stays.
        let changes = graph.solve_rig(knee, [180.0, 0.0], 1).unwrap();
        apply(&mut graph, &changes);
        let pose = |id| graph.bone_pose(id, 1).unwrap();
        assert!(
            near(pose(knee).tip(), [180.0, 0.0], 0.5),
            "{:?}",
            pose(knee).tip()
        );
        assert!(
            near(pose(foot).tip(), [280.0, 0.0], 0.5),
            "{:?}",
            pose(foot).tip()
        );
        assert!(near(pose(hip).posed.apply(0.0, 0.0), [0.0, 0.0], 1e-3));
        // Bones keep their length (rigid).
        let len = |id| {
            let p = pose(id);
            let (a, b) = (p.posed.apply(0.0, 0.0), p.tip());
            (b.0 - a.0).hypot(b.1 - a.1)
        };
        assert!((len(hip) - 100.0).abs() < 0.5 && (len(knee) - 100.0).abs() < 0.5);
    }

    #[test]
    fn stretchy_bones_reach_and_aim_anchors_aim() {
        let (mut graph, [hip, knee, foot]) = leg();
        for id in [hip, knee] {
            if let Some(NodeKind::Bone(b)) = graph.node_mut(id).map(|n| &mut n.kind) {
                b.stretch_mode = crate::rig::Stretch::Simple;
                b.stretch_limits = Some([0.5, 1.5]);
            }
        }
        // The foot aims straight down.
        if let Some(NodeKind::Bone(b)) = graph.node_mut(foot).map(|n| &mut n.kind) {
            b.anchor = Some(Box::new(crate::rig::Anchor {
                kind: crate::rig::AnchorKind::Aim,
                target: [Value::Constant(250.0), Value::Constant(1000.0)],
            }));
        }
        // Pull the knee out of rigid reach (250 > 200): the bones stretch.
        let changes = graph.solve_rig(knee, [250.0, 0.0], 1).unwrap();
        apply(&mut graph, &changes);
        let pose = |id| graph.bone_pose(id, 1).unwrap();
        assert!(
            near(pose(knee).tip(), [250.0, 0.0], 0.5),
            "{:?}",
            pose(knee).tip()
        );
        let (a, b) = (pose(foot).posed.apply(0.0, 0.0), pose(foot).tip());
        assert!(
            (b.0 - a.0).abs() < 1.0 && b.1 > a.1 + 90.0,
            "the foot points down"
        );
    }
}
