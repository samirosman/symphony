//! Skeleton setup view (the rig plans): open a Skeleton node in the Node
//! View panel to see its bones in their rest pose as diamonds, with a
//! thumbnail of the drawing each bone carries. Drag a base or a tip to
//! reshape the rig (a joint is one point: dragging it moves both bones
//! that meet there). Press and hold an endpoint, then drag, to grow a new
//! bone from it. Back returns to the graph.

use std::collections::HashMap;

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use symphony_core::graph::{NodeId, NodeKind};
use symphony_core::history::Edit;
use symphony_core::math::Affine;
use symphony_core::rig::Bone;
use symphony_ui::theme;

use crate::session::Session;

/// How long to hold an endpoint before a drag grows a new bone (seconds).
const HOLD: f64 = 0.35;
/// How near (screen points) a press must be to grab an endpoint.
const GRAB: f32 = 10.0;

#[derive(Default)]
pub struct SetupState {
    /// The skeleton open in the panel.
    pub open: Option<NodeId>,
    drag: Option<Drag>,
    /// Skeleton space -> screen, frozen while dragging (the view doesn't
    /// refit under the pointer).
    frozen: Option<Affine>,
    /// Thumbnails by bone, scene version, and frame.
    thumb_scene: String,
    thumbs: HashMap<(NodeId, u64, u32), Option<egui::TextureHandle>>,
}

#[derive(Clone, Copy)]
enum Drag {
    /// A bone's tip (also the base of the bones that hang from it).
    Tip(NodeId),
    /// A root bone's base.
    Base(NodeId),
}

/// An endpoint on screen: a root bone's base, or a bone's tip.
#[derive(Clone, Copy, PartialEq)]
enum End {
    Base(NodeId),
    Tip(NodeId),
}

