//! Function view (Harmony's curve editor): the selected layer's peg
//! functions, or the selected bone's angle, as curves over time. Zoom and pan, box select and drag
//! keys, shape Bézier segments with handles, and add or delete keys. The
//! selected keys' settings show in Layer Properties (`key_properties`).

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use symphony_core::anim::{Continuity, Interp, Value};
use symphony_core::graph::{NodeId, NodeKind, PegParams};
use symphony_core::history::Edit;
use symphony_ui::{theme, widgets};

use crate::session::Session;

/// The peg functions, in Harmony's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    X,
    Y,
    Rotation,
    ScaleX,
    ScaleY,
    /// A Control's position (kept in the `x` slot).
    Position,
}

impl Param {
    pub const ALL: [Param; 5] = [
        Param::X,
        Param::Y,
        Param::Rotation,
        Param::ScaleX,
        Param::ScaleY,
    ];

    fn name(self) -> &'static str {
        match self {
            Param::X => "Position X",
            Param::Y => "Position Y",
            Param::Rotation => "Angle",
            Param::ScaleX => "Scale X",
            Param::ScaleY => "Scale Y",
            Param::Position => "Control position",
        }
    }

    fn colour(self) -> Color32 {
        match self {
            Param::X | Param::Position => theme::FUNCTION_X,
            Param::Y => theme::FUNCTION_Y,
            Param::Rotation => theme::FUNCTION_ANGLE,
            Param::ScaleX => theme::FUNCTION_SCALE_X,
            Param::ScaleY => theme::FUNCTION_SCALE_Y,
        }
    }

    /// The function, if the peg has one (scale is absent until used).
    fn get(self, p: &PegParams) -> Option<&Value> {
        match self {
            Param::X | Param::Position => Some(&p.x),
            Param::Y => Some(&p.y),
            Param::Rotation => Some(&p.rotation),
            Param::ScaleX => p.scale_x.as_ref(),
            Param::ScaleY => p.scale_y.as_ref(),
        }
    }

    fn get_mut(self, p: &mut PegParams) -> &mut Value {
        match self {
            Param::X | Param::Position => &mut p.x,
            Param::Y => &mut p.y,
            Param::Rotation => &mut p.rotation,
            Param::ScaleX => p.scale_x.get_or_insert(Value::Constant(1.0)),
            Param::ScaleY => p.scale_y.get_or_insert(Value::Constant(1.0)),
        }
    }

    fn rest(self) -> f32 {
        match self {
            Param::ScaleX | Param::ScaleY => 1.0,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Key,
    LeftHandle,
    RightHandle,
}

/// A key: its function and frame.
type KeyId = (Param, u32);

/// Frames and values in sight.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    f0: f32,
    f1: f32,
    v0: f32,
    v1: f32,
}

impl Span {
    /// Zoom time by `k` (below 1 zooms in) keeping frame `at` in place.
    fn zoom_frames(self, at: f32, k: f32, most: f32) -> Span {
        let width = ((self.f1 - self.f0) * k).clamp(4.0, most);
        let t = (at - self.f0) / (self.f1 - self.f0);
        let f0 = at - t * width;
        Span {
            f0,
            f1: f0 + width,
            ..self
        }
    }

    /// Zoom values by `k` keeping value `at` in place.
    fn zoom_values(self, at: f32, k: f32) -> Span {
        let height = ((self.v1 - self.v0) * k).max(1e-3);
        let t = (at - self.v0) / (self.v1 - self.v0);
        let v0 = at - t * height;
        Span {
            v0,
            v1: v0 + height,
            ..self
        }
    }

    /// Slide by `delta` screen points in a `rect` (the art follows).
    fn pan(self, delta: Vec2, rect: Rect) -> Span {
        let df = -delta.x / rect.width() * (self.f1 - self.f0);
        let dv = delta.y / rect.height() * (self.v1 - self.v0);
        Span {
            f0: self.f0 + df,
            f1: self.f1 + df,
            v0: self.v0 + dv,
            v1: self.v1 + dv,
        }
    }
}

/// What a press in the graph turned into.
enum Drag {
    /// The selected keys, moved together from the peg as it was at the
    /// press.
    Keys {
        start: Box<PegParams>,
        from: Pos2,
        keys: Vec<KeyId>,
    },
    Handle(KeyId, Grab),
    /// A box select from this corner.
    Box(Pos2),
    /// The frame scale moves the playhead.
    Scrub,
}

pub struct FunctionState {
    shown: [bool; 5],
    /// The functions the shown node has (a bone has only its angle).
    params: &'static [Param],
    /// Selected keys.
    selected: Vec<KeyId>,
    /// The key whose handles show: the last one picked.
    active: Option<KeyId>,
    drag: Option<Drag>,
    /// What's in sight; `None` fits the whole scene.
    view: Option<Span>,
}

impl Default for FunctionState {
    fn default() -> Self {
        FunctionState {
            shown: [true; 5],
            params: &Param::ALL,
            selected: Vec::new(),
            active: None,
            drag: None,
            view: None,
        }
    }
}

