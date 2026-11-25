//! Editing a Mesh Warp in the Camera view (Harmony 27: select the node,
//! show its controls, and move its points). With a Mesh Warp node
//! selected and the Transform tool:
//!
//! - Grid: the grid shows over the artwork, bent as it bends the art;
//!   dragging a point bends everything under it. A clicked point shows
//!   its Bezier handles; dragging one bends the grid lines through the
//!   point, and its opposite handle turns by the same angle and keeps
//!   its length, as Harmony's deformer handles do (Alt-drag turns one
//!   handle alone). Dragging an empty space
//!   inside the grid moves the whole grid (Harmony's "grab the grid in
//!   the void spaces to pan it"); the squares outside its corners and the
//!   arrows outside its sides change its width and height.
//! - Points (like Harmony 24's Shape-Aware Deformation): first place the
//!   points: click the artwork to add one, drag one to move it, Alt-click
//!   one to remove it. Then Build mesh: a mesh is made over the artwork's
//!   shape, and dragging a point bends the art as rigidly as it can,
//!   moving only what's connected to it. Edit points goes back to
//!   placing them.
//!
//! In Animate mode a move keys the point on the current frame; otherwise
//! it sets it for every frame. One drag is one undo step.

use eframe::egui::{self, Pos2, Stroke as EStroke, Vec2};
use symphony_core::anim::Value;
use symphony_core::graph::{NodeId, NodeKind};
use symphony_core::history::Edit;
use symphony_core::math::Affine;
use symphony_core::warp::{Handle, MeshWarp, WarpField, WarpGrid, WarpKind};
use symphony_ui::{theme, widgets};

use crate::session::{Session, Tool};

/// How near (screen points) a press must be to grab a point.
const GRAB: f32 = 10.0;

#[derive(Default)]
pub struct WarpEdit {
    /// What is being dragged.
    dragging: Option<Grab>,
    hover: Option<Grab>,
    /// The grid point whose handles show, on which Mesh Warp.
    selected: Option<(NodeId, usize)>,
    /// Moving or sizing the grid: where the press was (the warp's space)
    /// and the grid's left, top, right and bottom then.
    start: Option<([f32; 2], [f32; 4])>,
}

/// What a press can take hold of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Point(usize),
    /// A grid point's handle, and whether its opposite turns with it.
    Handle(usize, Handle, bool),
    /// The whole grid, by an empty space inside it.
    Pan,
    /// A size control: which side moves across the columns (-1 the
    /// first, 1 the last, 0 none) and down the rows.
    Size(i8, i8),
}

impl Grab {
    fn same(self, other: Option<Grab>) -> bool {
        match (self, other) {
            (Grab::Point(a), Some(Grab::Point(b))) => a == b,
            (Grab::Handle(a, h, _), Some(Grab::Handle(b, k, _))) => a == b && h == k,
            (Grab::Pan, Some(Grab::Pan)) => true,
            (Grab::Size(a, b), Some(Grab::Size(c, d))) => (a, b) == (c, d),
            _ => false,
        }
    }
}

/// The handles of grid point `i` that exist (not at the border), with
/// where each is now.
fn handles_of(grid: &WarpGrid, i: usize) -> Vec<(Handle, [f32; 2])> {
    let cols = grid.columns as usize + 1;
    let (r, c) = ((i / cols) as u32, (i % cols) as u32);
    Handle::ALL
        .into_iter()
        .filter_map(|h| grid.handle(r, c, h).map(|p| (h, p)))
        .collect()
}

/// The selected Mesh Warp, and its space -> screen.
fn target(s: &Session, view: Affine) -> Option<(NodeId, MeshWarp, Affine)> {
    let id = s.selected_node?;
    let NodeKind::MeshWarp(w) = &s.doc.graph.node(id)?.kind else {
        return None;
    };
    // The warp works in its node's parent space (what the pegs above it
    // make of it), on the stage plane at their depth.
    let pose = s.doc.graph.world_pose(id, s.frame).ok()?;
    let plane = s.doc.project_plane(s.frame, pose.depth)?;
    Some((
        id,
        (**w).clone(),
        view.compose(plane).compose(pose.transform),
    ))
}

