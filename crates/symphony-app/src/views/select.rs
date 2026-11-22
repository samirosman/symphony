//! Select and Transform tools on the canvas, following Harmony 27:
//! - Select works on whole strokes of the current drawing. Click to pick,
//!   Shift to add, drag empty space for a Lasso (or Marquee) selection;
//!   Option/Alt swaps the mode while held, Shift+Option/Alt removes.
//! - Transform works on layers through their pegs (cut-out animation):
//!   click artwork to pick its layer, then move, rotate or scale it. In
//!   Animate mode this sets keyframes.
//!
//! Both use the same bounding box: drag inside to move, a handle to scale
//! (Shift keeps proportions), just outside a corner to rotate.

use eframe::egui::{self, Pos2, Rect, Stroke as EStroke, Vec2};
use symphony_core::id::Id;
use symphony_core::math::Affine;
use symphony_ui::icons::Icon;
use symphony_ui::theme;
use symphony_vector::{bounds_of, hits, inside_polygon, scale_of};

use crate::session::{PegValues, Session, Tool};

const HANDLE: f32 = 7.0;
/// How far outside a corner still grabs rotation, in screen points.
const ROTATE_ZONE: f32 = 24.0;
/// Pick tolerance in screen points.
const PICK: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Move,
    Scale(usize),
    Rotate,
}

struct Drag {
    kind: Kind,
    /// Drawing-local box at drag start.
    lo: [f32; 2],
    hi: [f32; 2],
    /// Drawing-local -> screen at drag start.
    to_screen: Affine,
    start: Pos2,
    /// Drawing -> scene at drag start (Transform tool).
    world: Affine,
    /// Peg values at drag start (Transform tool).
    peg: Option<PegValues>,
}

#[derive(Default)]
pub struct SelectState {
    drag: Option<Drag>,
    lasso: Vec<Pos2>,
    marquee: Option<Pos2>,
    /// Drawing-local transform being previewed while dragging (Select tool).
    pub preview: Option<Affine>,
    /// Cursor hint for the handle under the pointer (D8.11).
    pub hover: Option<Icon>,
}

impl SelectState {
    pub fn busy(&self) -> bool {
        self.drag.is_some() || !self.lasso.is_empty() || self.marquee.is_some()
    }
}

/// One drawn layer: (layer index, drawing, drawing -> scene transform).
pub type Item = (usize, Id, Affine);

fn corners(lo: [f32; 2], hi: [f32; 2]) -> [(f32, f32); 8] {
    let (mx, my) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    [
        (lo[0], lo[1]),
        (hi[0], lo[1]),
        (hi[0], hi[1]),
        (lo[0], hi[1]),
        (mx, lo[1]),
        (hi[0], my),
        (mx, hi[1]),
        (lo[0], my),
    ]
}

/// The handle opposite `i`, which stays put while scaling.
fn anchor(i: usize) -> usize {
    match i {
        0..=3 => (i + 2) % 4,
        4 => 6,
        5 => 7,
        6 => 4,
        _ => 5,
    }
}

fn pos(p: (f32, f32)) -> Pos2 {
    Pos2::new(p.0, p.1)
}

/// What the pointer would grab at `p`, given the box in screen space.
fn grab(p: Pos2, lo: [f32; 2], hi: [f32; 2], m: Affine) -> Option<Kind> {
    let pts = corners(lo, hi).map(|c| pos(m.apply(c.0, c.1)));
    if let Some(i) = pts.iter().position(|h| h.distance(p) <= HANDLE + 2.0) {
        return Some(Kind::Scale(i));
    }
    let quad: Vec<[f32; 2]> = pts[..4].iter().map(|q| [q.x, q.y]).collect();
    if inside_polygon(p.x, p.y, &quad) {
        return Some(Kind::Move);
    }
    if pts[..4].iter().any(|c| c.distance(p) <= ROTATE_ZONE) {
        return Some(Kind::Rotate);
    }
    None
}

