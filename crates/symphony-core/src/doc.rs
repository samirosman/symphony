//! The scene document (ENGINEERING E2.1): settings, layers, the xsheet, the
//! node graph, drawings and the palette. Everything a scene file holds.
//! UI state (current frame, selection, tool) is not here (E3.5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph::{Graph, Link, Node, NodeId, NodeKind, PegParams};
use crate::id::{Id, IdGen};
use crate::palette::{Palette, Swatch};
use crate::stroke::Drawing;
use crate::xsheet::{Column, Xsheet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub length: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            width: 1920,
            height: 1080,
            fps: 24.0,
            length: 48,
        }
    }
}

/// A timeline layer: its column plus its drawing node and peg.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer {
    pub column: Id,
    pub node: NodeId,
    pub peg: NodeId,
}

/// Everything needed to put a removed layer back exactly as it was.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerSnapshot {
    pub index: usize,
    pub layer: Layer,
    pub column: Column,
    pub node: Node,
    pub peg: Node,
    pub links: Vec<Link>,
}

/// A connection in the imported source graph, including its enclosing group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceConnection {
    pub group: String,
    pub from: String,
    pub output: usize,
    pub to: String,
    pub input: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    /// Source node counts for the Harmony import report.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub harmony_nodes: BTreeMap<String, usize>,
    /// Original scene XML retains source wiring, attributes and unknown modules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harmony_source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub harmony_connections: Vec<SourceConnection>,
    pub settings: Settings,
    pub xsheet: Xsheet,
    pub graph: Graph,
    /// Front to back: index 0 is the top of the Timeline and composite port 0.
    pub layers: Vec<Layer>,
    pub drawings: BTreeMap<Id, Drawing>,
    pub palette: Palette,
    /// Brush tips and paper grains used by textured strokes, by content
    /// key, so a scene carries its brushes with it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub textures: BTreeMap<u64, crate::stroke::Texture>,
    pub composite: NodeId,
    pub display: NodeId,
    pub ids: IdGen,
    /// Bumped by every applied edit (E3.6). Not saved.
    #[serde(skip)]
    pub version: u64,
}

impl Scene {
    pub fn new(settings: Settings) -> Self {
        let mut graph = Graph::default();
        let composite = graph.add("Composite", NodeKind::Composite);
        let display = graph.add("Display", NodeKind::Display);
        graph.connect(composite, display, 0);
        let mut ids = IdGen::default();
        let swatch = |ids: &mut IdGen, name: &str, rgb| Swatch {
            id: ids.fresh(),
            name: name.into(),
            rgb,
            protected: false,
        };
        let palette = Palette {
            name: "Scene palette".into(),
            swatches: vec![
                swatch(&mut ids, "Line", [24, 24, 32]),
                swatch(&mut ids, "Skin", [240, 196, 160]),
                swatch(&mut ids, "Shirt", [64, 112, 220]),
                swatch(&mut ids, "Hair", [92, 56, 40]),
                swatch(&mut ids, "Shadow", [120, 120, 150]),
                swatch(&mut ids, "Accent", [220, 70, 60]),
            ],
        };
        Scene {
            harmony_nodes: BTreeMap::new(),
            harmony_source: None,
            harmony_connections: Vec::new(),
            settings,
            xsheet: Xsheet::default(),
            graph,
            layers: Vec::new(),
            drawings: BTreeMap::new(),
            palette,
            textures: BTreeMap::new(),
            composite,
            display,
            ids,
            version: 0,
        }
    }

