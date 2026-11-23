//! Select and Transform tools on the canvas, following Harmony 27:
//! - Select works on whole strokes of the current drawing. Click to pick,
//!   Shift adds and Control subtracts. Click empty space to place polygon
//!   anchors, drag an anchor to curve it, or drag on empty space to lasso.
//! - Transform works on layers through their pegs (cut-out animation):
//!   click artwork to pick its layer, then move, rotate or scale it. In
//!   Animate mode this sets keyframes.
//!
//! Both use the same bounding box: drag inside to move, a handle to scale
//! (Shift keeps proportions), just outside a corner to rotate.

use eframe::egui::{self, Pos2, Rect, Stroke as EStroke, Vec2};
use symphony_core::graph::NodeId;
use symphony_core::id::Id;
use symphony_core::math::Affine;
use symphony_core::stroke::Drawing;
use symphony_ui::icons::Icon;
use symphony_ui::theme;
use symphony_vector::{bounds_of, hits, inside_polygon, scale_of};

use crate::session::{PegValues, Session, Tool};

const HANDLE: f32 = 7.0;
/// How far outside a corner still grabs rotation, in screen points.
const ROTATE_ZONE: f32 = 24.0;
/// Pick tolerance in screen points.
const PICK: f32 = 5.0;
const LASSO_THRESHOLD: f32 = 8.0;
const CLOSE_RADIUS: f32 = 9.0;

#[derive(Clone, Copy)]
struct Anchor {
    at: Pos2,
    handle: Vec2,
}

fn outline(anchors: &[Anchor], closed: bool) -> Vec<Pos2> {
    let Some(first) = anchors.first() else {
        return Vec::new();
    };
    let mut points = vec![first.at];
    for (i, pair) in anchors
        .iter()
        .enumerate()
        .take(anchors.len() - usize::from(!closed))
    {
        let next = anchors[(i + 1) % anchors.len()];
        if pair.handle == Vec2::ZERO && next.handle == Vec2::ZERO {
            points.push(next.at);
            continue;
        }
        let a = pair.at;
        let b = a + pair.handle;
        let d = next.at;
        let c = d - next.handle;
        let length = a.distance(b) + b.distance(c) + c.distance(d);
        let steps = (length / 4.0).ceil().clamp(4.0, 96.0) as usize;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let u = 1.0 - t;
            points.push(
                Pos2::ZERO
                    + a.to_vec2() * (u * u * u)
                    + b.to_vec2() * (3.0 * u * u * t)
                    + c.to_vec2() * (3.0 * u * t * t)
                    + d.to_vec2() * (t * t * t),
            );
        }
    }
    points
}

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
    /// Peg values at drag start (Transform tool, one layer).
    peg: Option<PegValues>,
    /// The layer whose peg moves (Transform tool, one layer).
    layer: usize,
    /// Each layer at drag start when several move together (Transform
    /// tool); empty for one layer.
    group: Vec<GroupStart>,
}

/// One of several layers the Transform tool moves together.
struct GroupStart {
    layer: usize,
    peg: PegValues,
    /// The peg's parent space -> the view's scene space at drag start.
    parent: Affine,
    /// The peg's pivot in the view's scene space at drag start.
    pivot: (f32, f32),
}

/// The Transform tool's box: `to_screen` maps the box to the screen. One
/// layer's box is in its drawing's space, so it turns with the layer;
/// several layers share one box in scene space.
struct TransformBox {
    lo: [f32; 2],
    hi: [f32; 2],
    to_screen: Affine,
    layers: Vec<usize>,
}

#[derive(Default)]
pub struct SelectState {
    /// Camera input uses the projected drawing for hit tests.
    pub camera_space: bool,
    drag: Option<Drag>,
    lasso: Vec<Pos2>,
    polygon: Vec<Anchor>,
    polygon_drag: Option<Pos2>,
    lasso_active: bool,
    polygon_owner: Option<(String, Option<Id>, symphony_core::stroke::ArtLayer)>,
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
        painter.rect_filled(r, 1.0, theme::ON_ACCENT);
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
    let (shift, alt, ctrl, enter, escape, view_pointer) = ui.input(|i| {
        (
            i.modifiers.shift,
            i.modifiers.alt,
            i.modifiers.ctrl,
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
            i.pointer.press_origin(),
        )
    });
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let transform_tool = s.tool == Tool::Transform;
    if transform_tool {
        follow_current_layer(s);
    }
    let boxed = if transform_tool {
        transform_box(s, view, items)
    } else {
        None
    };
    let box_layers = boxed.as_ref().map(|b| b.layers.clone()).unwrap_or_default();

