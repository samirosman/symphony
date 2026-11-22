//! Editable scene graph, with separate transform and image ports.
use std::collections::BTreeMap;

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use symphony_core::{
    graph::{Effect, Link, Node, NodeId, NodeKind},
    history::Edit,
};

use crate::session::Session;
use symphony_ui::{theme, widgets};

type Input = (NodeId, usize);

enum Drag {
    Pan,
    Node(NodeId, Vec2),
    Cable {
        source: NodeId,
        old_input: Option<Input>,
    },
}

#[derive(Default)]
pub struct NodeViewState {
    pan: Vec2,
    drag: Option<Drag>,
    library_search: String,
    selected: Option<NodeId>,
    properties: Option<(NodeId, Node, Node)>,
    preview: Option<egui::TextureHandle>,
    preview_open: bool,
}

const W: f32 = 136.0;
const H: f32 = 36.0;
const GRID: f32 = 24.0;

pub fn kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Drawing { .. } => "Drawing",
        NodeKind::Peg(_) => "Peg",
        NodeKind::Composite => "Composite",
        NodeKind::Display => "Camera",
        NodeKind::Write => "Write",
        NodeKind::Effect(Effect::Blur { .. }) => "Box Blur",
        NodeKind::Effect(Effect::Grain { .. }) => "Grain",
        NodeKind::Effect(Effect::Opacity { .. }) => "Opacity",
        NodeKind::Gradient(_) => "Gradient",
    }
}

fn effect_nodes() -> [NodeKind; 4] {
    [
        NodeKind::Effect(Effect::Blur { radius: 12.0 }),
        NodeKind::Effect(Effect::Grain {
            amount: 0.16,
            seed: 0,
        }),
        NodeKind::Gradient(Default::default()),
        NodeKind::Effect(Effect::Opacity { amount: 0.5 }),
    ]
}

pub fn addable_nodes() -> impl Iterator<Item = NodeKind> {
    [NodeKind::Peg(Default::default()), NodeKind::Composite]
        .into_iter()
        .chain(effect_nodes())
}

fn node_name(node: &Node) -> &str {
    if matches!(node.kind, NodeKind::Display) && node.name == "Display" {
        "Camera"
    } else {
        &node.name
    }
}

pub fn add_from_search(s: &mut Session, state: &mut NodeViewState, kind: NodeKind) {
    let x = (s.doc.layers.len().saturating_sub(1) as f32 / 2.0 + 1.0) * (W + 26.0);
    let mut y = 105.0;
    while s
        .doc
        .node_positions
        .values()
        .any(|p| (p[0] - x).abs() < W && (p[1] - y).abs() < H + 10.0)
    {
        y += H + 20.0;
    }
    add_node(s, state, kind, [x, y]);
    state.pan = Vec2::new(-x, (105.0 - y).min(0.0));
}

fn add_node(s: &mut Session, state: &mut NodeViewState, kind: NodeKind, position: [f32; 2]) {
    let id = s.doc.graph.next_id();
    s.history.apply(
        &mut s.doc,
        "Add node",
        vec![Edit::AddNode {
            id,
            node: Node {
                name: kind_label(&kind).into(),
                kind,
                enabled: true,
            },
            position,
        }],
    );
    state.selected = Some(id);
}

fn open_properties(s: &Session, state: &mut NodeViewState, id: NodeId) {
    if let Some(node) = s.doc.graph.node(id) {
        let mut draft = node.clone();
        draft.name = node_name(node).to_owned();
        state.properties = Some((id, node.clone(), draft));
    }
}