const LIST_W: f32 = 148.0;
const KEY_R: f32 = 4.0;
const HANDLE_R: f32 = 3.5;
/// The frame scale above the graph, where the Timeline has its ruler.
const SCALE_H: f32 = 18.0;

/// Whose functions the view shows: the selected bone's or Control's,
/// or else the selected layer's peg.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    Layer(usize),
    Bone(NodeId),
    Control(NodeId),
}

impl Source {
    fn params(self) -> &'static [Param] {
        match self {
            Source::Layer(_) => &Param::ALL,
            Source::Bone(_) => &[Param::Rotation],
            Source::Control(_) => &[Param::Position],
        }
    }
}

/// The functions to show, as peg values: a bone's angle goes in
/// `rotation`, a Control's position in `x`, and the rest stay unused.
fn source(s: &Session) -> Option<(Source, PegParams)> {
    if let Some((id, control)) = s
        .selected_node
        .and_then(|id| match &s.doc.graph.node(id)?.kind {
            NodeKind::Control(control) => Some((id, control)),
            _ => None,
        })
    {
        let values = PegParams {
            x: control.value.clone(),
            ..PegParams::default()
        };
        return Some((Source::Control(id), values));
    }
    if let Some((id, bone)) = s
        .selected_node
        .and_then(|id| Some((id, s.doc.graph.bone(id)?)))
    {
        let values = PegParams {
            rotation: bone.rotation.clone(),
            ..PegParams::default()
        };
        return Some((Source::Bone(id), values));
    }
    let layer = s.selected;
    Some((Source::Layer(layer), s.doc.peg(layer)?.clone()))
}

/// Write edited functions back to where they came from.
fn store(s: &mut Session, source: Source, values: PegParams) {
    match source {
        Source::Layer(layer) => s.set_peg(layer, values),
        Source::Control(id) => {
            let Some(before) = s.doc.graph.node(id).cloned() else {
                return;
            };
            let mut after = before.clone();
            let NodeKind::Control(control) = &mut after.kind else {
                return;
            };
            control.value = values.x;
            if after != before {
                s.history.apply_merged(
                    &mut s.doc,
                    "Animate control",
                    &format!("control-{}", id.0),
                    Edit::SetNodeProperties { id, before, after },
                );
            }
        }
        Source::Bone(id) => {
            let Some(before) = s.doc.graph.node(id).cloned() else {
                return;
            };
            let mut after = before.clone();
            let NodeKind::Bone(bone) = &mut after.kind else {
                return;
            };
            bone.rotation = values.rotation;
            if after != before {
                s.history.apply_merged(
                    &mut s.doc,
                    "Animate bone",
                    &format!("bone-{}", id.0),
                    Edit::SetNodeProperties { id, before, after },
                );
            }
        }
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, st: &mut FunctionState) {
    let Some((source, peg)) = source(s) else {
        ui.centered_and_justified(|ui| {
            ui.label("Select a layer to see its functions.");
        });
        return;
    };
    st.params = source.params();
    forget_missing_keys(st, &peg);
    toolbar(ui, s, st, &peg);
    ui.separator();
    let avail = ui.available_size();
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(LIST_W, avail.y),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(LIST_W);
                function_list(ui, st, &peg);
            },
        );
        ui.separator();
        graph(ui, s, st, &peg);
    });
}

/// Drop selected keys that are gone (undo, another view, a new layer).
fn forget_missing_keys(st: &mut FunctionState, peg: &PegParams) {
    let exists = |(p, f): &KeyId| p.get(peg).is_some_and(|v| v.has_key(*f));
    st.selected.retain(exists);
    if !st.active.as_ref().is_some_and(exists) {
        st.active = None;
    }
}

/// Move `keys` by `df` frames and `dv` value units, all at once, from the
/// peg `start`. `None` if a key would land before frame 1 or on a key
/// that isn't moving.
fn move_keys(start: &PegParams, keys: &[KeyId], df: i64, dv: f32) -> Option<PegParams> {
    let mut p = start.clone();
    for param in Param::ALL.into_iter().chain([Param::Position]) {
        let frames: Vec<u32> = keys
            .iter()
            .filter(|(q, _)| *q == param)
            .map(|(_, f)| *f)
            .collect();
        if frames.is_empty() {
            continue;
        }
        let Value::Keys(list) = param.get_mut(&mut p) else {
            continue;
        };
        let (mut moving, staying): (Vec<_>, Vec<_>) =
            list.drain(..).partition(|k| frames.contains(&k.frame));
        for k in &mut moving {
            let to = i64::from(k.frame) + df;
            if to < 1 || staying.iter().any(|s| i64::from(s.frame) == to) {
                return None;
            }
            k.frame = to as u32;
            k.value += dv;
        }
        list.extend(staying);
        list.extend(moving);
        list.sort_by_key(|k| k.frame);
    }
    Some(p)
}