/// Draw and edit the selected Mesh Warp. Returns true while a Mesh Warp
/// is being edited: the Transform tool then works on it only (not the
/// layer's box).
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
    let Some((id, warp, m)) = target(s, view) else {
        state.dragging = None;
        return false;
    };
    let screen = |p: [f32; 2]| {
        let (x, y) = m.apply(p[0], p[1]);
        Pos2::new(x, y)
    };
    let grid = (warp.kind == WarpKind::Grid).then(|| warp.grid_at(s.frame));
    // Where the points are (the warp's space).
    let points: Vec<[f32; 2]> = match &grid {
        Some(grid) => {
            draw_grid(painter, grid, &screen);
            (0..=grid.rows)
                .flat_map(|r| (0..=grid.columns).map(move |c| (r, c)))
                .map(|(r, c)| grid.point(r, c))
                .collect()
        }
        None => points_mode(painter, &warp, s.frame, &screen),
    };
    if state
        .selected
        .is_some_and(|(n, i)| n != id || i >= points.len() || grid.is_none())
    {
        state.selected = None;
    }
    let handles = match (&grid, state.selected) {
        (Some(grid), Some((_, i))) => handles_of(grid, i),
        _ => Vec::new(),
    };
    let on_screen: Vec<Pos2> = points.iter().map(|&p| screen(p)).collect();
    let sizes = grid
        .as_ref()
        .map_or_else(Vec::new, |grid| size_controls(grid, &screen));
    let grab_at =
        |p: Pos2| {
            let near = |q: Pos2| Some(q.distance(p)).filter(|d| *d <= GRAB);
            let selected = state.selected.map(|(_, i)| i);
            on_screen
                .iter()
                .enumerate()
                .filter_map(|(i, q)| near(*q).map(|d| (Grab::Point(i), d)))
                .chain(handles.iter().filter_map(|&(h, q)| {
                    Some((Grab::Handle(selected?, h, false), near(screen(q))?))
                }))
                .chain(sizes.iter().filter_map(|&(g, q, _)| Some((g, near(q)?))))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(g, _)| g)
                .or_else(|| {
                    let grid = grid.as_ref()?;
                    inside(&outline(grid, &screen), p).then_some(Grab::Pan)
                })
        };
    let pointer = response.hover_pos().or(response.interact_pointer_pos());
    let nearest = pointer.and_then(grab_at);
    state.hover = nearest;
    if response.drag_started_by(egui::PointerButton::Primary) {
        // What was under the press: egui starts a drag only once the
        // pointer has moved a few points, maybe off the control already.
        let pressed_on = ui.input(|i| i.pointer.press_origin()).and_then(grab_at);
        let alt = ui.input(|i| i.modifiers.alt);
        state.dragging = pressed_on.map(|g| match g {
            Grab::Handle(i, h, _) => Grab::Handle(i, h, !alt),
            g => g,
        });
        if let (Some(Grab::Point(i)), Some(_)) = (state.dragging, &grid) {
            state.selected = Some((id, i));
        }
        let origin = ui.input(|i| i.pointer.press_origin());
        state.start = match (&grid, origin, m.inverse()) {
            (Some(grid), Some(o), Some(back)) => {
                let (x, y) = back.apply(o.x, o.y);
                let ([cx, cy], [w, h]) = (grid.centre, grid.size);
                Some((
                    [x, y],
                    [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0],
                ))
            }
            _ => None,
        };
    }
    let placing = warp.kind == WarpKind::Points && warp.mesh.is_none();
    if placing && response.clicked() {
        place_or_remove(ui, s, id, nearest, pointer.zip(m.inverse()));
    }
    // A click on a grid point shows its handles; a click elsewhere hides
    // them.
    if grid.is_some() && response.clicked() {
        match nearest {
            Some(Grab::Point(i)) => state.selected = Some((id, i)),
            Some(Grab::Handle(..) | Grab::Size(..)) => {}
            Some(Grab::Pan) | None => state.selected = None,
        }
    }
    let active = |g: Grab| g.same(state.dragging) || g.same(state.hover);
    for &(g, q, dir) in &sizes {
        draw_size_control(
            painter,
            q,
            dir,
            matches!(g, Grab::Size(x, y) if x != 0 && y != 0),
            active(g),
        );
    }
    if let Some((_, i)) = state.selected {
        let handles: Vec<(Handle, Pos2)> = handles.iter().map(|&(h, q)| (h, screen(q))).collect();
        draw_handles(painter, &warp, s.frame, i, on_screen[i], &handles, &active);
    }
    draw_points(painter, &warp, s.frame, &on_screen, placing, |i| {
        active(Grab::Point(i)) || state.selected.is_some_and(|(_, j)| j == i)
    });
    if warp.kind == WarpKind::Points {
        mesh_button(ui, response, s, id, &warp);
    }
    if let Some(grab) = state.dragging {
        if response.dragged() {
            if let (Some(p), Some(back)) = (response.interact_pointer_pos(), m.inverse()) {
                let (x, y) = back.apply(p.x, p.y);
                match (grab, state.start) {
                    (Grab::Pan | Grab::Size(..), Some((from, rect))) => {
                        move_frame(s, id, grab, from, rect, [x, y]);
                    }
                    _ => drag(s, id, &warp, grid.as_ref(), grab, [x, y]),
                }
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if response.drag_stopped() {
            state.dragging = None;
            state.start = None;
            s.end_gesture();
        }
        return true;
    }
    if nearest == Some(Grab::Pan) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    } else if nearest.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    } else if placing && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    true
}

