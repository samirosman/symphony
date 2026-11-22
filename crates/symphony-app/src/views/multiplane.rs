//! A side view of camera-facing drawing planes, with depth staging controls.

use eframe::egui::{self, Align2, Pos2, Sense, Stroke, Vec2};
use symphony_core::anim::Value;
use symphony_core::graph::{NodeId, NodeKind, PegParams};
use symphony_core::history::Edit;
use symphony_ui::theme;

use super::node::{self, NodeViewState};
use crate::session::Session;

pub fn ui(ui: &mut egui::Ui, s: &mut Session, node_view: &mut NodeViewState) {
    let maintain_id = ui.scope_id().with("maintain-size");
    let mut maintain_size = ui.data(|data| data.get_temp(maintain_id)).unwrap_or(true);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Camera properties").clicked() {
            s.select_graph_node(s.doc.display, true);
            node::show_selected_properties(s, node_view);
        }
        let active = s.layer_name(s.selected).to_owned();
        egui::ComboBox::from_id_salt("stage-plane").selected_text(active).show_ui(ui, |ui| {
            for index in 0..s.doc.layers.len() {
                if ui.selectable_label(index == s.selected, s.layer_name(index)).clicked() {
                    select_plane(s, index);
                }
            }
        });
        if let Some((id, mut depth)) = s.transform_peg_id(s.selected).zip(s.transform_peg(s.selected).map(|peg| peg.z.at(s.frame))) {
            let scale = s.doc.stage_distance();
            let response = ui.add(egui::DragValue::new(&mut depth).speed((scale / 360.0).max(0.25))
                .prefix("Peg Z ").range(-scale * 32.0..=scale * 32.0))
                .on_hover_text("Positive Z moves the plane away from the camera.");
            if response.changed() {
                change_depth(s, id, depth, maintain_size);
            }
            if response.drag_stopped() || response.lost_focus() {
                s.end_gesture();
            }
        } else {
            ui.label("Connect a peg to set depth");
        }
        if ui.checkbox(&mut maintain_size, "Maintain Size")
            .on_hover_text("Keep the selected drawing's size and screen position as you change its depth at this frame.")
            .changed() {
            ui.data_mut(|data| data.insert_temp(maintain_id, maintain_size));
        }
    });

    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let viewport = response.rect;
    painter.rect_filled(
        viewport,
        theme::GLASS_RADIUS.saturating_sub(4),
        theme::BG_INPUT,
    );
    let diagram = viewport.shrink2(Vec2::new(22.0, 25.0));
    if !diagram.is_positive() {
        return;
    }
    let distance = s.doc.stage_distance();
    let [_, camera_y, dolly] = s.doc.camera_motion.at(s.frame);
    let camera_z = dolly - distance;
    let planes: Vec<_> = s
        .doc
        .layers
        .iter()
        .enumerate()
        .filter_map(|(index, layer)| {
            let pose = s.doc.graph.world_pose(layer.node, s.frame).ok()?;
            (pose.depth.is_finite() && pose.transform.ty.is_finite()).then_some((index, pose))
        })
        .collect();
    let near = planes
        .iter()
        .fold(camera_z - distance * 0.08, |bound, (_, pose)| {
            bound.min(pose.depth)
        });
    let far = planes
        .iter()
        .fold(camera_z + distance * 1.35, |bound, (_, pose)| {
            bound.max(pose.depth)
        });
    let original_half_height = s.doc.settings.height.max(1) as f32 * 0.5;
    let aperture = original_half_height / s.doc.camera_scale(s.frame) / distance;
    let kx = diagram.width() / (far - near + distance * 0.08);
    let ky = (diagram.height() * 0.42) / ((far - camera_z) * aperture).max(original_half_height);
    let at = |z: f32, y: f32| {
        Pos2::new(
            diagram.left() + (z - near) * kx,
            diagram.center().y + (y - camera_y) * ky,
        )
    };
    let camera = at(camera_z, camera_y);
    let cone_height = (far - camera_z) * aperture;
    let cone_end = [
        at(far, camera_y - cone_height),
        at(far, camera_y + cone_height),
    ];
    painter.add(egui::Shape::convex_polygon(
        vec![camera, cone_end[0], cone_end[1]],
        theme::ACCENT_SELECT.gamma_multiply(0.08),
        Stroke::new(1.0, theme::ACCENT_SELECT.gamma_multiply(0.4)),
    ));
    painter.circle_filled(camera, 4.0, theme::ACCENT_SELECT);
    painter.text(
        camera + Vec2::new(0.0, 10.0),
        Align2::CENTER_TOP,
        "Camera",
        theme::dense(),
        theme::TEXT_SECONDARY,
    );

    let drag_id = response.id.with("depth-drag");
    let mut drag: Option<PlaneDrag> = ui.data(|data| data.get_temp(drag_id));
    let pick_point = if response.drag_started() {
        ui.input(|input| input.pointer.press_origin())
    } else {
        response.interact_pointer_pos()
    };
    let mut hit = None;
    for (index, pose) in planes {
        let selected = index == s.selected;
        let half_height =
            (original_half_height * pose.transform.c.abs().max(pose.transform.d.abs())).max(12.0);
        let centre = at(pose.depth, pose.transform.ty);
        let bounds = [
            at(pose.depth, pose.transform.ty - half_height),
            at(pose.depth, pose.transform.ty + half_height),
        ];
        let ink = if selected {
            theme::ACCENT_SELECT
        } else {
            theme::TEXT_SECONDARY
        };
        painter.line_segment(bounds, Stroke::new(if selected { 2.5 } else { 1.0 }, ink));
        painter.circle_filled(centre, if selected { 4.0 } else { 3.0 }, ink);
        if selected {
            painter.text(
                centre + Vec2::new(8.0, 0.0),
                Align2::LEFT_CENTER,
                format!("{}  Z {:.0}", s.layer_name(index), pose.depth),
                theme::dense(),
                theme::TEXT_PRIMARY,
            );
        }
        if let Some(point) = pick_point {
            let gap = (point.x - centre.x).abs();
            if gap <= 9.0
                && point.y >= bounds[0].y - 8.0
                && point.y <= bounds[1].y + 8.0
                && hit.is_none_or(|(_, best)| gap < best)
            {
                hit = Some((index, gap));
            }
        }
    }
    if response.clicked() || response.drag_started() {
        if let Some((index, _)) = hit {
            select_plane(s, index);
            if response.drag_started() {
                if let (Some(start), Some(id), Some(peg)) = (
                    pick_point,
                    s.transform_peg_id(index),
                    s.transform_peg(index),
                ) {
                    drag = Some(PlaneDrag {
                        id,
                        start_x: start.x,
                        z: peg.z.at(s.frame),
                        scale: kx,
                        scene: s.key.clone(),
                    });
                }
            }
        }
    }
    if response.dragged() || response.drag_stopped() {
        if let (Some(drag), Some(point)) = (
            drag.as_ref().filter(|drag| drag.scene == s.key),
            response.interact_pointer_pos(),
        ) {
            let limit = distance * 32.0;
            let target = (drag.z + (point.x - drag.start_x) / drag.scale).clamp(-limit, limit);
            change_depth(s, drag.id, target, maintain_size);
        }
    }
    ui.data_mut(|data| {
        if response.drag_stopped() || drag.as_ref().is_some_and(|drag| drag.scene != s.key) {
            data.remove::<PlaneDrag>(drag_id);
        } else if let Some(drag) = drag {
            data.insert_temp(drag_id, drag);
        }
    });
    if response.drag_stopped() {
        s.end_gesture();
    }
    painter.text(
        diagram.right_bottom(),
        Align2::RIGHT_TOP,
        "Depth Z",
        theme::dense(),
        theme::TEXT_SECONDARY,
    );
}

