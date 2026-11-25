//! Editing a Curve or Envelope deformer in the Camera view (Harmony 27's
//! Curve and Envelope deformations). With the node selected and the
//! Transform tool:
//!
//! - Placing: a new node starts empty. Press to place a point and drag to
//!   pull out its handles (the one behind turns with it); a click places
//!   a point with no handles. An Envelope closes with a click on its
//!   first point; a Curve ends with Done.
//! - Animating: drag a point or a handle. A handle turns its opposite by
//!   the same angle, and the opposite keeps its length; Alt-drag turns one
//!   handle alone. On a Curve, the first point is the root: dragging it
//!   moves the whole curve, and the ring behind it turns the whole curve.
//!   Ctrl-drag a point to move it and every point after it; Ctrl-drag a
//!   handle to turn every point after it about its point. Arrow keys
//!   nudge the selected point (Shift: 10). In Animate mode a change keys
//!   what it changes on this frame; one drag is one undo step.
//! - Edit rest pose: drags change where the chain rests over the art
//!   (the art does not bend), as Harmony's Rigging tool does.

use eframe::egui::{self, Pos2, Stroke as EStroke, Vec2};
use symphony_core::anim::Value;
use symphony_core::envelope::{segments, CurveDeformer, CurvePoint, PointPose};
use symphony_core::graph::{NodeId, NodeKind};
use symphony_core::history::Edit;
use symphony_core::math::Affine;
use symphony_ui::{theme, widgets};

use crate::session::{Session, Tool};

/// How near (screen points) a press must be to grab a control.
const GRAB: f32 = 10.0;
/// How far behind the root its turning ring sits (screen points).
const RING: f32 = 36.0;

#[derive(Default)]
pub struct CurveEdit {
    dragging: Option<Grab>,
    hover: Option<Grab>,
    /// The point arrow keys nudge, on which node.
    selected: Option<(NodeId, usize)>,
    /// Placing points on this node.
    adding: Option<NodeId>,
    /// Editing this node's rest pose.
    rest: Option<NodeId>,
    /// The press (the node's space), the chain then, and the modifiers.
    start: Option<Start>,
}

#[derive(Clone)]
struct Start {
    at: [f32; 2],
    chain: CurveDeformer,
    alt: bool,
    ctrl: bool,
}

/// What a press can take hold of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Point(usize),
    /// Point `i`'s handle: `false` the one behind it, `true` ahead.
    Handle(usize, bool),
    /// An open curve's turning ring.
    Ring,
    /// Pulling out the handles of a point just placed.
    Placing(usize),
}

/// The selected Curve or Envelope, and its space -> screen.
fn target(s: &Session, view: Affine) -> Option<(NodeId, CurveDeformer, Affine)> {
    let id = s.selected_node?;
    let NodeKind::Curve(c) = &s.doc.graph.node(id)?.kind else {
        return None;
    };
    let pose = s.doc.graph.world_pose(id, s.frame).ok()?;
    let plane = s.doc.project_plane(s.frame, pose.depth)?;
    Some((
        id,
        (**c).clone(),
        view.compose(plane).compose(pose.transform),
    ))
}

/// The root's move and turn on `frame` (none in rest mode or on an
/// Envelope): pivot, offset, turn in degrees.
fn root(c: &CurveDeformer, frame: u32, rest: bool) -> ([f32; 2], [f32; 2], f32) {
    let pivot = c.points.first().map_or([0.0, 0.0], |p| p.rest.pos);
    if rest || c.closed {
        return (pivot, [0.0, 0.0], 0.0);
    }
    (
        pivot,
        [c.root.x.at(frame), c.root.y.at(frame)],
        c.root.angle.at(frame),
    )
}