/// The selected grid point's handles, joined to it by lines.
fn draw_handles(
    painter: &egui::Painter,
    warp: &MeshWarp,
    frame: u32,
    i: usize,
    point: Pos2,
    handles: &[(Handle, Pos2)],
    active: &impl Fn(Grab) -> bool,
) {
    let keyed = |h: Handle| {
        warp.handles
            .get(i)
            .is_some_and(|hs| hs[h as usize].iter().any(|v| v.has_key(frame)))
    };
    for &(h, q) in handles {
        painter.line_segment([point, q], EStroke::new(1.0, theme::ACCENT_SELECT));
        let r = if active(Grab::Handle(i, h, false)) {
            4.5
        } else {
            3.5
        };
        let fill = if keyed(h) {
            theme::ACCENT_TIME
        } else {
            theme::ACCENT_SELECT
        };
        let square = egui::Rect::from_center_size(q, Vec2::splat(2.0 * r));
        painter.rect_filled(square.expand(1.5), 0.0, theme::BG_BASE);
        painter.rect_filled(square, 0.0, fill);
    }
}

/// The warp's points: hollow while placing them, amber when keyed on
/// this frame, larger when `big`.
fn draw_points(
    painter: &egui::Painter,
    warp: &MeshWarp,
    frame: u32,
    on_screen: &[Pos2],
    placing: bool,
    big: impl Fn(usize) -> bool,
) {
    let keyed = |i: usize| {
        match warp.kind {
            WarpKind::Grid => warp.points.get(i),
            WarpKind::Points => warp.pin_moves.get(i),
        }
        .is_some_and(|p| p.iter().any(|v| v.has_key(frame)))
    };
    for (i, p) in on_screen.iter().enumerate() {
        let r = if big(i) { 6.0 } else { 4.5 };
        painter.circle_filled(*p, r + 1.5, theme::BG_BASE);
        // Amber marks a key on this frame (time), as on the Timeline.
        let fill = if keyed(i) {
            theme::ACCENT_TIME
        } else {
            theme::ACCENT_SELECT
        };
        if placing {
            painter.circle_stroke(*p, r, EStroke::new(2.0, fill));
        } else {
            painter.circle_filled(*p, r, fill);
        }
    }
}

/// The grid's lines, bent as the grid bends the art.
fn draw_grid(painter: &egui::Painter, grid: &WarpGrid, screen: &impl Fn([f32; 2]) -> Pos2) {
    let line = EStroke::new(1.0, theme::ACCENT_SELECT.gamma_multiply(0.7));
    let steps = 12;
    for r in 0..=grid.rows {
        let pts: Vec<Pos2> = (0..=grid.columns * steps)
            .map(|i| {
                let t = i as f32 / steps as f32 / grid.columns as f32;
                let (a, b) = (grid.rest(r, 0), grid.rest(r, grid.columns));
                screen(grid.apply([a[0] + (b[0] - a[0]) * t, a[1]]))
            })
            .collect();
        painter.add(egui::Shape::line(pts, line));
    }
    for c in 0..=grid.columns {
        let pts: Vec<Pos2> = (0..=grid.rows * steps)
            .map(|i| {
                let t = i as f32 / steps as f32 / grid.rows as f32;
                let (a, b) = (grid.rest(0, c), grid.rest(grid.rows, c));
                screen(grid.apply([a[0], a[1] + (b[1] - a[1]) * t]))
            })
            .collect();
        painter.add(egui::Shape::line(pts, line));
    }
}

