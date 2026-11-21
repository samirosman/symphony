//! Node view (read-only for now): the scene graph as Harmony shows it.
//! Data flows top to bottom: pegs -> drawings -> composite -> display.
//! Drag the background to pan. Click a peg or drawing to select its layer.
//! Editing connections comes in M6.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use symphony_core::graph::{NodeId, NodeKind};

use crate::scene::Scene;

#[derive(Default)]
pub struct NodeViewState {
    pan: Vec2,
}

const W: f32 = 110.0;
const H: f32 = 28.0;

fn colour(kind: &NodeKind) -> Color32 {
    match kind {
        NodeKind::Drawing { .. } => Color32::from_rgb(70, 110, 170),
        NodeKind::Peg(_) => Color32::from_rgb(150, 120, 60),
        NodeKind::Composite => Color32::from_rgb(90, 140, 90),
        NodeKind::Display | NodeKind::Write => Color32::from_rgb(120, 80, 140),
    }
}

pub fn ui(ui: &mut egui::Ui, scene: &mut Scene, state: &mut NodeViewState) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    painter.rect_filled(resp.rect, 0.0, Color32::from_gray(38));
    if resp.dragged() {
        state.pan += resp.drag_delta();
    }
    let n = scene.layers.len().max(1) as f32;
    let top = resp.rect.center_top() + Vec2::new(0.0, 30.0) + state.pan;
    let col_x = |i: usize| top.x + (i as f32 - (n - 1.0) / 2.0) * (W + 24.0);

    // Layout: layer i's peg and drawing sit in column i (front layer on the
    // left, matching composite port order); composite and display centred.
    let mut pos: Vec<(NodeId, Pos2)> = Vec::new();
    for (i, l) in scene.layers.iter().enumerate() {
        pos.push((l.peg, Pos2::new(col_x(i), top.y)));
        pos.push((l.node, Pos2::new(col_x(i), top.y + 70.0)));
    }
    pos.push((scene.composite, Pos2::new(top.x, top.y + 150.0)));
    pos.push((scene.display, Pos2::new(top.x, top.y + 220.0)));
    let at = |id: NodeId| pos.iter().find(|(n, _)| *n == id).map(|(_, p)| *p);

    for link in scene.graph.links() {
        let (Some(a), Some(b)) = (at(link.from), at(link.to)) else {
            continue;
        };
        let inputs = scene
            .graph
            .links()
            .iter()
            .filter(|l| l.to == link.to)
            .count()
            .max(1) as f32;
        let port_x = b.x - W / 2.0 + W * (link.port as f32 + 0.5) / inputs;
        let from = a + Vec2::new(0.0, H / 2.0);
        let to = Pos2::new(port_x, b.y - H / 2.0);
        let mid = (to.y - from.y) / 2.0;
        let curve = egui::epaint::CubicBezierShape::from_points_stroke(
            [
                from,
                from + Vec2::new(0.0, mid),
                to - Vec2::new(0.0, mid),
                to,
            ],
            false,
            Color32::TRANSPARENT,
            Stroke::new(1.5, Color32::from_gray(170)),
        );
        painter.add(curve);
        painter.circle_filled(to, 3.0, Color32::from_gray(210));
    }

    let selected = &scene.layers[scene.selected];
    let (sel_peg, sel_node) = (selected.peg, selected.node);
    let mut clicked_layer = None;
    for (id, p) in &pos {
        let Some(node) = scene.graph.node(*id) else {
            continue;
        };
        let r = Rect::from_center_size(*p, Vec2::new(W, H));
        painter.rect_filled(r, 4.0, colour(&node.kind));
        if *id == sel_peg || *id == sel_node {
            painter.rect_stroke(
                r,
                4.0,
                Stroke::new(2.0, Color32::WHITE),
                StrokeKind::Outside,
            );
        }
        let text_col = if node.enabled {
            Color32::WHITE
        } else {
            Color32::from_gray(140)
        };
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            &node.name,
            FontId::proportional(12.0),
            text_col,
        );
        if resp.clicked() && resp.interact_pointer_pos().is_some_and(|q| r.contains(q)) {
            clicked_layer = scene
                .layers
                .iter()
                .position(|l| l.peg == *id || l.node == *id);
        }
    }
    if let Some(i) = clicked_layer {
        scene.selected = i;
    }
}