fn turn(v: [f32; 2], degrees: f32) -> [f32; 2] {
    let (s, c) = degrees.to_radians().sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

/// A shown place back into the chain's own space (before the root).
fn unroot(c: &CurveDeformer, frame: u32, rest: bool, p: [f32; 2]) -> [f32; 2] {
    let (pivot, d, a) = root(c, frame, rest);
    let q = turn([p[0] - d[0] - pivot[0], p[1] - d[1] - pivot[1]], -a);
    [q[0] + pivot[0], q[1] + pivot[1]]
}

/// The points as shown: the rest pose, or the animated one on `frame`.
fn shown(c: &CurveDeformer, frame: u32, rest: bool) -> Vec<PointPose> {
    if rest {
        c.points.iter().map(|p| p.rest).collect()
    } else {
        c.poses_at(frame)
    }
}

/// Which handles a point shows: an open curve's ends have one each.
fn has_handle(c: &CurveDeformer, i: usize, ahead: bool) -> bool {
    c.closed || if ahead { i + 1 < c.points.len() } else { i > 0 }
}

/// Draw and edit the selected Curve or Envelope. Returns true while one
/// is being edited: the Transform tool then works on it only.
pub fn ui(
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    s: &mut Session,
    state: &mut CurveEdit,
    view: Affine,
) -> bool {
    if s.tool != Tool::Transform {
        state.dragging = None;
        return false;
    }
    let Some((id, c, m)) = target(s, view) else {
        state.dragging = None;
        return false;
    };
    if c.points.is_empty() && state.adding != Some(id) {
        state.adding = Some(id);
    }
    let adding = state.adding == Some(id) || !c.is_complete();
    let rest = !adding && state.rest == Some(id);
    let screen = |p: [f32; 2]| {
        let (x, y) = m.apply(p[0], p[1]);
        Pos2::new(x, y)
    };
    let poses = shown(&c, s.frame, rest);
    let controls = controls(&c, &poses, rest || adding, &screen);
    let grab_at = |p: Pos2| {
        controls
            .iter()
            .map(|&(g, q)| (g, q.distance(p)))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(g, _)| g)
    };
    let pointer = response.hover_pos().or(response.interact_pointer_pos());
    let nearest = pointer.and_then(grab_at);
    // A drag takes what was under the press: egui starts it only once the
    // pointer has moved a few points, maybe off the control already.
    let pressed_on = ui.input(|i| i.pointer.press_origin()).and_then(grab_at);
    state.hover = nearest;
    let look = Look {
        c: &c,
        poses: &poses,
        frame: s.frame,
        rest,
    };
    draw(painter, &look, state, &controls, &screen);
    step_button(ui, response, s, id, &c, state, adding);
    let back = m.inverse();
    let to_node = |p: Pos2| back.map(|b| b.apply(p.x, p.y)).map(|(x, y)| [x, y]);
    if adding {
        place(ui, response, s, state, (id, &c, &to_node), nearest);
    } else {
        if response.clicked() {
            state.selected = match nearest {
                Some(Grab::Point(i)) => Some((id, i)),
                _ => None,
            };
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            state.dragging = pressed_on;
            if let Some(Grab::Point(i)) = pressed_on {
                state.selected = Some((id, i));
            }
            let origin = ui.input(|i| i.pointer.press_origin()).and_then(to_node);
            let (alt, ctrl) = ui.input(|i| (i.modifiers.alt, i.modifiers.command));
            state.start = origin.map(|at| Start {
                at,
                chain: c.clone(),
                alt,
                ctrl,
            });
        }
        if let (Some(grab), Some(start)) = (state.dragging, state.start.clone()) {
            if response.dragged() {
                if let Some(to) = response.interact_pointer_pos().and_then(to_node) {
                    let after = dragged(&start, grab, to, s.frame, s.animate, rest);
                    replace(
                        s,
                        id,
                        "Move curve point",
                        Some(format!("curve-{}", id.0)),
                        after,
                    );
                }
            }
        }
        nudge(ui, s, id, &c, state, rest, &m);
    }
    if response.drag_stopped() && state.dragging.is_some() {
        state.dragging = None;
        state.start = None;
        s.end_gesture();
    }
    set_cursor(ui, response, state, nearest, adding);
    true
}

fn set_cursor(
    ui: &egui::Ui,
    response: &egui::Response,
    state: &CurveEdit,
    nearest: Option<Grab>,
    adding: bool,
) {
    let icon = if state.dragging.is_some() {
        egui::CursorIcon::Grabbing
    } else if nearest.is_some() {
        egui::CursorIcon::Grab
    } else if adding && response.hovered() {
        egui::CursorIcon::Crosshair
    } else {
        return;
    };
    ui.ctx().set_cursor_icon(icon);
}