/// The keys' info and segment settings, for the Layer Properties panel.
/// Shows nothing while no key is selected in the Function view.
pub fn key_properties(ui: &mut egui::Ui, s: &mut Session, st: &mut FunctionState) {
    let Some((layer, peg)) = source(s) else {
        return;
    };
    forget_missing_keys(st, &peg);
    if st.selected.is_empty() {
        return;
    }
    let key_of = |(p, f): KeyId| {
        p.get(&peg)
            .and_then(|v| v.keys().iter().find(|k| k.frame == f).copied())
    };
    egui::Frame::new()
        .inner_margin(egui::Margin::same(theme::CARD_PAD))
        .show(ui, |ui| {
            let one = (st.selected.len() == 1).then(|| st.selected[0]);
            let title = match one {
                Some((p, _)) => format!("Key on {}", p.name()),
                None => format!("{} keys", st.selected.len()),
            };
            ui.heading(title);
            let first = st.selected.iter().find_map(|&id| key_of(id));
            let Some(k) = first else { return };
            let mut interp = k.interp;
            let mut cont = k.continuity;
            egui::Grid::new("function-key")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Segment");
                    ui.horizontal(|ui| {
                        for (label, i, tip) in [
                            ("Linear", Interp::Linear, "A straight line to the next key"),
                            ("Bezier", Interp::Bezier, "A curve shaped by handles"),
                            (
                                "Stop-motion",
                                Interp::Stop,
                                "Hold until the next key (Harmony's stop-motion keyframe)",
                            ),
                        ] {
                            ui.selectable_value(&mut interp, i, label)
                                .on_hover_text(tip);
                        }
                    });
                    ui.end_row();
                    if interp == Interp::Bezier {
                        ui.label("Continuity");
                        egui::ComboBox::from_id_salt("continuity")
                            .selected_text(match cont {
                                Continuity::Smooth => "Smooth",
                                Continuity::Straight => "Straight",
                                Continuity::Corner => "Corner",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut cont, Continuity::Smooth, "Smooth");
                                ui.selectable_value(&mut cont, Continuity::Straight, "Straight");
                                ui.selectable_value(&mut cont, Continuity::Corner, "Corner");
                            });
                        ui.end_row();
                    }
                    if interp != k.interp || cont != k.continuity {
                        let mut p = peg.clone();
                        for &(param, f) in &st.selected {
                            if let Some(km) = param.get_mut(&mut p).key_mut(f) {
                                if interp == Interp::Bezier
                                    && km.interp != Interp::Bezier
                                    && km.right == [0.0, 0.0]
                                {
                                    // Start a new curve with gentle ease handles.
                                    km.right = [1.0, 0.0];
                                    km.left = [-1.0, 0.0];
                                }
                                km.interp = interp;
                                km.continuity = cont;
                            }
                        }
                        store(s, layer, p);
                        s.end_gesture();
                    }
                    let Some((param, f)) = one else { return };
                    let mut kf = i64::from(k.frame);
                    let mut kv = k.value;
                    ui.label("Frame");
                    let fr = ui.add(egui::DragValue::new(&mut kf).range(1..=100_000).speed(0.2));
                    ui.end_row();
                    ui.label("Value");
                    let vr = ui.add(egui::DragValue::new(&mut kv).speed(0.1).max_decimals(3));
                    ui.end_row();
                    if fr.changed() || vr.changed() {
                        let mut p = peg.clone();
                        if param.get_mut(&mut p).move_key(f, kf as u32, kv) {
                            st.selected = vec![(param, kf as u32)];
                            st.active = Some((param, kf as u32));
                            store(s, layer, p);
                        }
                    }
                    if fr.drag_stopped() || vr.drag_stopped() || fr.lost_focus() || vr.lost_focus()
                    {
                        s.end_gesture();
                    }
                });
        });
}

fn toolbar(ui: &mut egui::Ui, s: &mut Session, st: &mut FunctionState, peg: &PegParams) {
    let Some((layer, _)) = source(s) else {
        return;
    };
    let frame = s.frame;
    ui.horizontal(|ui| {
        if ui
            .button("Add key")
            .on_hover_text(
                "Add a key at the current frame to the functions of the selected keys, \
                 or to every shown function",
            )
            .clicked()
        {
            let mut p = peg.clone();
            let mut targets: Vec<Param> = st.selected.iter().map(|(q, _)| *q).collect();
            targets.dedup();
            if targets.is_empty() {
                targets = st
                    .params
                    .iter()
                    .copied()
                    .filter(|f| st.shown[*f as usize])
                    .collect();
            }
            for param in targets {
                let v = param.get(&p).map_or(param.rest(), |v| v.at(frame));
                param.get_mut(&mut p).set_key(frame, v);
            }
            store(s, layer, p);
            s.end_gesture();
        }
        if ui
            .add_enabled(!st.selected.is_empty(), egui::Button::new("Delete keys"))
            .on_hover_text("Delete the selected keys")
            .clicked()
        {
            let mut p = peg.clone();
            for &(param, f) in &st.selected {
                param.get_mut(&mut p).remove_key(f);
            }
            store(s, layer, p);
            s.end_gesture();
            st.selected.clear();
            st.active = None;
        }
        ui.separator();
        if ui
            .add_enabled(st.view.is_some(), egui::Button::new("Reset view"))
            .on_hover_text("Show the whole scene and every value again")
            .clicked()
        {
            st.view = None;
        }
        ui.label(
            egui::RichText::new(
                "Wheel zooms time, Ctrl+wheel values, middle drag pans. \
                 Drag on blank to box select.",
            )
            .font(theme::dense())
            .color(theme::TEXT_SECONDARY),
        );
    });
}

