//! Rig control widgets in the Camera view, driven with the mouse: a knob
//! drag moves the control position (and keys it in Animate mode), Ctrl
//! snaps to a pose, a double-click on a pose dot jumps there, and a bone
//! drag under an offset keys the bone without the offset.

use eframe::egui::{self, Modifiers};
use egui_kittest::Harness;
use symphony_core::anim::Value;
use symphony_core::control::{Channel, Control, Offset, Pose, Widget};
use symphony_core::graph::{NodeId, NodeKind, PegParams};
use symphony_core::rig::{Bone, Stretch};

use super::deformers::{click, drag, CTRL};
use super::{rig_from, Rig};
use crate::session::{Session, Tool};

struct Scene {
    h: Harness<'static, Rig>,
    control: NodeId,
    bone: NodeId,
}

/// A skeleton with one bone from the origin along +x, 200 long, under a
/// Control whose pose at 1 adds `turn` degrees and `grow` stretch to the
/// bone. The slider runs 200 long along y = -100, centred on x = 0 (Min
/// at the left). The bone is selected, with the Transform tool.
fn scene(turn: f32, grow: f32, animate: bool) -> Scene {
    let mut s = Session::new_scene();
    s.animate = animate;
    let graph = &mut s.doc.graph;
    let control = graph.add(
        "Smile",
        NodeKind::Control(Control {
            poses: vec![Pose {
                at: 1.0,
                offsets: vec![
                    Offset {
                        node: NodeId(0),
                        channel: Channel::Rotation,
                        amount: turn,
                    },
                    Offset {
                        node: NodeId(0),
                        channel: Channel::Stretch,
                        amount: grow,
                    },
                ],
            }],
            widget: Widget {
                position: [0.0, -100.0],
                size: 200.0,
                ..Widget::default()
            },
            ..Control::default()
        }),
    );
    let skeleton = graph.add("Skeleton", NodeKind::Skeleton(PegParams::default()));
    let bone = graph.add(
        "Arm",
        NodeKind::Bone(Bone {
            length: 200.0,
            stretch_mode: Stretch::Simple,
            ..Bone::default()
        }),
    );
    graph.connect(control, skeleton, 0);
    graph.connect(skeleton, bone, 0);
    if let Some(NodeKind::Control(c)) = graph.node_mut(control).map(|n| &mut n.kind) {
        for offset in &mut c.poses[0].offsets {
            offset.node = bone;
        }
    }
    s.selected_node = Some(bone);
    s.tool = Tool::Transform;
    s.end_gesture();
    let mut h = rig_from(s);
    h.run_steps(2);
    Scene { h, control, bone }
}

fn control(h: &Harness<'static, Rig>, id: NodeId) -> Control {
    match &h.state().s.doc.graph.node(id).unwrap().kind {
        NodeKind::Control(c) => c.clone(),
        _ => panic!("not a control"),
    }
}

/// The bone's tip on the stage.
fn tip(h: &Harness<'static, Rig>, bone: NodeId) -> (f32, f32) {
    let graph = &h.state().s.doc.graph;
    let (x, y) = graph.bone_pose(bone, 1).unwrap().tip();
    graph.rig_stage_pose(bone, 1).unwrap().transform.apply(x, y)
}

/// Two clicks in two frames: kittest's clock moves too far between two
/// [`click`] calls for egui to count a double-click.
fn double_click(h: &mut Harness<'static, Rig>, x: f32, y: f32) {
    let pos = h.state().canvas.to_screen(x, y);
    h.input_mut().events.push(egui::Event::PointerMoved(pos));
    for _ in 0..2 {
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
    }
    h.run_steps(2);
}

fn near(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 2.0 && (a.1 - b.1).abs() < 2.0
}

#[test]
fn a_knob_drag_moves_the_control_and_the_rig_follows() {
    let Scene {
        mut h,
        control: id,
        bone,
    } = scene(90.0, 0.0, false);
    let steps = h.state().s.history.undo_len();
    drag(&mut h, (0.0, -100.0), (100.0, -100.0), Modifiers::NONE);
    assert_eq!(control(&h, id).value.at(1), 1.0);
    assert!(near(tip(&h, bone), (0.0, 200.0)), "tip {:?}", tip(&h, bone));
    let s = &h.state().s;
    assert_eq!(s.doc.graph.bone(bone).unwrap().rotation.at(1), 0.0);
    assert_eq!(s.history.undo_len(), steps + 1, "one undo step");
    assert_eq!(s.selected_node, Some(bone), "the bone stays selected");
    h.state_mut().s.undo();
    assert_eq!(control(&h, id).value.at(1), 0.0);
}