/// Points mode: the mesh, bent, faintly; returns where the points are.
fn points_mode(
    painter: &egui::Painter,
    warp: &MeshWarp,
    frame: u32,
    screen: &impl Fn([f32; 2]) -> Pos2,
) -> Vec<[f32; 2]> {
    if let Some(WarpField::Points(f)) = warp.field_at(frame) {
        let faint = EStroke::new(0.75, theme::ACCENT_SELECT.gamma_multiply(0.35));
        for t in &f.mesh().triangles {
            let [a, b, c] = t.map(|i| screen(f.now()[i as usize]));
            painter.add(egui::Shape::closed_line(vec![a, b, c], faint));
        }
    }
    if warp.mesh.is_some() {
        warp.pins_at(frame)
    } else {
        warp.pins.clone()
    }
}

/// Placing points: a click adds one; Alt-click removes one.
fn place_or_remove(
    ui: &egui::Ui,
    s: &mut Session,
    id: NodeId,
    nearest: Option<Grab>,
    at: Option<(Pos2, Affine)>,
) {
    let alt = ui.input(|i| i.modifiers.alt);
    match (nearest, alt, at) {
        (Some(Grab::Point(i)), true, _) => edit(s, id, "Remove point", None, |w| {
            if i < w.pins.len() {
                w.pins.remove(i);
                if i < w.pin_moves.len() {
                    w.pin_moves.remove(i);
                }
            }
        }),
        (None, false, Some((p, back))) => {
            let (x, y) = back.apply(p.x, p.y);
            edit(s, id, "Place point", None, |w| {
                w.pins.push([x, y]);
                w.pin_moves
                    .push([Value::Constant(0.0), Value::Constant(0.0)]);
            });
        }
        _ => {}
    }
}