fn edit_link(s: &mut Session, source: Option<NodeId>, input: Input, old_input: Option<Input>) {
    if let Some(from) = source {
        if !s.doc.graph.can_connect(from, input.0) {
            s.notice =
                Some("These ports can’t connect. Check the data type or look for a loop.".into());
            return;
        }
    }
    let before = s.doc.graph.links().to_vec();
    let mut after = before.clone();
    after.retain(|link| {
        let endpoint = (link.to, link.port);
        endpoint != input && Some(endpoint) != old_input
    });
    if let Some(from) = source {
        after.push(Link {
            from,
            to: input.0,
            port: input.1,
        });
    }
    if before != after {
        let was_custom = s.doc.custom_wiring;
        let name = if source.is_some() {
            "Connect nodes"
        } else {
            "Disconnect nodes"
        };
        s.history.apply(
            &mut s.doc,
            name,
            vec![Edit::RewireNodes {
                before,
                after,
                was_custom,
            }],
        );
    }
}

fn curve(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    let bend = ((to.y - from.y).abs() * 0.5).clamp(16.0, 120.0);
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
        [from, from + Vec2::Y * bend, to - Vec2::Y * bend, to],
        false,
        egui::Color32::TRANSPARENT,
        stroke,
    ));
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, state: &mut NodeViewState) {
    let mut add = None;
    ui.horizontal(|ui| {
        ui.menu_button("Add node", |ui| {
            if ui.button("Drawing").clicked() {
                s.add_layer();
                ui.close();
            }
            for kind in [NodeKind::Peg(Default::default()), NodeKind::Composite] {
                if ui.button(kind_label(&kind)).clicked() {
                    add = Some(kind);
                    ui.close();
                }
            }
            ui.separator();
            for kind in effect_nodes() {
                if ui.button(kind_label(&kind)).clicked() {
                    add = Some(kind);
                    ui.close();
                }
            }
        });
        if ui.button("Reset pan").clicked() {
            state.pan = Vec2::ZERO;
        }
        if ui
            .add_enabled(
                state.selected.and_then(|id| s.doc.graph.node(id)).is_some(),
                egui::Button::new("Properties"),
            )
            .clicked()
        {
            if let Some(id) = state.selected {
                open_properties(s, state, id);
            }
        }
        if ui.button("Render preview").clicked() {
            state.preview_open = true;
            state.preview = None;
        }
    });
    canvas(ui, s, state, add);
    properties(ui.ctx(), s, state);
    preview(ui.ctx(), s, state);
}