/// Every control on screen: points, their handles, and an open curve's
/// turning ring (not while placing).
fn controls(
    c: &CurveDeformer,
    poses: &[PointPose],
    placing_or_rest: bool,
    screen: &impl Fn([f32; 2]) -> Pos2,
) -> Vec<(Grab, Pos2)> {
    let mut out = Vec::new();
    for (i, p) in poses.iter().enumerate() {
        out.push((Grab::Point(i), screen(p.pos)));
        let [back, ahead] = p.handles();
        if has_handle(c, i, false) {
            out.push((Grab::Handle(i, false), screen(back)));
        }
        if has_handle(c, i, true) {
            out.push((Grab::Handle(i, true), screen(ahead)));
        }
    }
    if !c.closed && !placing_or_rest {
        if let Some(ring) = ring_at(poses, screen) {
            out.push((Grab::Ring, ring));
        }
    }
    out
}

/// Where an open curve's turning ring is: behind the root, away from
/// where the curve goes.
fn ring_at(poses: &[PointPose], screen: &impl Fn([f32; 2]) -> Pos2) -> Option<Pos2> {
    let (first, next) = (poses.first()?, poses.get(1)?);
    let at = screen(first.pos);
    let away = at - screen(next.pos);
    let dir = if away.length() > 1e-3 {
        away.normalized()
    } else {
        -Vec2::X
    };
    Some(at + dir * RING)
}

/// What the chain looks like on screen this frame.
struct Look<'a> {
    c: &'a CurveDeformer,
    poses: &'a [PointPose],
    frame: u32,
    rest: bool,
}

fn draw(
    painter: &egui::Painter,
    look: &Look,
    state: &CurveEdit,
    controls: &[(Grab, Pos2)],
    screen: &impl Fn([f32; 2]) -> Pos2,
) {
    let Look {
        c,
        poses,
        frame,
        rest,
    } = *look;
    // Rest pose in the text colour, animated pose in the selection blue.
    let colour = if rest {
        theme::TEXT_PRIMARY
    } else {
        theme::ACCENT_SELECT
    };
    let line = EStroke::new(1.5, colour);
    for seg in segments(poses, c.closed) {
        let pts: Vec<Pos2> = (0..=24)
            .map(|k| screen(symphony_core::envelope::bezier(&seg, k as f32 / 24.0)))
            .collect();
        painter.add(egui::Shape::line(pts, line));
    }
    let active = |g: Grab| state.dragging == Some(g) || state.hover == Some(g);
    let keyed = |i: usize| {
        c.points
            .get(i)
            .is_some_and(|p| p.anim.values().iter().any(|v| v.has_key(frame)))
    };
    let thin = EStroke::new(1.0, colour.gamma_multiply(0.8));
    for &(g, q) in controls {
        match g {
            Grab::Handle(i, _) => {
                painter.line_segment([screen(poses[i].pos), q], thin);
                let r = if active(g) { 4.5 } else { 3.5 };
                painter.circle_filled(q, r + 1.0, theme::BG_BASE);
                painter.circle_filled(q, r, colour);
            }
            Grab::Ring => {
                if let Some(root) = poses.first() {
                    painter.line_segment([screen(root.pos), q], thin);
                }
                let r = if active(g) { 7.0 } else { 6.0 };
                painter.circle_stroke(q, r, EStroke::new(2.0, colour));
            }
            Grab::Point(_) | Grab::Placing(_) => {}
        }
    }
    for &(g, q) in controls {
        let Grab::Point(i) = g else { continue };
        let big = active(g) || state.selected.is_some_and(|(_, j)| j == i);
        let r = if big { 6.0 } else { 4.5 };
        // Amber marks a key on this frame (time), as on the Timeline.
        let fill = if keyed(i) && !rest {
            theme::ACCENT_TIME
        } else {
            colour
        };
        let square = egui::Rect::from_center_size(q, Vec2::splat(2.0 * r));
        painter.rect_filled(square.expand(1.5), 0.0, theme::BG_BASE);
        painter.rect_filled(square, 0.0, fill);
    }
}