#[derive(Clone)]
struct PlaneDrag {
    id: NodeId,
    start_x: f32,
    z: f32,
    scale: f32,
    scene: String,
}

fn select_plane(s: &mut Session, index: usize) {
    let layer = s.doc.layers[index];
    s.selected = index;
    let target = s.doc.graph.input(layer.node, 0).unwrap_or(layer.node);
    s.select_graph_node(target, true);
}

fn change_depth(s: &mut Session, peg_id: NodeId, depth: f32, maintain_size: bool) {
    let Some(NodeKind::Peg(peg)) = s.doc.graph.node(peg_id).map(|node| &node.kind) else {
        return;
    };
    let before = peg.clone();
    let mut after = before.clone();
    let frame = s.frame;
    if !depth.is_finite() || (depth - before.z.at(frame)).abs() < 0.0001 {
        return;
    }
    if maintain_size {
        if let Some(layer) = s.doc.layers.get(s.selected) {
            if let (Ok(plane), Ok(parent)) = (
                s.doc.graph.world_pose(layer.node, frame),
                s.doc.graph.world_pose(peg_id, frame),
            ) {
                let old = s.doc.distance_to_plane(frame, plane.depth);
                let new = old + depth - before.z.at(frame);
                let clip = s.doc.stage_distance() * 0.02;
                if old >= clip && new >= clip {
                    if let Some(inverse) = parent.transform.inverse() {
                        let key_compensation = s.animate || matches!(before.z, Value::Keys(_));
                        preserve_projection(
                            &mut after,
                            parent.transform,
                            inverse,
                            s.doc.camera_motion.at(frame),
                            new / old,
                            frame,
                            key_compensation,
                        );
                    }
                }
            }
        }
    }
    set_value(&mut after.z, frame, depth, s.animate);
    if before != after {
        s.history.apply_merged(
            &mut s.doc,
            "Set plane depth",
            &format!("depth-{}", peg_id.0),
            Edit::SetPeg {
                node: peg_id,
                before,
                after,
            },
        );
    }
}