fn properties(ctx: &egui::Context, s: &mut Session, state: &mut NodeViewState) {
    let Some((id, before, draft)) = &mut state.properties else {
        return;
    };
    if !s.doc.graph.node(*id).is_some_and(|node| node == before) {
        state.properties = None;
        return;
    }
    let mut open = true;
    let mut apply = false;
    egui::Window::new(format!("{} properties", kind_label(&draft.kind)))
        .id(egui::Id::unique("node-properties"))
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut draft.name);
            });
            ui.checkbox(&mut draft.enabled, "Enabled");
            ui.separator();
            match &mut draft.kind {
                NodeKind::Peg(peg) => {
                    peg_value(ui, "Position X", &mut peg.x, s.frame, s.animate);
                    peg_value(ui, "Position Y", &mut peg.y, s.frame, s.animate);
                    peg_value(ui, "Rotation", &mut peg.rotation, s.frame, s.animate);
                    for (label, value) in
                        [("Scale X", &mut peg.scale_x), ("Scale Y", &mut peg.scale_y)]
                    {
                        let mut draft = value
                            .clone()
                            .unwrap_or(symphony_core::anim::Value::Constant(1.0));
                        if peg_value(ui, label, &mut draft, s.frame, s.animate) {
                            *value = Some(draft);
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("Pivot");
                        ui.add(egui::DragValue::new(&mut peg.pivot.0).prefix("X "));
                        ui.add(egui::DragValue::new(&mut peg.pivot.1).prefix("Y "));
                    });
                }
                NodeKind::Drawing { column } => {
                    if let Some(source) = s.doc.xsheet.column(*column) {
                        ui.label(format!("Exposure column  {}", source.name));
                        ui.label(format!(
                            "Current drawing  {}",
                            source
                                .get(s.frame)
                                .and_then(|drawing| s.doc.drawings.get(&drawing))
                                .map_or("Empty", |d| &d.name)
                        ));
                    }
                }
                NodeKind::Composite => {
                    let count = s
                        .doc
                        .graph
                        .links()
                        .iter()
                        .filter(|link| link.to == *id)
                        .count();
                    ui.label(format!("{count} connected inputs"));
                    ui.label("Lower port numbers are in front.");
                }
                NodeKind::Display => {
                    let input = s
                        .doc
                        .graph
                        .input(*id, 0)
                        .and_then(|from| s.doc.graph.node(from));
                    ui.label(format!(
                        "Input  {}",
                        input.map_or("Disconnected", node_name)
                    ));
                    ui.label(format!(
                        "Stage  {} × {}",
                        s.doc.settings.width, s.doc.settings.height
                    ));
                }
                NodeKind::Effect(Effect::Blur { radius }) => {
                    ui.add(
                        egui::Slider::new(radius, 0.0..=128.0)
                            .text("Radius")
                            .suffix(" px"),
                    );
                }
                NodeKind::Effect(Effect::Grain { amount, seed }) => {
                    ui.add(egui::Slider::new(amount, 0.0..=1.0).text("Amount"));
                    ui.horizontal(|ui| {
                        ui.label("Seed");
                        ui.add(egui::DragValue::new(seed));
                    });
                }
                NodeKind::Effect(Effect::Opacity { amount }) => {
                    let mut percent = *amount * 100.0;
                    if ui
                        .add(
                            egui::Slider::new(&mut percent, 0.0..=100.0)
                                .text("Opacity")
                                .suffix("%"),
                        )
                        .changed()
                    {
                        *amount = percent / 100.0;
                    }
                }
                NodeKind::Gradient(gradient) => {
                    ui.horizontal(|ui| {
                        super::colour_picker::swatch(ui, &mut gradient.start);
                        ui.label("Start");
                        super::colour_picker::swatch(ui, &mut gradient.end);
                        ui.label("End");
                    });
                    ui.add(
                        egui::Slider::new(&mut gradient.angle, 0.0..=360.0)
                            .text("Angle")
                            .suffix("°"),
                    );
                }
                _ => {}
            }
            apply = ui
                .add_enabled(
                    before != draft && !draft.name.trim().is_empty(),
                    egui::Button::new("Apply"),
                )
                .clicked();
        });
    if apply {
        s.history.apply(
            &mut s.doc,
            "Change node properties",
            vec![Edit::SetNodeProperties {
                id: *id,
                before: before.clone(),
                after: draft.clone(),
            }],
        );
        *before = draft.clone();
        state.preview = None;
    }
    if !open {
        state.properties = None;
    }
}

fn peg_value(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut symphony_core::anim::Value,
    frame: u32,
    animate: bool,
) -> bool {
    use symphony_core::anim::Value;
    let mut number = value.at(frame);
    let changed = ui
        .horizontal(|ui| {
            ui.label(label);
            ui.add(egui::DragValue::new(&mut number).speed(0.1))
                .changed()
        })
        .inner;
    if changed {
        match value {
            Value::Constant(current) if !animate => *current = number,
            _ => value.set_key(frame, number),
        }
    }
    changed
}