/// The canvas popup: what to do next while placing (Done), or the rest
/// pose switch.
fn step_button(
    ui: &egui::Ui,
    response: &egui::Response,
    s: &mut Session,
    id: NodeId,
    c: &CurveDeformer,
    state: &mut CurveEdit,
    adding: bool,
) {
    let at = response.rect.left_top() + Vec2::new(12.0, 12.0);
    egui::Area::new(egui::Id::unique("curve-step"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                if adding {
                    let hint = if c.closed {
                        "Press to place each point, drag to pull out its handles. \
                         Click the first point to close the envelope."
                    } else {
                        "Press to place each point from the root out, drag to pull out \
                         its handles."
                    };
                    ui.label(
                        egui::RichText::new(hint)
                            .font(theme::dense())
                            .color(theme::TEXT_SECONDARY),
                    );
                    if !c.closed && widgets::primary_button(ui, "Done", c.is_complete()).clicked() {
                        state.adding = None;
                    }
                } else {
                    let on = state.rest == Some(id);
                    let label = if on {
                        "Done editing rest pose"
                    } else {
                        "Edit rest pose"
                    };
                    if ui
                        .selectable_label(on, label)
                        .on_hover_text(
                            "Move the chain over the artwork without bending it: where it \
                             rests is where the art is as drawn.",
                        )
                        .clicked()
                    {
                        state.rest = if on { None } else { Some(id) };
                        s.end_gesture();
                    }
                }
            });
        });
}

/// The node being edited, its chain, and screen -> its space.
type On<'a> = (
    NodeId,
    &'a CurveDeformer,
    &'a dyn Fn(Pos2) -> Option<[f32; 2]>,
);

/// Placing points: a press adds one and a drag pulls out its handles; a
/// click on an Envelope's first point closes it.
fn place(
    ui: &egui::Ui,
    response: &egui::Response,
    s: &mut Session,
    state: &mut CurveEdit,
    (id, c, to_node): On,
    nearest: Option<Grab>,
) {
    let closes = c.closed && c.points.len() >= 3 && nearest == Some(Grab::Point(0));
    let frame = s.frame;
    if response.clicked() {
        if closes {
            state.adding = None;
        } else if let Some(at) = response.interact_pointer_pos().and_then(to_node) {
            add_point(s, id, unroot(c, frame, false, at));
            s.end_gesture();
        }
        return;
    }
    if response.drag_started_by(egui::PointerButton::Primary) && !closes {
        let origin = ui.input(|i| i.pointer.press_origin()).and_then(to_node);
        if let Some(at) = origin {
            add_point(s, id, unroot(c, frame, false, at));
            state.dragging = Some(Grab::Placing(c.points.len()));
            state.start = None;
        }
    }
    if let (Some(Grab::Placing(i)), true) = (state.dragging, response.dragged()) {
        if let Some(to) = response.interact_pointer_pos().and_then(to_node) {
            let to = unroot(c, frame, false, to);
            edit(
                s,
                id,
                "Add curve point",
                Some(format!("curve-add-{}", id.0)),
                |c| {
                    if let Some(p) = c.points.get_mut(i) {
                        let pos = p.rest.pos;
                        *p = CurvePoint::new(PointPose::smooth(
                            pos,
                            [to[0] - pos[0], to[1] - pos[1]],
                        ));
                    }
                },
            );
        }
    }
}

fn add_point(s: &mut Session, id: NodeId, at: [f32; 2]) {
    edit(
        s,
        id,
        "Add curve point",
        Some(format!("curve-add-{}", id.0)),
        |c| {
            if c.points.len() < symphony_core::envelope::MAX_POINTS {
                c.points
                    .push(CurvePoint::new(PointPose::smooth(at, [0.0, 0.0])));
            }
        },
    );
}