/// Handle `h` of point (r, c) goes to `to`: how far the opposite handle
/// must move from its rest place to turn by the same angle about the
/// point and keep its length. None when there is no opposite handle or
/// no angle (a handle on its point).
fn turned_opposite(grid: &WarpGrid, r: u32, c: u32, h: Handle, to: [f32; 2]) -> Option<[f32; 2]> {
    let p = grid.point(r, c);
    let (now, other) = (grid.handle(r, c, h)?, grid.handle(r, c, h.opposite())?);
    let rest = grid.handle_rest(r, c, h.opposite())?;
    let angle = |q: [f32; 2]| (q[1] - p[1]).atan2(q[0] - p[0]);
    let tiny = |q: [f32; 2]| (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-6;
    if tiny(now) || tiny(to) {
        return None;
    }
    let (sin, cos) = (angle(to) - angle(now)).sin_cos();
    let o = [other[0] - p[0], other[1] - p[1]];
    let turned = [
        p[0] + o[0] * cos - o[1] * sin,
        p[1] + o[0] * sin + o[1] * cos,
    ];
    Some([turned[0] - rest[0], turned[1] - rest[1]])
}

/// Move what `grab` holds to `to` (the warp's space).
fn drag(
    s: &mut Session,
    id: NodeId,
    warp: &MeshWarp,
    grid: Option<&WarpGrid>,
    grab: Grab,
    to: [f32; 2],
) {
    let [x, y] = to;
    let placing = warp.kind == WarpKind::Points && warp.mesh.is_none();
    match (grab, grid) {
        (Grab::Point(i), Some(grid)) => {
            let cols = warp.columns as usize + 1;
            let rest = grid.rest((i / cols) as u32, (i % cols) as u32);
            move_point(s, id, i, [x - rest[0], y - rest[1]]);
        }
        (Grab::Handle(i, h, turn), Some(grid)) => {
            let cols = grid.columns as usize + 1;
            let (r, c) = ((i / cols) as u32, (i % cols) as u32);
            if let Some(rest) = grid.handle_rest(r, c, h) {
                let opposite = turn
                    .then(|| turned_opposite(grid, r, c, h, [x, y]))
                    .flatten();
                move_handle(s, id, i, h, [x - rest[0], y - rest[1]], opposite);
            }
        }
        (Grab::Point(i), None) if placing => {
            edit(
                s,
                id,
                "Move point",
                Some(format!("place-{}-{i}", id.0)),
                |w| {
                    if let Some(p) = w.pins.get_mut(i) {
                        *p = [x, y];
                    }
                },
            );
        }
        (Grab::Point(i), None) => {
            let rest = warp.pins.get(i).copied().unwrap_or([x, y]);
            move_point(s, id, i, [x - rest[0], y - rest[1]]);
        }
        (Grab::Handle(..) | Grab::Pan | Grab::Size(..), _) => {}
    }
}

/// The grid's size controls (outside its rest frame, so they never sit
/// on a point): each with where it is on screen and which way is out.
fn size_controls(grid: &WarpGrid, screen: &impl Fn([f32; 2]) -> Pos2) -> Vec<(Grab, Pos2, Vec2)> {
    let ([cx, cy], [w, h]) = (grid.centre, grid.size);
    let centre = screen(grid.centre);
    let mut out = Vec::new();
    for sy in -1i8..=1 {
        for sx in -1i8..=1 {
            if (sx, sy) == (0, 0) {
                continue;
            }
            let at = screen([cx + f32::from(sx) * w / 2.0, cy + f32::from(sy) * h / 2.0]);
            let out_dir = at - centre;
            let dir = if out_dir.length() > 1e-3 {
                out_dir.normalized()
            } else {
                Vec2::X
            };
            out.push((Grab::Size(sx, sy), at + dir * 14.0, dir));
        }
    }
    out
}

/// A corner's square or a side's arrow, pointing out.
fn draw_size_control(painter: &egui::Painter, at: Pos2, dir: Vec2, corner: bool, active: bool) {
    let fill = if active {
        theme::ACCENT_SELECT
    } else {
        theme::BG_BASE
    };
    let stroke = EStroke::new(1.5, theme::ACCENT_SELECT);
    if corner {
        let r = if active { 5.0 } else { 4.0 };
        let square = egui::Rect::from_center_size(at, Vec2::splat(2.0 * r));
        painter.rect(square, 0.0, fill, stroke, egui::StrokeKind::Middle);
    } else {
        let k = if active { 1.3 } else { 1.0 };
        let side = dir.rot90() * 5.0 * k;
        let arrow = vec![
            at + dir * 5.0 * k,
            at - dir * 3.0 * k + side,
            at - dir * 3.0 * k - side,
        ];
        painter.add(egui::Shape::convex_polygon(arrow, fill, stroke));
    }
}

/// The grid's border as it is bent, on screen.
fn outline(grid: &WarpGrid, screen: &impl Fn([f32; 2]) -> Pos2) -> Vec<Pos2> {
    let steps = 4;
    let (rows, cols) = (grid.rows * steps, grid.columns * steps);
    let at = |r: u32, c: u32| {
        let (a, b) = (grid.rest(0, 0), grid.rest(grid.rows, grid.columns));
        let (u, v) = (c as f32 / cols as f32, r as f32 / rows as f32);
        screen(grid.apply([a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * v]))
    };
    (0..cols)
        .map(|c| at(0, c))
        .chain((0..rows).map(|r| at(r, cols)))
        .chain((1..=cols).rev().map(|c| at(rows, c)))
        .chain((1..=rows).rev().map(|r| at(r, 0)))
        .collect()
}

/// Is `p` inside the closed polygon `border` (even-odd)?
fn inside(border: &[Pos2], p: Pos2) -> bool {
    let mut odd = false;
    for (i, a) in border.iter().enumerate() {
        let b = border[(i + 1) % border.len()];
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            odd = !odd;
        }
    }
    odd
}

/// Move the grid (`Pan`) or one or two of its sides (`Size`) by how far
/// the pointer went since `from`; `rect` is the grid's left, top, right
/// and bottom at the press. Keyed as [`move_point`] keys; only the
/// values the control changes are set.
fn move_frame(
    s: &mut Session,
    id: NodeId,
    grab: Grab,
    from: [f32; 2],
    rect: [f32; 4],
    to: [f32; 2],
) {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let [mut l, mut t, mut r, mut b] = rect;
    let (sx, sy) = match grab {
        Grab::Pan => {
            (l, r, t, b) = (l + dx, r + dx, t + dy, b + dy);
            (0, 0)
        }
        Grab::Size(sx, sy) => {
            match sx.signum() {
                -1 => l += dx,
                1 => r += dx,
                _ => {}
            }
            match sy.signum() {
                -1 => t += dy,
                1 => b += dy,
                _ => {}
            }
            (sx, sy)
        }
        _ => return,
    };
    let (animate, frame) = (s.animate, s.frame);
    let (name, pan) = match grab {
        Grab::Pan => ("Move grid", true),
        _ => ("Resize grid", false),
    };
    edit(s, id, name, Some(format!("mesh-frame-{}", id.0)), |w| {
        if pan || sx != 0 {
            set_value(&mut w.x, (l + r) / 2.0, animate, frame);
        }
        if pan || sy != 0 {
            set_value(&mut w.y, (t + b) / 2.0, animate, frame);
        }
        if sx != 0 {
            set_value(&mut w.width, (r - l).abs().max(1.0), animate, frame);
        }
        if sy != 0 {
            set_value(&mut w.height, (b - t).abs().max(1.0), animate, frame);
        }
    });
}

