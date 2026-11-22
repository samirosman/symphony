//! Node view (read-only for now): the scene graph as Harmony lays it out.
//! Data flows top to bottom: pegs -> drawings -> composite -> display.
//! Drag the background to pan. Click a peg or drawing to select its layer.
//! Editing connections comes in M6.

use std::collections::BTreeMap;

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use symphony_core::graph::{NodeId, NodeKind};

use crate::session::Session;
use symphony_ui::{
    icons::{self, Icon},
    theme, widgets,
};

#[derive(Default)]
pub struct NodeViewState {
    pan: Vec2,
    zoom: Option<f32>,
    library_search: String,
}

const W: f32 = 132.0;
const H: f32 = 36.0;
/// Dot grid spacing; dots use line.subtle so they stay in the background.
const GRID: f32 = 24.0;

/// Accessible names for the graph roles.
fn kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Drawing { .. } => "Drawing",
        NodeKind::Peg(_) => "Peg",
        NodeKind::Composite => "Composite",
        NodeKind::Display => "Display",
        NodeKind::Write => "Write",
    }
}

fn appearance(kind: &NodeKind) -> (Icon, egui::Color32) {
    match kind {
        NodeKind::Peg(_) => (Icon::Transform, theme::NODE_TRANSFORM),
        NodeKind::Drawing { .. } => (Icon::Pencil, theme::NODE_DRAWING),
        NodeKind::Composite => (Icon::Onion, theme::NODE_COMPOSITE),
        NodeKind::Display | NodeKind::Write => (Icon::Fit, theme::NODE_OUTPUT),
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, state: &mut NodeViewState) {
    let width = (s.doc.layers.len().max(1) - 1) as f32 * (W + 24.0) + W;
    let height = 240.0 + H;
    let mut fit = false;
    let mut step = 1.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        fit = icons::button(ui, Icon::Fit, false, true, "Fit graph").clicked();
        if icons::button(ui, Icon::ZoomOut, false, true, "Zoom out").clicked() {
            step = 0.8;
        }
        if icons::button(ui, Icon::Zoom, false, true, "Zoom in").clicked() {
            step = 1.25;
        }
        if let Some(zoom) = state.zoom {
            ui.colored_label(theme::TEXT_SECONDARY, format!("{:.0}%", zoom * 100.0));
        }
    });
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let r = resp.rect;
    painter.rect_filled(r, 0.0, theme::BG_INPUT);
    let fit_zoom = ((r.width() - 32.0) / width)
        .min((r.height() - 24.0) / height)
        .clamp(0.25, 1.0);
    if fit {
        state.zoom = None;
        state.pan = Vec2::ZERO;
    }
    if step != 1.0 {
        state.zoom = Some((state.zoom.unwrap_or(fit_zoom) * step).clamp(0.25, 2.5));
    }
    if resp.dragged() {
        state.pan += resp.drag_delta();
    }
    let zoom = state.zoom.unwrap_or(fit_zoom);
    let node_size = |id| {
        let width = if id == s.doc.composite {
            (W + 32.0).min(width)
        } else {
            W
        };
        Vec2::new(width, H) * zoom
    };
    let grid = (GRID * zoom).max(12.0);
    let midpoint = r.center() + state.pan;
    let start = r.min
        + Vec2::new(
            (midpoint.x - r.min.x).rem_euclid(grid),
            (midpoint.y - r.min.y).rem_euclid(grid),
        );
    let mut y = start.y;
    while y < r.max.y {
        let mut x = start.x;
        while x < r.max.x {
            painter.circle_filled(Pos2::new(x, y), 0.65, theme::LINE_SUBTLE);
            x += grid;
        }
        y += grid;
    }
    let top = midpoint - Vec2::new(0.0, 120.0 * zoom);
    let col_x = |i: usize| top.x + (i as f32 * (W + 24.0) + (W - width) / 2.0) * zoom;

    // Layer i's peg and drawing sit in column i (front layer on the left,
    // matching composite port order). Composite and display are centred.
    let mut pos = BTreeMap::<NodeId, Pos2>::new();
    for (i, l) in s.doc.layers.iter().enumerate() {
        pos.insert(l.peg, Pos2::new(col_x(i), top.y));
        pos.insert(l.node, Pos2::new(col_x(i), top.y + 76.0 * zoom));
    }
    pos.insert(s.doc.composite, Pos2::new(top.x, top.y + 160.0 * zoom));
    pos.insert(s.doc.display, Pos2::new(top.x, top.y + 240.0 * zoom));
    let selected = s.doc.layers.get(s.selected);
    let is_selected = |id| selected.is_some_and(|layer| id == layer.peg || id == layer.node);
    let mut inputs = BTreeMap::<_, Vec<_>>::new();
    for link in s.doc.graph.links() {
        if pos.contains_key(&link.from) && pos.contains_key(&link.to) {
            inputs.entry(link.to).or_default().push(link);
        }
    }
    let mut ports = Vec::new();
    for (target, links) in &mut inputs {
        links.sort_by_key(|link| link.port);
        let b = pos[target];
        for (index, link) in links.iter().enumerate() {
            let a = pos[&link.from];
            let port_x = b.x
                + node_size(*target).x * 0.72 * ((index as f32 + 0.5) / links.len() as f32 - 0.5);
            let from = a + Vec2::new(0.0, node_size(link.from).y / 2.0);
            let to = Pos2::new(port_x, b.y - node_size(*target).y / 2.0);
            let colour = if is_selected(link.from) {
                theme::ACCENT_SELECT
            } else {
                let role = s
                    .doc
                    .graph
                    .node(link.from)
                    .map_or(theme::LINE_CONTROL, |node| appearance(&node.kind).1);
                theme::BG_PANEL.lerp_to_gamma(role, 0.67)
            };
            let transform = matches!(
                s.doc.graph.node(link.from).map(|node| &node.kind),
                Some(NodeKind::Peg(_))
            );
            ports.push((from, colour, transform));
            ports.push((to, colour, transform));
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
                Stroke::new(1.5, colour),
            ));
        }
    }

    let mut clicked_layer = None;
    for (id, p) in &pos {
        let Some(node) = s.doc.graph.node(*id) else {
            continue;
        };
        let r = Rect::from_center_size(*p, node_size(*id));
        let sel = is_selected(*id);
        let hovered = resp.hovered() && resp.hover_pos().is_some_and(|point| r.contains(point));
        let (glyph, role) = appearance(&node.kind);
        let role = if node.enabled {
            role
        } else {
            theme::TEXT_DISABLED
        };
        let surface = if hovered {
            theme::BG_HOVER
        } else {
            theme::BG_PANEL
        };
        let fill = surface.lerp_to_gamma(role, if hovered { 0.19 } else { 0.13 });
        let edge = if sel {
            theme::ACCENT_SELECT
        } else {
            fill.lerp_to_gamma(role, 0.28)
        };
        painter.rect(
            r,
            theme::RADIUS_CONTROL,
            fill,
            Stroke::new(1.0, edge),
            StrokeKind::Inside,
        );

        if zoom >= 0.42 {
            let glyph_size = (16.0 * zoom).clamp(10.0, 20.0);
            let glyph_center = Pos2::new(r.left() + 9.0 * zoom + glyph_size / 2.0, r.center().y);
            icons::paint(
                &painter,
                Rect::from_center_size(glyph_center, Vec2::splat(glyph_size)),
                glyph,
                role,
            );
            let x = glyph_center.x + glyph_size / 2.0 + 7.0 * zoom;
            let name_col = if node.enabled {
                theme::TEXT_PRIMARY
            } else {
                theme::TEXT_DISABLED
            };
            let mut font = theme::body();
            font.size *= zoom.clamp(0.72, 1.25);
            widgets::fitted_text(
                ui,
                &painter,
                Pos2::new(x, r.center().y),
                Align2::LEFT_CENTER,
                &node.name,
                font,
                name_col,
                (r.right() - x - 9.0 * zoom).max(0.0),
            );
        }
        if hovered {
            resp.clone().on_hover_text_at_pointer(format!(
                "{} · {}",
                node.name,
                kind_label(&node.kind)
            ));
        }
        if resp.clicked() && resp.interact_pointer_pos().is_some_and(|q| r.contains(q)) {
            clicked_layer = s
                .doc
                .layers
                .iter()
                .position(|l| l.peg == *id || l.node == *id);
        }
    }
    for (point, colour, transform) in ports {
        let radius = (2.5 * zoom).max(1.5);
        if transform {
            painter.rect_filled(
                Rect::from_center_size(point, Vec2::splat(radius * 2.0)),
                0.5,
                colour,
            );
        } else {
            painter.circle_filled(point, radius, colour);
        }
    }
    if let Some(i) = clicked_layer {
        s.selected = i;
    }
}

