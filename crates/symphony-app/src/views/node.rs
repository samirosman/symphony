//! Node view (read-only for now): the scene graph as Harmony lays it out.
//! Data flows top to bottom: pegs -> drawings -> composite -> display.
//! Drag the background to pan. Click a peg or drawing to select its layer.
//! Editing connections comes in M6.

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use symphony_core::graph::{NodeId, NodeKind};

use crate::scene::Scene;
use symphony_ui::theme;

#[derive(Default)]
pub struct NodeViewState {
    pan: Vec2,
}

const W: f32 = 112.0;
const H: f32 = 32.0;

/// Node type is shown as text, not colour (D4.1: colour only for meaning).
fn kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Drawing { .. } => "Drawing",
        NodeKind::Peg(_) => "Peg",
        NodeKind::Composite => "Composite",
        NodeKind::Display => "Display",
        NodeKind::Write => "Write",
    }
}

pub fn ui(ui: &mut egui::Ui, scene: &mut Scene, state: &mut NodeViewState) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    painter.rect_filled(resp.rect, 0.0, theme::BG_PANEL);
    if resp.dragged() {
        state.pan += resp.drag_delta();
    }
    let n = scene.layers.len().max(1) as f32;
    let top = resp.rect.center_top() + Vec2::new(0.0, 32.0) + state.pan;
    let col_x = |i: usize| top.x + (i as f32 - (n - 1.0) / 2.0) * (W + 24.0);

    // Layer i's peg and drawing sit in column i (front layer on the left,
    // matching composite port order). Composite and display are centred.
    let mut pos: Vec<(NodeId, Pos2)> = Vec::new();
    for (i, l) in scene.layers.iter().enumerate() {
        pos.push((l.peg, Pos2::new(col_x(i), top.y)));
        pos.push((l.node, Pos2::new(col_x(i), top.y + 72.0)));
    }
    pos.push((scene.composite, Pos2::new(top.x, top.y + 152.0)));
    pos.push((scene.display, Pos2::new(top.x, top.y + 224.0)));
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
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [
                from,
                from + Vec2::new(0.0, mid),
                to - Vec2::new(0.0, mid),
                to,
            ],
            false,
            egui::Color32::TRANSPARENT,
            Stroke::new(1.5, theme::TEXT_SECONDARY),
        ));
        painter.circle_filled(to, 3.0, theme::TEXT_PRIMARY);
    }

    let selected = &scene.layers[scene.selected];
    let (sel_peg, sel_node) = (selected.peg, selected.node);
    let mut clicked_layer = None;
    for (id, p) in &pos {
        let Some(node) = scene.graph.node(*id) else {
            continue;
        };
        let r = Rect::from_center_size(*p, Vec2::new(W, H));
        let sel = *id == sel_peg || *id == sel_node;
        painter.rect_filled(r, theme::RADIUS_CONTROL as f32, theme::BG_RAISED);
        let outline = if sel {
            Stroke::new(2.0, theme::ACCENT_SELECT)
        } else {
            Stroke::new(1.0, theme::LINE_STRONG)
        };
        painter.rect_stroke(r, theme::RADIUS_CONTROL as f32, outline, StrokeKind::Inside);
        let name_col = if node.enabled {
            theme::TEXT_PRIMARY
        } else {
            theme::TEXT_DISABLED
        };
        painter.text(
            r.center() - Vec2::new(0.0, 6.0),
            Align2::CENTER_CENTER,
            &node.name,
            theme::body(),
            name_col,
        );
        painter.text(
            r.center() + Vec2::new(0.0, 8.0),
            Align2::CENTER_CENTER,
            kind_label(&node.kind),
            theme::dense(),
            theme::TEXT_SECONDARY,
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
