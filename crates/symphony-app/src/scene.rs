//! App-side scene state: the core model plus what the views need
//! (layer list, drawings' strokes, palette, current frame, playback).
//! Strokes here are a stand-in until the vector engine (M3) lands.

use std::collections::HashMap;

use symphony_core::anim::Value;
use symphony_core::graph::{Graph, NodeId, NodeKind, PegParams};
use symphony_core::xsheet::Xsheet;

/// Stage size in scene units (like a 1920×1080 camera).
pub const STAGE_W: f32 = 1920.0;
pub const STAGE_H: f32 = 1080.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
}

#[derive(Clone, Debug)]
pub struct Stroke {
    pub points: Vec<StrokePoint>,
    pub colour: usize,
    pub max_width: f32,
    pub min_width: f32,
}

/// A timeline layer = one drawing column + its drawing node + its peg.
pub struct Layer {
    pub name: String,
    pub node: NodeId,
    pub peg: NodeId,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Swatch {
    pub name: &'static str,
    pub rgb: [u8; 3],
}

pub struct Brush {
    pub max_width: f32,
    pub min_width: f32,
    pub colour: usize,
}

pub struct Scene {
    pub xsheet: Xsheet,
    pub graph: Graph,
    pub display: NodeId,
    pub composite: NodeId,
    /// Front to back (index 0 = top of the Timeline = composite port 0).
    pub layers: Vec<Layer>,
    pub drawings: HashMap<(String, String), Vec<Stroke>>,
    pub palette: Vec<Swatch>,
    pub brush: Brush,
    pub frame: u32,
    pub length: u32,
    pub fps: f32,
    pub playing: bool,
    pub selected: usize,
    /// Undo stack of (column, drawing) that got a stroke.
    pub history: Vec<(String, String)>,
}

impl Scene {
    pub fn demo() -> Self {
        let mut graph = Graph::default();
        let composite = graph.add("Composite", NodeKind::Composite);
        let display = graph.add("Display", NodeKind::Display);
        graph.connect(composite, display, 0);

        let mut scene = Scene {
            xsheet: Xsheet::default(),
            graph,
            display,
            composite,
            layers: Vec::new(),
            drawings: HashMap::new(),
            palette: vec![
                Swatch {
                    name: "Line",
                    rgb: [24, 24, 32],
                },
                Swatch {
                    name: "Skin",
                    rgb: [240, 196, 160],
                },
                Swatch {
                    name: "Shirt",
                    rgb: [64, 112, 220],
                },
                Swatch {
                    name: "Hair",
                    rgb: [92, 56, 40],
                },
                Swatch {
                    name: "Shadow",
                    rgb: [120, 120, 150],
                },
                Swatch {
                    name: "Accent",
                    rgb: [220, 70, 60],
                },
            ],
            brush: Brush {
                max_width: 10.0,
                min_width: 0.6,
                colour: 0,
            },
            frame: 1,
            length: 48,
            fps: 24.0,
            playing: false,
            selected: 0,
            history: Vec::new(),
        };
        for name in ["Head", "Body", "BG"] {
            scene.add_layer(name);
        }
        // A little motion so pegs and the timeline have something to show.
        if let Some(NodeKind::Peg(p)) = scene
            .graph
            .node_mut(scene.layers[0].peg)
            .map(|n| &mut n.kind)
        {
            p.x.set_key(1, 0.0);
            p.x.set_key(24, 120.0);
            p.x.set_key(48, 0.0);
        }
        scene
    }

    /// Add a layer at the back.
    pub fn add_layer(&mut self, name: &str) {
        self.xsheet.add_column(name);
        let peg = self
            .graph
            .add(format!("{name}-P"), NodeKind::Peg(PegParams::default()));
        let node = self.graph.add(
            name,
            NodeKind::Drawing {
                column: name.into(),
            },
        );
        self.graph.connect(peg, node, 0);
        let port = self.layers.len();
        self.graph.connect(node, self.composite, port);
        self.layers.push(Layer {
            name: name.into(),
            node,
            peg,
            visible: true,
        });
    }

    pub fn column_name(&self, layer: usize) -> &str {
        &self.layers[layer].name
    }

    /// Drawing exposed on the selected layer at the current frame. Drawing on
    /// an empty cell creates the next numbered drawing there, like Harmony.
    pub fn drawing_for_edit(&mut self) -> (String, String) {
        let col = self.column_name(self.selected).to_owned();
        if let Some(d) = self.xsheet.column(&col).and_then(|c| c.get(self.frame)) {
            return (col, d.to_owned());
        }
        let next = (1..)
            .map(|n: u32| n.to_string())
            .find(|n| !self.drawings.contains_key(&(col.clone(), n.clone())))
            .expect("unbounded");
        if let Some(c) = self.xsheet.column_mut(&col) {
            c.set(self.frame, Some(&next));
        }
        (col, next)
    }

    pub fn undo(&mut self) {
        if let Some(key) = self.history.pop() {
            if let Some(strokes) = self.drawings.get_mut(&key) {
                strokes.pop();
            }
        }
    }

    pub fn peg_params(&self, layer: usize) -> Option<&PegParams> {
        match &self.graph.node(self.layers[layer].peg)?.kind {
            NodeKind::Peg(p) => Some(p),
            _ => None,
        }
    }

    pub fn peg_params_mut(&mut self, layer: usize) -> Option<&mut PegParams> {
        match &mut self.graph.node_mut(self.layers[layer].peg)?.kind {
            NodeKind::Peg(p) => Some(p),
            _ => None,
        }
    }

    pub fn sync_visibility(&mut self) {
        for l in &self.layers {
            if let Some(n) = self.graph.node_mut(l.node) {
                n.enabled = l.visible;
            }
        }
    }
}

/// Set a key on `value` at `frame`, keeping a constant value constant if
/// the user hasn't keyed it yet.
pub fn set_value(value: &mut Value, frame: u32, v: f32, animate: bool) {
    match value {
        Value::Constant(c) if !animate => *c = v,
        _ => value.set_key(frame, v),
    }
}