#[test]
fn in_animate_mode_a_knob_drag_keys_the_control_only() {
    let Scene {
        mut h,
        control: id,
        bone,
    } = scene(90.0, 0.0, true);
    drag(&mut h, (0.0, -100.0), (50.0, -100.0), Modifiers::NONE);
    let c = control(&h, id);
    assert!(c.has_key(1), "the control position is keyed");
    assert!((c.value.at(1) - 0.5).abs() < 0.02, "at {}", c.value.at(1));
    let rotation = &h.state().s.doc.graph.bone(bone).unwrap().rotation;
    assert!(matches!(rotation, Value::Constant(_)), "no rig key");
}

#[test]
fn ctrl_snaps_the_knob_to_the_nearest_pose() {
    let Scene {
        mut h, control: id, ..
    } = scene(90.0, 0.0, false);
    drag(&mut h, (0.0, -100.0), (60.0, -100.0), CTRL);
    assert_eq!(control(&h, id).value.at(1), 1.0);
    drag(&mut h, (100.0, -100.0), (30.0, -100.0), CTRL);
    assert_eq!(control(&h, id).value.at(1), 0.0, "the rest pose");
}

#[test]
fn a_double_click_on_a_pose_dot_jumps_there() {
    let Scene {
        mut h, control: id, ..
    } = scene(90.0, 0.0, false);
    // One click beside the rest dot puts the knob at the click; the
    // second click of a double-click puts it on the pose itself.
    click(&mut h, 1.5, -100.0);
    assert!((control(&h, id).value.at(1) - 0.015).abs() < 0.002);
    double_click(&mut h, 1.5, -100.0);
    assert_eq!(control(&h, id).value.at(1), 0.0);
}

#[test]
fn a_press_on_the_track_moves_the_knob_there() {
    let Scene {
        mut h, control: id, ..
    } = scene(90.0, 0.0, false);
    let bone = h.state().s.selected_node;
    click(&mut h, -50.0, -100.0);
    assert!((control(&h, id).value.at(1) + 0.5).abs() < 0.02);
    assert_eq!(h.state().s.selected_node, bone, "the selection stays");
}

#[test]
fn hidden_rig_controls_hide_the_widget() {
    let Scene {
        mut h, control: id, ..
    } = scene(90.0, 0.0, false);
    h.state_mut().s.show_rig_controls = false;
    h.run_steps(1);
    drag(&mut h, (0.0, -100.0), (100.0, -100.0), Modifiers::NONE);
    assert_eq!(control(&h, id).value.at(1), 0.0);
}

#[test]
fn the_widget_moves_with_the_peg_it_follows() {
    let Scene {
        mut h,
        control: id,
        bone,
    } = scene(90.0, 0.0, false);
    let graph = &mut h.state_mut().s.doc.graph;
    let peg = graph.add(
        "Head",
        NodeKind::Peg(PegParams {
            x: Value::Constant(300.0),
            ..PegParams::default()
        }),
    );
    if let Some(NodeKind::Control(c)) = graph.node_mut(id).map(|n| &mut n.kind) {
        c.widget.follow = Some(peg);
    }
    h.run_steps(1);
    drag(&mut h, (300.0, -100.0), (400.0, -100.0), Modifiers::NONE);
    assert_eq!(control(&h, id).value.at(1), 1.0);
    assert!(near(tip(&h, bone), (0.0, 200.0)), "tip {:?}", tip(&h, bone));
}