fn preview(ctx: &egui::Context, s: &Session, state: &mut NodeViewState) {
    if !state.preview_open {
        return;
    }
    egui::Window::new("Display render")
        .open(&mut state.preview_open)
        .collapsible(false)
        .show(ctx, |ui| {
            let refresh = ui.button("Render current frame").clicked();
            if refresh || state.preview.is_none() {
                let w = s.doc.settings.width.max(1);
                let h = s.doc.settings.height.max(1);
                let scale = (640.0 / w.max(h) as f32).min(1.0);
                let (w, h) = (
                    (w as f32 * scale).round().max(1.0) as u32,
                    (h as f32 * scale).round().max(1.0) as u32,
                );
                if let Some(image) = symphony_render::render_frame(&s.doc, s.frame, w, h) {
                    state.preview = Some(ctx.load_texture(
                        "node-display-preview",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [w as usize, h as usize],
                            image.as_raw(),
                        ),
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            if let Some(texture) = &state.preview {
                ui.image(texture);
            } else {
                ui.label("Couldn’t render the current frame.");
            }
        });
}

fn canvas(ui: &mut egui::Ui, s: &mut Session, state: &mut NodeViewState, add: Option<NodeKind>) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let viewport = resp.rect;
    let origin = viewport.center_top() + state.pan;
    let grid_origin = viewport.min
        + Vec2::new(
            (origin.x - viewport.left()).rem_euclid(GRID),
            state.pan.y.rem_euclid(GRID),
        );
    painter.rect_filled(viewport, 0.0, theme::BG_INPUT);
    let mut y = grid_origin.y;
    while y < viewport.bottom() {
        let mut x = grid_origin.x;
        while x < viewport.right() {
            painter.circle_filled(Pos2::new(x, y), 0.65, theme::LINE_SUBTLE);
            x += GRID;
        }
        y += GRID;
    }

    let mut positions = BTreeMap::new();
    let row_scale = ((viewport.height() - H) / 291.0).clamp(0.62, 1.0);
    let middle = s.doc.layers.len().saturating_sub(1) as f32 / 2.0;
    for (index, layer) in s.doc.layers.iter().enumerate() {
        let x = (index as f32 - middle) * (W + 26.0);
        positions.insert(layer.peg, Vec2::new(x, 43.0 * row_scale));
        positions.insert(layer.node, Vec2::new(x, 120.0 * row_scale));
    }
    positions.insert(s.doc.composite, Vec2::new(0.0, 209.0 * row_scale));
    positions.insert(s.doc.display, Vec2::new(0.0, 291.0 * row_scale));

    let mut cards = BTreeMap::new();
    let mut inputs = BTreeMap::<Input, Pos2>::new();
    let mut outputs = BTreeMap::new();
    for (id, node) in s.doc.graph.nodes() {
        let mut position = s
            .doc
            .node_positions
            .get(&id)
            .copied()
            .map(|p| Vec2::new(p[0], p[1]))
            .or_else(|| positions.get(&id).copied())
            .unwrap_or_else(|| Vec2::new(W + 36.0, 43.0 + id.0 as f32 * 58.0));
        if let Some(Drag::Node(moving, delta)) = state.drag {
            if id == moving {
                position += delta;
            }
        }
        let center = origin + position;
        let ports = if matches!(node.kind, NodeKind::Composite) {
            let mut used: Vec<_> = s
                .doc
                .graph
                .links()
                .iter()
                .filter(|l| l.to == id)
                .map(|l| l.port)
                .collect();
            used.sort_unstable();
            let free = (0..)
                .find(|port| used.binary_search(port).is_err())
                .unwrap_or(0);
            used.push(free);
            used.sort_unstable();
            used
        } else if matches!(node.kind, NodeKind::Gradient(_)) {
            vec![]
        } else {
            vec![0]
        };
        let width = W.max(ports.len() as f32 * 16.0 + 16.0);
        let card = Rect::from_center_size(center, Vec2::new(width, H));
        cards.insert(id, (card, position));
        let count = ports.len() as f32;
        for (index, port) in ports.into_iter().enumerate() {
            let x = card.left() + width * ((index as f32 + 0.5) / count);
            inputs.insert((id, port), Pos2::new(x, card.top()));
        }
        if !matches!(node.kind, NodeKind::Display | NodeKind::Write) {
            outputs.insert(id, card.center_bottom());
        }
    }

    if let Some(kind) = add {
        let mut center = viewport.center() + Vec2::new(W * 0.65, 0.0);
        while cards.values().any(|(card, _)| {
            card.expand(12.0)
                .intersects(Rect::from_center_size(center, Vec2::new(W, H)))
        }) {
            center.y += H + 20.0;
        }
        let position = center - origin;
        add_node(s, state, kind, [position.x, position.y]);
        ui.ctx().request_repaint();
    }

    let input_at = |p: Pos2| {
        inputs
            .iter()
            .find(|(_, at)| at.distance(p) <= 9.0)
            .map(|(&key, _)| key)
    };
    let output_at = |p: Pos2| {
        outputs
            .iter()
            .find(|(_, at)| at.distance(p) <= 9.0)
            .map(|(&key, _)| key)
    };
    let node_at = |p| {
        cards
            .iter()
            .rev()
            .find(|(_, (card, _))| card.contains(p))
            .map(|(&id, _)| id)
    };
    if resp.drag_started() {
        if let Some(point) = ui.input(|i| i.pointer.press_origin()) {
            state.drag = if let Some(input) = input_at(point) {
                s.doc
                    .graph
                    .input(input.0, input.1)
                    .map(|source| Drag::Cable {
                        source,
                        old_input: Some(input),
                    })
            } else if let Some(source) = output_at(point) {
                Some(Drag::Cable {
                    source,
                    old_input: None,
                })
            } else if let Some(node) = node_at(point) {
                Some(Drag::Node(node, Vec2::ZERO))
            } else {
                Some(Drag::Pan)
            };
        }
    }
    if resp.dragged() {
        match state.drag.as_mut() {
            Some(Drag::Pan) => state.pan += resp.drag_delta(),
            Some(Drag::Node(_, delta)) => *delta += resp.drag_delta(),
            _ => {}
        }
        ui.ctx().request_repaint();
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.drag = None;
    }
    if resp.drag_stopped() {
        match state.drag.take() {
            Some(Drag::Node(id, delta)) if delta.length_sq() > 0.0 => {
                if let Some((_, position)) = cards.get(&id) {
                    // cards reflect the drag offset accumulated before this frame.
                    let before = s.doc.node_positions.get(&id).copied();
                    s.history.apply(
                        &mut s.doc,
                        "Move node",
                        vec![Edit::MoveNode {
                            id,
                            before,
                            after: [position.x, position.y],
                        }],
                    );
                }
            }
            Some(Drag::Cable { source, old_input }) => {
                if let Some(point) = resp
                    .interact_pointer_pos()
                    .filter(|point| viewport.contains(*point))
                {
                    if let Some(input) = input_at(point) {
                        if old_input != Some(input) {
                            edit_link(s, Some(source), input, old_input);
                        }
                    } else if node_at(point).is_none() && output_at(point).is_none() {
                        if let Some(input) = old_input {
                            edit_link(s, None, input, None);
                        }
                    }
                }
            }
            _ => {}
        }
        ui.ctx().request_repaint();
    }
    if resp.clicked() || resp.double_clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            state.selected = node_at(pos);
            if let Some(id) = state.selected {
                if resp.double_clicked() {
                    open_properties(s, state, id);
                }
                if let Some(layer) = s
                    .doc
                    .layers
                    .iter()
                    .position(|l| l.node == id || l.peg == id)
                {
                    s.selected = layer;
                }
            }
        }
    }

    for link in s.doc.graph.links() {
        let input = (link.to, link.port);
        if matches!(state.drag, Some(Drag::Cable { old_input: Some(old), .. }) if old == input) {
            continue;
        }
        if let (Some(&from), Some(&to)) = (outputs.get(&link.from), inputs.get(&input)) {
            curve(&painter, from, to, Stroke::new(1.5, theme::LINE_CONTROL));
        }
    }
    if let Some(Drag::Cable { source, .. }) = state.drag {
        if let (Some(&from), Some(to)) =
            (outputs.get(&source), ui.input(|i| i.pointer.latest_pos()))
        {
            curve(&painter, from, to, Stroke::new(2.0, theme::ACCENT_SELECT));
        }
    }
    for (id, (card, _)) in &cards {
        let Some(node) = s.doc.graph.node(*id) else {
            continue;
        };
        let selected = state.selected == Some(*id)
            || s.doc
                .layers
                .get(s.selected)
                .is_some_and(|l| l.peg == *id || l.node == *id);
        let edge = if selected {
            theme::ACCENT_SELECT
        } else {
            theme::LINE_STRONG
        };
        painter.rect(
            *card,
            theme::RADIUS_CONTROL,
            theme::BG_RAISED,
            Stroke::new(1.0, edge),
            StrokeKind::Inside,
        );
        widgets::fitted_text(
            ui,
            &painter,
            card.center(),
            Align2::CENTER_CENTER,
            node_name(node),
            theme::body(),
            theme::TEXT_PRIMARY,
            card.width() - 16.0,
        );
    }

    let hover = resp.hover_pos();
    let mut menu_input = ui.data(|d| d.get_temp::<Input>(resp.id.with("menu-input")));
    if resp.secondary_clicked() {
        menu_input = resp.interact_pointer_pos().and_then(input_at);
        ui.data_mut(|d| {
            d.remove::<Input>(resp.id.with("menu-input"));
            if let Some(input) = menu_input {
                d.insert_temp(resp.id.with("menu-input"), input);
            }
        });
    }
    let mut disconnect = None;
    if let Some(input) = menu_input.filter(|&(node, port)| s.doc.graph.input(node, port).is_some())
    {
        resp.context_menu(|ui| {
            if ui.button("Disconnect input").clicked() {
                disconnect = Some(input);
                ui.close();
            }
        });
    }
    if let Some(input) = disconnect {
        edit_link(s, None, input, None);
    }

    for (input, point) in inputs {
        let node = s.doc.graph.node(input.0);
        let transform =
            node.is_some_and(|n| matches!(n.kind, NodeKind::Peg(_) | NodeKind::Drawing { .. }));
        let compatible = match state.drag {
            Some(Drag::Cable { source, .. }) => s.doc.graph.can_connect(source, input.0),
            _ => true,
        };
        let filled = s.doc.graph.input(input.0, input.1).is_some();
        let highlighted = hover.is_some_and(|p| p.distance(point) <= 9.0) && compatible;
        socket(&painter, point, transform, filled, highlighted);
    }
    for (id, point) in outputs {
        let transform = s
            .doc
            .graph
            .node(id)
            .is_some_and(|n| matches!(n.kind, NodeKind::Peg(_)));
        let highlighted = hover.is_some_and(|p| p.distance(point) <= 9.0);
        socket(&painter, point, transform, true, highlighted);
    }
}

