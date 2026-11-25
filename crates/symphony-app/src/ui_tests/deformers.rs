//! The Curve, Envelope and Mesh Warp editors, driven in the real Camera
//! view with the mouse the way an artist would: place, drag, Alt and Ctrl,
//! arrow keys, the canvas buttons, undo, and a save and open.

use eframe::egui::{self, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use symphony_core::anim::Value;
use symphony_core::envelope::{CurveDeformer, CurvePoint, PointPose};
use symphony_core::graph::{NodeId, NodeKind};
use symphony_core::warp::{Handle, MeshWarp};

use super::{dark_rows, line_scene, rig_from, Rig};
use crate::session::Tool;

/// Press at the first scene point, move through the rest, release at the
/// last, one frame each, with `mods` held.
fn mouse(h: &mut Harness<'static, Rig>, pts: &[(f32, f32)], mods: Modifiers) {
    mouse_with(h, pts, mods, egui::PointerButton::Primary);
}

/// [`mouse`] with any button.
fn mouse_with(
    h: &mut Harness<'static, Rig>,
    pts: &[(f32, f32)],
    mods: Modifiers,
    button: egui::PointerButton,
) {
    let n = pts.len();
    for (i, &(x, y)) in pts.iter().enumerate() {
        let pos = h.state().canvas.to_screen(x, y);
        if i == 0 {
            h.input_mut()
                .events
                .push(egui::Event::ModifiersChanged(mods));
        }
        h.input_mut().events.push(egui::Event::PointerMoved(pos));
        if i == 0 {
            h.input_mut().events.push(egui::Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers: mods,
            });
        }
        h.step();
        // Release in a frame of its own: egui applies no move on the
        // frame the button comes up.
        if i + 1 == n {
            h.input_mut().events.push(egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers: mods,
            });
            h.step();
        }
    }
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn drag(h: &mut Harness<'static, Rig>, from: (f32, f32), to: (f32, f32), mods: Modifiers) {
    mouse(h, &super::line(from, to, 8), mods);
}

fn click(h: &mut Harness<'static, Rig>, x: f32, y: f32) {
    mouse(h, &[(x, y), (x, y)], Modifiers::NONE);
}

fn key(h: &mut Harness<'static, Rig>, key: egui::Key, mods: Modifiers) {
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(mods));
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: mods,
        });
    }
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

const ALT: Modifiers = Modifiers {
    alt: true,
    ctrl: false,
    shift: false,
    mac_cmd: false,
    command: false,
};

const CTRL: Modifiers = Modifiers {
    alt: false,
    ctrl: true,
    shift: false,
    mac_cmd: false,
    command: true,
};

/// The line scene (a straight line from x -600 to 600 at y 0) with `kind`
/// put between its peg and drawing, selected, and the Transform tool.
fn deformer_rig(kind: NodeKind) -> (Harness<'static, Rig>, NodeId) {
    let (mut s, layer, _) = line_scene();
    s.animate = false;
    let node = s.doc.layers[layer].node;
    crate::views::node::insert_near(&mut s, node, kind);
    let id = s.doc.graph.input(node, 0).expect("the deformer is wired");
    s.selected_node = Some(id);
    s.tool = Tool::Transform;
    s.end_gesture();
    let mut h = rig_from(s);
    h.run_steps(2);
    (h, id)
}

fn chain(h: &Harness<'static, Rig>, id: NodeId) -> CurveDeformer {
    match &h.state().s.doc.graph.node(id).unwrap().kind {
        NodeKind::Curve(c) => (**c).clone(),
        k => panic!("not a curve: {k:?}"),
    }
}

fn warp(h: &Harness<'static, Rig>, id: NodeId) -> MeshWarp {
    match &h.state().s.doc.graph.node(id).unwrap().kind {
        NodeKind::MeshWarp(w) => (**w).clone(),
        k => panic!("not a mesh warp: {k:?}"),
    }
}

/// A straight Curve along the line: points at x -300, 0, 300, handles
/// 100 long.
fn straight_curve() -> CurveDeformer {
    let mut c = CurveDeformer::new(false);
    c.points = [-300.0, 0.0, 300.0]
        .into_iter()
        .map(|x| CurvePoint::new(PointPose::smooth([x, 0.0], [100.0, 0.0])))
        .collect();
    c
}