/// Documented names and links come from the Harmony 27 Premium reference index.
pub fn library(ui: &mut egui::Ui, s: &Session, state: &mut NodeViewState) {
    egui::Frame::new().inner_margin(egui::Margin::same(theme::CARD_PAD)).show(ui, |ui| {
        ui.add(egui::TextEdit::singleline(&mut state.library_search).hint_text("Find a node"));
        let query = state.library_search.trim().to_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if !s.doc.harmony_nodes.is_empty() {
                ui.heading("Imported scene");
                ui.label("Layers and exposure timing are editable. Cells have no artwork. Source wiring and attributes are retained. Effects and rigs still need conversion.");
                for (name, count) in &s.doc.harmony_nodes {
                    if name.to_lowercase().contains(&query) { ui.label(format!("{name} × {count}")); }
                }
            }
            if !s.doc.harmony_connections.is_empty() {
                ui.collapsing("Source connections", |ui| {
                    for link in &s.doc.harmony_connections {
                        if [link.group.as_str(), link.from.as_str(), link.to.as_str()].iter().any(|name| name.to_lowercase().contains(&query)) {
                            ui.label(format!("{} [{}] → {} [{}]", link.from, link.output, link.to, link.input))
                                .on_hover_text(&link.group);
                        }
                    }
                });
            }
            ui.heading("Symphony nodes");
            for &(id, name) in symphony_io::harmony::NODE_LIBRARY {
                if name.to_lowercase().contains(&query) || id.to_lowercase().contains(&query) {
                    ui.label(format!("{name} ({id})"));
                }
            }
            ui.heading("Harmony reference");
            let mut matches = 0;
            for row in include_str!("../../../../docs/reference/harmony-27-nodes.tsv").lines() {
                let mut fields = row.split('\t');
                let (Some(category), Some(name), Some(path)) = (fields.next(), fields.next(), fields.next()) else {continue};
                if category.contains(&query) || name.to_lowercase().contains(&query) {
                    ui.hyperlink_to(name, format!("https://docs.toonboom.com/help/harmony-27/premium/{path}")).on_hover_text(category);
                    matches += 1;
                }
            }
            if matches == 0 { ui.label("No matching reference entries."); }
        });
    });
}