/// Points mode's next step, on the canvas: Build mesh once points are
/// placed, Edit points once it's built.
fn mesh_button(
    ui: &egui::Ui,
    response: &egui::Response,
    s: &mut Session,
    id: NodeId,
    w: &MeshWarp,
) {
    let at = response.rect.left_top() + Vec2::new(12.0, 12.0);
    egui::Area::new(egui::Id::unique("mesh-warp-step"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                if w.mesh.is_none() {
                    ui.label(
                        egui::RichText::new(
                            "Click the artwork to place points (Alt-click removes one).",
                        )
                        .font(theme::dense())
                        .color(theme::TEXT_SECONDARY),
                    );
                    if widgets::primary_button(ui, "Build mesh", !w.pins.is_empty()).clicked() {
                        if let Err(e) = build_mesh(s, id) {
                            s.note(e);
                        }
                    }
                } else if ui
                    .button("Edit points")
                    .on_hover_text("Go back to placing points (the mesh is rebuilt after).")
                    .clicked()
                {
                    edit(s, id, "Edit points", None, |w| w.mesh = None);
                }
            });
        });
}

/// The drawings a Mesh Warp bends on this frame, each with its drawing ->
/// warp space transform (the pegs between them).
fn drawings_under(s: &Session, id: NodeId) -> Vec<(symphony_core::id::Id, Affine)> {
    let graph = &s.doc.graph;
    let mut out = Vec::new();
    for (layer, l) in s.doc.layers.iter().enumerate() {
        let mut chain = Vec::new();
        let mut at = graph.input(l.node, 0);
        let mut found = false;
        while let Some(n) = at {
            if n == id {
                found = true;
                break;
            }
            if chain.len() > 256 {
                break;
            }
            chain.push(n);
            at = graph.input(n, 0);
        }
        if !found {
            continue;
        }
        // Drawing -> warp: the pegs from the warp down to the drawing.
        let mut m = Affine::IDENTITY;
        for &n in chain.iter().rev() {
            if let Some(NodeKind::Peg(p)) = graph.node(n).map(|n| &n.kind) {
                m = m.compose(p.transform_at(s.frame));
            }
        }
        if let Some(d) = s.doc.exposed(layer, s.frame) {
            out.push((d, m));
        }
    }
    out
}

/// Build the Points mesh over the artwork under the warp (on this frame).
pub fn build_mesh(s: &mut Session, id: NodeId) -> Result<(), String> {
    let Some(NodeKind::MeshWarp(w)) = s.doc.graph.node(id).map(|n| &n.kind) else {
        return Err("Select a Mesh Warp first".into());
    };
    if w.pins.is_empty() {
        return Err("Place some points on the artwork first".into());
    }
    let (pins, density) = (w.pins.clone(), w.density);
    let under = drawings_under(s, id);
    let drawings: Vec<(&symphony_core::stroke::Drawing, Affine)> = under
        .iter()
        .filter_map(|(d, m)| s.doc.drawings.get(d).map(|dr| (dr, *m)))
        .collect();
    let shapes = symphony_vector::puppet_mesh::silhouette(&drawings, density * 0.3);
    let (mesh, placed) = symphony_vector::puppet_mesh::build(&shapes, &pins, density)
        .ok_or("There's no artwork under this Mesh Warp on this frame to build a mesh on")?;
    edit(s, id, "Build mesh", None, |w| {
        w.pins = placed;
        w.pin_moves
            .resize(w.pins.len(), [Value::Constant(0.0), Value::Constant(0.0)]);
        w.mesh = Some(mesh);
    });
    Ok(())
}