fn near(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

/// Degrees, wrapped to -180..180.
fn wrap(a: f32) -> f32 {
    (a + 540.0).rem_euclid(360.0) - 180.0
}

// ------------------------------------------------------------ Curve

/// A new Curve starts empty and in placing mode: a click places a point,
/// a press and drag places one and pulls out its handles, Done ends it,
/// and a click after Done places nothing.
#[test]
fn a_curve_is_placed_by_clicks_and_drags_then_done() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(CurveDeformer::new(false))));
    click(&mut h, -300.0, 0.0);
    drag(&mut h, (0.0, 0.0), (80.0, -60.0), Modifiers::NONE);
    click(&mut h, 300.0, 0.0);
    let c = chain(&h, id);
    assert_eq!(c.points.len(), 3, "{:?}", c.points);
    let p = c.points[1].rest;
    assert!(
        near(p.pos[0], 0.0, 2.0) && near(p.pos[1], 0.0, 2.0),
        "{p:?}"
    );
    let out = p.handles()[1];
    assert!(
        near(out[0], 80.0, 3.0) && near(out[1], -60.0, 3.0),
        "{out:?}"
    );
    assert!(near(p.in_length, p.out_length, 0.01), "smooth: {p:?}");
    h.get_by_label("Done").click();
    h.run_steps(2);
    click(&mut h, 200.0, 200.0);
    assert_eq!(chain(&h, id).points.len(), 3, "Done ends the placing");
    assert!(
        h.query_by_label("Edit rest pose").is_some(),
        "the canvas popup offers the rest pose"
    );
    // Each placed point is its own undo step.
    h.state_mut().s.undo();
    h.run_steps(1);
    assert_eq!(chain(&h, id).points.len(), 2);
}

/// An Envelope closes with a click on its first point; after that a
/// click places nothing, and the closed chain bends the art.
#[test]
fn an_envelope_closes_on_its_first_point() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(CurveDeformer::new(true))));
    assert!(
        h.query_by_label("Done").is_none(),
        "an Envelope has no Done"
    );
    for (x, y) in [(0.0, -200.0), (300.0, 0.0), (0.0, 200.0), (-300.0, 0.0)] {
        click(&mut h, x, y);
    }
    click(&mut h, 0.0, -200.0);
    let c = chain(&h, id);
    assert!(c.closed && c.points.len() == 4, "{:?}", c.points);
    click(&mut h, 100.0, 100.0);
    assert_eq!(chain(&h, id).points.len(), 4, "closed: no more points");
    // Pull the bottom point down: the line inside follows.
    let flat = dark_rows(&h.state().s, 960);
    drag(&mut h, (0.0, 200.0), (0.0, 400.0), Modifiers::NONE);
    let bent = dark_rows(&h.state().s, 960);
    assert_ne!(bent, flat, "the envelope bends the line");
}

/// Dragging the middle point bends the art; the drag is one undo step.
#[test]
fn dragging_a_curve_point_bends_the_art_as_one_undo_step() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
    let flat = dark_rows(&h.state().s, 960);
    drag(&mut h, (0.0, 0.0), (0.0, -100.0), Modifiers::NONE);
    let c = chain(&h, id);
    assert!(
        near(c.points[1].anim.y.at(1), -100.0, 1.5),
        "{:?}",
        c.points[1]
    );
    assert_eq!(c.points[1].rest.pos, [0.0, 0.0], "the rest pose stays");
    let bent = dark_rows(&h.state().s, 960);
    let shift = bent[0] as i64 - flat[0] as i64;
    assert!((shift + 100).abs() <= 4, "the art follows: {shift}");
    h.state_mut().s.undo();
    h.run_steps(1);
    assert_eq!(
        chain(&h, id),
        straight_curve(),
        "one undo takes the whole drag back"
    );
}

/// Only the primary button drags a point: a drag with the other button
/// (a pen's side button can send one) leaves the chain alone.
#[test]
fn only_the_primary_button_drags_a_curve_point() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
    let path = super::line((0.0, 0.0), (0.0, -100.0), 8);
    mouse_with(
        &mut h,
        &path,
        Modifiers::NONE,
        egui::PointerButton::Secondary,
    );
    assert_eq!(chain(&h, id), straight_curve());
}

/// In Animate mode a drag keys what it moves on this frame only.
#[test]
fn animate_mode_keys_a_curve_drag_on_this_frame() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
    h.state_mut().s.animate = true;
    h.state_mut().s.frame = 10;
    h.run_steps(2);
    drag(&mut h, (300.0, 0.0), (300.0, 80.0), Modifiers::NONE);
    let p = &chain(&h, id).points[2].anim;
    assert!(p.y.has_key(10) && near(p.y.at(10), 80.0, 1.5), "{p:?}");
    assert!(!p.x.has_key(10) || near(p.x.at(10), 300.0, 1.5));
    assert!(
        !chain(&h, id).points[1].anim.y.has_key(10),
        "other points keep no key"
    );
}