    // The box we're working with: drawing-local bounds and local -> screen.
    let target: Option<([f32; 2], [f32; 2], Affine)> = if transform_tool {
        boxed.map(|b| (b.lo, b.hi, b.to_screen))
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

    let drawing = s.selection.drawing;
    let has_polygon = |st: &SelectState| !st.polygon.is_empty() || st.polygon_drag.is_some();
    if has_polygon(st)
        && (transform_tool
            || escape
            || !st
                .polygon_owner
                .as_ref()
                .is_some_and(|(key, d, art)| key == &s.key && *d == drawing && *art == s.art))
    {
        st.polygon.clear();
        st.polygon_drag = None;
    }
    let mut finish_polygon = enter && !st.polygon.is_empty() && response.hovered();

    let camera_space = st.camera_space;
    // Start a gesture.
    if response.drag_started_by(egui::PointerButton::Primary) {
        if let Some(p) = view_pointer.or(response.interact_pointer_pos()) {
            let on_box = target.and_then(|(lo, hi, m)| grab(p, lo, hi, m).map(|k| (k, lo, hi, m)));
            let start_drag = |kind: Kind,
                              lo: [f32; 2],
                              hi: [f32; 2],
                              m: Affine,
                              layers: &[usize],
                              s: &mut Session| {
                let group: Vec<GroupStart> = if transform_tool && layers.len() > 1 {
                    layers.iter().filter_map(|&l| group_start(s, l)).collect()
                } else {
                    Vec::new()
                };
                let layer = layers.first().copied().unwrap_or(s.selected);
                let peg = if transform_tool && group.is_empty() {
                    s.transform_peg(layer).map(|_| current_peg(s, layer))
                } else {
                    None
                };
                if transform_tool && group.is_empty() && kind != Kind::Move {
                    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
                    s.ensure_pivot(layer, (cx, cy));
                }
                let world = if camera_space {
                    s.doc
                        .layers
                        .get(layer)
                        .and_then(|layer| s.doc.graph.world_transform(layer.node, s.frame).ok())
                        .unwrap_or(Affine::IDENTITY)
                } else {
                    view.inverse().unwrap_or(Affine::IDENTITY).compose(m)
                };
                Drag {
                    kind,
                    lo,
                    hi,
                    to_screen: m,
                    start: p,
                    world,
                    peg,
                    layer,
                    group,
                }
            };
            if !st.polygon.is_empty() {
                st.polygon_drag = Some(p);
            } else if let Some((kind, lo, hi, m)) = on_box.filter(|(k, ..)| {
                // A handle always scales or rotates, so Shift can keep the
                // proportions from the press; elsewhere Shift and Ctrl pick.
                *k != Kind::Move
                    || (!shift
                        && !ctrl
                        && (s.easy_drag || hit_selection(s, p, view, items, transform_tool)))
            }) {
                st.drag = Some(start_drag(kind, lo, hi, m, &box_layers, s));
            } else if transform_tool && layer_at(s, p, view, items).is_some() {
                if !shift && !ctrl {
                    // Dragging unselected artwork selects its layer and moves it at once.
                    transform_click(s, p, view, items, false, false);
                    if let Some(b) = transform_box(s, view, items) {
                        st.drag = Some(start_drag(
                            Kind::Move,
                            b.lo,
                            b.hi,
                            b.to_screen,
                            &b.layers,
                            s,
                        ));
                    }
                } else {
                    transform_click(s, p, view, items, shift, ctrl);
                }
            } else if !transform_tool
                && !shift
                && !ctrl
                && pick(s, p, view, items, false, false).is_some()
            {
                // Dragging unselected artwork selects it and moves it at once.
                let world = world_of(s, items);
                let fresh =
                    bounds_of(s.selected_strokes()).map(|(lo, hi)| (lo, hi, view.compose(world)));
                if let Some((lo, hi, m)) = fresh {
                    st.drag = Some(start_drag(Kind::Move, lo, hi, m, &[], s));
                }
            } else {
                let marquee = s.marquee_mode != alt;
                if marquee {
                    st.marquee = Some(p);
                } else {
                    st.lasso = vec![p];
                    st.lasso_active = false;
                }
            }
        }
    }

    // Continue it.
    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(p) = response.interact_pointer_pos() {
            if let Some(d) = &st.drag {
                if let Some(l) = drag_transform(d, p, shift) {
                    if transform_tool && !d.group.is_empty() {
                        apply_group_drag(s, d, l);
                    } else if transform_tool {
                        apply_peg_drag(s, d, l);
                    } else {
                        st.preview = Some(l);
                    }
                }
            } else if let Some(start) = st.lasso.first() {
                st.lasso_active |= start.distance(p) >= LASSO_THRESHOLD;
                if st.lasso_active && st.lasso.last().is_some_and(|q| q.distance(p) > 2.0) {
                    st.lasso.push(p);
                }
            }
        }
    }