fn function_list(ui: &mut egui::Ui, st: &mut FunctionState, peg: &PegParams) {
    egui::ScrollArea::vertical()
        .id_salt("function-list")
        .show(ui, |ui| {
            ui.add_space(4.0);
            for &param in st.params {
                let i = param as usize;
                let keys = param.get(peg).map_or(0, |v| v.keys().len());
                ui.horizontal(|ui| {
                    ui.add_space(theme::CARD_PAD as f32 * 0.5);
                    let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 14.0), Sense::hover());
                    ui.painter().rect_filled(r, 1.0, param.colour());
                    widgets::checkbox(ui, &mut st.shown[i], param.name()).on_hover_text(
                        match keys {
                            0 => "Not animated".to_owned(),
                            1 => "1 key".to_owned(),
                            n => format!("{n} keys"),
                        },
                    );
                    if keys > 0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(keys.to_string())
                                    .font(theme::numbers())
                                    .color(theme::TEXT_SECONDARY),
                            );
                        });
                    }
                });
            }
        });
}

/// Maps frames and values to the graph rectangle.
struct Map {
    rect: Rect,
    f0: f32,
    f1: f32,
    v0: f32,
    v1: f32,
}

impl Map {
    fn pos(&self, f: f32, v: f32) -> Pos2 {
        Pos2::new(
            self.rect.left() + (f - self.f0) / (self.f1 - self.f0) * self.rect.width(),
            self.rect.bottom() - (v - self.v0) / (self.v1 - self.v0) * self.rect.height(),
        )
    }
    fn frame(&self, x: f32) -> f32 {
        self.f0 + (x - self.rect.left()) / self.rect.width() * (self.f1 - self.f0)
    }
    fn value(&self, y: f32) -> f32 {
        self.v0 + (self.rect.bottom() - y) / self.rect.height() * (self.v1 - self.v0)
    }
    /// Value units per screen point, vertically.
    fn per_point(&self) -> f32 {
        (self.v1 - self.v0) / self.rect.height()
    }
}

/// The whole scene and every shown value, with some room.
fn fit(peg: &PegParams, shown: &[Param], len: f32) -> Span {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in shown {
        let v = p.get(peg).cloned().unwrap_or(Value::Constant(p.rest()));
        for i in 0..=200 {
            let x = v.at_f(1.0 + (len - 1.0) * i as f32 / 200.0);
            lo = lo.min(x);
            hi = hi.max(x);
        }
        for k in v.keys() {
            for h in [k.left, k.right] {
                lo = lo.min(k.value + h[1]);
                hi = hi.max(k.value + h[1]);
            }
        }
    }
    if lo > hi {
        (lo, hi) = (0.0, 1.0);
    }
    if hi - lo < 1.0 {
        let mid = (hi + lo) * 0.5;
        (lo, hi) = (mid - 0.5, mid + 0.5);
    }
    let pad = (hi - lo) * 0.12;
    Span {
        f0: 1.0,
        f1: len,
        v0: lo - pad,
        v1: hi + pad,
    }
}

/// A round step (1, 2, 5, 10, 20, 50 ...) at least `least` units wide.
fn nice_step(least: f32) -> f32 {
    let mut step = 1e-3_f32;
    loop {
        for m in [1.0, 2.0, 5.0] {
            if step * m >= least {
                return step * m;
            }
        }
        step *= 10.0;
    }
}

/// Wheel zooms time at the pointer, Ctrl+wheel zooms values, a sideways
/// wheel (or Shift+wheel) slides time, and the middle button pans.
fn navigate(
    ui: &egui::Ui,
    resp: &egui::Response,
    st: &mut FunctionState,
    span: Span,
    map: &Map,
    len: f32,
) {
    let mut view = span;
    if let Some(p) = resp.hover_pos().filter(|_| resp.hovered()) {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let most = (len * 4.0).max(40.0);
        if zoom != 1.0 {
            view = view.zoom_values(map.value(p.y), 1.0 / zoom);
        }
        if scroll.y != 0.0 {
            view = view.zoom_frames(map.frame(p.x), (-scroll.y * 0.0025).exp(), most);
        }
        if scroll.x != 0.0 {
            view = view.pan(Vec2::new(scroll.x, 0.0), map.rect);
        }
    }
    if view != span {
        st.view = Some(view);
    }
}