/// A handle turns its opposite by the same angle and the opposite keeps
/// its length; Alt-drag turns one handle alone.
#[test]
fn a_curve_handle_turns_its_opposite_unless_alt() {
    for (mods, opposite_turns) in [(Modifiers::NONE, true), (ALT, false)] {
        let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
        // The middle point's handle ahead, from (100, 0) a quarter turn
        // to (0, 150).
        drag(&mut h, (100.0, 0.0), (0.0, 150.0), mods);
        let p = chain(&h, id).points[1].anim.at(1);
        assert!(
            near(wrap(p.out_angle), 90.0, 1.0) && near(p.out_length, 150.0, 2.0),
            "{p:?}"
        );
        let turned_in = if opposite_turns { -90.0 } else { 180.0 };
        assert!(
            near(wrap(p.in_angle - turned_in), 0.0, 1.0),
            "{mods:?}: in angle {}",
            p.in_angle
        );
        assert!(
            near(p.in_length, 100.0, 0.5),
            "the opposite keeps its length"
        );
    }
}

/// Ctrl-drag a point moves it and every point after it.
#[test]
fn ctrl_drag_moves_a_curve_point_and_those_after_it() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
    drag(&mut h, (0.0, 0.0), (0.0, 60.0), CTRL);
    let ys: Vec<f32> = chain(&h, id)
        .points
        .iter()
        .map(|p| p.anim.y.at(1))
        .collect();
    assert!(
        near(ys[0], 0.0, 0.5) && near(ys[1], 60.0, 1.5) && near(ys[2], 60.0, 1.5),
        "{ys:?}"
    );
}

/// Dragging the root moves the whole curve; the ring behind it turns it.
#[test]
fn the_root_moves_and_the_ring_turns_the_curve() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
    drag(&mut h, (-300.0, 0.0), (-250.0, 40.0), Modifiers::NONE);
    let c = chain(&h, id);
    assert!(
        near(c.root.x.at(1), 50.0, 1.5) && near(c.root.y.at(1), 40.0, 1.5),
        "{:?}",
        c.root
    );
    h.state_mut().s.undo();
    h.run_steps(2);
    // The ring sits 36 screen points behind the root, away from the next
    // point: at the left, in scene units 36 / zoom.
    let (root, ring) = {
        let canvas = &h.state().canvas;
        let root = canvas.to_screen(-300.0, 0.0);
        let right = canvas.to_screen(0.0, 0.0);
        (root, root + (root - right).normalized() * 36.0)
    };
    let to_scene = |p: egui::Pos2| h.state().canvas.to_scene(p);
    let (rx, ry) = to_scene(ring);
    let (cx, cy) = to_scene(root);
    let r = (rx - cx).hypot(ry - cy);
    // A quarter turn about the root: from behind it to below it.
    let path: Vec<(f32, f32)> = (0..=8)
        .map(|k| {
            let a = std::f32::consts::PI - std::f32::consts::FRAC_PI_2 * k as f32 / 8.0;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    mouse(&mut h, &path, Modifiers::NONE);
    let turn = wrap(chain(&h, id).root.angle.at(1));
    assert!(near(turn.abs(), 90.0, 3.0), "a quarter turn: {turn}");
}

/// A clicked point is nudged by the arrow keys: 1 unit, or 10 with
/// Shift.
#[test]
fn arrow_keys_nudge_the_selected_curve_point() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
    click(&mut h, 300.0, 0.0);
    key(&mut h, egui::Key::ArrowRight, Modifiers::NONE);
    key(&mut h, egui::Key::ArrowDown, Modifiers::SHIFT);
    let p = chain(&h, id).points[2].anim.at(1);
    assert!(near(p.pos[0], 301.0, 0.01), "{p:?}");
    assert!(near(p.pos[1].abs(), 10.0, 0.01), "{p:?}");
}

/// Edit rest pose: a drag moves where the chain rests over the art, and
/// the art does not bend. Done editing rest pose goes back.
#[test]
fn edit_rest_pose_moves_the_chain_without_bending_the_art() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
    let flat = dark_rows(&h.state().s, 960);
    h.get_by_label("Edit rest pose").click();
    h.run_steps(2);
    drag(&mut h, (0.0, 0.0), (0.0, -100.0), Modifiers::NONE);
    let c = chain(&h, id);
    assert!(
        near(c.points[1].rest.pos[1], -100.0, 1.5),
        "{:?}",
        c.points[1]
    );
    assert_eq!(dark_rows(&h.state().s, 960), flat, "the art stays as drawn");
    h.get_by_label("Done editing rest pose").click();
    h.run_steps(2);
    // Now a drag bends again.
    drag(&mut h, (0.0, -100.0), (0.0, 0.0), Modifiers::NONE);
    assert_ne!(
        dark_rows(&h.state().s, 960),
        flat,
        "out of rest mode, a drag bends"
    );
}

