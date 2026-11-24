//! Editing a Mesh Warp in the Camera view (Harmony 27: select the node,
//! show its controls, and move the grid's points). With a Mesh Warp node
//! selected and the Transform tool, the grid shows over the artwork, bent
//! as it bends the art; dragging a point bends everything under it. In
//! Animate mode the move keys the point on the current frame; otherwise
//! it sets the point for every frame. One drag is one undo step.

use eframe::egui::{self, Pos2, Stroke as EStroke};
use symphony_core::graph::{NodeId, NodeKind};
use symphony_core::history::Edit;
use symphony_core::math::Affine;
use symphony_core::warp::WarpGrid;
use symphony_ui::theme;

use crate::session::{Session, Tool};

/// How near (screen points) a press must be to grab a grid point.
const GRAB: f32 = 10.0;

#[derive(Default)]
pub struct WarpEdit {
    /// The point being dragged (index into the grid's points).
    dragging: Option<usize>,
    hover: Option<usize>,
}

/// The selected Mesh Warp, its grid on this frame, and grid space ->
/// screen.
fn target(s: &Session, view: Affine) -> Option<(NodeId, WarpGrid, Affine)> {
    let id = s.selected_node?;
    let NodeKind::MeshWarp(w) = &s.doc.graph.node(id)?.kind else {
        return None;
    };
    // The grid sits in its node's parent space (what the pegs above it
    // make of it), on the stage plane at their depth.
    let pose = s.doc.graph.world_pose(id, s.frame).ok()?;
    let plane = s.doc.project_plane(s.frame, pose.depth)?;
    Some((
        id,
        w.grid_at(s.frame),
        view.compose(plane).compose(pose.transform),
    ))
}

/// Draw and edit the selected Mesh Warp's grid. Returns true while a
/// Mesh Warp is being edited: the Transform tool then works on its grid
/// only (not the layer's box).
pub fn ui(
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    s: &mut Session,
    state: &mut WarpEdit,
    view: Affine,
) -> bool {
    if s.tool != Tool::Transform {
        state.dragging = None;
        return false;
    }
    let Some((id, grid, m)) = target(s, view) else {
        state.dragging = None;
        return false;
    };
    let screen = |p: [f32; 2]| {
        let (x, y) = m.apply(p[0], p[1]);
        Pos2::new(x, y)
    };
    // The grid's lines, bent as the art is.
    let line = EStroke::new(1.0, theme::ACCENT_SELECT.gamma_multiply(0.7));
    let steps = 12;
    for r in 0..=grid.rows {
        let pts: Vec<Pos2> = (0..=grid.columns * steps)
            .map(|i| {
                let c = i as f32 / steps as f32;
                let a = grid.rest(r, 0);
                let b = grid.rest(r, grid.columns);
                let t = c / grid.columns as f32;
                screen(grid.apply([a[0] + (b[0] - a[0]) * t, a[1]]))
            })
            .collect();
        painter.add(egui::Shape::line(pts, line));
    }
    for c in 0..=grid.columns {
        let pts: Vec<Pos2> = (0..=grid.rows * steps)
            .map(|i| {
                let r = i as f32 / steps as f32;
                let a = grid.rest(0, c);
                let b = grid.rest(grid.rows, c);
                let t = r / grid.rows as f32;
                screen(grid.apply([a[0], a[1] + (b[1] - a[1]) * t]))
            })
            .collect();
        painter.add(egui::Shape::line(pts, line));
    }
    // The points.
    let cols = grid.columns as usize + 1;
    let points: Vec<Pos2> = (0..=grid.rows)
        .flat_map(|r| (0..=grid.columns).map(move |c| (r, c)))
        .map(|(r, c)| screen(grid.point(r, c)))
        .collect();
    let pointer = response.hover_pos().or(response.interact_pointer_pos());
    let nearest = pointer.and_then(|p| {
        points
            .iter()
            .enumerate()
            .map(|(i, q)| (i, q.distance(p)))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });
    state.hover = nearest;
    if response.drag_started() {
        state.dragging = nearest;
    }
    let keyed = |i: usize| {
        s.doc.graph.node(id).is_some_and(|n| match &n.kind {
            NodeKind::MeshWarp(w) => w
                .points
                .get(i)
                .is_some_and(|p| p.iter().any(|v| v.has_key(s.frame))),
            _ => false,
        })
    };
    for (i, p) in points.iter().enumerate() {
        let active = state.dragging == Some(i) || state.hover == Some(i);
        let r = if active { 6.0 } else { 4.5 };
        painter.circle_filled(*p, r + 1.5, theme::BG_BASE);
        let fill = if keyed(i) {
            // Amber marks a key on this frame (time), as on the Timeline.
            theme::ACCENT_TIME
        } else {
            theme::ACCENT_SELECT
        };
        painter.circle_filled(*p, r, fill);
    }
    if let Some(i) = state.dragging {
        if response.dragged() {
            if let (Some(p), Some(back)) = (response.interact_pointer_pos(), m.inverse()) {
                let (x, y) = back.apply(p.x, p.y);
                let (r, c) = ((i / cols) as u32, (i % cols) as u32);
                let rest = grid.rest(r, c);
                move_point(s, id, i, [x - rest[0], y - rest[1]]);
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if response.drag_stopped() {
            state.dragging = None;
            s.end_gesture();
        }
        return true;
    }
    if nearest.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    true
}

/// Set point `i`'s offset, keyed on this frame in Animate mode (or when
/// it's already animated), as one merged undo step per drag.
fn move_point(s: &mut Session, id: NodeId, i: usize, offset: [f32; 2]) {
    use symphony_core::anim::Value;
    let Some(before) = s.doc.graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    let NodeKind::MeshWarp(w) = &mut after.kind else {
        return;
    };
    let Some(point) = w.points.get_mut(i) else {
        return;
    };
    for (value, v) in point.iter_mut().zip(offset) {
        match value {
            Value::Constant(c) if !s.animate => *c = v,
            _ => value.set_key(s.frame, v),
        }
    }
    if after != before {
        s.history.apply_merged(
            &mut s.doc,
            "Move mesh point",
            &format!("mesh-point-{}-{i}", id.0),
            Edit::SetNodeProperties { id, before, after },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moving a point in Animate mode keys it on this frame only; off
    /// Animate it changes for every frame. Undo takes the drag back.
    #[test]
    fn moving_a_point_keys_it_in_animate_mode() {
        let mut s = Session::new_scene();
        let node = s.doc.layers[0].node;
        crate::views::node::insert_near(&mut s, node, crate::views::node::mesh_warp(100, 100));
        let id = s.doc.graph.input(node, 0).unwrap();
        assert!(matches!(
            s.doc.graph.node(id).unwrap().kind,
            NodeKind::MeshWarp(_)
        ));
        let points = |s: &Session| match &s.doc.graph.node(id).unwrap().kind {
            NodeKind::MeshWarp(w) => w.clone(),
            _ => unreachable!(),
        };
        s.animate = false;
        move_point(&mut s, id, 12, [5.0, 0.0]);
        s.end_gesture();
        assert_eq!(points(&s).grid_at(30).offsets[12], [5.0, 0.0]);
        s.animate = true;
        s.frame = 10;
        move_point(&mut s, id, 12, [20.0, 0.0]);
        s.end_gesture();
        assert!(points(&s).has_key(10));
        assert_eq!(points(&s).grid_at(10).offsets[12], [20.0, 0.0]);
        s.undo();
        assert!(!points(&s).has_key(10));
    }
}