/// Drawing-local transform for a drag from `start` to `now`.
fn drag_transform(d: &Drag, now: Pos2, uniform: bool) -> Option<Affine> {
    let inv = d.to_screen.inverse()?;
    let (sx, sy) = inv.apply(d.start.x, d.start.y);
    let (nx, ny) = inv.apply(now.x, now.y);
    let c = corners(d.lo, d.hi);
    Some(match d.kind {
        Kind::Move => Affine::translate(nx - sx, ny - sy),
        Kind::Rotate => {
            let (cx, cy) = ((d.lo[0] + d.hi[0]) / 2.0, (d.lo[1] + d.hi[1]) / 2.0);
            let a = (sy - cy).atan2(sx - cx);
            let b = (ny - cy).atan2(nx - cx);
            Affine::translate(cx, cy)
                .compose(Affine::rotate_deg((b - a).to_degrees()))
                .compose(Affine::translate(-cx, -cy))
        }
        Kind::Scale(i) => {
            let (ax, ay) = c[anchor(i)];
            let ratio = |n: f32, s: f32, a: f32| {
                if (s - a).abs() < 1e-3 {
                    1.0
                } else {
                    (n - a) / (s - a)
                }
            };
            let (mut fx, mut fy) = (ratio(nx, sx, ax), ratio(ny, sy, ay));
            match i {
                4 | 6 => fx = 1.0,
                5 | 7 => fy = 1.0,
                _ => {}
            }
            if uniform && i < 4 {
                let f = if fx.abs() > fy.abs() { fx } else { fy };
                (fx, fy) = (f, f);
            }
            Affine::translate(ax, ay)
                .compose(Affine::scale(fx, fy))
                .compose(Affine::translate(-ax, -ay))
        }
    })
}

fn draw_box(painter: &egui::Painter, lo: [f32; 2], hi: [f32; 2], m: Affine) {
    let pts = corners(lo, hi).map(|c| pos(m.apply(c.0, c.1)));
    let outline = vec![pts[0], pts[1], pts[2], pts[3]];
    painter.add(egui::Shape::closed_line(
        outline,
        EStroke::new(1.0, theme::ACCENT_SELECT),
    ));
    for p in pts {
        let r = Rect::from_center_size(p, Vec2::splat(HANDLE));
        painter.rect_filled(r, 1.0, theme::CANVAS_STAGE);
        painter.rect_stroke(
            r,
            1.0,
            EStroke::new(1.0, theme::ACCENT_SELECT),
            egui::StrokeKind::Inside,
        );
    }
}