fn graph(ui: &mut egui::Ui, s: &mut Session, st: &mut FunctionState, peg: &PegParams) {
    let Some((layer, _)) = source(s) else {
        return;
    };
    let size = ui.available_size().max(Vec2::new(120.0, 80.0));
    let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let (resp, pan) = widgets::middle_pan(ui, resp);
    let full = resp.rect;
    theme::feather_panel(&painter, full, theme::BG_INPUT, 5.0);
    let rect = Rect::from_min_max(
        full.min + Vec2::new(50.0, SCALE_H + 8.0),
        full.max - Vec2::new(14.0, 10.0),
    );
    let len = s.length().max(2) as f32;
    let shown: Vec<Param> = st
        .params
        .iter()
        .copied()
        .filter(|p| st.shown[*p as usize])
        .collect();
    let mut span = st.view.unwrap_or_else(|| fit(peg, &shown, len));
    if let Some(delta) = pan {
        span = span.pan(delta, rect);
        st.view = Some(span);
    }
    let map = Map {
        rect,
        f0: span.f0,
        f1: span.f1,
        v0: span.v0,
        v1: span.v1,
    };
    navigate(ui, &resp, st, span, &map, len);

    paint_grid(&painter, &map, full, len);
    let art = painter.with_clip_rect(rect.expand(KEY_R + 1.0));
    paint_curves(&art, st, peg, &shown, &map);
    // The playhead: a line and the current frame on the frame scale.
    let cx = map.pos(s.frame as f32, 0.0).x;
    if (rect.left()..=rect.right()).contains(&cx) {
        painter.line_segment(
            [Pos2::new(cx, rect.top()), Pos2::new(cx, rect.bottom())],
            Stroke::new(1.5, theme::ACCENT_TIME),
        );
        crate::views::timing::frame_tag(
            &painter,
            Pos2::new(cx, rect.top() - SCALE_H / 2.0 - 2.0),
            s.frame,
        );
    }

    let pointer = resp.interact_pointer_pos();
    let press = ui.input(|i| i.pointer.press_origin()).or(pointer);
    let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
    let last = s.length();
    let frame_at = |x: f32| (map.frame(x).round().max(1.0) as u32).min(last);
    let active = st.active;
    let hit = |pos: Pos2| hit_key(pos, active, peg, &shown, &map);

    if let Some(at) = press.filter(|_| resp.drag_started() || resp.clicked()) {
        if at.y < rect.top() {
            s.frame = frame_at(at.x);
            st.drag = resp.drag_started().then_some(Drag::Scrub);
        } else if let Some((id, grab)) = hit(at) {
            if grab != Grab::Key {
                st.drag = resp.drag_started().then_some(Drag::Handle(id, grab));
            } else if ctrl {
                st.selected.retain(|k| *k != id);
            } else {
                if !st.selected.contains(&id) {
                    if !shift {
                        st.selected.clear();
                    }
                    st.selected.push(id);
                }
                st.active = Some(id);
                if resp.drag_started() {
                    st.drag = Some(Drag::Keys {
                        start: Box::new(peg.clone()),
                        from: at,
                        keys: st.selected.clone(),
                    });
                }
            }
        } else if resp.drag_started() {
            st.drag = Some(Drag::Box(at));
        } else {
            if !(shift || ctrl) {
                st.selected.clear();
                st.active = None;
            }
            s.frame = frame_at(at.x);
        }
    }
    if let (Some(pos), true) = (pointer, resp.double_clicked()) {
        if hit(pos).is_none() && pos.y >= rect.top() {
            add_key_on_nearest_curve(s, st, peg, &shown, &map, pos);
        }
    }
    if let (Some(pos), true) = (pointer, resp.dragged()) {
        match &st.drag {
            Some(Drag::Scrub) => s.frame = frame_at(pos.x),
            Some(Drag::Keys { start, from, keys }) => {
                let df = (map.frame(pos.x) - map.frame(from.x)).round() as i64;
                let dv = map.value(pos.y) - map.value(from.y);
                if let Some(p) = move_keys(start, keys, df, dv) {
                    let moved = |(q, f): KeyId| (q, (i64::from(f) + df) as u32);
                    st.selected = keys.iter().map(|&k| moved(k)).collect();
                    st.active = st
                        .active
                        .and_then(|a| keys.iter().position(|k| *k == a))
                        .map(|i| moved(keys[i]));
                    store(s, layer, p);
                }
            }
            Some(Drag::Handle((p, f), grab)) => {
                let mut np = peg.clone();
                if let Some(k) = p.get_mut(&mut np).key_mut(*f) {
                    drag_handle(k, *grab, &map, pos);
                }
                store(s, layer, np);
            }
            Some(Drag::Box(from)) => {
                let r = Rect::from_two_pos(*from, pos);
                painter.rect_filled(r, 0.0, theme::ACCENT_SELECT.gamma_multiply(0.12));
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.0, theme::ACCENT_SELECT),
                    egui::StrokeKind::Inside,
                );
            }
            None => {}
        }
    }
    if resp.drag_stopped() {
        match st.drag.take() {
            Some(Drag::Box(from)) => {
                let end = pointer.or(ui.input(|i| i.pointer.latest_pos()));
                let r = Rect::from_two_pos(from, end.unwrap_or(from));
                let inside = keys_in(r, peg, &shown, &map);
                if ctrl {
                    st.selected.retain(|k| !inside.contains(k));
                } else {
                    if !shift {
                        st.selected.clear();
                    }
                    for k in inside {
                        if !st.selected.contains(&k) {
                            st.selected.push(k);
                        }
                    }
                }
                st.active = st.selected.last().copied();
            }
            Some(Drag::Keys { .. } | Drag::Handle(..)) => s.end_gesture(),
            _ => {}
        }
    }
    if shown.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Tick a function on the left to show it",
            theme::body(),
            theme::TEXT_SECONDARY,
        );
    }
}