    // Finish it.
    if response.drag_stopped_by(egui::PointerButton::Primary) {
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
        if let Some(at) = st.polygon_drag.take() {
            let end = response.interact_pointer_pos().unwrap_or(at);
            if st.polygon.len() >= 3
                && at.distance(st.polygon[0].at) <= CLOSE_RADIUS
                && at.distance(end) < LASSO_THRESHOLD
            {
                finish_polygon = true;
            } else {
                st.polygon.push(Anchor {
                    at,
                    handle: if at.distance(end) >= LASSO_THRESHOLD {
                        end - at
                    } else {
                        Vec2::ZERO
                    },
                });
            }
        }
        let area: Option<Vec<[f32; 2]>> = if let Some(a) = st.marquee.take() {
            let b = response.interact_pointer_pos().unwrap_or(a);
            let r = Rect::from_two_pos(a, b);
            Some(vec![
                [r.min.x, r.min.y],
                [r.max.x, r.min.y],
                [r.max.x, r.max.y],
                [r.min.x, r.max.y],
            ])
        } else if std::mem::take(&mut st.lasso_active) {
            let mut points = std::mem::take(&mut st.lasso);
            if let Some(end) = response.interact_pointer_pos() {
                points.push(end);
            }
            (points.len() >= 3).then(|| points.into_iter().map(|p| [p.x, p.y]).collect())
        } else {
            if let Some(at) = st.lasso.first().copied() {
                if transform_tool {
                    transform_click(s, at, view, items, shift, ctrl);
                } else if pick(s, at, view, items, shift, ctrl).is_none() {
                    st.polygon.push(Anchor {
                        at,
                        handle: Vec2::ZERO,
                    });
                }
            }
            st.lasso.clear();
            None
        };
        if let Some(poly) = area {
            if transform_tool {
                select_layers_in(s, &poly, view, items, shift, ctrl);
            } else {
                select_area(s, &poly, view, items, shift, ctrl);
            }
        }
    }