/// Handle Select and Transform input and draw their overlays.
/// `view` maps scene -> screen. `items` are the visible layers.
pub fn ui(
    ui: &egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    s: &mut Session,
    st: &mut SelectState,
    view: Affine,
    items: &[Item],
) {
    let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let transform_tool = s.tool == Tool::Transform;

    // The box we're working with: drawing-local bounds and local -> screen.
    let target: Option<([f32; 2], [f32; 2], Affine)> = if transform_tool {
        let layer = s.selected;
        items
            .iter()
            .find(|(l, _, _)| *l == layer)
            .and_then(|(_, d, world)| {
                let dr = s.doc.drawings.get(d)?;
                let (lo, hi) = bounds_of(&dr.strokes)?;
                Some((lo, hi, view.compose(*world)))
            })
    } else {
        s.validate_selection();
        let world = world_of(s, items);
        bounds_of(s.selected_strokes()).map(|(lo, hi)| (lo, hi, view.compose(world)))
    };

    st.hover = match (&st.drag, target, pointer) {
        (Some(d), _, _) => Some(if d.kind == Kind::Rotate {
            Icon::RotateRight
        } else {
            Icon::Transform
        }),
        (None, Some((lo, hi, m)), Some(p)) => match grab(p, lo, hi, m) {
            Some(Kind::Rotate) => Some(Icon::RotateRight),
            Some(Kind::Scale(_)) => Some(Icon::Transform),
            _ => None,
        },
        _ => None,
    };

    // Start a gesture.
    if response.drag_started() {
        if let Some(p) = response.interact_pointer_pos() {
            let on_box = target.and_then(|(lo, hi, m)| grab(p, lo, hi, m).map(|k| (k, lo, hi, m)));
            let start_drag =
                |kind: Kind, lo: [f32; 2], hi: [f32; 2], m: Affine, s: &mut Session| {
                    let peg = if transform_tool {
                        s.doc.peg(s.selected).map(|_| current_peg(s))
                    } else {
                        None
                    };
                    if transform_tool && kind != Kind::Move {
                        let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
                        s.ensure_pivot(s.selected, (cx, cy));
                    }
                    let world = view.inverse().unwrap_or(Affine::IDENTITY).compose(m);
                    Drag {
                        kind,
                        lo,
                        hi,
                        to_screen: m,
                        start: p,
                        world,
                        peg,
                    }
                };
            if let Some((kind, lo, hi, m)) = on_box.filter(|(k, ..)| {
                *k != Kind::Move || s.easy_drag || hit_selection(s, p, view, items, transform_tool)
            }) {
                st.drag = Some(start_drag(kind, lo, hi, m, s));
            } else if pick(s, p, view, items, transform_tool, shift).is_some() {
                // Dragging unselected artwork selects it and moves it at once.
                let fresh = if transform_tool {
                    items
                        .iter()
                        .find(|(l, _, _)| *l == s.selected)
                        .and_then(|(_, d, w)| {
                            let (lo, hi) = bounds_of(&s.doc.drawings.get(d)?.strokes)?;
                            Some((lo, hi, view.compose(*w)))
                        })
                } else {
                    let world = world_of(s, items);
                    bounds_of(s.selected_strokes()).map(|(lo, hi)| (lo, hi, view.compose(world)))
                };
                if let Some((lo, hi, m)) = fresh {
                    st.drag = Some(start_drag(Kind::Move, lo, hi, m, s));
                }
            } else if !transform_tool {
                let marquee = s.marquee_mode != alt || (shift && alt && s.marquee_mode);
                if marquee {
                    st.marquee = Some(p);
                } else {
                    st.lasso = vec![p];
                }
            }
        }
    }

    // Continue it.
    if response.dragged() {
        if let Some(p) = response.interact_pointer_pos() {
            if let Some(d) = &st.drag {
                if let Some(l) = drag_transform(d, p, shift) {
                    if transform_tool {
                        apply_peg_drag(s, d, l);
                    } else {
                        st.preview = Some(l);
                    }
                }
            } else if !st.lasso.is_empty() && st.lasso.last().is_none_or(|q| q.distance(p) > 2.0) {
                st.lasso.push(p);
            }
        }
    }

    // Finish it.
    if response.drag_stopped() {
        if let (Some(d), Some(p)) = (st.drag.take(), response.interact_pointer_pos()) {
            if transform_tool {
                s.end_gesture();
            } else if let Some(l) = drag_transform(&d, p, shift) {
                let name = match d.kind {
                    Kind::Move => "Move selection",
                    Kind::Rotate => "Rotate selection",
                    Kind::Scale(_) => "Scale selection",
                };
                s.transform_selection(l, name);
            }
        }
        st.preview = None;
        let area: Option<Vec<[f32; 2]>> = if let Some(a) = st.marquee.take() {
            let b = response.interact_pointer_pos().unwrap_or(a);
            let r = Rect::from_two_pos(a, b);
            Some(vec![
                [r.min.x, r.min.y],
                [r.max.x, r.min.y],
                [r.max.x, r.max.y],
                [r.min.x, r.max.y],
            ])
        } else if st.lasso.len() > 2 {
            Some(
                std::mem::take(&mut st.lasso)
                    .into_iter()
                    .map(|q| [q.x, q.y])
                    .collect(),
            )
        } else {
            st.lasso.clear();
            None
        };
        if let Some(poly) = area {
            select_area(s, &poly, view, items, shift, shift && alt);
        }
    }

    // Plain click: pick, or clear when clicking empty space.
    if response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            if pick(s, p, view, items, transform_tool, shift).is_none() && !shift && !transform_tool
            {
                s.deselect();
            }
        }
    }

    // Overlays.
    if let Some(a) = st.marquee {
        if let Some(b) = response.interact_pointer_pos() {
            painter.rect_stroke(
                Rect::from_two_pos(a, b),
                0.0,
                EStroke::new(1.0, theme::ACCENT_SELECT),
                egui::StrokeKind::Middle,
            );
        }
    }
    if st.lasso.len() > 1 {
        painter.add(egui::Shape::line(
            st.lasso.clone(),
            EStroke::new(1.0, theme::ACCENT_SELECT),
        ));
    }
    if let Some((lo, hi, m)) = target {
        let m = st.preview.map_or(m, |l| m.compose(l));
        draw_box(painter, lo, hi, m);
        if transform_tool {
            if let Some(p) = s.doc.peg(s.selected) {
                // The pivot is amber, because it belongs to animation (DESIGN §4.3).
                let world = items
                    .iter()
                    .find(|(l, _, _)| *l == s.selected)
                    .map(|(_, _, w)| *w)
                    .unwrap_or(Affine::IDENTITY);
                let c = view.compose(world).apply(p.pivot.0, p.pivot.1);
                painter.circle_stroke(pos(c), 5.0, EStroke::new(2.0, theme::ACCENT_TIME));
            }
        }
    }
}

/// The selected layer's drawing -> scene transform as this view draws it
/// (the Drawing view shows drawings without pegs).
fn world_of(s: &Session, items: &[Item]) -> Affine {
    items
        .iter()
        .find(|(l, _, _)| *l == s.selected)
        .map(|(_, _, w)| *w)
        .unwrap_or_else(|| {
            s.doc
                .layers
                .get(s.selected)
                .and_then(|l| s.doc.graph.world_transform(l.node, s.frame).ok())
                .unwrap_or(Affine::IDENTITY)
        })
}

fn current_peg(s: &Session) -> PegValues {
    let p = s.doc.peg(s.selected).cloned().unwrap_or_default();
    let f = s.frame;
    PegValues {
        x: p.x.at(f),
        y: p.y.at(f),
        rotation: p.rotation.at(f),
        scale_x: p.scale_x.as_ref().map_or(1.0, |v| v.at(f)),
        scale_y: p.scale_y.as_ref().map_or(1.0, |v| v.at(f)),
    }
}

