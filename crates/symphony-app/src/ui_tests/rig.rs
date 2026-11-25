//! Bone handles in the Camera view, driven with the mouse: a drag on a
//! bone's body turns it about its base (Ctrl: and stretches it), as in
//! Harmony's Skeleton Controller; the skeleton's base still moves the
//! whole rig.

use eframe::egui::Modifiers;
use egui_kittest::Harness;
use symphony_core::graph::{NodeId, NodeKind, PegParams};
use symphony_core::rig::{Bone, Stretch};

use super::deformers::{drag, CTRL};
use super::{rig_from, Rig};
use crate::session::{Session, Tool};

/// A skeleton with one bone from the origin along +x, 200 long,
/// selected, with the Transform tool.
fn one_bone(stretch_mode: Stretch) -> (Harness<'static, Rig>, NodeId, NodeId) {
    let mut s = Session::new_scene();
    s.animate = false;
    let skeleton = s
        .doc
        .graph
        .add("Skeleton", NodeKind::Skeleton(PegParams::default()));
    let bone = s.doc.graph.add(
        "Arm",
        NodeKind::Bone(Bone {
            length: 200.0,
            stretch_mode,
            ..Bone::default()
        }),
    );
    s.doc.graph.connect(skeleton, bone, 0);
    s.selected_node = Some(bone);
    s.tool = Tool::Transform;
    s.end_gesture();
    let mut h = rig_from(s);
    h.run_steps(2);
    (h, skeleton, bone)
}

/// The bone's tip on the stage.
fn tip(h: &Harness<'static, Rig>, bone: NodeId) -> (f32, f32) {
    let graph = &h.state().s.doc.graph;
    let (x, y) = graph.bone_pose(bone, 1).unwrap().tip();
    graph.rig_stage_pose(bone, 1).unwrap().transform.apply(x, y)
}

fn near(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 2.0 && (a.1 - b.1).abs() < 2.0
}

#[test]
fn a_drag_on_the_body_turns_the_bone_about_its_base() {
    let (mut h, skeleton, bone) = one_bone(Stretch::Rigid);
    let steps = h.state().s.history.undo_len();
    drag(&mut h, (100.0, 0.0), (0.0, 100.0), Modifiers::NONE);
    assert!(near(tip(&h, bone), (0.0, 200.0)), "tip {:?}", tip(&h, bone));
    let s = &h.state().s;
    let NodeKind::Skeleton(peg) = &s.doc.graph.node(skeleton).unwrap().kind else {
        panic!("not a skeleton");
    };
    assert_eq!((peg.x.at(1), peg.y.at(1)), (0.0, 0.0), "the rig stayed");
    assert_eq!(s.history.undo_len(), steps + 1, "one undo step");
}

#[test]
fn ctrl_turns_and_stretches_a_stretchable_bone() {
    let (mut h, _, bone) = one_bone(Stretch::Simple);
    drag(&mut h, (100.0, 0.0), (150.0, 0.0), CTRL);
    assert!(near(tip(&h, bone), (300.0, 0.0)), "tip {:?}", tip(&h, bone));
    let stretch = h.state().s.doc.graph.bone(bone).unwrap().stretch.at(1);
    assert!((stretch - 1.5).abs() < 0.01, "stretch {stretch}");
}

#[test]
fn without_ctrl_a_turn_keeps_the_length() {
    let (mut h, _, bone) = one_bone(Stretch::Simple);
    drag(&mut h, (100.0, 0.0), (150.0, 150.0), Modifiers::NONE);
    let (x, y) = tip(&h, bone);
    assert!((x.hypot(y) - 200.0).abs() < 2.0, "tip ({x}, {y})");
}

#[test]
fn the_skeleton_base_still_moves_the_whole_rig() {
    let (mut h, skeleton, bone) = one_bone(Stretch::Rigid);
    drag(&mut h, (0.0, 0.0), (50.0, 40.0), Modifiers::NONE);
    let NodeKind::Skeleton(peg) = &h.state().s.doc.graph.node(skeleton).unwrap().kind else {
        panic!("not a skeleton");
    };
    assert!((peg.x.at(1) - 50.0).abs() < 1.0 && (peg.y.at(1) - 40.0).abs() < 1.0);
    assert!(
        near(tip(&h, bone), (250.0, 40.0)),
        "tip {:?}",
        tip(&h, bone)
    );
}

#[test]
fn a_turn_stops_at_the_angle_limit() {
    let (mut h, _, bone) = one_bone(Stretch::Rigid);
    let graph = &mut h.state_mut().s.doc.graph;
    if let Some(NodeKind::Bone(b)) = graph.node_mut(bone).map(|n| &mut n.kind) {
        b.angle_limits = Some([-30.0, 30.0]);
    }
    h.run_steps(1);
    // Sweep a quarter turn; the bone stops a third of the way.
    drag(&mut h, (100.0, 0.0), (0.0, 100.0), Modifiers::NONE);
    let rotation = h.state().s.doc.graph.bone(bone).unwrap().rotation.at(1);
    assert!((rotation.abs() - 30.0).abs() < 0.01, "rotation {rotation}");
    let (x, y) = tip(&h, bone);
    let angle = y.atan2(x).to_degrees();
    assert!((angle - 30.0).abs() < 0.5, "tip at {angle} degrees");
}