/// Change the Mesh Warp `id` as one undo step (merged by `merge` while a
/// drag goes on).
fn edit(
    s: &mut Session,
    id: NodeId,
    name: &str,
    merge: Option<String>,
    f: impl FnOnce(&mut MeshWarp),
) {
    let Some(before) = s.doc.graph.node(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    let NodeKind::MeshWarp(w) = &mut after.kind else {
        return;
    };
    f(w);
    if after == before {
        return;
    }
    let e = Edit::SetNodeProperties { id, before, after };
    match merge {
        Some(key) => s.history.apply_merged(&mut s.doc, name, &key, e),
        None => s.history.apply(&mut s.doc, name, vec![e]),
    }
}

/// Set point `i`'s offset (a grid point, or a placed point once the mesh
/// is built), keyed on this frame in Animate mode (or when it's already
/// animated), as one merged undo step per drag.
fn move_point(s: &mut Session, id: NodeId, i: usize, offset: [f32; 2]) {
    let (animate, frame) = (s.animate, s.frame);
    edit(
        s,
        id,
        "Move mesh point",
        Some(format!("mesh-point-{}-{i}", id.0)),
        |w| {
            let point = match w.kind {
                WarpKind::Grid => w.points.get_mut(i),
                WarpKind::Points => w.pin_moves.get_mut(i),
            };
            if let Some(point) = point {
                set_pair(point, offset, animate, frame);
            }
        },
    );
}

/// Set grid point `i`'s handle `h` to `moved` from its rest place, and
/// the opposite handle to `opposite` from its own when given; keyed as
/// [`move_point`] keys.
fn move_handle(
    s: &mut Session,
    id: NodeId,
    i: usize,
    h: Handle,
    moved: [f32; 2],
    opposite: Option<[f32; 2]>,
) {
    let (animate, frame) = (s.animate, s.frame);
    edit(
        s,
        id,
        "Move mesh handle",
        Some(format!("mesh-handle-{}-{i}", id.0)),
        |w| {
            if let Some(pair) = w.handle_mut(i, h) {
                set_pair(pair, moved, animate, frame);
            }
            if let Some(to) = opposite {
                if let Some(pair) = w.handle_mut(i, h.opposite()) {
                    set_pair(pair, to, animate, frame);
                }
            }
        },
    );
}

/// Set an (x, y) pair: keyed on `frame` in Animate mode or when it is
/// already animated, otherwise for every frame.
fn set_pair(pair: &mut [Value; 2], to: [f32; 2], animate: bool, frame: u32) {
    for (value, v) in pair.iter_mut().zip(to) {
        set_value(value, v, animate, frame);
    }
}

/// [`set_pair`] for one value.
fn set_value(value: &mut Value, to: f32, animate: bool, frame: u32) {
    match value {
        Value::Constant(c) if !animate => *c = to,
        _ => value.set_key(frame, to),
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

    /// A dragged handle turns its opposite by the same angle, and the
    /// opposite keeps its length; Alt (no turn) leaves it. In Animate
    /// mode both are keyed, and undo takes the drag back.
    #[test]
    fn a_handle_turns_its_opposite_and_keeps_its_length() {
        let mut s = Session::new_scene();
        let node = s.doc.layers[0].node;
        crate::views::node::insert_near(&mut s, node, crate::views::node::mesh_warp(100, 100));
        let id = s.doc.graph.input(node, 0).unwrap();
        let warp = |s: &Session| match &s.doc.graph.node(id).unwrap().kind {
            NodeKind::MeshWarp(w) => (**w).clone(),
            _ => unreachable!(),
        };
        // Point 12 is (row 2, column 2) of the 4 x 4 grid.
        let arm = |w: &MeshWarp, h: Handle| {
            let g = w.grid_at(1);
            let (p, q) = (g.point(2, 2), g.handle(2, 2, h).unwrap());
            [q[0] - p[0], q[1] - p[1]]
        };
        let drag_to = |s: &mut Session, h: Handle, arm_to: [f32; 2], turn: bool| {
            let g = warp(s).grid_at(s.frame);
            let p = g.point(2, 2);
            let to = [p[0] + arm_to[0], p[1] + arm_to[1]];
            let rest = g.handle_rest(2, 2, h).unwrap();
            let opposite = turn.then(|| turned_opposite(&g, 2, 2, h, to)).flatten();
            move_handle(s, id, 12, h, [to[0] - rest[0], to[1] - rest[1]], opposite);
            s.end_gesture();
        };
        s.animate = false;
        let before = arm(&warp(&s), Handle::PrevColumn);
        let length = before[0].hypot(before[1]);
        // Turn the next-column handle a quarter turn, and make it longer.
        let next = arm(&warp(&s), Handle::NextColumn);
        drag_to(
            &mut s,
            Handle::NextColumn,
            [-next[1] * 2.0, next[0] * 2.0],
            true,
        );
        let after = arm(&warp(&s), Handle::PrevColumn);
        assert!(
            (after[0].hypot(after[1]) - length).abs() < 1e-3,
            "{after:?}"
        );
        let turned = [-before[1], before[0]];
        assert!(
            (after[0] - turned[0]).abs() < 1e-3 && (after[1] - turned[1]).abs() < 1e-3,
            "{after:?} vs {turned:?}"
        );
        // Alt: the opposite stays.
        drag_to(&mut s, Handle::NextColumn, [5.0, 5.0], false);
        assert_eq!(arm(&warp(&s), Handle::PrevColumn), after);
        s.animate = true;
        s.frame = 7;
        drag_to(&mut s, Handle::PrevRow, [3.0, -9.0], true);
        assert!(warp(&s).has_key(7));
        s.undo();
        assert!(!warp(&s).has_key(7));
    }

    /// Dragging an empty space moves the grid (keyed in Animate mode,
    /// size untouched); a corner sizes it with the opposite corner
    /// staying put; a side arrow changes one size only.
    #[test]
    fn the_grid_moves_and_sizes_by_its_frame() {
        let mut s = Session::new_scene();
        let node = s.doc.layers[0].node;
        crate::views::node::insert_near(&mut s, node, crate::views::node::mesh_warp(100, 100));
        let id = s.doc.graph.input(node, 0).unwrap();
        let warp = |s: &Session| match &s.doc.graph.node(id).unwrap().kind {
            NodeKind::MeshWarp(w) => (**w).clone(),
            _ => unreachable!(),
        };
        let rect = |w: &MeshWarp| {
            let g = w.grid_at(1);
            let ([cx, cy], [sw, sh]) = (g.centre, g.size);
            [cx - sw / 2.0, cy - sh / 2.0, cx + sw / 2.0, cy + sh / 2.0]
        };
        let r0 = rect(&warp(&s));
        s.animate = true;
        move_frame(&mut s, id, Grab::Pan, [0.0, 0.0], r0, [10.0, -4.0]);
        s.end_gesture();
        let w = warp(&s);
        assert!(w.x.has_key(1) && w.y.has_key(1));
        assert!(!w.width.has_key(1) && !w.height.has_key(1));
        let r1 = rect(&w);
        assert_eq!(r1, [r0[0] + 10.0, r0[1] - 4.0, r0[2] + 10.0, r0[3] - 4.0]);
        s.animate = false;
        move_frame(&mut s, id, Grab::Size(1, 1), [0.0, 0.0], r1, [20.0, 6.0]);
        s.end_gesture();
        let r2 = rect(&warp(&s));
        assert!((r2[0] - r1[0]).abs() < 1e-3 && (r2[1] - r1[1]).abs() < 1e-3);
        assert!((r2[2] - r1[2] - 20.0).abs() < 1e-3 && (r2[3] - r1[3] - 6.0).abs() < 1e-3);
        move_frame(&mut s, id, Grab::Size(-1, 0), [0.0, 0.0], r2, [5.0, 50.0]);
        s.end_gesture();
        let r3 = rect(&warp(&s));
        assert!((r3[0] - r2[0] - 5.0).abs() < 1e-3);
        assert!((r3[1] - r2[1]).abs() < 1e-3 && (r3[3] - r2[3]).abs() < 1e-3);
        s.undo();
        assert_eq!(rect(&warp(&s)), r2);
    }

    #[test]
    fn an_empty_space_is_inside_the_bent_grid() {
        let mut w = MeshWarp::new(2, 2, 0.0, 0.0, 100.0, 100.0);
        w.points[1] = [Value::Constant(0.0), Value::Constant(-30.0)];
        let g = w.grid_at(1);
        let screen = |p: [f32; 2]| Pos2::new(p[0], p[1]);
        let border = outline(&g, &screen);
        assert!(inside(&border, Pos2::new(10.0, 10.0)));
        assert!(inside(&border, Pos2::new(0.0, -70.0)), "the bulge");
        assert!(!inside(&border, Pos2::new(-40.0, -70.0)));
        assert!(!inside(&border, Pos2::new(80.0, 0.0)));
    }
}
