use eframe::egui::{self, Pos2, Stroke};
use symphony_core::{
    anim::Value,
    control::Channel,
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
    Tip(NodeId, Pos2),
    /// A drag on a bone's body turns it about its base (Ctrl: and
    /// stretches it), as Harmony's Skeleton Controller does.
    Turn {
        id: NodeId,
        base: Pos2,
        tip: Pos2,
        grab: Pos2,
    },
    /// An anchor's target (on the bone's tip).
    Anchor(NodeId, Pos2),
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
    paint_bone(painter, start, tip, ink);
    let outline = theme::BG_BASE.gamma_multiply(0.75);
    painter.circle(start, 4.0, outline, Stroke::new(1.2, ink));
    painter.circle(tip, 3.0, outline, Stroke::new(1.2, ink));
}

/// A bone's handle colour: the selection blue, else its own colour,
/// faded when it is locked.
/// The ink of a rig colour (bones and control widgets).
pub(super) fn colour_ink(colour: symphony_core::rig::BoneColour) -> egui::Color32 {
    use symphony_core::rig::BoneColour;
    match colour {
        BoneColour::Plain => theme::TEXT_SECONDARY,
        BoneColour::Red => theme::RIG_RED,
        BoneColour::Orange => theme::RIG_ORANGE,
        BoneColour::Yellow => theme::RIG_YELLOW,
        BoneColour::Green => theme::RIG_GREEN,
        BoneColour::Teal => theme::RIG_TEAL,
        BoneColour::Purple => theme::RIG_PURPLE,
        BoneColour::Pink => theme::RIG_PINK,
    }
}

fn bone_ink(bone: Option<&symphony_core::rig::Bone>, selected: bool) -> egui::Color32 {
    let base = if selected {
        theme::ACCENT_SELECT
    } else {
        colour_ink(bone.map_or_else(Default::default, |bone| bone.colour))
    };
    if bone.is_some_and(|bone| bone.locked) {
        base.gamma_multiply(0.5)
    } else {
        base
    }
}

/// The bone's name beside its middle, on a dark chip so it reads over
/// any artwork.
pub(super) fn paint_label(
    painter: &egui::Painter,
    start: Pos2,
    tip: Pos2,
    name: &str,
    ink: egui::Color32,
) {
    let along = (tip - start).normalized();
    let side = egui::vec2(-along.y, along.x) * 12.0;
    let at = start + (tip - start) * 0.5 + side;
    let galley = painter.layout_no_wrap(
        name.to_owned(),
        egui::FontId::proportional(theme::SIZE_DENSE),
        ink,
    );
    let rect = egui::Align2::CENTER_CENTER
        .anchor_size(at, galley.size())
        .expand2(egui::vec2(4.0, 1.0));
    painter.rect_filled(rect, 3.0, theme::BG_BASE.gamma_multiply(0.8));
    painter.galley(rect.shrink2(egui::vec2(4.0, 1.0)).min, galley, ink);
}

/// A bone as a long diamond. A dark edge under the coloured line keeps
/// every bone colour above 3:1 on the light stage too (D4.4).
fn paint_bone(painter: &egui::Painter, start: Pos2, tip: Pos2, ink: egui::Color32) {
    let edge = Stroke::new(3.0, theme::BG_BASE.gamma_multiply(0.85));
    let delta = tip - start;
    if delta.length() > 12.0 {
        let along = delta.normalized();
        let across = egui::vec2(-along.y, along.x) * 4.0;
        let shoulder = start + along * 7.0;
        let outline = vec![start, shoulder + across, tip, shoulder - across];
        // Each side as its own segment: a closed line joins the sides
        // with a miter, and the sharp tip makes a spike.
        let sides: Vec<[Pos2; 2]> = (0..outline.len())
            .map(|i| [outline[i], outline[(i + 1) % outline.len()]])
            .collect();
        for &side in &sides {
            painter.line_segment(side, edge);
        }
        painter.add(egui::Shape::convex_polygon(
            outline,
            ink.gamma_multiply(0.28),
            Stroke::NONE,
        ));
        for &side in &sides {
            painter.line_segment(side, Stroke::new(1.0, ink));
        }
    } else {
        painter.line_segment([start, tip], edge);
        painter.line_segment([start, tip], Stroke::new(1.0, ink));
    }
}

