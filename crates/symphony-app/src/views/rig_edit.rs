use eframe::egui::{self, Pos2, Stroke};
use symphony_core::{
    anim::Value,
    graph::{NodeId, NodeKind},
    history::Edit,
    math::Affine,
};
use symphony_ui::theme;

use crate::session::{Session, Tool};

#[derive(Default)]
pub struct RigEdit {
    drag: Option<Drag>,
}

#[derive(Clone, Copy)]
enum Drag {
    Tip(NodeId),
    /// An anchor's target (on the bone's tip).
    Anchor(NodeId),
    MoveSkeleton {
        id: NodeId,
        grab: Pos2,
        start: [f32; 2],
    },
}

/// Reference only in Drawing View, where artwork is edited in its rest
/// coordinates. The filled bone and its endpoints stay legible over ink.
pub fn draw_rest_guide(painter: &egui::Painter, s: &Session, view: Affine) {
    let Some(layer) = s.doc.layers.get(s.selected) else {
        return;
    };
    let Some(endpoints) = s.doc.graph.drawing_bone_guide(layer.node, s.frame) else {
        return;
    };
    let [start, tip] = endpoints.map(|(x, y)| pos(view.apply(x, y)));
    let ink = theme::ACCENT_SELECT.gamma_multiply(0.8);
    let outline = theme::BG_BASE.gamma_multiply(0.75);
    let delta = tip - start;
    if delta.length() > 12.0 {
        let along = delta.normalized();
        let across = egui::vec2(-along.y, along.x) * 4.0;
        let shoulder = start + along * 7.0;
        painter.add(egui::Shape::convex_polygon(
            vec![start, shoulder + across, tip, shoulder - across],
            ink.gamma_multiply(0.28),
            Stroke::new(1.0, ink),
        ));
    } else {
        painter.line_segment([start, tip], Stroke::new(1.0, ink));
    }
    painter.circle(start, 4.0, outline, Stroke::new(1.2, ink));
    painter.circle(tip, 3.0, outline, Stroke::new(1.2, ink));
}

