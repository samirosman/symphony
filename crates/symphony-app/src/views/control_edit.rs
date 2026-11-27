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
//! - Setup mode (the panel at the top left, with the Control selected):
//!   the knob turns white and is a test position that makes no key. Add
//!   pose puts a pose at the knob; while the knob is on a pose, a change
//!   to a bone's rotation or stretch, or a peg's position, rotation or
//!   scale, under the Control goes into that pose instead of the rig's
//!   keys. Done puts the knob back on its keyed position.
//!
//! The widget sits at its position in the space of the node it follows
//! (the Control's own parent unless a peg or bone is set), so it moves
//! with the character. Shift+C hides it with the other rig controls.

use std::collections::BTreeMap;

use eframe::egui::{self, Pos2, Stroke};
use symphony_core::control::{channel_values, restore_channels, Control};
use symphony_core::graph::{NodeId, NodeKind};
use symphony_core::history::Edit;
use symphony_core::math::Affine;
use symphony_ui::theme;

use crate::session::{Session, Tool};

/// How near (screen points) a press must be to take the widget.
const GRAB: f32 = 10.0;
/// The knob's radius on screen, its dark edge included.
const KNOB: f32 = 7.5;

#[derive(Default)]
pub struct ControlEdit {
    drag: Option<NodeId>,
    /// A press dropped the Control selection: the rest of that press
    /// goes to the Select and Transform tools.
    dropped: bool,
}