#[test]
fn a_bone_drag_under_an_offset_keys_the_bone_without_it() {
    let Scene { mut h, bone, .. } = scene(90.0, 0.5, false);
    drag(&mut h, (0.0, -100.0), (100.0, -100.0), Modifiers::NONE);
    // The bone shows 90 degrees and 1.5 long: its tip at (0, 300).
    assert!(near(tip(&h, bone), (0.0, 300.0)), "tip {:?}", tip(&h, bone));
    // Turn it on to 180 degrees and pull it longer with Ctrl.
    drag(&mut h, (0.0, 150.0), (-180.0, 0.0), CTRL);
    let (x, y) = tip(&h, bone);
    assert!(x < -300.0 && y.abs() < 2.0, "tip ({x}, {y})");
    let graph = &h.state().s.doc.graph;
    let b = graph.bone(bone).unwrap();
    assert!(
        (b.rotation.at(1) - 90.0).abs() < 0.5,
        "key {}",
        b.rotation.at(1)
    );
    let shown = graph.bone_pose(bone, 1).unwrap().scale[0];
    let key = b.stretch.at(1);
    assert!(shown > 1.5, "longer: {shown}");
    assert!((shown - 0.5 - key).abs() < 0.01, "key {key}, shown {shown}");
}

fn pose_turn(h: &Harness<'static, Rig>, id: NodeId, bone: NodeId, at: f32) -> Option<f32> {
    let c = control(h, id);
    let pose = c.poses.iter().find(|p| (p.at - at).abs() < 0.02)?;
    Some(pose.amount(bone, Channel::Rotation))
}

#[test]
fn setup_mode_records_a_bone_turn_into_the_pose_at_the_knob() {
    let Scene {
        mut h,
        control: id,
        bone,
    } = scene(0.0, 0.0, true);
    crate::views::control_edit::start_setup(&mut h.state_mut().s, id);
    h.run_steps(1);
    // The knob is a test position: no key, even in Animate mode.
    drag(&mut h, (0.0, -100.0), (100.0, -100.0), Modifiers::NONE);
    assert!(!control(&h, id).has_key(1), "no key on the control");
    assert_eq!(control(&h, id).value.at(1), 0.0);
    let steps = h.state().s.history.undo_len();
    drag(&mut h, (100.0, 0.0), (0.0, 100.0), Modifiers::NONE);
    assert!(near(tip(&h, bone), (0.0, 200.0)), "tip {:?}", tip(&h, bone));
    let rotation = &h.state().s.doc.graph.bone(bone).unwrap().rotation;
    assert!(
        matches!(rotation, Value::Constant(r) if *r == 0.0),
        "no rig key: {rotation:?}"
    );
    let turn = pose_turn(&h, id, bone, 1.0).unwrap();
    assert!((turn - 90.0).abs() < 0.5, "pose turn {turn}");
    assert_eq!(h.state().s.history.undo_len(), steps + 1, "one undo step");
    // Done: the knob goes back to its keyed position (0, the rest).
    crate::views::control_edit::end_setup(&mut h.state_mut().s);
    assert!(near(tip(&h, bone), (200.0, 0.0)), "tip {:?}", tip(&h, bone));
    // Undo takes the turn out of the pose.
    h.state_mut().s.undo();
    assert_eq!(pose_turn(&h, id, bone, 1.0), Some(0.0));
}

#[test]
fn setup_mode_adds_a_pose_at_the_knob_from_its_panel() {
    use egui_kittest::kittest::Queryable;
    let Scene {
        mut h,
        control: id,
        bone,
    } = scene(0.0, 0.0, false);
    h.state_mut().s.selected_node = Some(id);
    h.run_steps(2);
    h.get_by_label("Setup mode").click();
    h.run_steps(2);
    drag(&mut h, (0.0, -100.0), (-50.0, -100.0), Modifiers::NONE);
    h.get_by_label("Add pose").click();
    h.run_steps(2);
    assert!(pose_turn(&h, id, bone, -0.5).is_some(), "a pose at -0.5");
    // Select the bone and turn it: the turn goes into the new pose.
    h.state_mut().s.selected_node = Some(bone);
    h.run_steps(2);
    drag(&mut h, (100.0, 0.0), (0.0, 100.0), Modifiers::NONE);
    let turn = pose_turn(&h, id, bone, -0.5).unwrap();
    assert!((turn - 90.0).abs() < 0.5, "pose turn {turn}");
    h.get_by_label("Done").click();
    h.run_steps(2);
    assert!(h.state().s.control_setup.is_none());
}