fn preserve_projection(
    peg: &mut PegParams,
    parent: symphony_core::math::Affine,
    inverse: symphony_core::math::Affine,
    camera: [f32; 3],
    ratio: f32,
    frame: u32,
    animate: bool,
) {
    let (px, py) = peg.pivot;
    let world = parent.apply(px + peg.x.at(frame), py + peg.y.at(frame));
    let target = inverse.apply(
        camera[0] + (world.0 - camera[0]) * ratio,
        camera[1] + (world.1 - camera[1]) * ratio,
    );
    set_value(&mut peg.x, frame, target.0 - px, animate);
    set_value(&mut peg.y, frame, target.1 - py, animate);
    for scale in [&mut peg.scale_x, &mut peg.scale_y] {
        let track = scale.get_or_insert(Value::Constant(1.0));
        let value = track.at(frame) * ratio;
        set_value(track, frame, value, animate);
    }
}

fn set_value(value: &mut Value, frame: u32, next: f32, animate: bool) {
    match value {
        Value::Constant(current) if !animate => *current = next,
        _ => value.set_key(frame, next),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintain_size_preserves_framing_under_a_transformed_parent() {
        let mut s = Session::new_scene();
        s.animate = false;
        s.doc.camera_motion.x = Value::Constant(42.0);
        s.doc.camera_motion.y = Value::Constant(-18.0);
        let layer = s.doc.layers[0];
        let parent = s.doc.graph.add(
            "Parent",
            NodeKind::Peg(PegParams {
                rotation: Value::Constant(21.0),
                x: Value::Constant(29.0),
                scale_x: Some(Value::Constant(1.5)),
                ..Default::default()
            }),
        );
        s.doc.graph.connect(parent, layer.peg, 0);
        let before = s.doc.camera_transform(layer.node, 1).unwrap();
        let depth = s.doc.stage_distance();
        change_depth(&mut s, layer.peg, depth, true);
        let after = s.doc.camera_transform(layer.node, 1).unwrap();
        assert_eq!(s.doc.graph.world_pose(layer.node, 1).unwrap().depth, depth);
        for (x, y) in [(0.0, 0.0), (23.0, -42.0)] {
            let a = before.apply(x, y);
            let b = after.apply(x, y);
            assert!((a.0 - b.0).abs() < 0.001 && (a.1 - b.1).abs() < 0.001);
        }
        s.undo();
        assert_eq!(s.doc.graph.world_pose(layer.node, 1).unwrap().depth, 0.0);
    }
}
