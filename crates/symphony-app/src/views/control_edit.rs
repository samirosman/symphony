//! Rig control widgets in the Camera view (Harmony's Master Controller
//! slider over an Attribute Controller). A Control shows its widget when
//! it is selected, or when the selected node or layer is under it. With
//! the Transform tool:
//!
//! - Drag the knob, or press anywhere on the track, to move the control
//!   position. In Animate mode this keys the control position on this
//!   frame, never the rig's own values. One drag is one undo step.
//! - Ctrl while dragging snaps the knob to the nearest pose (the rest
//!   pose at 0 included).
//! - Double-click a pose dot to put the knob on that pose.
//!
//! The widget sits at its position in the space of the node it follows
//! (the Control's own parent unless a peg or bone is set), so it moves
//! with the character. Shift+C hides it with the other rig controls.

use eframe::egui::{self, Pos2, Stroke};
use symphony_core::control::Control;
use symphony_core::graph::{NodeId, NodeKind};
use symphony_core::history::Edit;
use symphony_core::math::Affine;
use symphony_ui::theme;

use crate::session::{Session, Tool};

/// How near (screen points) a press must be to take the widget.
const GRAB: f32 = 10.0;

#[derive(Default)]
pub struct ControlEdit {
    drag: Option<NodeId>,
}

/// One widget as shown this frame.
struct Shown {
    id: NodeId,
    control: Control,
    /// The track's ends on screen: the range's Min, then its Max.
    low: Pos2,
    high: Pos2,
}

impl Shown {
    /// The screen place of control position `at`.
    fn place(&self, at: f32) -> Pos2 {
        let [a, b] = self.control.range;
        let t = if (b - a).abs() < 1e-6 {
            0.5
        } else {
            (at - a) / (b - a)
        };
        self.low + (self.high - self.low) * t.clamp(0.0, 1.0)
    }

    /// The control position nearest to screen point `p`.
    fn position(&self, p: Pos2) -> f32 {
        let along = self.high - self.low;
        let t = (p - self.low).dot(along) / along.length_sq().max(1e-6);
        let [a, b] = self.control.range;
        self.control.clamp(a + (b - a) * t.clamp(0.0, 1.0))
    }

    /// The rest pose and every pose inside the range.
    fn stops(&self) -> Vec<f32> {
        let mut stops: Vec<f32> = std::iter::once(0.0)
            .chain(self.control.poses.iter().map(|p| p.at))
            .filter(|at| at.is_finite() && self.control.clamp(*at) == *at)
            .collect();
        stops.sort_by(f32::total_cmp);
        stops.dedup();
        stops
    }

    fn distance(&self, p: Pos2) -> f32 {
        let along = self.high - self.low;
        let t = ((p - self.low).dot(along) / along.length_sq().max(1e-6)).clamp(0.0, 1.0);
        p.distance(self.low + along * t)
    }
}

/// The Controls to show: the selected one, and every Control above the
/// selected node or layer.
fn controls(s: &Session) -> Vec<NodeId> {
    let graph = &s.doc.graph;
    let start = s
        .selected_node
        .or_else(|| s.doc.layers.get(s.selected).map(|layer| layer.node));
    let mut found = Vec::new();
    let mut at = start;
    for _ in 0..256 {
        let Some(id) = at else { break };
        if matches!(graph.node(id).map(|n| &n.kind), Some(NodeKind::Control(_))) {
            found.push(id);
        }
        at = graph.input(id, 0);
    }
    found
}

/// Screen space of the widget of Control `id`: the node it follows, or
/// the Control's own parent.
fn widget_space(s: &Session, id: NodeId, follow: Option<NodeId>, view: Affine) -> Option<Affine> {
    let graph = &s.doc.graph;
    let frame = s.frame;
    let pose = match follow.filter(|&n| graph.node(n).is_some()) {
        Some(bone) if graph.bone(bone).is_some() => {
            let stage = graph.rig_stage_pose(bone, frame)?;
            symphony_core::graph::StagePose {
                transform: stage.transform.compose(graph.bone_pose(bone, frame)?.posed),
                depth: stage.depth,
            }
        }
        Some(node) => graph.node_space(node, frame).ok()?,
        None => graph.world_pose(id, frame).ok()?,
    };
    let plane = s.doc.project_plane(frame, pose.depth)?;
    Some(view.compose(plane).compose(pose.transform))
}