#[test]
fn off_a_pose_setup_mode_leaves_edits_as_keys() {
    let Scene {
        mut h,
        control: id,
        bone,
    } = scene(0.0, 0.0, false);
    crate::views::control_edit::start_setup(&mut h.state_mut().s, id);
    h.run_steps(1);
    drag(&mut h, (0.0, -100.0), (50.0, -100.0), Modifiers::NONE);
    drag(&mut h, (100.0, 0.0), (0.0, 100.0), Modifiers::NONE);
    let rotation = h.state().s.doc.graph.bone(bone).unwrap().rotation.at(1);
    assert!((rotation - 90.0).abs() < 0.5, "a normal edit: {rotation}");
    assert_eq!(pose_turn(&h, id, bone, 1.0), Some(0.0));
}

#[test]
fn setup_mode_records_a_skeleton_move_as_peg_offsets() {
    let Scene {
        mut h,
        control: id,
        bone,
    } = scene(0.0, 0.0, false);
    let skeleton = h.state().s.doc.graph.input(bone, 0).unwrap();
    crate::views::control_edit::start_setup(&mut h.state_mut().s, id);
    h.run_steps(1);
    drag(&mut h, (0.0, -100.0), (100.0, -100.0), Modifiers::NONE);
    drag(&mut h, (0.0, 0.0), (50.0, 40.0), Modifiers::NONE);
    let c = control(&h, id);
    let pose = c.poses.iter().find(|p| p.at == 1.0).unwrap();
    let (x, y) = (
        pose.amount(skeleton, Channel::X),
        pose.amount(skeleton, Channel::Y),
    );
    assert!(
        (x - 50.0).abs() < 1.0 && (y - 40.0).abs() < 1.0,
        "({x}, {y})"
    );
    let NodeKind::Skeleton(peg) = &h.state().s.doc.graph.node(skeleton).unwrap().kind else {
        panic!("not a skeleton");
    };
    assert_eq!((peg.x.at(1), peg.y.at(1)), (0.0, 0.0), "no rig key");
    assert!(
        near(tip(&h, bone), (250.0, 40.0)),
        "tip {:?}",
        tip(&h, bone)
    );
}

/// Layer 0 with a line from (-100, 0) to (100, 0), under a Control whose
/// slider runs along y = -200. The Control is selected, with the
/// Transform tool.
fn art_under_a_selected_control() -> (Harness<'static, Rig>, NodeId) {
    let mut h = super::rig();
    h.state_mut().s.tool = Tool::Brush;
    super::drag(
        &mut h,
        &super::line((-100.0, 0.0), (100.0, 0.0), 20),
        1.0,
        symphony_pen::Tool::Pen,
    );
    let s = &mut h.state_mut().s;
    let layer = s.doc.layers[0].node;
    let control = s.doc.graph.add(
        "Kick",
        NodeKind::Control(Control {
            widget: Widget {
                position: [0.0, -200.0],
                size: 200.0,
                ..Widget::default()
            },
            ..Control::default()
        }),
    );
    // At the top of the layer's chain, as the Node view places it.
    let mut top = layer;
    while let Some(up) = s.doc.graph.input(top, 0) {
        top = up;
    }
    s.doc.graph.connect(control, top, 0);
    s.selected_node = Some(control);
    s.tool = Tool::Transform;
    s.end_gesture();
    h.run_steps(2);
    (h, control)
}

#[test]
fn a_selected_control_hides_the_layer_box_and_a_press_drops_it() {
    let (mut h, _) = art_under_a_selected_control();
    // No box: no handle hint over the art's corner.
    let corner = h.state().canvas.to_screen(-100.0, 0.0);
    h.input_mut().events.push(egui::Event::PointerMoved(corner));
    h.run_steps(2);
    assert!(h.state().canvas.select.hover.is_none(), "no transform box");
    let steps = h.state().s.history.undo_len();
    drag(&mut h, (0.0, 0.0), (60.0, 60.0), Modifiers::NONE);
    assert_eq!(h.state().s.history.undo_len(), steps, "the art stays");
    assert_eq!(h.state().s.selected_node, None, "the press drops it");
    // The next press reaches the artwork again.
    drag(&mut h, (0.0, 0.0), (60.0, 60.0), Modifiers::NONE);
    assert_eq!(h.state().s.history.undo_len(), steps + 1, "the art moves");
}

#[test]
fn a_knob_drag_keeps_the_control_selected() {
    let (mut h, control) = art_under_a_selected_control();
    drag(&mut h, (0.0, -200.0), (50.0, -200.0), Modifiers::NONE);
    assert_eq!(h.state().s.selected_node, Some(control));
}