/// The frame scale's step for `frames` shown across `width` points: a
/// whole number of frames, about 60 points apart. (nice_step builds its
/// steps from 0.001, so its sums are a little off: 20.000002.)
fn frame_step(frames: f32, width: f32) -> f32 {
    nice_step(60.0 * frames / width).max(1.0).round()
}

/// Grid lines at round steps, frame numbers on the frame scale, values
/// on the left. Frames outside the scene are shaded.
fn paint_grid(painter: &egui::Painter, map: &Map, full: Rect, len: f32) {
    let rect = map.rect;
    let fstep = frame_step(map.f1 - map.f0, rect.width());
    let mut k = (map.f0 / fstep).ceil();
    loop {
        let f = k * fstep;
        if f > map.f1 {
            break;
        }
        let x = map.pos(f, 0.0).x;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, theme::LINE_SUBTLE),
        );
        if f >= 1.0 {
            painter.text(
                Pos2::new(x, rect.top() - SCALE_H / 2.0 - 2.0),
                Align2::CENTER_CENTER,
                format!("{f:.0}"),
                theme::numbers(),
                theme::TEXT_SECONDARY,
            );
        }
        k += 1.0;
    }
    for (a, b) in [(map.f0, 1.0), (len, map.f1)] {
        if a < b {
            let r = Rect::from_x_y_ranges(
                map.pos(a, 0.0).x.max(rect.left())..=map.pos(b, 0.0).x.min(rect.right()),
                rect.y_range(),
            );
            painter.rect_filled(r, 0.0, theme::BG_BASE.gamma_multiply(0.5));
        }
    }
    let vstep = nice_step(40.0 * (map.v1 - map.v0) / rect.height());
    let decimals = if vstep >= 1.0 {
        0
    } else {
        (-vstep.log10()).ceil() as usize
    };
    let mut v = (map.v0 / vstep).ceil() * vstep;
    while v <= map.v1 {
        let y = map.pos(map.f0, v).y;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, theme::LINE_SUBTLE),
        );
        painter.text(
            Pos2::new(rect.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{v:.decimals$}"),
            theme::numbers(),
            theme::TEXT_SECONDARY,
        );
        v += vstep;
    }
    let _ = full;
}

/// Curves, then keys and the active key's handles on top.
fn paint_curves(
    painter: &egui::Painter,
    st: &FunctionState,
    peg: &PegParams,
    shown: &[Param],
    map: &Map,
) {
    let rect = map.rect;
    for p in shown {
        let v = p.get(peg).cloned().unwrap_or(Value::Constant(p.rest()));
        let steps = (rect.width() / 2.0).max(20.0) as usize;
        let pts: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let f = map.f0 + (map.f1 - map.f0) * i as f32 / steps as f32;
                map.pos(f, v.at_f(f.max(1.0)))
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, p.colour())));
        for k in v.keys() {
            let c = map.pos(k.frame as f32, k.value);
            let id = (*p, k.frame);
            if st.active == Some(id) {
                for (h, show) in [
                    (k.left, prev_is_bezier(&v, k.frame)),
                    (k.right, k.interp == Interp::Bezier),
                ] {
                    if show {
                        let hp = map.pos(k.frame as f32 + h[0], k.value + h[1]);
                        painter.line_segment([c, hp], Stroke::new(1.0, theme::TEXT_SECONDARY));
                        painter.circle_filled(hp, HANDLE_R, theme::TEXT_PRIMARY);
                    }
                }
            }
            let r = Rect::from_center_size(c, Vec2::splat(KEY_R * 2.0));
            let fill = if st.selected.contains(&id) {
                theme::ACCENT_SELECT
            } else {
                p.colour()
            };
            painter.rect_filled(r, 1.0, fill);
            painter.rect_stroke(
                r,
                1.0,
                Stroke::new(1.0, theme::BG_BASE),
                egui::StrokeKind::Outside,
            );
        }
    }
}

/// The key or active-key handle nearest `pos`, within 8 points.
fn hit_key(
    pos: Pos2,
    active: Option<KeyId>,
    peg: &PegParams,
    shown: &[Param],
    map: &Map,
) -> Option<(KeyId, Grab)> {
    let mut best: Option<(f32, (KeyId, Grab))> = None;
    for p in shown {
        let Some(v) = p.get(peg) else { continue };
        for k in v.keys() {
            let id = (*p, k.frame);
            let mut cands = vec![(map.pos(k.frame as f32, k.value), Grab::Key)];
            if active == Some(id) {
                if k.interp == Interp::Bezier {
                    cands.push((
                        map.pos(k.frame as f32 + k.right[0], k.value + k.right[1]),
                        Grab::RightHandle,
                    ));
                }
                if prev_is_bezier(v, k.frame) {
                    cands.push((
                        map.pos(k.frame as f32 + k.left[0], k.value + k.left[1]),
                        Grab::LeftHandle,
                    ));
                }
            }
            for (q, g) in cands {
                let d = q.distance(pos);
                if d <= 8.0 && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, (id, g)));
                }
            }
        }
    }
    best.map(|b| b.1)
}