/// After Clear points (an empty chain) the Camera view places points
/// again.
#[test]
fn a_cleared_curve_is_placed_again() {
    let (mut h, id) = deformer_rig(NodeKind::Curve(Box::new(straight_curve())));
    let s = &mut h.state_mut().s;
    if let Some(NodeKind::Curve(c)) = s.doc.graph.node_mut(id).map(|n| &mut n.kind) {
        c.points.clear();
    }
    h.run_steps(2);
    click(&mut h, -100.0, 0.0);
    click(&mut h, 100.0, 0.0);
    assert_eq!(chain(&h, id).points.len(), 2);
    assert!(h.query_by_label("Done").is_some(), "placing again");
}

// ------------------------------------------------------------ Mesh Warp

/// A 2 x 2 grid, 400 square, over the middle of the line.
fn small_grid() -> MeshWarp {
    MeshWarp::new(2, 2, 0.0, 0.0, 400.0, 400.0)
}

/// Where the middle point's handle `which` is now.
fn handle_at(w: &MeshWarp, which: Handle) -> [f32; 2] {
    w.grid_at(1)
        .handle(1, 1, which)
        .expect("an inner point has 4 handles")
}

/// A grid point shows its handles only once clicked: before that, a press
/// where a handle would be moves the whole grid.
#[test]
fn a_clicked_grid_point_shows_handles_that_bend() {
    let (mut h, id) = deformer_rig(NodeKind::MeshWarp(Box::new(small_grid())));
    let ahead = handle_at(&warp(&h, id), Handle::NextColumn);
    drag(
        &mut h,
        (ahead[0], ahead[1]),
        (ahead[0], ahead[1] + 50.0),
        Modifiers::NONE,
    );
    let w = warp(&h, id);
    assert!(
        near(w.y.at(1), 50.0, 1.5) && w.handles.is_empty(),
        "no handle yet: the grid moved"
    );
    h.state_mut().s.undo();
    h.run_steps(2);
    click(&mut h, 0.0, 0.0);
    drag(
        &mut h,
        (ahead[0], ahead[1]),
        (0.0, ahead[0]),
        Modifiers::NONE,
    );
    let w = warp(&h, id);
    assert_eq!(w.y.at(1), 0.0, "the grid stays");
    assert!(!w.handles.is_empty(), "the handle moved");
    let now = handle_at(&w, Handle::NextColumn);
    assert!(
        near(now[0], 0.0, 1.5) && near(now[1], ahead[0], 1.5),
        "{now:?}"
    );
    // The opposite turned a quarter turn too, and kept its length.
    let back = handle_at(&w, Handle::PrevColumn);
    assert!(
        near(back[0], 0.0, 1.5) && near(back[1], -ahead[0], 1.5),
        "{back:?}"
    );
    let flat = {
        let (s, _, _) = line_scene();
        dark_rows(&s, 960 + 60)
    };
    assert_ne!(
        dark_rows(&h.state().s, 960 + 60),
        flat,
        "the handle bends the line"
    );
}

/// Alt-drag turns one grid handle alone.
#[test]
fn alt_drag_turns_one_grid_handle_alone() {
    let (mut h, id) = deformer_rig(NodeKind::MeshWarp(Box::new(small_grid())));
    let before = handle_at(&warp(&h, id), Handle::PrevColumn);
    let ahead = handle_at(&warp(&h, id), Handle::NextColumn);
    click(&mut h, 0.0, 0.0);
    drag(&mut h, (ahead[0], ahead[1]), (0.0, ahead[0]), ALT);
    let w = warp(&h, id);
    let back = handle_at(&w, Handle::PrevColumn);
    assert!(
        near(back[0], before[0], 0.5) && near(back[1], before[1], 0.5),
        "{back:?}"
    );
}