/// A Control in Setup mode (Harmony's Setup Mode).
pub struct Setup {
    pub control: NodeId,
    /// The bones and pegs under the Control as last seen.
    seen: BTreeMap<NodeId, NodeKind>,
    /// `History::made` when last seen.
    made: u64,
    /// A rig change off a pose was refused: the panel says why.
    refused: bool,
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
    let mut found: Vec<NodeId> = s.control_setup.iter().map(|setup| setup.control).collect();
    let mut at = start;
    for _ in 0..256 {
        let Some(id) = at else { break };
        if matches!(graph.node(id).map(|n| &n.kind), Some(NodeKind::Control(_)))
            && !found.contains(&id)
        {
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
    let testing = s
        .control_setup
        .as_ref()
        .is_some_and(|setup| setup.control == w.id);
    let at = s
        .doc
        .graph
        .control_shown_position(w.id, s.frame)
        .unwrap_or_else(|| w.control.position_at(s.frame));
    let knob = w.place(at);
    painter.circle_filled(knob, KNOB, theme::BG_BASE);
    // Setup mode: a white knob, a test position with no key.
    painter.circle_filled(knob, 5.5, if testing { theme::ON_ACCENT } else { ink });
    if w.control.widget.show_label {
        if let Some(node) = s.doc.graph.node(w.id) {
            // Ends swapped: the label sits above a horizontal track and
            // left of a vertical one, clear of the knob.
            super::rig_edit::paint_label_clear(painter, w.high, w.low, &node.name, ink, KNOB + 4.0);
        }
    }
}

/// Set Control `id` to `position` on this frame (a key in Animate mode).
/// In Setup mode it moves the test position only.
fn set_position(s: &mut Session, id: NodeId, position: f32) {
    let Some(before) = s.doc.graph.node(id).cloned() else {
        return;
    };
    if s.control_setup
        .as_ref()
        .is_some_and(|setup| setup.control == id)
    {
        if let NodeKind::Control(control) = &before.kind {
            let test = Some((id, control.clamp(position)));
            s.doc.graph.set_control_test(test);
        }
        return;
    }
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
    // The rest of a press that dropped the selection belongs to the
    // Select and Transform tools.
    if state.dropped {
        state.dropped = ui.input(|i| i.pointer.primary_down());
        if state.dropped {
            return false;
        }
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
    // A selected Control owns the pointer, as a selected bone does: the
    // layer's transform box stays hidden. A press that no widget takes
    // drops the selection and goes on to the Select and Transform tools
    // in the same click, so one click picks another object.
    let control_selected = s
        .selected_node
        .is_some_and(|id| widgets.iter().any(|w| w.id == id));
    if control_selected && !over && state.drag.is_none() && response.is_pointer_button_down_on() {
        if ui.input(|i| i.pointer.primary_pressed()) {
            s.selected_node = None;
            state.dropped = true;
            return false;
        }
        return true;
    }
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
    let busy = state.drag.is_some() || over || control_selected;
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

/// The bones and pegs under Control `id`, as they are now.
fn rig_under(s: &Session, id: NodeId) -> BTreeMap<NodeId, NodeKind> {
    let graph = &s.doc.graph;
    graph
        .nodes()
        .filter(|(_, node)| {
            matches!(
                node.kind,
                NodeKind::Bone(_) | NodeKind::Peg(_) | NodeKind::Skeleton(_)
            )
        })
        .filter(|&(node, _)| {
            let mut at = graph.input(node, 0);
            for _ in 0..256 {
                match at {
                    Some(n) if n == id => return true,
                    Some(n) => at = graph.input(n, 0),
                    None => break,
                }
            }
            false
        })
        .map(|(node, n)| (node, n.kind.clone()))
        .collect()
}

/// Put Control `id` in Setup mode, the knob on its keyed position.
pub fn start_setup(s: &mut Session, id: NodeId) {
    let Some(position) = s.doc.graph.control_shown_position(id, s.frame) else {
        return;
    };
    s.end_gesture();
    s.doc.graph.set_control_test(Some((id, position)));
    s.control_setup = Some(Setup {
        control: id,
        seen: rig_under(s, id),
        made: s.history.made(),
        refused: false,
    });
}

/// Leave Setup mode: the knob goes back to its keyed position.
pub fn end_setup(s: &mut Session) {
    s.control_setup = None;
    s.doc.graph.set_control_test(None);
}

/// The Control in Setup mode, its test position, and whether a pose
/// sits there.
fn setup_state(s: &Session) -> Option<(NodeId, f32, bool)> {
    let (id, at) = s.doc.graph.control_test()?;
    let NodeKind::Control(control) = &s.doc.graph.node(id)?.kind else {
        return None;
    };
    let on_pose = at != 0.0 && control.poses.iter().any(|p| p.at == at);
    Some((id, at, on_pose))
}

/// Add a pose at the test position (one undo step).
pub fn add_pose(s: &mut Session) {
    let Some((id, at, false)) = setup_state(s).filter(|&(_, at, _)| at != 0.0) else {
        return;
    };
    let Some(before) = s.doc.graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    // The new pose holds what the rig shows at the knob now.
    if let NodeKind::Control(control) = &mut after.kind {
        control.add_pose(at);
    }
    s.history.apply(
        &mut s.doc,
        "Add pose",
        vec![Edit::SetNodeProperties { id, before, after }],
    );
    record_setup(s, false);
}

/// In Setup mode, move each change to the rig under the Control into
/// the pose at the knob, as part of the edit's own undo step. It waits
/// until the edit ends (`pointer_down` false), so a drag records once.
/// Off a pose, a rig change is refused at once, even mid-drag: its undo
/// step is discarded, so the rig stays still. Undo and redo change
/// nothing here: they only refresh what was seen.
pub fn record_setup(s: &mut Session, pointer_down: bool) {
    let Some(mut setup) = s.control_setup.take() else {
        return;
    };
    let Some((id, at, on_pose)) = setup_state(s).filter(|t| t.0 == setup.control) else {
        end_setup(s);
        return;
    };
    if !on_pose && setup.made != s.history.made() && rig_changed(s, &setup.seen) {
        s.history.discard(&mut s.doc);
        setup.refused = true;
    }
    setup.refused &= !on_pose;
    if pointer_down {
        s.control_setup = Some(setup);
        return;
    }
    if setup.made != s.history.made() && on_pose {
        let frame = s.frame;
        let mut edits = Vec::new();
        let mut control = s.doc.graph.node(id).cloned();
        for (node, old) in &setup.seen {
            let Some(now) = s.doc.graph.node(*node) else {
                continue;
            };
            let mut changed = false;
            for ((channel, value), (_, was)) in channel_values(&now.kind, frame)
                .into_iter()
                .zip(channel_values(old, frame))
            {
                if (value - was).abs() > 1e-5 {
                    if let Some(NodeKind::Control(c)) = control.as_mut().map(|n| &mut n.kind) {
                        changed |= c.add_offset(at, *node, channel, value - was);
                    }
                }
            }
            if changed {
                let mut after = now.clone();
                restore_channels(&mut after.kind, old);
                edits.push(Edit::SetNodeProperties {
                    id: *node,
                    before: now.clone(),
                    after,
                });
            }
        }
        if let Some(after) = control.filter(|_| !edits.is_empty()) {
            if let Some(before) = s.doc.graph.node(id).cloned() {
                edits.push(Edit::SetNodeProperties { id, before, after });
            }
            s.history.amend(&mut s.doc, "Record pose", edits);
        }
    }
    setup.seen = rig_under(s, id);
    setup.made = s.history.made();
    s.control_setup = Some(setup);
}

/// Whether a value a pose can offset differs from `seen` on any bone or
/// peg under the Control.
fn rig_changed(s: &Session, seen: &BTreeMap<NodeId, NodeKind>) -> bool {
    seen.iter().any(|(node, old)| {
        s.doc.graph.node(*node).is_some_and(|now| {
            channel_values(&now.kind, s.frame)
                .into_iter()
                .zip(channel_values(old, s.frame))
                .any(|((_, value), (_, was))| (value - was).abs() > 1e-5)
        })
    })
}

/// The Setup mode panel and frame in the Camera view, and the recording.
pub fn setup_ui(
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    s: &mut Session,
) {
    let pointer_down = ui.input(|i| i.pointer.any_down());
    record_setup(s, pointer_down);
    let selected = s.selected_node.filter(|&id| {
        matches!(
            s.doc.graph.node(id).map(|n| &n.kind),
            Some(NodeKind::Control(_))
        )
    });
    let setup = setup_state(s).filter(|_| s.control_setup.is_some());
    let refused = s.control_setup.as_ref().is_some_and(|setup| setup.refused);
    if setup.is_none() && (selected.is_none() || !s.show_rig_controls) {
        return;
    }
    if setup.is_some() {
        // A coloured frame: rig edits now go into a pose.
        painter.rect_stroke(
            response.rect.shrink(1.5),
            0.0,
            Stroke::new(3.0, theme::ACCENT_TIME),
            egui::StrokeKind::Inside,
        );
    }
    let at = response.rect.left_top() + egui::vec2(12.0, 12.0);
    egui::Area::new(response.id.with("control-setup"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| match setup {
                Some((id, at, on_pose)) => {
                    let name = s.doc.graph.node(id).map_or("", |n| n.name.as_str());
                    ui.label(
                        egui::RichText::new(format!("Setup mode: {name}"))
                            .color(theme::ACCENT_TIME),
                    );
                    let hint = if on_pose {
                        format!(
                            "Pose the rig. The changes go into the pose at {at:.2}, not into keys."
                        )
                    } else if at == 0.0 {
                        "Move the knob away from the rest pose, then add a pose. \
                         The rig stays still until then."
                            .to_owned()
                    } else {
                        "Add a pose first, or move the knob onto a pose dot. \
                         The rig stays still until then."
                            .to_owned()
                    };
                    // After a refused change the hint takes the mode's colour.
                    let ink = if refused {
                        theme::ACCENT_TIME
                    } else {
                        theme::TEXT_SECONDARY
                    };
                    ui.label(egui::RichText::new(hint).font(theme::dense()).color(ink));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!on_pose && at != 0.0, egui::Button::new("Add pose"))
                            .clicked()
                        {
                            add_pose(s);
                        }
                        if symphony_ui::widgets::primary_button(ui, "Done", true).clicked() {
                            end_setup(s);
                        }
                    });
                }
                None => {
                    if let Some(id) = selected {
                        if ui
                            .button("Setup mode")
                            .on_hover_text(
                                "Make poses for this control: the knob becomes a test \
                                 position, and rig changes go into the pose at the knob.",
                            )
                            .clicked()
                        {
                            start_setup(s, id);
                        }
                    }
                }
            });
        });
}