/// The chain after a drag from `start` to `to` (the node's space).
fn dragged(
    start: &Start,
    grab: Grab,
    to: [f32; 2],
    frame: u32,
    animate: bool,
    rest: bool,
) -> CurveDeformer {
    let c0 = &start.chain;
    let mut c = c0.clone();
    let poses = shown(c0, frame, rest);
    let (pivot, offset, a) = root(c0, frame, rest);
    // The pointer's move, in the chain's own space.
    let d = turn([to[0] - start.at[0], to[1] - start.at[1]], -a);
    let set = Setter {
        frame,
        animate,
        rest,
    };
    match grab {
        Grab::Point(0) if !c0.closed && !start.ctrl => {
            if rest {
                for i in 0..c.points.len() {
                    set.shift(&mut c, i, d);
                }
            } else {
                set.value(&mut c.root.x, c0.root.x.at(frame) + to[0] - start.at[0]);
                set.value(&mut c.root.y, c0.root.y.at(frame) + to[1] - start.at[1]);
            }
        }
        Grab::Point(i) => {
            let last = if start.ctrl && !c0.closed {
                c.points.len()
            } else {
                i + 1
            };
            for j in i..last {
                set.shift(&mut c, j, d);
            }
        }
        Grab::Handle(i, ahead) => {
            let Some(p) = poses.get(i) else { return c };
            let v = [to[0] - p.pos[0], to[1] - p.pos[1]];
            let angle = v[1].atan2(v[0]).to_degrees();
            let old = if ahead { p.out_angle } else { p.in_angle };
            let spin = angle - old;
            let other = (!start.alt).then_some(spin);
            set.handle(&mut c, (i, ahead), [angle - a, v[0].hypot(v[1])], other);
            if start.ctrl && !c0.closed {
                let about = c0.points.get(i).map_or([0.0, 0.0], |q| set.pose(q).pos);
                for j in i + 1..c.points.len() {
                    set.spin_about(&mut c, j, about, spin);
                }
            }
        }
        Grab::Ring => {
            let centre = [pivot[0] + offset[0], pivot[1] + offset[1]];
            let angle = |q: [f32; 2]| (q[1] - centre[1]).atan2(q[0] - centre[0]).to_degrees();
            set.value(&mut c.root.angle, a + angle(to) - angle(start.at));
        }
        Grab::Placing(_) => {}
    }
    c
}

/// Writes a change to the rest pose (rest mode) or to the animated pose.
struct Setter {
    frame: u32,
    animate: bool,
    rest: bool,
}

impl Setter {
    /// The pose being edited, in the chain's own space.
    fn pose(&self, p: &CurvePoint) -> PointPose {
        if self.rest {
            p.rest
        } else {
            p.anim.at(self.frame)
        }
    }

    /// Set a value: keyed on this frame in Animate mode or when already
    /// animated, otherwise for every frame.
    fn value(&self, value: &mut Value, to: f32) {
        match value {
            Value::Constant(c) if !self.animate => *c = to,
            _ => value.set_key(self.frame, to),
        }
    }

    /// Point `i` becomes `to`. In rest mode the rest pose changes, and so
    /// do animated values that are not animated (so the art stays as
    /// drawn); otherwise only the animated values that change are set.
    fn set(&self, c: &mut CurveDeformer, i: usize, to: PointPose) {
        let Some(p) = c.points.get_mut(i) else { return };
        let from = self.pose(p);
        let fields = |q: PointPose| {
            [
                q.pos[0],
                q.pos[1],
                q.in_angle,
                q.in_length,
                q.out_angle,
                q.out_length,
            ]
        };
        let (old, new) = (fields(from), fields(to));
        if self.rest {
            p.rest = to;
        }
        for ((value, was), now) in p.anim.values_mut().into_iter().zip(old).zip(new) {
            if (was - now).abs() < 1e-6 {
                continue;
            }
            match value {
                Value::Constant(v) if self.rest => *v = now,
                _ if self.rest => {}
                _ => self.value(value, now),
            }
        }
    }

    fn shift(&self, c: &mut CurveDeformer, i: usize, d: [f32; 2]) {
        let Some(p) = c.points.get(i) else { return };
        let mut q = self.pose(p);
        q.pos = [q.pos[0] + d[0], q.pos[1] + d[1]];
        self.set(c, i, q);
    }