/// Dragging a grid point bends the art, as one undo step.
#[test]
fn dragging_a_grid_point_bends_as_one_undo_step() {
    let (mut h, id) = deformer_rig(NodeKind::MeshWarp(Box::new(small_grid())));
    let flat = dark_rows(&h.state().s, 960);
    drag(&mut h, (0.0, 0.0), (0.0, 120.0), Modifiers::NONE);
    let w = warp(&h, id);
    assert!(near(w.points[4][1].at(1), 120.0, 1.5), "{:?}", w.points[4]);
    let shift = dark_rows(&h.state().s, 960)[0] as i64 - flat[0] as i64;
    assert!((shift - 120).abs() <= 4, "the art follows: {shift}");
    h.state_mut().s.undo();
    h.run_steps(1);
    assert_eq!(warp(&h, id), small_grid());
}

/// An empty space inside the grid moves it; in Animate mode the move is
/// keyed on this frame.
#[test]
fn an_empty_space_moves_the_grid_and_keys_it_in_animate_mode() {
    let (mut h, id) = deformer_rig(NodeKind::MeshWarp(Box::new(small_grid())));
    h.state_mut().s.animate = true;
    h.state_mut().s.frame = 5;
    h.run_steps(2);
    drag(&mut h, (100.0, 100.0), (160.0, 130.0), Modifiers::NONE);
    let w = warp(&h, id);
    assert!(w.x.has_key(5) && near(w.x.at(5), 60.0, 1.5), "{:?}", w.x);
    assert!(w.y.has_key(5) && near(w.y.at(5), 30.0, 1.5), "{:?}", w.y);
    assert!(!w.width.has_key(5), "a move keeps the size");
}

/// The square outside a corner sizes the grid from that corner; the
/// opposite corner stays.
#[test]
fn a_corner_square_sizes_the_grid() {
    let (mut h, id) = deformer_rig(NodeKind::MeshWarp(Box::new(small_grid())));
    // Bottom-right corner (200, 200); its square is 14 screen points out
    // along the diagonal.
    let (from, to) = {
        let canvas = &h.state().canvas;
        let corner = canvas.to_screen(200.0, 200.0);
        let centre = canvas.to_screen(0.0, 0.0);
        let at = corner + (corner - centre).normalized() * 14.0;
        let s = canvas.to_scene(at);
        (s, (s.0 + 100.0, s.1 + 50.0))
    };
    drag(&mut h, from, to, Modifiers::NONE);
    let w = warp(&h, id);
    let (wd, ht) = (w.width.at(1), w.height.at(1));
    assert!(near(wd, 500.0, 2.0) && near(ht, 450.0, 2.0), "{wd} x {ht}");
    let left = w.x.at(1) - wd / 2.0;
    let top = w.y.at(1) - ht / 2.0;
    assert!(
        near(left, -200.0, 2.0) && near(top, -200.0, 2.0),
        "the opposite corner stays"
    );
}

// ------------------------------------------------------------ files

/// Bent handles and a Curve come back the same after a save and open.
#[test]
fn deformers_survive_a_save_and_open() {
    let (mut h, warp_id) = deformer_rig(NodeKind::MeshWarp(Box::new(small_grid())));
    click(&mut h, 0.0, 0.0);
    let ahead = handle_at(&warp(&h, warp_id), Handle::NextColumn);
    drag(
        &mut h,
        (ahead[0], ahead[1]),
        (ahead[0], 60.0),
        Modifiers::NONE,
    );
    let mut curve = straight_curve();
    curve.points[1].anim.y = Value::default();
    curve.points[1].anim.y.set_key(1, 0.0);
    curve.points[1].anim.y.set_key(12, -80.0);
    curve.root.angle = Value::Constant(15.0);
    let s = &mut h.state_mut().s;
    let layer_peg = s.doc.layers[0].peg;
    let curve_id = s
        .doc
        .graph
        .add("Curve", NodeKind::Curve(Box::new(curve.clone())));
    s.doc.graph.connect(layer_peg, curve_id, 0);
    s.doc.graph.connect(curve_id, warp_id, 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Deformers.symphony");
    symphony_io::save(&h.state().s.doc, &path).unwrap();
    let back = symphony_io::open(&path).unwrap();
    let node = |id: NodeId| back.graph.node(id).map(|n| n.kind.clone());
    assert_eq!(
        node(warp_id),
        Some(NodeKind::MeshWarp(Box::new(warp(&h, warp_id))))
    );
    assert_eq!(node(curve_id), Some(NodeKind::Curve(Box::new(curve))));
    assert!(!warp(&h, warp_id).handles.is_empty());
    // Both bend the render as before.
    let rows = |doc: &symphony_core::doc::Scene| {
        symphony_render::render_frame(doc, 12, 480, 270)
            .unwrap()
            .into_raw()
    };
    assert_eq!(rows(&back), rows(&h.state().s.doc));
}