/// The skeleton's base handle. It sits on the first bone, so a locked
/// first bone pins it, and it shows faded.
fn paint_base(painter: &egui::Painter, origin: Pos2, locked: bool) {
    let ink = if locked {
        theme::ACCENT_SELECT.gamma_multiply(0.4)
    } else {
        theme::ACCENT_SELECT
    };
    painter.circle_filled(origin, 7.0, theme::BG_BASE);
    painter.circle_stroke(origin, 5.0, Stroke::new(2.0, ink));
}

pub fn ui(
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    s: &mut Session,
    state: &mut RigEdit,
    view: Affine,
    blocked: bool,
) -> bool {
    if !s.show_rig_controls {
        finish(s, state);
        return false;
    }
    let selected = s
        .selected_node
        .or_else(|| s.doc.layers.get(s.selected).map(|layer| layer.node));
    let Some(selected) = selected.and_then(|id| match &s.doc.graph.node(id)?.kind {
        NodeKind::Bone(_) | NodeKind::Skeleton(_) => Some(id),
        NodeKind::Drawing { .. } => s.doc.graph.drawing_bone(id, s.frame),
        _ => None,
    }) else {
        finish(s, state);
        return false;
    };
    let root = s.doc.graph.rig_root(selected);
    let graph = &s.doc.graph;
    let locked_bones: Vec<NodeId> = graph
        .nodes()
        .filter(|(_, node)| matches!(&node.kind, NodeKind::Bone(bone) if bone.locked))
        .map(|(id, _)| id)
        .collect();
    let locked = |id: NodeId| locked_bones.contains(&id);
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
        let ink = bone_ink(graph.bone(id), id == selected);
        paint_bone(painter, from, tip, ink);
        if let Some(name) = graph
            .node(id)
            .filter(|_| graph.bone(id).is_some_and(|bone| bone.show_label))
            .map(|node| node.name.as_str())
        {
            paint_label(painter, from, tip, name, ink);
        }
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
            controller.is_none_or(|owner| owner == id) && !locked(id),
        ));
    }
    // Draw joints over the tips they share with their parent.
    for &(id, joint, _, connected, _) in &handles {
        if connected {
            let ink = bone_ink(graph.bone(id), id == selected);
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
    let skeleton = graph.node(root).and_then(|node| {
        let NodeKind::Skeleton(peg) = &node.kind else {
            return None;
        };
        let (first, origin, _, _, _) = handles.iter().find(|(_, _, _, connected, _)| !connected)?;
        Some((
            *origin,
            [peg.x.at(s.frame), peg.y.at(s.frame)],
            locked(*first),
        ))
    });
    if let Some((origin, _, base_locked)) = skeleton {
        paint_base(painter, origin, base_locked);
    }
    // A locked first bone keeps the base still: the press falls through
    // to the bone, which takes no drag either.
    let skeleton = skeleton
        .filter(|&(_, _, base_locked)| !base_locked)
        .map(|(origin, start, _)| (origin, start));
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
    // A control widget over the rig has the pointer.
    if s.tool != Tool::Transform || blocked {
        finish(s, state);
        return false;
    }
    let nearest_anchor = |pointer: Pos2| {
        anchors
            .iter()
            .filter(|&&(id, _)| !locked(id))
            .map(|&(id, at)| (id, pointer.distance(at)))
            .filter(|(_, d)| *d <= 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    };
    let bone_at = |pointer: Pos2| {
        handles
            .iter()
            .filter_map(|&(id, start, end, _, _)| {
                super::skeleton_setup::bone_body_distance(pointer, start, end).map(|d| (id, d))
            })
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
                    id: root,
                    grab,
                    start,
                });
            }
            nearest_anchor(grab)
                .map(|id| Drag::Anchor(id, grab))
                .or_else(|| nearest(grab, false).map(|id| Drag::Tip(id, grab)))
                .or_else(|| {
                    let id = bone_at(grab).filter(|&id| !locked(id))?;
                    let &(_, base, tip, _, _) = handles.iter().find(|h| h.0 == id)?;
                    Some(Drag::Turn {
                        id,
                        base,
                        tip,
                        grab,
                    })
                })
        });
        if let Some(Drag::Tip(id, _) | Drag::Anchor(id, _) | Drag::Turn { id, .. }) = state.drag {
            s.select_graph_node(id, true);
        }
    }
    if response.clicked_by(egui::PointerButton::Primary) {
        let p = response.interact_pointer_pos().unwrap_or_default();
        let hit = nearest(p, true)
            .or_else(|| nearest(p, false))
            .or_else(|| bone_at(p));
        match hit {
            Some(id) => s.select_graph_node(id, true),
            // A click on empty space drops the bone selection (Harmony:
            // click the negative space to discard the selection), which
            // brings the drawing tools back.
            None if nearest_anchor(p).is_none() => s.selected_node = None,
            None => {}
        }
    }
    if let Some(drag) = state.drag {
        if let Some(p) = response.interact_pointer_pos() {
            let snap = ui.input(|i| i.modifiers.shift);
            match drag {
                Drag::Tip(id, grab) => {
                    let stretch = ui.input(|i| i.modifiers.command);
                    let at = handles.iter().find(|h| h.0 == id).map(|h| (h.1, h.2));
                    let target = tip_target(s, id, at, snapped(p, grab, snap), stretch);
                    pose_tip(s, id, view, target, false);
                }
                Drag::Turn {
                    id,
                    base,
                    tip,
                    grab,
                } => {
                    let stretch = ui.input(|i| i.modifiers.command);
                    let target = turned(base, tip, grab, p, stretch);
                    pose_tip(s, id, view, target, snap);
                }
                Drag::Anchor(id, grab) => move_anchor(s, id, view, snapped(p, grab, snap)),
                Drag::MoveSkeleton { id, grab, start } => {
                    move_skeleton(s, id, view, grab, snapped(p, grab, snap), start)
                }
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    } else if response.hover_pos().is_some_and(|p| {
        nearest_anchor(p).is_some() || nearest(p, false).is_some() || bone_at(p).is_some()
    }) {
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

fn snapped(pointer: Pos2, start: Pos2, snap: bool) -> Pos2 {
    if !snap {
        return pointer;
    }
    let delta = pointer - start;
    let step = 22.5_f32.to_radians();
    let angle = (delta.y.atan2(delta.x) / step).round() * step;
    start + egui::vec2(angle.cos(), angle.sin()) * delta.length()
}

/// Where a turned bone's tip goes: the tip at the press, turned about
/// the base by the angle the pointer swept, and with `stretch` also
/// scaled by how far the pointer moved from the base.
fn turned(base: Pos2, tip: Pos2, grab: Pos2, pointer: Pos2, stretch: bool) -> Pos2 {
    let (from, to) = (grab - base, pointer - base);
    if from.length() < 1.0 || to.length() < 1.0 {
        return tip;
    }
    let angle = to.y.atan2(to.x) - from.y.atan2(from.x);
    let scale = if stretch {
        to.length() / from.length()
    } else {
        1.0
    };
    let arm = tip - base;
    let (sin, cos) = angle.sin_cos();
    base + egui::vec2(arm.x * cos - arm.y * sin, arm.x * sin + arm.y * cos) * scale
}

/// Where a tip drag puts the tip. Only Ctrl changes a bone's length
/// (Harmony's Skeleton Bone): without it, a stretchable bone keeps its
/// length and turns toward the pointer. `at` is the bone's base and tip
/// on screen.
fn tip_target(
    s: &Session,
    id: NodeId,
    at: Option<(Pos2, Pos2)>,
    pointer: Pos2,
    stretch: bool,
) -> Pos2 {
    let stretchable = s
        .doc
        .graph
        .bone(id)
        .is_some_and(|b| b.stretch_mode != symphony_core::rig::Stretch::Rigid);
    match at {
        Some((base, tip)) if stretchable && !stretch && (pointer - base).length() > 1.0 => {
            base + (pointer - base).normalized() * (tip - base).length()
        }
        _ => pointer,
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
    if let Some(solution) = graph.solve_rig(id, target, frame) {
        apply_rig_solution(s, root, &key, solution, Some(id));
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
        // The bone shows its key plus the rig controls' offsets, inside
        // its limits; the key is what is shown less the offset.
        let offset = |channel| graph.offset(id, channel, frame);
        let turn = offset(Channel::Rotation);
        let wanted = bone.rotation_plus(frame, turn) + y.atan2(x).to_degrees();
        let rotation = bone.clamp_rotation(bone.spring_rotation(wanted)) - turn;
        set_value(&mut bone.rotation, rotation, frame, animate);
        // A stretchable bone also follows the pull's length (resisting
        // near its limits).
        if bone.stretch_mode != symphony_core::rig::Stretch::Rigid {
            let wanted = x.hypot(y) / bone.length();
            let stretch = bone.resist(wanted) - offset(Channel::Stretch);
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

/// Anchor drags go through the same constraint solve as free tip drags.
fn move_anchor(s: &mut Session, id: NodeId, view: Affine, pointer: Pos2) {
    pose_tip(s, id, view, pointer, false);
}

pub(super) fn apply_rig_solution(
    s: &mut Session,
    root: NodeId,
    key: &str,
    solution: symphony_core::rig::RigSolution,
    moved_anchor: Option<NodeId>,
) {
    let (frame, animate) = (s.frame, s.animate);
    let changed: std::collections::BTreeMap<_, _> = solution
        .changes
        .into_iter()
        .map(|(id, rotation, stretch)| (id, (rotation, stretch)))
        .collect();
    for id in s.doc.graph.rig_bones(root) {
        let Some(before) = s.doc.graph.node(id).cloned() else {
            continue;
        };
        let mut after = before.clone();
        let NodeKind::Bone(bone) = &mut after.kind else {
            continue;
        };
        if let Some(&(rotation, stretch)) = changed.get(&id) {
            set_value(&mut bone.rotation, rotation, frame, animate);
            if bone.stretch_mode != symphony_core::rig::Stretch::Rigid {
                set_value(&mut bone.stretch, stretch, frame, animate);
            }
        }
        if let Some(anchor) = bone.anchor.as_mut() {
            let target = if Some(id) == moved_anchor {
                solution.target
            } else if solution.translation != [0.0; 2] {
                Some([0, 1].map(|i| anchor.target[i].at(frame) - solution.translation[i]))
            } else {
                None
            };
            if let Some(target) = target {
                for (value, next) in anchor.target.iter_mut().zip(target) {
                    if (value.at(frame) - next).abs() > 0.0001 {
                        set_value(value, next, frame, animate);
                    }
                }
            }
        }
        if after != before {
            s.history.apply_merged(
                &mut s.doc,
                "Pose rig",
                key,
                Edit::SetNodeProperties { id, before, after },
            );
        }
    }
}

/// The rig of the selected bone, skeleton, or drawing on a bone.
fn selected_rig(s: &Session) -> Option<NodeId> {
    let id = s
        .selected_node
        .or_else(|| s.doc.layers.get(s.selected).map(|layer| layer.node))?;
    let bone = match &s.doc.graph.node(id)?.kind {
        NodeKind::Bone(_) | NodeKind::Skeleton(_) => id,
        NodeKind::Drawing { .. } => s.doc.graph.drawing_bone(id, s.frame)?,
        _ => return None,
    };
    Some(s.doc.graph.rig_root(bone))
}

/// Key every bone of the selected rig, and its skeleton's position, on
/// this frame with the values they show now (Moho's Freeze Pose). A key
/// made later then starts its motion here, not at the last key.
pub fn key_rig(s: &mut Session) {
    let Some(root) = selected_rig(s) else {
        s.note("Select a bone to key its rig");
        return;
    };
    let frame = s.frame;
    let key = |value: &mut Value| {
        let now = value.at(frame);
        value.set_key(frame, now);
    };
    let mut edits = Vec::new();
    let mut ids = s.doc.graph.rig_bones(root);
    ids.push(root);
    for id in ids {
        let Some(before) = s.doc.graph.node(id).cloned() else {
            continue;
        };
        let mut after = before.clone();
        match &mut after.kind {
            NodeKind::Bone(bone) => {
                key(&mut bone.rotation);
                if bone.stretch_mode != symphony_core::rig::Stretch::Rigid {
                    key(&mut bone.stretch);
                }
                if let Some(joint) = bone.smart.as_mut() {
                    joint.target.iter_mut().for_each(key);
                }
                if let Some(anchor) = bone.anchor.as_mut() {
                    anchor.target.iter_mut().for_each(key);
                }
            }
            NodeKind::Skeleton(peg) => {
                key(&mut peg.x);
                key(&mut peg.y);
                key(&mut peg.rotation);
            }
            _ => continue,
        }
        if after != before {
            edits.push(Edit::SetNodeProperties { id, before, after });
        }
    }
    if edits.is_empty() {
        return;
    }
    s.end_gesture();
    s.history
        .apply(&mut s.doc, "Set keyframe on whole rig", edits);
}

/// A: cycle the selected bone's anchor: none, Locked, Aim.
pub fn cycle_anchor(s: &mut Session) {
    use symphony_core::rig::AnchorKind;
    let Some(id) = s.selected_node else { return };
    let kind = match s
        .doc
        .graph
        .bone(id)
        .and_then(|b| b.anchor.as_ref())
        .map(|a| a.kind)
    {
        None => Some(AnchorKind::Locked),
        Some(AnchorKind::Locked) => Some(AnchorKind::Aim),
        Some(AnchorKind::Aim) => None,
    };
    set_anchor_mode(s, id, kind);
}

pub(super) fn update_anchor_mode(
    bone: &mut symphony_core::rig::Bone,
    kind: Option<symphony_core::rig::AnchorKind>,
    tip: (f32, f32),
) {
    use symphony_core::rig::{Anchor, AnchorKind};
    if bone.anchor.as_ref().map(|anchor| anchor.kind) == kind {
        return;
    }
    let (x, y) = tip;
    bone.anchor = kind.map(|kind| {
        let target = bone
            .anchor
            .as_ref()
            .filter(|_| kind == AnchorKind::Aim)
            .map(|anchor| anchor.target.clone())
            .unwrap_or([Value::Constant(x), Value::Constant(y)]);
        Box::new(Anchor { kind, target })
    });
}

pub fn set_anchor_mode(s: &mut Session, id: NodeId, kind: Option<symphony_core::rig::AnchorKind>) {
    let graph = &s.doc.graph;
    let Some(before) = graph.node(id).cloned() else {
        return;
    };
    let Some(pose) = graph.bone_pose(id, s.frame) else {
        return;
    };
    let mut after = before.clone();
    let NodeKind::Bone(bone) = &mut after.kind else {
        return;
    };
    let root = graph.rig_root(id);
    update_anchor_mode(bone, kind, pose.tip());
    if before == after {
        return;
    }
    let key = format!("rig-pose-{}", root.0);
    s.end_gesture();
    s.history.apply_merged(
        &mut s.doc,
        "Change anchor mode",
        &key,
        Edit::SetNodeProperties { id, before, after },
    );
    if let Some(solution) = s.doc.graph.solve_rig_translation(root, [0.0; 2], s.frame) {
        apply_rig_solution(s, root, &key, solution, None);
    }
    s.end_gesture();
}

/// F: flip the bend of the selected bone's smart joint.
pub fn flip_bend(s: &mut Session) {
    let Some(id) = s.selected_node else { return };
    let graph = &s.doc.graph;
    let root = graph.rig_root(id);
    if graph.rig_has_anchors(root, s.frame) {
        if let Some(solution) = graph.flip_anchored_bend(id, s.frame) {
            let key = format!("rig-pose-{}", root.0);
            s.end_gesture();
            apply_rig_solution(s, root, &key, solution, None);
            s.end_gesture();
        }
        return;
    }
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
    let (x, y) = s
        .doc
        .graph
        .bone(id)
        .and_then(|b| b.anchor.as_ref())
        .map_or_else(
            || pose.tip(),
            |a| (a.target[0].at(s.frame), a.target[1].at(s.frame)),
        );
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
    translate_skeleton(s, id, [start[0] + x - x0, start[1] + y - y0]);
}

/// Move the skeleton peg through the same solver as a tip or anchor.
pub fn translate_skeleton(s: &mut Session, id: NodeId, target: [f32; 2]) {
    let (frame, animate) = (s.frame, s.animate);
    let graph = &s.doc.graph;
    let Some(before) = graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    let NodeKind::Skeleton(peg) = &mut after.kind else {
        return;
    };
    let current = [peg.x.at(frame), peg.y.at(frame)];
    let delta = [target[0] - current[0], target[1] - current[1]];
    if delta[0].hypot(delta[1]) < 0.0001 {
        return;
    }
    let transform = peg.transform_at(frame);
    let linear = Affine {
        tx: 0.0,
        ty: 0.0,
        ..transform
    };
    let Some(inverse) = linear.inverse() else {
        return;
    };
    let (dx, dy) = inverse.apply(delta[0], delta[1]);
    let key = format!("rig-pose-{}", id.0);
    let delta = if let Some(solution) = graph.solve_rig_translation(id, [dx, dy], frame) {
        let [x, y] = solution.translation;
        let (x, y) = linear.apply(x, y);
        apply_rig_solution(s, id, &key, solution, None);
        [x, y]
    } else {
        delta
    };
    if delta[0].hypot(delta[1]) >= 0.0001 {
        set_value(&mut peg.x, current[0] + delta[0], frame, animate);
        set_value(&mut peg.y, current[1] + delta[1], frame, animate);
    }
    if before != after {
        s.history.apply_merged(
            &mut s.doc,
            "Move skeleton",
            &key,
            Edit::SetNodeProperties { id, before, after },
        );
    }
}

pub(super) fn set_value(value: &mut Value, number: f32, frame: u32, animate: bool) {
    match value {
        Value::Constant(current) if !animate => *current = number,
        _ => value.set_key(frame, number),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphony_core::rig::{Anchor, AnchorKind, Bone};

    #[test]
    fn moving_the_body_updates_the_pose_and_anchor_in_one_undo_step() {
        let mut s = Session::new_scene();
        let graph = &mut s.doc.graph;
        let root = graph.add("Skeleton", NodeKind::Skeleton(Default::default()));
        let upper = graph.add(
            "Upper",
            NodeKind::Bone(Bone {
                length: 100.0,
                ..Bone::default()
            }),
        );
        let lower = graph.add(
            "Lower",
            NodeKind::Bone(Bone {
                length: 100.0,
                anchor: Some(Box::new(Anchor {
                    kind: AnchorKind::Locked,
                    target: [Value::Constant(200.0), Value::Constant(0.0)],
                })),
                ..Bone::default()
            }),
        );
        graph.connect(root, upper, 0);
        graph.connect(upper, lower, 0);

        translate_skeleton(&mut s, root, [-90.0, 0.0]);
        assert_eq!(s.history.undo_len(), 0);
        translate_skeleton(&mut s, root, [40.0, 0.0]);
        let graph = &s.doc.graph;
        let pose = graph.bone_pose(lower, 1).unwrap();
        let (x, y) = pose.tip();
        let stage = graph.rig_stage_pose(lower, 1).unwrap().transform;
        let (x, y) = stage.apply(x, y);
        assert!((x - 200.0).abs() < 0.03 && y.abs() < 0.03);
        assert_eq!(stage.tx, 40.0);
        s.undo();
        let graph = &s.doc.graph;
        assert_eq!(graph.rig_stage_pose(lower, 1).unwrap().transform.tx, 0.0);
        assert_eq!(graph.bone_pose(lower, 1).unwrap().tip(), (200.0, 0.0));
    }
}