fn shown(s: &Session, view: Affine) -> Vec<Shown> {
    controls(s)
        .into_iter()
        .filter_map(|id| {
            let NodeKind::Control(control) = &s.doc.graph.node(id)?.kind else {
                return None;
            };
            let w = &control.widget;
            let m = widget_space(s, id, w.follow, view)?;
            let half = w.size.max(1.0) * 0.5;
            // The stage's y points down: a vertical slider has Max on top.
            let (dx, dy) = if w.vertical {
                (0.0, -half)
            } else {
                (half, 0.0)
            };
            let [x, y] = w.position;
            let end = |sign: f32| {
                let (px, py) = m.apply(x + dx * sign, y + dy * sign);
                Pos2::new(px, py)
            };
            Some(Shown {
                id,
                control: control.clone(),
                low: end(-1.0),
                high: end(1.0),
            })
        })
        .collect()
}

fn paint(painter: &egui::Painter, s: &Session, w: &Shown) {
    let ink = super::rig_edit::colour_ink(w.control.widget.colour);
    let ink = if s.selected_node == Some(w.id) {
        theme::ACCENT_SELECT
    } else {
        ink
    };
    // A dark edge under the line keeps it readable on a light stage.
    painter.line_segment(
        [w.low, w.high],
        Stroke::new(4.0, theme::BG_BASE.gamma_multiply(0.85)),
    );
    painter.line_segment([w.low, w.high], Stroke::new(1.5, ink));
    for at in w.stops() {
        let p = w.place(at);
        painter.circle_filled(p, 4.0, theme::BG_BASE);
        if at == 0.0 {
            painter.circle_stroke(p, 2.5, Stroke::new(1.5, ink));
        } else {
            painter.circle_filled(p, 2.5, ink);
        }
    }
    let knob = w.place(w.control.position_at(s.frame));
    painter.circle_filled(knob, 7.5, theme::BG_BASE);
    painter.circle_filled(knob, 5.5, ink);
    if w.control.widget.show_label {
        if let Some(node) = s.doc.graph.node(w.id) {
            // Ends swapped: the label sits above a horizontal track and
            // left of a vertical one.
            super::rig_edit::paint_label(painter, w.high, w.low, &node.name, ink);
        }
    }
}

/// Set Control `id` to `position` on this frame (a key in Animate mode).
fn set_position(s: &mut Session, id: NodeId, position: f32) {
    let Some(before) = s.doc.graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    let NodeKind::Control(control) = &mut after.kind else {
        return;
    };
    let position = control.clamp(position);
    super::rig_edit::set_value(&mut control.value, position, s.frame, s.animate);
    if before != after {
        s.history.apply_merged(
            &mut s.doc,
            "Move control",
            &format!("control-drag-{}", id.0),
            Edit::SetNodeProperties { id, before, after },
        );
    }
}

/// Draw the rig control widgets and take presses on them. Returns true
/// while a widget has the pointer: the rig handles and the Transform
/// tool then leave it alone.
pub fn ui(
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    s: &mut Session,
    state: &mut ControlEdit,
    view: Affine,
) -> bool {
    let widgets = if s.show_rig_controls {
        shown(s, view)
    } else {
        Vec::new()
    };
    for w in &widgets {
        paint(painter, s, w);
    }
    if s.tool != Tool::Transform || widgets.is_empty() {
        finish(s, state);
        return false;
    }
    let hit = |p: Pos2| {
        widgets
            .iter()
            .map(|w| (w, w.distance(p)))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(w, _)| w)
    };
    let pointer = ui.input(|i| i.pointer.press_origin().or(i.pointer.hover_pos()));
    let over = pointer.and_then(hit).is_some();
    if response.double_clicked() {
        let stop = response.interact_pointer_pos().and_then(|p| {
            let w = hit(p)?;
            let at = w
                .stops()
                .into_iter()
                .find(|&at| w.place(at).distance(p) <= GRAB)?;
            Some((w.id, at))
        });
        if let Some((id, at)) = stop {
            set_position(s, id, at);
            s.end_gesture();
        }
    } else if response.drag_started_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary)
    {
        state.drag = pointer.and_then(hit).map(|w| w.id);
    }
    if let Some(w) = state
        .drag
        .and_then(|id| widgets.iter().find(|w| w.id == id))
    {
        if let Some(p) = response.interact_pointer_pos().or(pointer) {
            let mut at = w.position(p);
            if ui.input(|i| i.modifiers.command) {
                if let Some(&near) = w
                    .stops()
                    .iter()
                    .min_by(|a, b| (*a - at).abs().total_cmp(&(*b - at).abs()))
                {
                    at = near;
                }
            }
            set_position(s, w.id, at);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    } else if over {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let busy = state.drag.is_some() || over;
    if !ui.input(|i| i.pointer.primary_down()) {
        finish(s, state);
    }
    busy
}

pub fn finish(s: &mut Session, state: &mut ControlEdit) {
    if state.drag.take().is_some() {
        s.end_gesture();
    }
}