/// The setup view, in place of the graph. Returns false once Back is
/// pressed (or the skeleton is gone).
pub fn ui(ui: &mut egui::Ui, s: &mut Session, state: &mut SetupState) -> bool {
    let Some(skeleton) = state.open.filter(|&id| {
        matches!(
            s.doc.graph.node(id).map(|n| &n.kind),
            Some(NodeKind::Skeleton(_))
        )
    }) else {
        state.open = None;
        return false;
    };
    let mut back = false;
    ui.horizontal(|ui| {
        if ui.button("← Back").clicked() {
            back = true;
        }
        let name = s.doc.graph.node(skeleton).map(|n| n.name.clone()).unwrap_or_default();
        ui.label(egui::RichText::new(name).strong());
        ui.label(
            egui::RichText::new(
                "Rest pose. Drag a base or tip to reshape; hold an endpoint, then drag, to grow a bone.",
            )
            .font(theme::dense())
            .color(theme::TEXT_SECONDARY),
        );
    });
    if back {
        state.open = None;
        return false;
    }
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;
    painter.rect_filled(
        rect,
        theme::RADIUS_CONTROL,
        theme::BG_BASE.gamma_multiply(0.6),
    );
    let graph = &s.doc.graph;
    let bones: Vec<NodeId> = graph
        .rig_bones(skeleton)
        .into_iter()
        .filter(|&b| graph.bone(b).is_some())
        .collect();
    // Rest endpoints in skeleton space.
    let rest: Vec<(NodeId, [f32; 2], [f32; 2])> = bones
        .iter()
        .filter_map(|&b| {
            let pose = graph.bone_pose(b, s.frame)?;
            let (bx, by) = pose.rest.apply(0.0, 0.0);
            let (tx, ty) = pose.rest.apply(pose.length, 0.0);
            Some((b, [bx, by], [tx, ty]))
        })
        .collect();
    // Fit the rig in the panel (kept while dragging).
    let to_screen = match (state.frozen, state.drag) {
        (Some(m), Some(_)) => m,
        _ => {
            let (mut lo, mut hi) = ([-100.0f32, -100.0], [100.0f32, 100.0]);
            for (_, a, b) in &rest {
                for p in [a, b] {
                    lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                    hi = [hi[0].max(p[0]), hi[1].max(p[1])];
                }
            }
            let pad = 60.0;
            let k = ((rect.width() - pad * 2.0) / (hi[0] - lo[0]).max(1.0))
                .min((rect.height() - pad * 2.0) / (hi[1] - lo[1]).max(1.0))
                .clamp(0.02, 4.0);
            let c = rect.center();
            Affine::translate(c.x, c.y)
                .compose(Affine::scale(k, k))
                .compose(Affine::translate(
                    -(lo[0] + hi[0]) / 2.0,
                    -(lo[1] + hi[1]) / 2.0,
                ))
        }
    };
    let screen = |p: [f32; 2]| {
        let (x, y) = to_screen.apply(p[0], p[1]);
        Pos2::new(x, y)
    };
    let painter = painter.with_clip_rect(rect);
    // Thumbnails of each bone's drawings, beside its middle.
    for &(b, a, z) in &rest {
        if let Some(tex) = thumbnail(s, state, b, ui.ctx()) {
            let mid = screen([(a[0] + z[0]) / 2.0, (a[1] + z[1]) / 2.0]);
            let side = 56.0;
            let r = Rect::from_center_size(mid + Vec2::new(0.0, -side * 0.75), Vec2::splat(side));
            painter.rect_filled(r, 6.0, theme::GLASS_PANEL);
            painter.image(
                tex.id(),
                r.shrink(3.0),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }
    // Bones as diamonds, base to tip.
    let selected = s.selected_node;
    for &(b, a, z) in &rest {
        let (pa, pz) = (screen(a), screen(z));
        let d = pz - pa;
        let len = d.length();
        let ink = if selected == Some(b) {
            theme::ACCENT_SELECT
        } else {
            theme::TEXT_SECONDARY
        };
        if len > 4.0 {
            let along = d / len;
            let across = Vec2::new(-along.y, along.x) * (len * 0.12).clamp(4.0, 16.0);
            let shoulder = pa + along * (len * 0.22);
            painter.add(egui::Shape::convex_polygon(
                vec![pa, shoulder + across, pz, shoulder - across],
                ink.gamma_multiply(0.22),
                Stroke::new(1.5, ink),
            ));
        }
        if let Some(n) = s.doc.graph.node(b) {
            painter.text(
                pa + d * 0.5 + Vec2::new(0.0, 14.0),
                Align2::CENTER_TOP,
                &n.name,
                theme::dense(),
                theme::TEXT_SECONDARY,
            );
        }
    }
    // Endpoints: root bases and tips (a tip is also its children's base).
    let mut ends: Vec<(End, Pos2)> = Vec::new();
    for &(b, a, z) in &rest {
        if s.doc.graph.parent_bone(b).is_none() {
            ends.push((End::Base(b), screen(a)));
        }
        ends.push((End::Tip(b), screen(z)));
    }
    let pointer = response.hover_pos().or(response.interact_pointer_pos());
    let nearest = |p: Pos2| {
        ends.iter()
            .map(|&(e, q)| (e, q.distance(p)))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .map(|(e, _)| e)
    };
    let hovered = pointer.and_then(nearest);
    // Holding an endpoint: a ring fills up, then a drag grows a bone.
    let held_for = ui.input(|i| {
        i.pointer
            .press_start_time()
            .filter(|_| i.pointer.primary_down())
            .map(|t| i.time - t)
    });
    for &(e, q) in &ends {
        let hot = hovered == Some(e);
        painter.circle_filled(q, if hot { 6.5 } else { 5.0 }, theme::BG_BASE);
        painter.circle_stroke(
            q,
            if hot { 5.0 } else { 3.8 },
            Stroke::new(1.6, theme::ACCENT_SELECT),
        );
        if hot && state.drag.is_none() {
            if let Some(t) = held_for {
                let p = (t / HOLD).min(1.0) as f32;
                let arc: Vec<Pos2> = (0..=24)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2
                            + p * std::f32::consts::TAU * i as f32 / 24.0;
                        q + Vec2::new(a.cos(), a.sin()) * 10.0
                    })
                    .collect();
                painter.add(egui::Shape::line(arc, Stroke::new(2.0, theme::ACCENT_TIME)));
                if p < 1.0 {
                    ui.ctx().request_repaint();
                }
            }
        }
    }
    if rest.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No bones yet: use Add Bone with the skeleton selected.",
            theme::body(),
            theme::TEXT_SECONDARY,
        );
    }
    // Interaction.
    if response.drag_started() {
        let grab = ui.input(|i| i.pointer.press_origin());
        let held = held_for.is_some_and(|t| t >= HOLD);
        state.frozen = Some(to_screen);
        state.drag = grab.and_then(nearest).and_then(|e| {
            if held {
                // Grow a new bone from this endpoint.
                let parent = match e {
                    End::Tip(b) => b,
                    End::Base(_) => skeleton,
                };
                let origin = match e {
                    End::Base(b) => rest.iter().find(|r| r.0 == b).map(|r| r.1),
                    End::Tip(_) => None,
                };
                grow_bone(s, parent, origin).map(Drag::Tip)
            } else {
                Some(match e {
                    End::Tip(b) => Drag::Tip(b),
                    End::Base(b) => Drag::Base(b),
                })
            }
        });
        if let Some(Drag::Tip(b) | Drag::Base(b)) = state.drag {
            s.select_graph_node(b, true);
        }
    }
    if let (Some(drag), Some(p), Some(back)) = (
        state.drag,
        response.interact_pointer_pos(),
        state.frozen.and_then(|m| m.inverse()),
    ) {
        let (x, y) = back.apply(p.x, p.y);
        match drag {
            Drag::Tip(b) => set_tip(s, b, [x, y]),
            Drag::Base(b) => set_origin(s, b, [x, y]),
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if response.drag_stopped() || (state.drag.is_some() && !ui.input(|i| i.pointer.primary_down()))
    {
        state.drag = None;
        state.frozen = None;
        s.end_gesture();
    }
    if response.clicked() {
        if let Some(End::Tip(b) | End::Base(b)) = pointer.and_then(nearest) {
            s.select_graph_node(b, true);
        }
    }
    true
}

/// Change bone `b`'s rest (one undo step per drag).
fn edit_bone(s: &mut Session, b: NodeId, name: &str, f: impl FnOnce(&mut Bone)) {
    let Some(before) = s.doc.graph.node(b).cloned() else {
        return;
    };
    let mut after = before.clone();
    let NodeKind::Bone(bone) = &mut after.kind else {
        return;
    };
    f(bone);
    if after != before {
        s.history.apply_merged(
            &mut s.doc,
            name,
            &format!("skeleton-setup-{}", b.0),
            Edit::SetNodeProperties {
                id: b,
                before,
                after,
            },
        );
    }
}

/// Put bone `b`'s rest tip at `p` (skeleton space): its length and rest
/// angle follow; bones hanging from it come along.
pub fn set_tip(s: &mut Session, b: NodeId, p: [f32; 2]) {
    let Some(pose) = s.doc.graph.bone_pose(b, s.frame) else {
        return;
    };
    let (bx, by) = pose.rest.apply(0.0, 0.0);
    let (dx, dy) = (p[0] - bx, p[1] - by);
    if dx.hypot(dy) < 1.0 {
        return;
    }
    let parent_angle = s
        .doc
        .graph
        .parent_bone(b)
        .and_then(|q| s.doc.graph.bone_pose(q, s.frame))
        .map_or(0.0, |q| q.rest.b.atan2(q.rest.a));
    let angle = (dy.atan2(dx) - parent_angle).to_degrees();
    edit_bone(s, b, "Reshape rig", |bone| {
        bone.length = dx.hypot(dy);
        bone.rest_angle = (angle + 180.0).rem_euclid(360.0) - 180.0;
    });
}

/// Move a root bone's base (its origin).
fn set_origin(s: &mut Session, b: NodeId, p: [f32; 2]) {
    edit_bone(s, b, "Reshape rig", |bone| bone.origin = p);
}

/// Grow a bone from `parent` (a bone's tip, or the skeleton at `origin`):
/// a short bone, connected and selected, ready to be dragged out.
fn grow_bone(s: &mut Session, parent: NodeId, origin: Option<[f32; 2]>) -> Option<NodeId> {
    let bone = Bone {
        length: 1.0,
        origin: origin.unwrap_or([0.0, 0.0]),
        ..Bone::default()
    };
    super::node::add_bone_under(s, parent, bone)
}

/// Combined preview of the bone’s attached drawings at the current frame.
fn thumbnail(
    s: &Session,
    state: &mut SetupState,
    b: NodeId,
    ctx: &egui::Context,
) -> Option<egui::TextureHandle> {
    let key = (b, s.doc.version, s.frame);
    if state.thumb_scene != s.key || state.thumbs.len() > 256 {
        state.thumb_scene.clone_from(&s.key);
        state.thumbs.clear();
    }
    state
        .thumbs
        .entry(key)
        .or_insert_with(|| super::node::bone_thumbnail(s, b, ctx))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphony_core::graph::PegParams;

    /// Reshaping in the rest pose: dragging a tip changes the bone's
    /// length and angle, the bone hanging from it stays attached, and
    /// growing from a tip adds a connected bone. Each is undoable.
    #[test]
    fn reshaping_and_growing_bones_in_the_rest_pose() {
        let mut s = Session::new_scene();
        let skeleton = s
            .doc
            .graph
            .add("Skeleton", NodeKind::Skeleton(PegParams::default()));
        let upper = s.doc.graph.add(
            "Upper",
            NodeKind::Bone(Bone {
                length: 100.0,
                ..Bone::default()
            }),
        );
        let lower = s.doc.graph.add(
            "Lower",
            NodeKind::Bone(Bone {
                length: 100.0,
                ..Bone::default()
            }),
        );
        s.doc.graph.connect(skeleton, upper, 0);
        s.doc.graph.connect(upper, lower, 0);
        set_tip(&mut s, upper, [0.0, 50.0]);
        s.end_gesture();
        let rest = |s: &Session, b| {
            let p = s.doc.graph.bone_pose(b, 1).unwrap();
            let (x, y) = p.rest.apply(p.length, 0.0);
            [x, y]
        };
        let tip = rest(&s, upper);
        assert!(
            tip[0].abs() < 1e-3 && (tip[1] - 50.0).abs() < 1e-3,
            "{tip:?}"
        );
        // The lower bone still starts at the upper's tip, pointing on.
        let low = s.doc.graph.bone_pose(lower, 1).unwrap();
        let (bx, by) = low.rest.apply(0.0, 0.0);
        assert!(bx.abs() < 1e-3 && (by - 50.0).abs() < 1e-3);
        assert!((rest(&s, lower)[1] - 150.0).abs() < 1e-3);
        // Grow a bone from the lower's tip.
        let grown = grow_bone(&mut s, lower, None).unwrap();
        set_tip(&mut s, grown, [40.0, 150.0]);
        s.end_gesture();
        assert_eq!(s.doc.graph.parent_bone(grown), Some(lower));
        assert!((s.doc.graph.bone(grown).unwrap().length - 40.0).abs() < 1e-3);
        s.undo();
        s.undo();
        assert!(s.doc.graph.node(grown).is_none(), "growing undoes");
    }
}