    /// Build the parts of a new layer without inserting it. Use
    /// `Edit::AddLayer` to insert it undoably.
    pub fn make_layer(&mut self, name: &str, index: usize) -> LayerSnapshot {
        let column = Column::new(self.ids.fresh(), name);
        let peg = Node {
            name: format!("{name}-P"),
            kind: NodeKind::Peg(PegParams::default()),
            enabled: true,
        };
        let node = Node {
            name: name.into(),
            kind: NodeKind::Drawing { column: column.id },
            enabled: true,
        };
        // Reserve graph slots now so the IDs are stable from the start.
        let peg_id = self.graph.add("", NodeKind::Display);
        let node_id = self.graph.add("", NodeKind::Display);
        self.graph.remove(peg_id);
        self.graph.remove(node_id);
        LayerSnapshot {
            index,
            layer: Layer {
                column: column.id,
                node: node_id,
                peg: peg_id,
            },
            column,
            node,
            peg,
            links: vec![Link {
                from: peg_id,
                to: node_id,
                port: 0,
            }],
        }
    }

    /// Everything needed to remove layer `index` undoably, without removing it.
    pub fn snapshot_layer(&self, index: usize) -> Option<LayerSnapshot> {
        let layer = *self.layers.get(index)?;
        Some(LayerSnapshot {
            index,
            layer,
            column: self.xsheet.column(layer.column)?.clone(),
            node: self.graph.node(layer.node)?.clone(),
            peg: self.graph.node(layer.peg)?.clone(),
            links: vec![Link {
                from: layer.peg,
                to: layer.node,
                port: 0,
            }],
        })
    }

    pub(crate) fn insert_layer(&mut self, s: &LayerSnapshot) {
        let at = s.index.min(self.layers.len());
        self.xsheet
            .columns
            .insert(at.min(self.xsheet.columns.len()), s.column.clone());
        self.graph.restore(s.layer.peg, s.peg.clone(), &[]);
        self.graph.restore(s.layer.node, s.node.clone(), &s.links);
        self.layers.insert(at, s.layer);
        self.rewire();
    }

    pub(crate) fn take_layer(&mut self, index: usize) -> Option<LayerSnapshot> {
        let layer = *self.layers.get(index)?;
        let pos = self
            .xsheet
            .columns
            .iter()
            .position(|c| c.id == layer.column)?;
        let column = self.xsheet.columns.remove(pos);
        let (node, mut links) = self.graph.remove(layer.node)?;
        let (peg, peg_links) = self.graph.remove(layer.peg)?;
        links.extend(peg_links);
        links.retain(|l| l.to != self.composite);
        links.dedup();
        self.layers.remove(index);
        self.rewire();
        Some(LayerSnapshot {
            index,
            layer,
            column,
            node,
            peg,
            links,
        })
    }

    /// Composite inputs follow layer order (port 0 = front).
    pub fn rewire(&mut self) {
        self.graph.disconnect_inputs(self.composite);
        for (port, l) in self.layers.iter().enumerate() {
            self.graph.connect(l.node, self.composite, port);
        }
    }

    pub fn column(&self, layer: usize) -> Option<&Column> {
        self.xsheet.column(self.layers.get(layer)?.column)
    }

    pub fn layer_of(&self, column: Id) -> Option<usize> {
        self.layers.iter().position(|l| l.column == column)
    }

    /// Drawing exposed on `layer` at `frame`.
    pub fn exposed(&self, layer: usize, frame: u32) -> Option<Id> {
        self.column(layer)?.get(frame)
    }

    pub fn peg(&self, layer: usize) -> Option<&PegParams> {
        match &self.graph.node(self.layers.get(layer)?.peg)?.kind {
            NodeKind::Peg(p) => Some(p),
            _ => None,
        }
    }

    /// Next free drawing name in a column: "1", "2", ... like Harmony.
    pub fn next_drawing_name(&self, column: Id) -> String {
        let max = self
            .drawings
            .values()
            .filter(|d| d.column == column)
            .filter_map(|d| d.name.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        (max + 1).to_string()
    }

    /// A fresh empty drawing for `column` (not inserted).
    pub fn make_drawing(&mut self, column: Id) -> Drawing {
        Drawing {
            id: self.ids.fresh(),
            column,
            name: self.next_drawing_name(column),
            strokes: Vec::new(),
            fills: Vec::new(),
        }
    }

    pub fn swatch_rgb(&self, swatch: Id) -> [u8; 3] {
        self.palette.get(swatch).map(|s| s.rgb).unwrap_or_default()
    }
}