    if response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            if transform_tool {
                transform_click(s, p, view, items, shift, ctrl);
            } else if st.polygon.len() >= 3 && p.distance(st.polygon[0].at) <= CLOSE_RADIUS {
                finish_polygon = true;
            } else if !st.polygon.is_empty() || pick(s, p, view, items, shift, ctrl).is_none() {
                st.polygon.push(Anchor {
                    at: p,
                    handle: Vec2::ZERO,
                });
            }
        }
    }
    if !st.polygon.is_empty() {
        st.polygon_owner = Some((s.key.clone(), drawing, s.art));
    }
    if finish_polygon && st.polygon.len() >= 3 {
        let anchors = std::mem::take(&mut st.polygon);
        let points: Vec<_> = outline(&anchors, true)
            .into_iter()
            .map(|p| [p.x, p.y])
            .collect();
        select_area(s, &points, view, items, shift, ctrl);
    }
    if !st.polygon.is_empty() {
        let mut anchors = st.polygon.clone();
        if let Some(at) = st.polygon_drag {
            anchors.push(Anchor {
                at,
                handle: pointer.unwrap_or(at) - at,
            });
        } else if let Some(at) = response.hover_pos() {
            anchors.push(Anchor {
                at,
                handle: Vec2::ZERO,
            });
        }
        let line = outline(&anchors, false);
        if line.len() > 1 {
            painter.add(egui::Shape::line(
                line,
                EStroke::new(1.2, theme::ACCENT_SELECT),
            ));
        }
        for anchor in &st.polygon {
            painter.circle_filled(anchor.at, 3.5, theme::ACCENT_SELECT);
        }
        if let (Some(at), Some(end)) = (st.polygon_drag, pointer) {
            painter.line_segment(
                [at - (end - at), end],
                EStroke::new(1.0, theme::ACCENT_SELECT),
            );
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
        if let (true, [layer]) = (transform_tool, box_layers.as_slice()) {
            if let Some(p) = s.transform_peg(*layer) {
                // The pivot is amber, because it belongs to animation (DESIGN §4.3).
                let world = items
                    .iter()
                    .find(|(l, _, _)| l == layer)
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

fn current_peg(s: &Session, layer: usize) -> PegValues {
    let p = s.transform_peg(layer).cloned().unwrap_or_default();
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
    let layer = d.layer;
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
            let (dx, dy) = s
                .transform_peg_id(layer)
                .and_then(|id| s.doc.graph.world_transform(id, s.frame).ok())
                .and_then(|parent| {
                    Affine {
                        tx: 0.0,
                        ty: 0.0,
                        ..parent
                    }
                    .inverse()
                })
                .map_or((dx, dy), |parent| parent.apply(dx, dy));
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

/// Turn a scene-space drag of several layers into each layer's peg values:
/// each pivot moves with the box, and each layer turns or scales by the
/// box's amount around its own pivot.
fn apply_group_drag(s: &mut Session, d: &Drag, g: Affine) {
    let (turn, fx, fy) = match d.kind {
        Kind::Move => (0.0, 1.0, 1.0),
        Kind::Rotate => (g.b.atan2(g.a).to_degrees(), 1.0, 1.0),
        Kind::Scale(_) => (0.0, g.a, g.d),
    };
    for start in &d.group {
        let (px, py) = start.pivot;
        let (nx, ny) = g.apply(px, py);
        let linear = Affine {
            tx: 0.0,
            ty: 0.0,
            ..start.parent
        };
        let (dx, dy) = linear
            .inverse()
            .map_or((nx - px, ny - py), |inv| inv.apply(nx - px, ny - py));
        let v0 = start.peg;
        s.adjust_group_peg(start.layer, |v| {
            v.x = v0.x + dx;
            v.y = v0.y + dy;
            v.rotation = v0.rotation + turn;
            v.scale_x = v0.scale_x * fx;
            v.scale_y = v0.scale_y * fy;
        });
    }
}

/// A layer's peg and pivot at the start of a group drag.
fn group_start(s: &Session, layer: usize) -> Option<GroupStart> {
    let peg = s.transform_peg(layer)?;
    let id = s.transform_peg_id(layer)?;
    let node = s.doc.layers.get(layer)?.node;
    let depth = s.doc.graph.world_pose(node, s.frame).ok()?.depth;
    let parent = s
        .doc
        .project_plane(s.frame, depth)?
        .compose(s.doc.graph.world_transform(id, s.frame).ok()?);
    let v = current_peg(s, layer);
    let (px, py) = peg.pivot;
    Some(GroupStart {
        layer,
        peg: v,
        parent,
        pivot: parent.apply(v.x + px, v.y + py),
    })
}

/// The Transform tool's layers, as indices, front to back as listed.
fn transform_indices(s: &Session) -> Vec<usize> {
    s.transform_layers
        .iter()
        .filter_map(|n| s.doc.layers.iter().position(|l| l.node == *n))
        .collect()
}

/// When the current layer changed outside the Transform tool (the
/// Timeline, a new layer), the tool works on that layer alone. Drops
/// layers that no longer exist.
fn follow_current_layer(s: &mut Session) {
    let current = s.doc.layers.get(s.selected).map(|l| l.node);
    if s.transform_for != current {
        s.transform_layers = current.into_iter().collect();
        s.transform_for = current;
    }
    let nodes: Vec<NodeId> = s.doc.layers.iter().map(|l| l.node).collect();
    s.transform_layers.retain(|n| nodes.contains(n));
}

/// Set the Transform tool's layers. The current layer stays if it is in
/// the list, and otherwise becomes the last one listed.
fn set_transform_layers(s: &mut Session, layers: Vec<NodeId>) {
    let current = s.doc.layers.get(s.selected).map(|l| l.node);
    if !current.is_some_and(|c| layers.contains(&c)) {
        if let Some(i) = layers
            .last()
            .and_then(|n| s.doc.layers.iter().position(|l| l.node == *n))
        {
            s.selected = i;
        }
    }
    s.transform_layers = layers;
    s.transform_for = s.doc.layers.get(s.selected).map(|l| l.node);
}

/// A Transform tool click: artwork picks its layer (Shift adds it,
/// Control removes it); a click where no layer has artwork clears the
/// list.
fn transform_click(
    s: &mut Session,
    p: Pos2,
    view: Affine,
    items: &[Item],
    add: bool,
    remove: bool,
) {
    let Some(layer) = layer_at(s, p, view, items) else {
        if !add && !remove {
            set_transform_layers(s, Vec::new());
        }
        return;
    };
    let Some(node) = s.doc.layers.get(layer).map(|l| l.node) else {
        return;
    };
    let mut layers = s.transform_layers.clone();
    if remove {
        layers.retain(|n| *n != node);
    } else {
        if !add {
            layers.clear();
        }
        if !layers.contains(&node) {
            layers.push(node);
        }
        s.selected = layer;
    }
    set_transform_layers(s, layers);
}

/// Transform tool lasso or marquee (screen): pick the layers with a
/// stroke or an image whose middle falls inside `poly`, as the Select
/// tool picks strokes.
fn select_layers_in(
    s: &mut Session,
    poly: &[[f32; 2]],
    view: Affine,
    items: &[Item],
    add: bool,
    remove: bool,
) {
    let inside = |m: Affine, (lo, hi): ([f32; 2], [f32; 2])| {
        let (x, y) = m.apply((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
        inside_polygon(x, y, poly)
    };
    let mut found: Vec<NodeId> = Vec::new();
    for (layer, d, world) in items {
        let Some(dr) = s.doc.drawings.get(d) else {
            continue;
        };
        let m = view.compose(*world);
        let hit = image_rect(s, dr).is_some_and(|r| inside(m, r))
            || dr
                .strokes
                .iter()
                .filter_map(symphony_vector::bounds)
                .any(|r| inside(m, r));
        if let Some(n) = s.doc.layers.get(*layer).map(|l| l.node) {
            if hit && !found.contains(&n) {
                found.push(n);
            }
        }
    }
    let mut layers = if add || remove {
        s.transform_layers.clone()
    } else {
        Vec::new()
    };
    if remove {
        layers.retain(|n| !found.contains(n));
    } else {
        for n in found {
            if !layers.contains(&n) {
                layers.push(n);
            }
        }
    }
    set_transform_layers(s, layers);
}

/// The Transform tool's box around its layers drawn at this frame.
fn transform_box(s: &Session, view: Affine, items: &[Item]) -> Option<TransformBox> {
    let layers = transform_indices(s);
    let mine: Vec<&Item> = items.iter().filter(|(l, ..)| layers.contains(l)).collect();
    if let [(layer, d, world)] = mine.as_slice() {
        let (lo, hi) = drawing_bounds(s, s.doc.drawings.get(d)?)?;
        return Some(TransformBox {
            lo,
            hi,
            to_screen: view.compose(*world),
            layers: vec![*layer],
        });
    }
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    let mut found = Vec::new();
    for (layer, d, world) in mine {
        let Some((a, b)) = s.doc.drawings.get(d).and_then(|dr| drawing_bounds(s, dr)) else {
            continue;
        };
        for (x, y) in [(a[0], a[1]), (b[0], a[1]), (b[0], b[1]), (a[0], b[1])] {
            let (x, y) = world.apply(x, y);
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
        }
        found.push(*layer);
    }
    (!found.is_empty()).then_some(TransformBox {
        lo,
        hi,
        to_screen: view,
        layers: found,
    })
}

/// Is `p` on the current selection's artwork?
fn hit_selection(s: &Session, p: Pos2, view: Affine, items: &[Item], transform_tool: bool) -> bool {
    if transform_tool {
        let layers = transform_indices(s);
        return layer_at(s, p, view, items).is_some_and(|l| layers.contains(&l));
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
        let on_image = image_rect(s, dr)
            .is_some_and(|(lo, hi)| (lo[0]..=hi[0]).contains(&x) && (lo[1]..=hi[1]).contains(&y));
        (on_image || dr.strokes.iter().any(|st| hits(st, x, y, r))).then_some(*layer)
    })
}

/// A drawing's imported image, as drawing-local bounds.
fn image_rect(s: &Session, dr: &Drawing) -> Option<([f32; 2], [f32; 2])> {
    let r = dr.image?;
    let [x0, y0, x1, y1] = r.rect(s.doc.images.get(&r.key)?, &s.doc.settings);
    Some(([x0.min(x1), y0.min(y1)], [x0.max(x1), y0.max(y1)]))
}

/// The Transform tool's box for a drawing: its strokes and its image.
fn drawing_bounds(s: &Session, dr: &Drawing) -> Option<([f32; 2], [f32; 2])> {
    match (bounds_of(&dr.strokes), image_rect(s, dr)) {
        (Some((a0, a1)), Some((b0, b1))) => Some((
            [a0[0].min(b0[0]), a0[1].min(b0[1])],
            [a1[0].max(b1[0]), a1[1].max(b1[1])],
        )),
        (a, b) => a.or(b),
    }
}

/// Select tool click picking: a stroke in the current drawing (Shift adds,
/// Control subtracts).
fn pick(
    s: &mut Session,
    p: Pos2,
    view: Affine,
    items: &[Item],
    add: bool,
    remove: bool,
) -> Option<usize> {
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
    if remove {
        s.selection.strokes.retain(|id| *id != hit);
    } else if add {
        if !s.selection.strokes.contains(&hit) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    #[test]
    fn short_gesture_places_an_anchor_and_next_drag_bends_the_outline() {
        let mut session = Session::new_scene();
        session.tool = Tool::Select;
        let mut harness = Harness::builder()
            .with_size(Vec2::new(500.0, 400.0))
            .build_ui_state(
                |ui, (session, state)| {
                    let (response, painter) =
                        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                    super::ui(
                        ui,
                        &response,
                        &painter,
                        session,
                        state,
                        Affine::IDENTITY,
                        &[],
                    );
                },
                (session, SelectState::default()),
            );
        let first = Pos2::new(100.0, 100.0);
        let next = Pos2::new(180.0, 100.0);
        let handle = Vec2::new(30.0, 35.0);
        let mut pointer = |pos, pressed| {
            harness
                .input_mut()
                .events
                .push(egui::Event::PointerMoved(pos));
            if let Some(pressed) = pressed {
                harness.input_mut().events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            harness.run_steps(1);
        };
        pointer(first, Some(true));
        pointer(first + Vec2::new(4.0, 1.0), None);
        pointer(first + Vec2::new(4.0, 1.0), Some(false));
        pointer(next, Some(true));
        pointer(next + handle, None);
        pointer(next + handle, Some(false));
        let anchors = &harness.state().1.polygon;
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].handle, Vec2::ZERO);
        assert_eq!(anchors[1].at, next);
        assert_eq!(anchors[1].handle, handle);
        assert!(outline(anchors, false).iter().any(|point| point.y < 95.0));
    }

    #[test]
    fn the_transform_tool_picks_and_moves_an_image_layer() {
        let mut session = Session::new_scene();
        let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let file = symphony_io::media::image_from_bytes(bytes).unwrap();
        session.import_images("Red", vec![file]);
        let image_layer = session.selected;
        let d = session.doc.exposed(image_layer, 1).unwrap();
        // Start on the other layer: a click on the image must pick it.
        session.selected = image_layer + 1;
        session.tool = Tool::Transform;
        let x0 = session
            .transform_peg(image_layer)
            .map_or(0.0, |p| p.x.at(1));
        // Scene to screen: a fifth of the size, centred in the view.
        let view = Affine::translate(250.0, 200.0).compose(Affine::scale(0.2, 0.2));
        let items = vec![(image_layer, d, Affine::IDENTITY)];
        let mut harness = Harness::builder()
            .with_size(Vec2::new(500.0, 400.0))
            .build_ui_state(
                move |ui, (session, state)| {
                    let (response, painter) =
                        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                    super::ui(ui, &response, &painter, session, state, view, &items);
                },
                (session, SelectState::default()),
            );
        let mut pointer = |pos: Pos2, pressed: Option<bool>| {
            harness
                .input_mut()
                .events
                .push(egui::Event::PointerMoved(pos));
            if let Some(pressed) = pressed {
                harness.input_mut().events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            harness.run_steps(1);
        };
        // Inside the image, away from its strokes (it has none).
        let start = Pos2::new(240.0, 190.0);
        pointer(start, Some(true));
        for i in 1..=5 {
            pointer(start + Vec2::new(10.0 * i as f32, 0.0), None);
        }
        pointer(start + Vec2::new(50.0, 0.0), Some(false));
        let s = &harness.state().0;
        assert_eq!(s.selected, image_layer);
        let x1 = s.transform_peg(image_layer).map_or(0.0, |p| p.x.at(1));
        // 50 screen points at a fifth of the size: 250 scene units.
        assert!((x1 - x0 - 250.0).abs() < 1.0, "moved {}", x1 - x0);
    }

    type SelectHarness<'a> = Harness<'a, (Session, SelectState)>;

    fn pointer(
        h: &mut SelectHarness,
        pos: Pos2,
        pressed: Option<bool>,
        modifiers: egui::Modifiers,
    ) {
        pointer_by(h, pos, egui::PointerButton::Primary, pressed, modifiers);
    }

    fn pointer_by(
        h: &mut SelectHarness,
        pos: Pos2,
        button: egui::PointerButton,
        pressed: Option<bool>,
        modifiers: egui::Modifiers,
    ) {
        h.input_mut()
            .events
            .push(egui::Event::ModifiersChanged(modifiers));
        h.input_mut().events.push(egui::Event::PointerMoved(pos));
        if let Some(pressed) = pressed {
            h.input_mut().events.push(egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            });
        }
        h.run_steps(1);
    }

    /// A session with one 64 x 32 image layer, selected, and the
    /// Transform tool; the view shows the scene at a fifth of its size.
    fn image_harness<'a>() -> (SelectHarness<'a>, usize, Affine) {
        let mut session = Session::new_scene();
        let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let file = symphony_io::media::image_from_bytes(bytes).unwrap();
        session.import_images("Red", vec![file]);
        let layer = session.selected;
        let d = session.doc.exposed(layer, 1).unwrap();
        session.tool = Tool::Transform;
        let view = Affine::translate(250.0, 200.0).compose(Affine::scale(0.2, 0.2));
        let items = vec![(layer, d, Affine::IDENTITY)];
        let h = Harness::builder()
            .with_size(Vec2::new(500.0, 400.0))
            .build_ui_state(
                move |ui, (session, state)| {
                    let (response, painter) =
                        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                    super::ui(ui, &response, &painter, session, state, view, &items);
                },
                (session, SelectState::default()),
            );
        (h, layer, view)
    }

    #[test]
    fn shift_on_a_corner_handle_scales_in_proportion() {
        let (mut h, layer, view) = image_harness();
        let none = egui::Modifiers::NONE;
        // Pick the layer, then find its bottom-right corner handle.
        click(&mut h, Pos2::new(250.0, 200.0), none);
        assert_eq!(transform_indices(&h.state().0), vec![layer]);
        let d = h.state().0.doc.exposed(layer, 1).unwrap();
        let b = transform_box(&h.state().0, view, &[(layer, d, Affine::IDENTITY)]).unwrap();
        let (cx, cy) = b.to_screen.apply(b.hi[0], b.hi[1]);
        let corner = Pos2::new(cx, cy);
        assert!(matches!(
            grab(corner, b.lo, b.hi, b.to_screen),
            Some(Kind::Scale(_))
        ));
        // Shift held from the press: a wide, flat drag still scales evenly.
        let shift = egui::Modifiers::SHIFT;
        pointer(&mut h, corner, Some(true), shift);
        for i in 1..=5 {
            let step = Vec2::new(12.0, 2.0) * i as f32;
            pointer(&mut h, corner + step, None, shift);
        }
        pointer(&mut h, corner + Vec2::new(60.0, 10.0), Some(false), shift);
        let s = &h.state().0;
        assert_eq!(transform_indices(s), vec![layer], "Shift picked instead");
        let peg = s.transform_peg(layer).unwrap();
        let sx = peg.scale_x.as_ref().map_or(1.0, |v| v.at(1));
        let sy = peg.scale_y.as_ref().map_or(1.0, |v| v.at(1));
        assert!(sx > 1.05, "no scale: {sx}");
        assert!((sx - sy).abs() < 1e-3, "not in proportion: {sx} x {sy}");
    }

    #[test]
    fn a_middle_drag_does_not_move_the_selection() {
        let (mut h, layer, _) = image_harness();
        let none = egui::Modifiers::NONE;
        let x = |h: &SelectHarness| h.state().0.transform_peg(layer).map_or(0.0, |p| p.x.at(1));
        click(&mut h, Pos2::new(250.0, 200.0), none);
        let x0 = x(&h);
        let middle = egui::PointerButton::Middle;
        let start = Pos2::new(245.0, 195.0);
        pointer_by(&mut h, start, middle, Some(true), none);
        for i in 1..=5 {
            let at = start + Vec2::new(10.0 * i as f32, 0.0);
            pointer_by(&mut h, at, middle, None, none);
        }
        pointer_by(
            &mut h,
            start + Vec2::new(50.0, 0.0),
            middle,
            Some(false),
            none,
        );
        assert_eq!(x(&h), x0);
        assert!(h.state().1.drag.is_none());
        // The same drag with the primary button moves it.
        pointer(&mut h, start, Some(true), none);
        for i in 1..=5 {
            pointer(&mut h, start + Vec2::new(10.0 * i as f32, 0.0), None, none);
        }
        pointer(&mut h, start + Vec2::new(50.0, 0.0), Some(false), none);
        assert!((x(&h) - x0 - 250.0).abs() < 1.0, "moved {}", x(&h) - x0);
    }

    fn click(h: &mut SelectHarness, at: Pos2, m: egui::Modifiers) {
        pointer(h, at, Some(true), m);
        pointer(h, at, Some(false), m);
        pointer(h, at, None, egui::Modifiers::NONE);
    }

    /// Two layers, each with one short horizontal line: layer 0 at
    /// x 400..500 and layer 1 at x -500..-400, both at y 0.
    fn two_layers() -> (Session, Vec<Item>) {
        let mut s = Session::new_scene();
        s.add_layer();
        for (layer, x) in [(0, 400.0), (1, -500.0)] {
            let stroke = symphony_core::stroke::Stroke {
                id: s.doc.ids.fresh(),
                kind: symphony_core::stroke::StrokeKind::Pencil,
                art: Default::default(),
                swatch: s.doc.palette.swatches[0].id,
                min_width: 8.0,
                max_width: 8.0,
                points: [x, x + 50.0, x + 100.0]
                    .map(|x| symphony_core::stroke::Point {
                        x,
                        y: 0.0,
                        pressure: 1.0,
                    })
                    .to_vec(),
                tips: Default::default(),
                texture: None,
                shape: Vec::new(),
            };
            s.commit_stroke(layer, 1, stroke);
        }
        let items = [1, 0]
            .map(|l| (l, s.doc.exposed(l, 1).unwrap(), Affine::IDENTITY))
            .to_vec();
        (s, items)
    }

    #[test]
    fn transform_clicks_add_remove_and_clear_layers() {
        let (mut session, items) = two_layers();
        session.tool = Tool::Transform;
        session.marquee_mode = true;
        session.selected = 0;
        // Scene to screen: a fifth of the size, centred. Layer 0's line
        // is at screen x 330..350, layer 1's at 150..170, both at y 200.
        let view = Affine::translate(250.0, 200.0).compose(Affine::scale(0.2, 0.2));
        let mut h = Harness::builder()
            .with_size(Vec2::new(500.0, 400.0))
            .build_ui_state(
                move |ui, (session, state)| {
                    let (response, painter) =
                        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                    super::ui(ui, &response, &painter, session, state, view, &items);
                },
                (session, SelectState::default()),
            );
        let layers = |h: &Harness<'_, (Session, SelectState)>| {
            let mut l = transform_indices(&h.state().0);
            l.sort();
            l
        };
        let (a, b) = (Pos2::new(160.0, 200.0), Pos2::new(340.0, 200.0));
        let none = egui::Modifiers::NONE;

        click(&mut h, a, none);
        assert_eq!(layers(&h), vec![1]);
        assert_eq!(h.state().0.selected, 1);
        click(&mut h, b, egui::Modifiers::SHIFT);
        assert_eq!(layers(&h), vec![0, 1]);
        assert_eq!(h.state().0.selected, 0);

        // Drag one of them: both move, as one undo step.
        pointer(&mut h, a, Some(true), none);
        for i in 1..=5 {
            pointer(&mut h, a + Vec2::new(10.0 * i as f32, 0.0), None, none);
        }
        pointer(&mut h, a + Vec2::new(50.0, 0.0), Some(false), none);
        let x = |h: &Harness<'_, (Session, SelectState)>, l: usize| {
            h.state().0.transform_peg(l).map_or(0.0, |p| p.x.at(1))
        };
        assert!((x(&h, 0) - 250.0).abs() < 1.0, "layer 0 moved {}", x(&h, 0));
        assert!((x(&h, 1) - 250.0).abs() < 1.0, "layer 1 moved {}", x(&h, 1));
        h.state_mut().0.undo();
        assert_eq!((x(&h, 0), x(&h, 1)), (0.0, 0.0));
        assert_eq!(layers(&h), vec![0, 1]);

        click(&mut h, a, egui::Modifiers::CTRL);
        assert_eq!(layers(&h), vec![0]);
        // A blank click clears the list and keeps the current layer.
        click(&mut h, Pos2::new(250.0, 350.0), none);
        assert_eq!(layers(&h), Vec::<usize>::new());
        assert_eq!(h.state().0.selected, 0);

        // A marquee from blank over both lines picks both layers.
        let start = Pos2::new(100.0, 150.0);
        pointer(&mut h, start, Some(true), none);
        for i in 1..=5 {
            pointer(&mut h, start + Vec2::new(60.0, 20.0) * i as f32, None, none);
        }
        pointer(&mut h, Pos2::new(400.0, 250.0), Some(false), none);
        assert_eq!(layers(&h), vec![0, 1]);
    }

    #[test]
    fn a_group_drag_moves_each_layer_with_the_box() {
        let (mut s, _) = two_layers();
        // Layer 1 already turned and scaled on its own, around its middle.
        s.ensure_pivot(1, (-450.0, 0.0));
        s.adjust_peg(1, |v| {
            v.rotation = 30.0;
            v.scale_x = 2.0;
            v.scale_y = 2.0;
            v.x = 10.0;
        });
        s.end_gesture();
        let world = |s: &Session, l: usize| {
            let node = s.doc.layers[l].node;
            let depth = s.doc.graph.world_pose(node, 1).unwrap().depth;
            s.doc
                .project_plane(1, depth)
                .unwrap()
                .compose(s.doc.graph.world_transform(node, 1).unwrap())
        };
        let around = |g: Affine| {
            Affine::translate(30.0, -20.0)
                .compose(g)
                .compose(Affine::translate(-30.0, 20.0))
        };
        for (kind, g) in [
            (Kind::Rotate, around(Affine::rotate_deg(90.0))),
            (Kind::Scale(2), around(Affine::scale(1.5, 1.5))),
            (Kind::Move, Affine::translate(70.0, -40.0)),
        ] {
            let before = [world(&s, 0), world(&s, 1)];
            let drag = Drag {
                kind,
                lo: [0.0; 2],
                hi: [0.0; 2],
                to_screen: Affine::IDENTITY,
                start: Pos2::ZERO,
                world: Affine::IDENTITY,
                peg: None,
                layer: 0,
                group: [0, 1].iter().filter_map(|&l| group_start(&s, l)).collect(),
            };
            apply_group_drag(&mut s, &drag, g);
            s.end_gesture();
            for (l, b) in before.iter().enumerate() {
                let want = g.compose(*b);
                let got = world(&s, l);
                for (x, y) in [(400.0, 0.0), (-500.0, 0.0), (0.0, 100.0)] {
                    let (w, h) = (want.apply(x, y), got.apply(x, y));
                    assert!(
                        (w.0 - h.0).abs() < 0.01 && (w.1 - h.1).abs() < 0.01,
                        "{kind:?}, layer {l}: want {w:?}, got {h:?}"
                    );
                }
            }
        }
    }
}