pub fn ui(
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    s: &mut Session,
    state: &mut RigEdit,
    view: Affine,
) -> bool {
    let selected = s.selected_node.filter(|&id| {
        s.doc
            .graph
            .node(id)
            .is_some_and(|node| matches!(node.kind, NodeKind::Bone(_) | NodeKind::Skeleton(_)))
    });
    let Some(selected) = selected.filter(|_| s.tool == Tool::Transform) else {
        finish(s, state);
        return false;
    };
    let root = s.doc.graph.rig_root(selected);
    let graph = &s.doc.graph;
    let mut handles = Vec::new();
    for (id, _) in graph.nodes().filter(|(id, _)| graph.bone(*id).is_some()) {
        if graph.rig_root(id) != root {
            continue;
        }
        let Some((pose, screen)) = graph.bone_pose(id, s.frame).zip(screen_basis(s, id, view))
        else {
            continue;
        };
        let from = pos(screen.apply(pose.posed.tx, pose.posed.ty));
        let (x, y) = pose.tip();
        let tip = pos(screen.apply(x, y));
        let controller = graph.smart_controller(id, s.frame);
        let target = graph
            .bone(id)
            .filter(|_| controller == Some(id))
            .and_then(|bone| bone.smart.as_ref())
            .and_then(|joint| {
                let axes = screen.compose(graph.smart_target_basis(id, s.frame)?);
                Some(pos(axes.apply(
                    joint.target[0].at(s.frame),
                    joint.target[1].at(s.frame),
                )))
            });
        let handle = target.unwrap_or(tip);
        let ink = if id == selected {
            theme::ACCENT_SELECT
        } else {
            theme::TEXT_SECONDARY
        };
        painter.line_segment([from, tip], Stroke::new(5.0, theme::BG_BASE));
        painter.line_segment([from, tip], Stroke::new(2.0, ink));
        painter.circle_filled(handle, 5.5, theme::BG_BASE);
        if target.is_some() {
            if handle.distance(tip) > 6.0 {
                painter.line_segment([tip, handle], Stroke::new(1.0, ink.gamma_multiply(0.6)));
            }
            painter.rect_stroke(
                egui::Rect::from_center_size(handle, egui::vec2(8.0, 8.0)),
                1.0,
                Stroke::new(1.5, ink),
                egui::StrokeKind::Inside,
            );
        } else {
            painter.circle_filled(tip, 3.0, ink);
        }
        handles.push((
            id,
            from,
            handle,
            graph.parent_bone(id).is_some(),
            controller.is_none_or(|owner| owner == id),
        ));
    }
    // Draw joints over the tips they share with their parent.
    for &(id, joint, _, connected, _) in &handles {
        if connected {
            let ink = if id == selected {
                theme::ACCENT_SELECT
            } else {
                theme::TEXT_SECONDARY
            };
            painter.circle_filled(joint, 6.0, theme::BG_BASE);
            painter.circle_stroke(joint, 4.0, Stroke::new(1.5, ink));
        }
    }
    let nearest = |pointer: Pos2, joint: bool| {
        handles
            .iter()
            .filter(|(_, _, _, connected, movable)| if joint { *connected } else { *movable })
            .map(|&(id, start, tip, _, _)| (id, pointer.distance(if joint { start } else { tip })))
            .filter(|(_, distance)| *distance <= 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    };
    let skeleton = graph.node(selected).and_then(|node| {
        let NodeKind::Skeleton(peg) = &node.kind else {
            return None;
        };
        let (_, origin, _, _, _) = handles.iter().find(|(_, _, _, connected, _)| !connected)?;
        Some((*origin, [peg.x.at(s.frame), peg.y.at(s.frame)]))
    });
    if let Some((origin, _)) = skeleton {
        painter.circle_filled(origin, 7.0, theme::BG_BASE);
        painter.circle_stroke(origin, 5.0, Stroke::new(2.0, theme::ACCENT_SELECT));
    }
    // Anchors: a pin (Locked) or a ring (Aim) on their targets, tied to
    // the joint they hold.
    let mut anchors: Vec<(NodeId, Pos2)> = Vec::new();
    for &(id, _, tip, _, _) in &handles {
        let Some(anchor) = graph.bone(id).and_then(|b| b.anchor.as_ref()) else {
            continue;
        };
        let Some(screen) = screen_basis(s, id, view) else {
            continue;
        };
        let at = pos(screen.apply(anchor.target[0].at(s.frame), anchor.target[1].at(s.frame)));
        let ink = theme::ACCENT_TIME;
        if at.distance(tip) > 3.0 {
            painter.line_segment([tip, at], Stroke::new(1.0, ink.gamma_multiply(0.6)));
        }
        match anchor.kind {
            symphony_core::rig::AnchorKind::Locked => {
                painter.circle_filled(at, 7.0, theme::BG_BASE);
                painter.rect_filled(
                    egui::Rect::from_center_size(at, egui::vec2(9.0, 9.0)),
                    2.0,
                    ink,
                );
            }
            symphony_core::rig::AnchorKind::Aim => {
                painter.circle_filled(at, 7.0, theme::BG_BASE);
                painter.circle_stroke(at, 5.5, Stroke::new(2.0, ink));
                painter.circle_filled(at, 1.8, ink);
            }
        }
        anchors.push((id, at));
    }
    let nearest_anchor = |pointer: Pos2| {
        anchors
            .iter()
            .map(|&(id, at)| (id, pointer.distance(at)))
            .filter(|(_, d)| *d <= 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    };
    let menu_key = response.id.with("rig-joint-menu");
    if response.secondary_clicked() {
        let id = response
            .interact_pointer_pos()
            .and_then(|p| nearest(p, true));
        ui.data_mut(|data| data.insert_temp(menu_key, id));
    }
    let menu = ui
        .data(|data| data.get_temp::<Option<NodeId>>(menu_key))
        .flatten();
    if let Some((id, label)) =
        menu.and_then(|id| super::node::smart_joint_label(s, id).map(|label| (id, label)))
    {
        response.context_menu(|ui| {
            if ui.button(label).clicked() {
                super::node::toggle_smart_joint(s, id);
                ui.close();
            }
        });
    }
    if response.drag_started_by(egui::PointerButton::Primary) {
        state.drag = ui.input(|i| i.pointer.press_origin()).and_then(|grab| {
            if let Some((_, start)) = skeleton.filter(|(origin, _)| origin.distance(grab) <= 10.0) {
                return Some(Drag::MoveSkeleton {
                    id: selected,
                    grab,
                    start,
                });
            }
            nearest_anchor(grab)
                .map(Drag::Anchor)
                .or_else(|| nearest(grab, false).map(Drag::Tip))
        });
        if let Some(Drag::Tip(id)) = state.drag {
            s.select_graph_node(id, true);
        }
    }
    if response.clicked_by(egui::PointerButton::Primary) {
        if let Some(id) = response
            .interact_pointer_pos()
            .and_then(|p| nearest(p, true).or_else(|| nearest(p, false)))
        {
            s.select_graph_node(id, true);
        }
    }
    if let Some(drag) = state.drag {
        if let Some(p) = response.interact_pointer_pos() {
            let snap = ui.input(|i| i.modifiers.shift);
            match drag {
                Drag::Tip(id) => pose_tip(s, id, view, p, snap),
                Drag::Anchor(id) => move_anchor(s, id, view, p),
                Drag::MoveSkeleton { id, grab, start } => {
                    move_skeleton(s, id, view, grab, p, start)
                }
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    } else if response
        .hover_pos()
        .is_some_and(|p| nearest(p, false).is_some())
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        finish(s, state);
    }
    true
}

pub fn finish(s: &mut Session, state: &mut RigEdit) {
    if state.drag.take().is_some() {
        s.end_gesture();
    }
}

fn pos((x, y): (f32, f32)) -> Pos2 {
    Pos2::new(x, y)
}

fn screen_basis(s: &Session, id: NodeId, view: Affine) -> Option<Affine> {
    let pose = s.doc.graph.rig_stage_pose(id, s.frame)?;
    let plane = s.doc.project_plane(s.frame, pose.depth)?;
    Some(view.compose(plane).compose(pose.transform))
}

fn pose_tip(s: &mut Session, id: NodeId, view: Affine, pointer: Pos2, snap: bool) {
    let Some(back) = screen_basis(s, id, view).and_then(|m| m.inverse()) else {
        return;
    };
    let (x, y) = back.apply(pointer.x, pointer.y);
    pose_to(s, id, [x, y], snap);
}

/// Put bone `id`'s tip at `target` (skeleton space). With anchors in the
/// rig, the rig is solved around them (smart rigs); otherwise the bone
/// turns (and stretches, if it may) to point there. `snap` keeps the
/// bone's direction to 22.5° steps.
pub fn pose_to(s: &mut Session, id: NodeId, target: [f32; 2], snap: bool) {
    let graph = &s.doc.graph;
    let (frame, animate) = (s.frame, s.animate);
    let mut target = target;
    if snap {
        if let Some(pose) = graph.bone_pose(id, frame) {
            let (bx, by) = pose.posed.apply(0.0, 0.0);
            let (dx, dy) = (target[0] - bx, target[1] - by);
            let step = 22.5f32.to_radians();
            let a = (dy.atan2(dx) / step).round() * step;
            let len = dx.hypot(dy);
            target = [bx + a.cos() * len, by + a.sin() * len];
        }
    }
    let root = graph.rig_root(id);
    let key = format!("rig-pose-{}", root.0);
    if let Some(changes) = graph.solve_rig(id, target, frame) {
        for (bone, rotation, stretch) in changes {
            let Some(before) = s.doc.graph.node(bone).cloned() else {
                continue;
            };
            let mut after = before.clone();
            if let NodeKind::Bone(b) = &mut after.kind {
                set_value(&mut b.rotation, rotation, frame, animate);
                if b.stretch_mode != symphony_core::rig::Stretch::Rigid {
                    set_value(&mut b.stretch, stretch, frame, animate);
                }
            }
            if before != after {
                s.history.apply_merged(
                    &mut s.doc,
                    "Pose rig",
                    &key,
                    Edit::SetNodeProperties {
                        id: bone,
                        before,
                        after,
                    },
                );
            }
        }
        return;
    }
    let Some(before) = graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    let NodeKind::Bone(bone) = &mut after.kind else {
        return;
    };
    if let Some(joint) = bone
        .smart
        .as_mut()
        .filter(|_| graph.smart_controller(id, frame) == Some(id))
    {
        let Some(back) = graph
            .smart_target_basis(id, frame)
            .and_then(|basis| basis.inverse())
        else {
            return;
        };
        let (x, y) = back.apply(target[0], target[1]);
        set_value(&mut joint.target[0], x, frame, animate);
        set_value(&mut joint.target[1], y, frame, animate);
    } else {
        let Some(pose) = graph.bone_pose(id, frame) else {
            return;
        };
        let Some(back) = pose.posed.inverse() else {
            return;
        };
        let (x, y) = back.apply(target[0], target[1]);
        if x.hypot(y) < 1e-3 {
            return;
        }
        let rotation = bone.rotation.at(frame) + y.atan2(x).to_degrees();
        set_value(&mut bone.rotation, rotation, frame, animate);
        // A stretchable bone also follows the pull's length (resisting
        // near its limits).
        if bone.stretch_mode != symphony_core::rig::Stretch::Rigid {
            let wanted = x.hypot(y) / bone.length();
            let stretch = bone.resist(wanted);
            set_value(&mut bone.stretch, stretch, frame, animate);
        }
    }
    if before != after {
        s.history.apply_merged(
            &mut s.doc,
            "Pose bone",
            &key,
            Edit::SetNodeProperties { id, before, after },
        );
    }
}

/// Drag an anchor's target (skeleton space), then re-solve so the rig
/// follows its pins.
fn move_anchor(s: &mut Session, id: NodeId, view: Affine, pointer: Pos2) {
    let Some(back) = screen_basis(s, id, view).and_then(|m| m.inverse()) else {
        return;
    };
    let (x, y) = back.apply(pointer.x, pointer.y);
    let (frame, animate) = (s.frame, s.animate);
    let Some(before) = s.doc.graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    let Some(anchor) = (match &mut after.kind {
        NodeKind::Bone(b) => b.anchor.as_mut(),
        _ => None,
    }) else {
        return;
    };
    set_value(&mut anchor.target[0], x, frame, animate);
    set_value(&mut anchor.target[1], y, frame, animate);
    let locked = anchor.kind == symphony_core::rig::AnchorKind::Locked;
    let key = format!("rig-pose-{}", s.doc.graph.rig_root(id).0);
    s.history.apply_merged(
        &mut s.doc,
        "Move anchor",
        &key,
        Edit::SetNodeProperties { id, before, after },
    );
    // A locked joint goes where its pin goes; an aim turns its bone.
    let tip = if locked {
        [x, y]
    } else {
        match s.doc.graph.bone_pose(id, frame) {
            Some(p) => {
                let (tx, ty) = p.tip();
                [tx, ty]
            }
            None => return,
        }
    };
    pose_to(s, id, tip, false);
}

/// A: cycle the selected bone's anchor: none, Locked, Aim.
pub fn cycle_anchor(s: &mut Session) {
    use symphony_core::rig::{Anchor, AnchorKind};
    let Some(id) = s.selected_node else { return };
    let Some(before) = s.doc.graph.node(id).cloned() else {
        return;
    };
    let tip = s.doc.graph.bone_pose(id, s.frame).map(|p| p.tip());
    let mut after = before.clone();
    let NodeKind::Bone(bone) = &mut after.kind else {
        return;
    };
    let (x, y) = tip.unwrap_or((0.0, 0.0));
    let (anchor, note) = match bone.anchor.as_ref().map(|a| a.kind) {
        None => (
            Some(Box::new(Anchor {
                kind: AnchorKind::Locked,
                target: [Value::Constant(x), Value::Constant(y)],
            })),
            "Locked anchor: this joint stays put",
        ),
        Some(AnchorKind::Locked) => (
            bone.anchor.clone().map(|a| {
                Box::new(Anchor {
                    kind: AnchorKind::Aim,
                    ..*a
                })
            }),
            "Aim anchor: this bone points at its target",
        ),
        Some(AnchorKind::Aim) => (None, "Anchor removed"),
    };
    bone.anchor = anchor;
    s.history.apply(
        &mut s.doc,
        "Toggle anchor",
        vec![Edit::SetNodeProperties { id, before, after }],
    );
    s.note(note);
}

/// F: flip the bend of the selected bone's smart joint.
pub fn flip_bend(s: &mut Session) {
    let Some(id) = s.selected_node else { return };
    let graph = &s.doc.graph;
    let lower = graph
        .smart_controller(id, s.frame)
        .or_else(|| graph.bone(id).and_then(|b| b.smart.as_ref()).map(|_| id));
    let Some(lower) = lower else {
        s.note("Flip bend works on a smart joint");
        return;
    };
    let Some(before) = s.doc.graph.node(lower).cloned() else {
        return;
    };
    let mut after = before.clone();
    if let NodeKind::Bone(b) = &mut after.kind {
        if let Some(j) = b.smart.as_mut() {
            j.flip = !j.flip;
        }
    }
    s.history.apply(
        &mut s.doc,
        "Flip bend",
        vec![Edit::SetNodeProperties {
            id: lower,
            before,
            after,
        }],
    );
}

/// Arrow keys: nudge the selected bone's tip (one unit, or ten with
/// Shift), solving the rig as a drag would.
pub fn nudge(s: &mut Session, dx: f32, dy: f32) {
    let Some(id) = s.selected_node.filter(|&id| s.doc.graph.bone(id).is_some()) else {
        return;
    };
    let Some(pose) = s.doc.graph.bone_pose(id, s.frame) else {
        return;
    };
    let (x, y) = pose.tip();
    pose_to(s, id, [x + dx, y + dy], false);
    s.end_gesture();
}

fn move_skeleton(s: &mut Session, id: NodeId, view: Affine, grab: Pos2, p: Pos2, start: [f32; 2]) {
    let graph = &s.doc.graph;
    let Some(before) = graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    let NodeKind::Skeleton(peg) = &mut after.kind else {
        return;
    };
    let Ok(parent) = graph.world_pose(id, s.frame) else {
        return;
    };
    let Some(plane) = s
        .doc
        .project_plane(s.frame, parent.depth + peg.z.at(s.frame))
    else {
        return;
    };
    let Some(back) = view.compose(plane).compose(parent.transform).inverse() else {
        return;
    };
    let (x0, y0) = back.apply(grab.x, grab.y);
    let (x, y) = back.apply(p.x, p.y);
    set_value(&mut peg.x, start[0] + x - x0, s.frame, s.animate);
    set_value(&mut peg.y, start[1] + y - y0, s.frame, s.animate);
    if before != after {
        s.history.apply_merged(
            &mut s.doc,
            "Move skeleton",
            &format!("skeleton-{}", id.0),
            Edit::SetNodeProperties { id, before, after },
        );
    }
}

fn set_value(value: &mut Value, number: f32, frame: u32, animate: bool) {
    match value {
        Value::Constant(current) if !animate => *current = number,
        _ => value.set_key(frame, number),
    }
}
