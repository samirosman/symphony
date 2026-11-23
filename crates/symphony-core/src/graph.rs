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
            (Some(Effect::Blur { radius: a }), Some(Effect::Blur { radius: b })) => {
                Some(Effect::Blur { radius: lerp(a, b) })
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
    pub transform: Affine,
    pub depth: f32,
    pub matte_pass: bool,
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
            || self
                .node(id)
                .is_some_and(|node| matches!(&node.kind, NodeKind::Peg(peg) if peg.has_key(frame)))
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

    pub(crate) fn replace_links(&mut self, links: Vec<Link>) {
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
            NodeKind::Peg(_) => matches!(target.kind, NodeKind::Peg(_) | NodeKind::Drawing { .. }),
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
                    out.push(RenderCommand::Drawing(RenderItem {
                        node: id,
                        column: *column,
                        drawing,
                        transform: pose.transform,
                        depth: pose.depth,
                        matte_pass,
                    }));
                }
            }
            NodeKind::Peg(_) => {} // a peg carries no image
        }
        Ok(())
    }

    fn eval_transform(&self, id: NodeId, frame: u32, depth: usize) -> Result<StagePose, EvalError> {
        let node = self.get(id, depth)?;
        let parent = match self.input(id, 0) {
            Some(p) => self.eval_transform(p, frame, depth + 1)?,
            None => StagePose::default(),
        };
        Ok(match &node.kind {
            NodeKind::Peg(p)
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
    fn remove_and_restore_keeps_ids() {
        let (mut g, xs, display, body_peg) = scene();
        let (node, links) = g.remove(body_peg).unwrap();
        assert!(g.node(body_peg).is_none());
        g.restore(body_peg, node, &links);
        assert_eq!(g.evaluate(display, &xs, 2).unwrap()[0].transform.tx, 10.0);
    }
}