    /// Point `i`'s handle ahead (or behind) to an angle (the chain's own
    /// space) and a length; the other handle turns by `other` degrees
    /// when given.
    fn handle(
        &self,
        c: &mut CurveDeformer,
        (i, ahead): (usize, bool),
        [angle, length]: [f32; 2],
        other: Option<f32>,
    ) {
        let Some(p) = c.points.get(i) else { return };
        let mut q = self.pose(p);
        let spin = other.unwrap_or(0.0);
        if ahead {
            (q.out_angle, q.out_length) = (angle, length);
            q.in_angle += spin;
        } else {
            (q.in_angle, q.in_length) = (angle, length);
            q.out_angle += spin;
        }
        self.set(c, i, q);
    }

    /// Turn point `i` about `about` by `spin` degrees, its handles too.
    fn spin_about(&self, c: &mut CurveDeformer, i: usize, about: [f32; 2], spin: f32) {
        let Some(p) = c.points.get(i) else { return };
        let mut q = self.pose(p);
        let v = turn([q.pos[0] - about[0], q.pos[1] - about[1]], spin);
        q.pos = [about[0] + v[0], about[1] + v[1]];
        q.in_angle += spin;
        q.out_angle += spin;
        self.set(c, i, q);
    }
}

/// Arrow keys nudge the selected point (Shift: 10), as one undo step
/// each, unless a text field has the keyboard.
fn nudge(
    ui: &egui::Ui,
    s: &mut Session,
    id: NodeId,
    c: &CurveDeformer,
    state: &CurveEdit,
    rest: bool,
    m: &Affine,
) {
    let Some((_, i)) = state
        .selected
        .filter(|(n, i)| *n == id && *i < c.points.len())
    else {
        return;
    };
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    use egui::{Key, Modifiers};
    let d = ui.input_mut(|input| {
        let step = if input.modifiers.shift { 10.0 } else { 1.0 };
        let mut d = Vec2::ZERO;
        for (key, v) in [
            (Key::ArrowLeft, -Vec2::X),
            (Key::ArrowRight, Vec2::X),
            (Key::ArrowUp, -Vec2::Y),
            (Key::ArrowDown, Vec2::Y),
        ] {
            if input.consume_key(Modifiers::NONE, key) || input.consume_key(Modifiers::SHIFT, key) {
                d += v * step;
            }
        }
        d
    });
    if d == Vec2::ZERO {
        return;
    }
    // Screen directions, `step` stage units long, in the node's space.
    let Some(back) = m.inverse() else { return };
    let (x0, y0) = back.apply(0.0, 0.0);
    let (x1, y1) = back.apply(d.x, d.y);
    let v = [x1 - x0, y1 - y0];
    let k = d.length() / v[0].hypot(v[1]).max(1e-6);
    let to = [v[0] * k, v[1] * k];
    let start = Start {
        at: [0.0, 0.0],
        chain: c.clone(),
        alt: false,
        ctrl: false,
    };
    // As a drag would: a Curve's root moves the whole curve.
    let after = dragged(&start, Grab::Point(i), to, s.frame, s.animate, rest);
    replace(s, id, "Nudge curve point", None, after);
}

/// Change the node `id` as one undo step (merged by `merge` while a drag
/// goes on).
fn edit(
    s: &mut Session,
    id: NodeId,
    name: &str,
    merge: Option<String>,
    f: impl FnOnce(&mut CurveDeformer),
) {
    let Some(NodeKind::Curve(c)) = s.doc.graph.node(id).map(|n| &n.kind) else {
        return;
    };
    let mut c = (**c).clone();
    f(&mut c);
    replace(s, id, name, merge, c);
}