/// Turn a drawing-local drag transform into peg values from the drag start.
fn apply_peg_drag(s: &mut Session, d: &Drag, l: Affine) {
    let Some(start) = d.peg else { return };
    let layer = s.selected;
    match d.kind {
        Kind::Move => {
            // A drawing-local offset, carried into scene space by the
            // drawing's own rotation and scale.
            let linear = Affine {
                tx: 0.0,
                ty: 0.0,
                ..d.world
            };
            let (dx, dy) = linear.apply(l.tx, l.ty);
            s.adjust_peg(layer, |v| {
                v.x = start.x + dx;
                v.y = start.y + dy;
            });
        }
        Kind::Rotate => {
            let deg = l.b.atan2(l.a).to_degrees();
            s.adjust_peg(layer, |v| v.rotation = start.rotation + deg);
        }
        Kind::Scale(_) => {
            let (fx, fy) = (l.a, l.d);
            s.adjust_peg(layer, |v| {
                v.scale_x = start.scale_x * fx;
                v.scale_y = start.scale_y * fy;
            });
        }
    }
}

/// Is `p` on the current selection's artwork?
fn hit_selection(s: &Session, p: Pos2, view: Affine, items: &[Item], transform_tool: bool) -> bool {
    if transform_tool {
        return layer_at(s, p, view, items) == Some(s.selected);
    }
    let world = world_of(s, items);
    let m = view.compose(world);
    let Some(inv) = m.inverse() else { return false };
    let (x, y) = inv.apply(p.x, p.y);
    let r = PICK / scale_of(m).max(1e-6);
    s.selected_strokes().iter().any(|st| hits(st, x, y, r))
}

/// The front-most layer with artwork under `p`.
fn layer_at(s: &Session, p: Pos2, view: Affine, items: &[Item]) -> Option<usize> {
    items.iter().rev().find_map(|(layer, d, world)| {
        let m = view.compose(*world);
        let inv = m.inverse()?;
        let (x, y) = inv.apply(p.x, p.y);
        let r = PICK / scale_of(m).max(1e-6);
        let dr = s.doc.drawings.get(d)?;
        dr.strokes
            .iter()
            .any(|st| hits(st, x, y, r))
            .then_some(*layer)
    })
}

/// Click picking. Transform picks a layer; Select picks a stroke in the
/// current drawing (Shift adds or toggles).
fn pick(
    s: &mut Session,
    p: Pos2,
    view: Affine,
    items: &[Item],
    transform_tool: bool,
    shift: bool,
) -> Option<usize> {
    if transform_tool {
        let layer = layer_at(s, p, view, items)?;
        s.selected = layer;
        return Some(layer);
    }
    s.validate_selection();
    let d = s.selection.drawing?;
    let world = world_of(s, items);
    let m = view.compose(world);
    let inv = m.inverse()?;
    let (x, y) = inv.apply(p.x, p.y);
    let r = PICK / scale_of(m).max(1e-6);
    let hit = s
        .doc
        .drawings
        .get(&d)?
        .strokes
        .iter()
        .rev()
        .find(|st| st.art == s.art && hits(st, x, y, r))?
        .id;
    if shift {
        if let Some(i) = s.selection.strokes.iter().position(|id| *id == hit) {
            s.selection.strokes.remove(i);
        } else {
            s.selection.strokes.push(hit);
        }
    } else if !s.selection.strokes.contains(&hit) {
        s.selection.strokes = vec![hit];
    }
    Some(s.selected)
}

/// Lasso or marquee: select strokes whose middle falls inside `poly` (screen).
fn select_area(
    s: &mut Session,
    poly: &[[f32; 2]],
    view: Affine,
    items: &[Item],
    add: bool,
    remove: bool,
) {
    s.validate_selection();
    let Some(d) = s.selection.drawing else { return };
    let world = world_of(s, items);
    let m = view.compose(world);
    let Some(dr) = s.doc.drawings.get(&d) else {
        return;
    };
    let inside: Vec<Id> = dr
        .strokes
        .iter()
        .filter(|st| st.art == s.art)
        .filter(|st| {
            symphony_vector::bounds(st).is_some_and(|(lo, hi)| {
                let (x, y) = m.apply((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
                inside_polygon(x, y, poly)
            })
        })
        .map(|st| st.id)
        .collect();
    if remove {
        s.selection.strokes.retain(|id| !inside.contains(id));
    } else if add {
        for id in inside {
            if !s.selection.strokes.contains(&id) {
                s.selection.strokes.push(id);
            }
        }
    } else {
        s.selection.strokes = inside;
    }
}
