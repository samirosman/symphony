//! Node graph. Data flows top to bottom like Harmony's Node view:
//! Peg -> Drawing -> Composite -> Display / Write.
//!
//! Node IDs are slots that are never reused, so removing a node leaves a
//! gap instead of renumbering the others (E2.2).

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
    /// What the Camera view shows.
    Display,
    /// Render output.
    Write,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub enabled: bool,
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
}

/// One drawing to draw, with its accumulated transform.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderItem {
    pub node: NodeId,
    pub column: Id,
    pub drawing: Id,
    pub transform: Affine,
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

    /// Image and transform ports carry different data. Reject incompatible
    /// endpoints and cycles before committing an interactive connection.
    pub fn can_connect(&self, from: NodeId, to: NodeId) -> bool {
        let (Some(source), Some(target)) = (self.node(from), self.node(to)) else {
            return false;
        };
        let valid = match source.kind {
            NodeKind::Peg(_) => matches!(target.kind, NodeKind::Peg(_) | NodeKind::Drawing { .. }),
            NodeKind::Composite | NodeKind::Drawing { .. } => matches!(
                target.kind,
                NodeKind::Composite | NodeKind::Display | NodeKind::Write
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
        let mut items = Vec::new();
        self.eval_image(out, xsheet, frame, 0, &mut items)?;
        Ok(items)
    }

    /// The full transform a drawing node gets from its pegs on `frame`.
    pub fn world_transform(&self, drawing: NodeId, frame: u32) -> Result<Affine, EvalError> {
        match self.input(drawing, 0) {
            Some(p) => self.eval_transform(p, frame, 0),
            None => Ok(Affine::IDENTITY),
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
        out: &mut Vec<RenderItem>,
    ) -> Result<(), EvalError> {
        let node = self.get(id, depth)?;
        if !node.enabled {
            return Ok(());
        }
        match &node.kind {
            NodeKind::Display | NodeKind::Write => {
                if let Some(src) = self.input(id, 0) {
                    self.eval_image(src, xs, frame, depth + 1, out)?;
                }
            }
            NodeKind::Composite => {
                // Highest port is furthest back, so it's emitted first.
                for (_, src) in self.inputs(id).into_iter().rev() {
                    self.eval_image(src, xs, frame, depth + 1, out)?;
                }
            }
            NodeKind::Drawing { column } => {
                let col = xs.column(*column);
                if let Some(drawing) = col.filter(|c| c.visible).and_then(|c| c.get(frame)) {
                    let transform = match self.input(id, 0) {
                        Some(p) => self.eval_transform(p, frame, depth + 1)?,
                        None => Affine::IDENTITY,
                    };
                    out.push(RenderItem {
                        node: id,
                        column: *column,
                        drawing,
                        transform,
                    });
                }
            }
            NodeKind::Peg(_) => {} // a peg carries no image
        }
        Ok(())
    }

    fn eval_transform(&self, id: NodeId, frame: u32, depth: usize) -> Result<Affine, EvalError> {
        let node = self.get(id, depth)?;
        let parent = match self.input(id, 0) {
            Some(p) => self.eval_transform(p, frame, depth + 1)?,
            None => Affine::IDENTITY,
        };
        Ok(match &node.kind {
            NodeKind::Peg(p) if node.enabled => parent.compose(p.transform_at(frame)),
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