fn replace(s: &mut Session, id: NodeId, name: &str, merge: Option<String>, c: CurveDeformer) {
    let Some(before) = s.doc.graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    after.kind = NodeKind::Curve(Box::new(c));
    if after == before {
        return;
    }
    let e = Edit::SetNodeProperties { id, before, after };
    match merge {
        Some(key) => s.history.apply_merged(&mut s.doc, name, &key, e),
        None => s.history.apply(&mut s.doc, name, vec![e]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight open curve along x: points at 0, 100, 200.
    fn line() -> CurveDeformer {
        let mut c = CurveDeformer::new(false);
        for x in [0.0, 100.0, 200.0] {
            c.points
                .push(CurvePoint::new(PointPose::smooth([x, 0.0], [30.0, 0.0])));
        }
        c
    }

    fn start(c: &CurveDeformer, at: [f32; 2], ctrl: bool, alt: bool) -> Start {
        Start {
            at,
            chain: c.clone(),
            alt,
            ctrl,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn dragging_the_root_moves_the_whole_curve() {
        let c = line();
        let after = dragged(
            &start(&c, [0.0, 0.0], false, false),
            Grab::Point(0),
            [5.0, 7.0],
            1,
            false,
            false,
        );
        assert_eq!((after.root.x.at(1), after.root.y.at(1)), (5.0, 7.0));
        assert_eq!(after.points, c.points, "the points themselves stay");
    }

    #[test]
    fn ctrl_drag_moves_a_point_and_those_after_it() {
        let c = line();
        let after = dragged(
            &start(&c, [100.0, 0.0], true, false),
            Grab::Point(1),
            [100.0, 10.0],
            1,
            false,
            false,
        );
        let ys: Vec<f32> = after.points.iter().map(|p| p.anim.y.at(1)).collect();
        assert_eq!(ys, [0.0, 10.0, 10.0]);
        let plain = dragged(
            &start(&c, [100.0, 0.0], false, false),
            Grab::Point(1),
            [100.0, 10.0],
            1,
            false,
            false,
        );
        let ys: Vec<f32> = plain.points.iter().map(|p| p.anim.y.at(1)).collect();
        assert_eq!(ys, [0.0, 10.0, 0.0]);
    }

    #[test]
    fn a_handle_turns_its_opposite_unless_alt() {
        let c = line();
        // Point 1's handle ahead, from (130, 0) to (100, 60): a quarter
        // turn, twice as long.
        let after = dragged(
            &start(&c, [130.0, 0.0], false, false),
            Grab::Handle(1, true),
            [100.0, 60.0],
            1,
            false,
            false,
        );
        let p = after.points[1].anim.at(1);
        assert!(close(p.out_angle, 90.0) && close(p.out_length, 60.0));
        assert!(
            close(p.in_angle, 270.0) && close(p.in_length, 30.0),
            "{p:?}"
        );
        let alone = dragged(
            &start(&c, [130.0, 0.0], false, true),
            Grab::Handle(1, true),
            [100.0, 60.0],
            1,
            false,
            false,
        );
        assert!(close(alone.points[1].anim.in_angle.at(1), 180.0));
    }

    #[test]
    fn animate_mode_keys_only_what_moves() {
        let c = line();
        let after = dragged(
            &start(&c, [200.0, 0.0], false, false),
            Grab::Point(2),
            [200.0, 9.0],
            4,
            true,
            false,
        );
        let p = &after.points[2].anim;
        assert!(p.y.has_key(4) && !p.x.has_key(4) && !p.out_angle.has_key(4));
        assert!(!after.points[1].anim.y.has_key(4));
    }

    #[test]
    fn the_rest_pose_moves_without_bending_the_art() {
        let c = line();
        let after = dragged(
            &start(&c, [100.0, 0.0], false, false),
            Grab::Point(1),
            [100.0, 20.0],
            1,
            false,
            true,
        );
        assert_eq!(after.points[1].rest.pos, [100.0, 20.0]);
        assert_eq!(after.points[1].anim.y.at(1), 20.0, "follows: not animated");
        assert!(after.field_at(1).unwrap().is_flat());
    }

    #[test]
    fn the_ring_turns_the_curve_about_its_root() {
        let c = line();
        // From behind the root (-36, 0) to below it (0, -36): a quarter
        // turn.
        let after = dragged(
            &start(&c, [-36.0, 0.0], false, false),
            Grab::Ring,
            [0.0, 36.0],
            1,
            false,
            false,
        );
        assert!(close(after.root.angle.at(1), -90.0) || close(after.root.angle.at(1), 270.0));
    }
}