/// The shown keys inside `r` (screen).
fn keys_in(r: Rect, peg: &PegParams, shown: &[Param], map: &Map) -> Vec<KeyId> {
    shown
        .iter()
        .filter_map(|p| p.get(peg).map(|v| (p, v)))
        .flat_map(|(p, v)| {
            v.keys()
                .iter()
                .filter(|k| r.contains(map.pos(k.frame as f32, k.value)))
                .map(|k| (*p, k.frame))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Double-click on blank: a key on the nearest shown curve at that frame.
fn add_key_on_nearest_curve(
    s: &mut Session,
    st: &mut FunctionState,
    peg: &PegParams,
    shown: &[Param],
    map: &Map,
    pos: Pos2,
) {
    let f = (map.frame(pos.x).round().max(1.0) as u32).min(s.length());
    let near = shown
        .iter()
        .map(|p| {
            let v = p.get(peg).cloned().unwrap_or(Value::Constant(p.rest()));
            let y = map.pos(f as f32, v.at(f)).y;
            ((y - pos.y).abs(), *p, v.at(f))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, p, value)) = near {
        let mut np = peg.clone();
        p.get_mut(&mut np).set_key(f, value);
        if let Some((source, _)) = source(s) {
            store(s, source, np);
        }
        s.end_gesture();
        st.selected = vec![(p, f)];
        st.active = Some((p, f));
    }
}

/// Drag one Bézier handle to `pos`; the other follows the continuity.
fn drag_handle(k: &mut symphony_core::anim::Key, grab: Grab, map: &Map, pos: Pos2) {
    let off = [
        map.frame(pos.x) - k.frame as f32,
        map.value(pos.y) - k.value,
    ];
    let right = grab == Grab::RightHandle;
    let off = if right {
        [off[0].max(0.0), off[1]]
    } else {
        [off[0].min(0.0), off[1]]
    };
    let (mine, other) = if right {
        (&mut k.right, &mut k.left)
    } else {
        (&mut k.left, &mut k.right)
    };
    *mine = off;
    // Continuity: Smooth mirrors angle and length, Straight keeps the
    // other handle's length.
    let other_len = (other[0] * other[0] + (other[1] / map.per_point()).powi(2)).sqrt();
    match k.continuity {
        Continuity::Smooth => *other = [-off[0], -off[1]],
        Continuity::Straight => {
            let len = (off[0] * off[0] + (off[1] / map.per_point()).powi(2))
                .sqrt()
                .max(1e-6);
            let scale = other_len / len;
            *other = [-off[0] * scale, -off[1] * scale];
        }
        Continuity::Corner => {}
    }
}

/// Is the segment that ends at `frame` a Bézier one (so the key's left
/// handle matters)?
fn prev_is_bezier(v: &Value, frame: u32) -> bool {
    let keys = v.keys();
    keys.iter()
        .position(|k| k.frame == frame)
        .and_then(|i| i.checked_sub(1))
        .is_some_and(|i| keys[i].interp == Interp::Bezier)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A peg with keys on X at frames 3 and 10, and on Y at 6.
    fn keyed() -> PegParams {
        let mut p = PegParams::default();
        p.x.set_key(3, 0.0);
        p.x.set_key(10, 5.0);
        p.y.set_key(6, 2.0);
        p
    }

    fn frames(v: &Value) -> Vec<(u32, f32)> {
        v.keys().iter().map(|k| (k.frame, k.value)).collect()
    }

    /// A selected bone shows its angle, and edits go back to the bone as
    /// one undo step.
    #[test]
    fn a_selected_bone_shows_and_takes_its_angle() {
        let mut s = Session::new_scene();
        let bone = s
            .doc
            .graph
            .add("Bone", NodeKind::Bone(symphony_core::rig::Bone::default()));
        s.selected_node = Some(bone);
        let (source, mut values) = source(&s).unwrap();
        assert_eq!(source, Source::Bone(bone));
        assert_eq!(source.params(), &[Param::Rotation]);
        values.rotation.set_key(1, 0.0);
        values.rotation.set_key(12, 45.0);
        store(&mut s, source, values);
        let angle = |s: &Session| s.doc.graph.bone(bone).unwrap().rotation.at(12);
        assert_eq!(angle(&s), 45.0);
        assert_eq!(source_values(&s).rotation.keys().len(), 2);
        s.undo();
        assert_eq!(angle(&s), 0.0);
    }

    /// A selected Control shows its position, and edits and key moves
    /// go back to the Control.
    #[test]
    fn a_selected_control_shows_and_takes_its_position() {
        let mut s = Session::new_scene();
        let control = s
            .doc
            .graph
            .add("Control", NodeKind::Control(Default::default()));
        s.selected_node = Some(control);
        let (source, mut values) = source(&s).unwrap();
        assert_eq!(source, Source::Control(control));
        assert_eq!(source.params(), &[Param::Position]);
        values.x.set_key(1, 0.0);
        values.x.set_key(12, 1.0);
        store(&mut s, source, values);
        let position = |s: &Session| match &s.doc.graph.node(control).unwrap().kind {
            NodeKind::Control(c) => c.value.clone(),
            _ => unreachable!(),
        };
        assert_eq!(position(&s).at(12), 1.0);
        let moved = move_keys(&source_values(&s), &[(Param::Position, 12)], 2, 0.0).unwrap();
        assert_eq!(frames(&moved.x), vec![(1, 0.0), (14, 1.0)]);
        s.undo();
        assert_eq!(position(&s).at(12), 0.0);
    }

    fn source_values(s: &Session) -> PegParams {
        source(s).unwrap().1
    }

    #[test]
    fn the_frame_scale_counts_whole_frames_at_any_zoom() {
        for frames in [3.0, 10.0, 47.5, 120.0, 999.0, 24_000.0] {
            for width in [120.0, 400.0, 1333.0, 3000.0] {
                let step = frame_step(frames, width);
                assert!(
                    step >= 1.0 && step.fract() == 0.0,
                    "{frames} in {width}: {step}"
                );
                // Every label k * step is a whole frame.
                for k in 0..50 {
                    assert_eq!((k as f32 * step).fract(), 0.0);
                }
                // Labels stay apart, unless one frame is already wider.
                let points = step * width / frames;
                assert!(points >= 60.0 - 1e-3 || step == 1.0, "{points}");
            }
        }
    }

    #[test]
    fn selected_keys_move_together() {
        let keys = [(Param::X, 3), (Param::Y, 6)];
        let p = move_keys(&keyed(), &keys, 2, 1.0).expect("room to move");
        assert_eq!(frames(&p.x), [(5, 1.0), (10, 5.0)]);
        assert_eq!(frames(&p.y), [(8, 3.0)]);
        // Both X keys may pass each other's old frames together.
        let both = [(Param::X, 3), (Param::X, 10)];
        let p = move_keys(&keyed(), &both, 7, 0.0).expect("both move");
        assert_eq!(frames(&p.x), [(10, 0.0), (17, 5.0)]);
    }

    #[test]
    fn keys_never_land_on_a_still_key_or_before_frame_one() {
        let keys = [(Param::X, 3)];
        assert!(
            move_keys(&keyed(), &keys, 7, 0.0).is_none(),
            "frame 10 is taken"
        );
        assert!(move_keys(&keyed(), &keys, -3, 0.0).is_none(), "frame 0");
        assert!(move_keys(&keyed(), &keys, -2, 0.0).is_some(), "frame 1");
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let span = Span {
            f0: 1.0,
            f1: 101.0,
            v0: -10.0,
            v1: 10.0,
        };
        let z = span.zoom_frames(26.0, 0.5, 1000.0);
        assert!((z.f1 - z.f0 - 50.0).abs() < 1e-4);
        assert!(((26.0 - z.f0) / (z.f1 - z.f0) - 0.25).abs() < 1e-5);
        let z = span.zoom_values(5.0, 2.0);
        assert!((z.v1 - z.v0 - 40.0).abs() < 1e-4);
        assert!(((5.0 - z.v0) / (z.v1 - z.v0) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn a_box_drag_selects_keys_and_delete_removes_them_all() {
        use egui_kittest::kittest::Queryable;
        let mut session = Session::new_scene();
        session.set_peg(0, keyed());
        session.end_gesture();
        let mut h = egui_kittest::Harness::builder()
            .with_size(Vec2::new(900.0, 420.0))
            .build_ui_state(
                |u, (s, st)| super::ui(u, s, st),
                (session, FunctionState::default()),
            );
        h.run_steps(2);
        // From the blank margin left of frame 1 to past the last key, below
        // the frame scale.
        let (from, to) = (Pos2::new(190.0, 68.0), Pos2::new(885.0, 400.0));
        let none = egui::Modifiers::NONE;
        let button = egui::PointerButton::Primary;
        h.input_mut().events.push(egui::Event::PointerMoved(from));
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: from,
            button,
            pressed: true,
            modifiers: none,
        });
        h.run_steps(1);
        for k in 1..=5 {
            let at = from + (to - from) * (k as f32 / 5.0);
            h.input_mut().events.push(egui::Event::PointerMoved(at));
            h.run_steps(1);
        }
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: to,
            button,
            pressed: false,
            modifiers: none,
        });
        h.run_steps(2);
        let mut picked = h.state().1.selected.clone();
        picked.sort_by_key(|(p, f)| (*p as usize, *f));
        assert_eq!(picked, [(Param::X, 3), (Param::X, 10), (Param::Y, 6)]);
        h.get_by_label("Delete keys").click();
        h.run_steps(2);
        let peg = h.state().0.doc.peg(0).cloned().expect("a peg");
        assert!(peg.x.keys().is_empty() && peg.y.keys().is_empty());
    }
}