fn socket(painter: &egui::Painter, point: Pos2, transform: bool, filled: bool, hover: bool) {
    let radius = if hover { 5.0 } else { 3.5 };
    let color = if hover {
        theme::ACCENT_SELECT
    } else {
        theme::LINE_CONTROL
    };
    let fill = if filled { color } else { theme::BG_INPUT };
    let stroke = Stroke::new(1.0, color);
    if transform {
        painter.rect(
            Rect::from_center_size(point, Vec2::splat(radius * 2.0)),
            1.0,
            fill,
            stroke,
            StrokeKind::Inside,
        );
    } else {
        painter.circle(point, radius, fill, stroke);
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
                    let name = if name == "Display" { "Camera" } else { name };
                    ui.label(format!("{name} ({id})"));
                }
            }
            for kind in effect_nodes() {
                let name = kind_label(&kind);
                if name.to_lowercase().contains(&query) {
                    ui.label(name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    #[test]
    fn camera_opens_properties_on_double_click_and_search_adds_a_node() {
        let mut harness = Harness::builder()
            .with_size(Vec2::new(760.0, 560.0))
            .build_ui_state(
                |ui, (session, state, origin)| {
                    *origin = ui.available_rect_before_wrap().center_top();
                    canvas(ui, session, state, None);
                },
                (Session::new_scene(), NodeViewState::default(), Pos2::ZERO),
            );
        harness.run_steps(2);
        let (session, _, origin) = harness.state();
        let camera = session.doc.display;
        let pos = *origin + Vec2::Y * 291.0;
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(pos));
        harness.run_steps(1);
        for (i, pressed) in [true, false, true, false].into_iter().enumerate() {
            harness.input_mut().time = Some(1.0 + i as f64 * 0.03);
            harness.input_mut().events.extend([
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            harness.run_steps(1);
            if i == 1 {
                assert_eq!(harness.state().1.selected, Some(camera));
            }
        }
        let (session, state, _) = harness.state_mut();
        let (selected, _, draft) = state.properties.as_ref().unwrap();
        assert_eq!(*selected, camera);
        assert_eq!(draft.name, "Camera");

        add_from_search(session, state, NodeKind::Gradient(Default::default()));
        let added = state.selected.unwrap();
        assert!(matches!(
            session.doc.graph.node(added).unwrap().kind,
            NodeKind::Gradient(_)
        ));
        session.undo();
        assert!(session.doc.graph.node(added).is_none());
    }

    #[test]
    fn drag_nodes_and_rewire_inputs_with_undo() {
        let mut h = Harness::builder()
            .with_size(Vec2::new(760.0, 560.0))
            .build_ui_state(
                |ui, (session, state, origin, add)| {
                    *origin = ui.available_rect_before_wrap().center_top();
                    canvas(ui, session, state, add.take());
                },
                (
                    Session::new_scene(),
                    NodeViewState::default(),
                    Pos2::ZERO,
                    None::<NodeKind>,
                ),
            );
        h.run_steps(2);
        let (session, _, origin, _) = h.state();
        let (peg, drawing) = (session.doc.layers[0].peg, session.doc.layers[0].node);
        let (composite, display) = (session.doc.composite, session.doc.display);
        let point = |x, y| *origin + Vec2::new(x, y);
        let peg_center = point(0.0, 43.0);
        let moved_peg = point(-110.0, 58.0);
        let peg_output = moved_peg + Vec2::Y * (H / 2.0);
        let drawing_input = point(0.0, 120.0 - H / 2.0);
        let composite_output = point(0.0, 209.0 + H / 2.0);
        let display_input = point(0.0, 291.0 - H / 2.0);
        let blank = point(190.0, 110.0);
        let drag = |h: &mut Harness<'_, _>, from, to| {
            for (pos, pressed) in [(from, Some(true)), (to, None), (to, Some(false))] {
                h.input_mut().events.push(egui::Event::PointerMoved(pos));
                if let Some(pressed) = pressed {
                    h.input_mut().events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                h.run_steps(1);
            }
        };

        drag(&mut h, peg_center, moved_peg);
        assert_eq!(h.state().0.doc.node_positions[&peg], [-110.0, 58.0]);
        drag(&mut h, drawing_input, blank);
        assert_eq!(h.state().0.doc.graph.input(drawing, 0), None);
        drag(&mut h, peg_output, drawing_input);
        assert_eq!(h.state().0.doc.graph.input(drawing, 0), Some(peg));

        h.state_mut().3 = Some(NodeKind::Composite);
        let extra = h.state().0.doc.graph.next_id();
        h.run_steps(2);
        let [x, y] = h.state().0.doc.node_positions[&extra];
        let extra_output = blank + Vec2::new(x - 190.0, y - 110.0 + H / 2.0);
        drag(&mut h, extra_output, display_input);
        assert_eq!(h.state().0.doc.graph.input(display, 0), Some(extra));
        drag(&mut h, composite_output, display_input);
        assert_eq!(h.state().0.doc.graph.input(display, 0), Some(composite));

        let session = &mut h.state_mut().0;
        session.history.undo(&mut session.doc);
        assert_eq!(session.doc.graph.input(display, 0), Some(extra));
        session.history.redo(&mut session.doc);
        assert_eq!(session.doc.graph.input(display, 0), Some(composite));

        let extra_input = extra_output - Vec2::Y * H;
        drag(&mut h, display_input, extra_input);
        let session = &mut h.state_mut().0;
        assert_eq!(session.doc.graph.input(display, 0), None);
        assert_eq!(session.doc.graph.input(extra, 0), Some(composite));
        session.history.undo(&mut session.doc);
        assert_eq!(session.doc.graph.input(display, 0), Some(composite));
        assert_eq!(session.doc.graph.input(extra, 0), None);
    }
}
