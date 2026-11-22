//! Function view (Harmony's curve editor): the selected layer's peg
//! functions as curves over time. Select and drag keys, shape Bézier
//! segments with handles, switch a segment between Linear, Bezier and
//! Stop-motion, and add or delete keys.

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use symphony_core::anim::{Continuity, Interp, Value};
use symphony_core::graph::PegParams;
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
        }
    }

    fn colour(self) -> Color32 {
        match self {
            Param::X => theme::FUNCTION_X,
            Param::Y => theme::FUNCTION_Y,
            Param::Rotation => theme::FUNCTION_ANGLE,
            Param::ScaleX => theme::FUNCTION_SCALE_X,
            Param::ScaleY => theme::FUNCTION_SCALE_Y,
        }
    }

    /// The function, if the peg has one (scale is absent until used).
    fn get(self, p: &PegParams) -> Option<&Value> {
        match self {
            Param::X => Some(&p.x),
            Param::Y => Some(&p.y),
            Param::Rotation => Some(&p.rotation),
            Param::ScaleX => p.scale_x.as_ref(),
            Param::ScaleY => p.scale_y.as_ref(),
        }
    }

    fn get_mut(self, p: &mut PegParams) -> &mut Value {
        match self {
            Param::X => &mut p.x,
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

#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Key,
    LeftHandle,
    RightHandle,
}

pub struct FunctionState {
    shown: [bool; 5],
    /// Selected key: function and frame.
    selected: Option<(Param, u32)>,
    drag: Option<(Param, u32, Grab)>,
}

impl Default for FunctionState {
    fn default() -> Self {
        FunctionState {
            shown: [true; 5],
            selected: None,
            drag: None,
        }
    }
}

const LIST_W: f32 = 148.0;
const KEY_R: f32 = 4.0;
const HANDLE_R: f32 = 3.5;

pub fn ui(ui: &mut egui::Ui, s: &mut Session, st: &mut FunctionState) {
    let layer = s.selected;
    let Some(peg) = s.doc.peg(layer).cloned() else {
        ui.centered_and_justified(|ui| {
            ui.label("Select a layer to see its functions.");
        });
        return;
    };
    // Forget a selection whose key is gone.
    if let Some((p, f)) = st.selected {
        if !p.get(&peg).is_some_and(|v| v.has_key(f)) {
            st.selected = None;
        }
    }
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

fn toolbar(ui: &mut egui::Ui, s: &mut Session, st: &mut FunctionState, peg: &PegParams) {
    let layer = s.selected;
    let frame = s.frame;
    ui.horizontal(|ui| {
        // Add a key at the current frame to the selected function, or to
        // every shown function when none is selected.
        if ui
            .button("Add key")
            .on_hover_text("Add a key at the current frame to the selected function, or to every shown function")
            .clicked()
        {
            let mut p = peg.clone();
            let targets: Vec<Param> = match st.selected {
                Some((param, _)) => vec![param],
                None => Param::ALL
                    .into_iter()
                    .filter(|f| st.shown[*f as usize])
                    .collect(),
            };
            for param in targets {
                let v = param.get(&p).map_or(param.rest(), |v| v.at(frame));
                param.get_mut(&mut p).set_key(frame, v);
            }
            s.set_peg(layer, p);
            s.end_gesture();
        }
        let sel = st.selected;
        if ui
            .add_enabled(sel.is_some(), egui::Button::new("Delete key"))
            .clicked()
        {
            if let Some((param, f)) = sel {
                let mut p = peg.clone();
                param.get_mut(&mut p).remove_key(f);
                s.set_peg(layer, p);
                s.end_gesture();
                st.selected = None;
            }
        }
        ui.separator();
        let key = sel.and_then(|(param, f)| {
            param.get(peg).and_then(|v| v.keys().iter().find(|k| k.frame == f).copied())
        });
        match key {
            Some(k) => {
                let (param, f) = sel.unwrap_or((Param::X, 0));
                let mut interp = k.interp;
                ui.label(egui::RichText::new("Segment").font(theme::dense()).color(theme::TEXT_SECONDARY));
                for (label, i, tip) in [
                    ("Linear", Interp::Linear, "A straight line to the next key"),
                    ("Bezier", Interp::Bezier, "A curve shaped by handles"),
                    ("Stop-motion", Interp::Stop, "Hold until the next key (Harmony's stop-motion keyframe)"),
                ] {
                    ui.selectable_value(&mut interp, i, label).on_hover_text(tip);
                }
                let mut cont = k.continuity;
                if interp == Interp::Bezier {
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
                }
                ui.separator();
                let mut kf = k.frame as i64;
                let mut kv = k.value;
                ui.label(egui::RichText::new("Frame").font(theme::dense()).color(theme::TEXT_SECONDARY));
                let fr = ui.add(egui::DragValue::new(&mut kf).range(1..=100_000).speed(0.2));
                ui.label(egui::RichText::new("Value").font(theme::dense()).color(theme::TEXT_SECONDARY));
                let vr = ui.add(egui::DragValue::new(&mut kv).speed(0.1).max_decimals(3));
                if interp != k.interp || cont != k.continuity || fr.changed() || vr.changed() {
                    let mut p = peg.clone();
                    let v = param.get_mut(&mut p);
                    if let Some(km) = v.key_mut(f) {
                        if interp == Interp::Bezier && km.interp != Interp::Bezier && km.right == [0.0, 0.0] {
                            // Start a new curve with gentle ease handles.
                            km.right = [1.0, 0.0];
                            km.left = [-1.0, 0.0];
                        }
                        km.interp = interp;
                        km.continuity = cont;
                    }
                    if v.move_key(f, kf as u32, kv) {
                        st.selected = Some((param, kf as u32));
                    }
                    s.set_peg(layer, p);
                }
                if fr.drag_stopped() || vr.drag_stopped() || fr.lost_focus() || vr.lost_focus() {
                    s.end_gesture();
                }
            }
            None => {
                ui.label(
                    egui::RichText::new("Click a key to edit it. Double-click a curve to add one.")
                        .font(theme::dense())
                        .color(theme::TEXT_SECONDARY),
                );
            }
        }
    });
}

fn function_list(ui: &mut egui::Ui, st: &mut FunctionState, peg: &PegParams) {
    egui::ScrollArea::vertical()
        .id_salt("function-list")
        .show(ui, |ui| {
            ui.add_space(4.0);
            for param in Param::ALL {
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

fn graph(ui: &mut egui::Ui, s: &mut Session, st: &mut FunctionState, peg: &PegParams) {
    let layer = s.selected;
    let size = ui.available_size().max(Vec2::new(120.0, 80.0));
    let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let full = resp.rect;
    painter.rect_filled(full, 0.0, theme::BG_INPUT);
    let rect = full
        .shrink2(Vec2::new(36.0, 14.0))
        .translate(Vec2::new(14.0, 0.0));
    let len = s.length().max(2) as f32;
    let shown: Vec<Param> = Param::ALL
        .into_iter()
        .filter(|p| st.shown[*p as usize])
        .collect();

    // Fit the shown functions' values over the scene, with some room.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in &shown {
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
    let map = Map {
        rect,
        f0: 1.0,
        f1: len,
        v0: lo - pad,
        v1: hi + pad,
    };

    // Grid: frames every 5 (labels every 10), a few value lines.
    for f in (5..=len as u32).step_by(5) {
        let x = map.pos(f as f32, 0.0).x;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, theme::LINE_SUBTLE),
        );
        if f % 10 == 0 {
            painter.text(
                Pos2::new(x, full.bottom() - 2.0),
                Align2::CENTER_BOTTOM,
                f.to_string(),
                theme::numbers(),
                theme::TEXT_SECONDARY,
            );
        }
    }
    for i in 0..=4 {
        let v = map.v0 + (map.v1 - map.v0) * i as f32 / 4.0;
        let y = map.pos(1.0, v).y;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, theme::LINE_SUBTLE),
        );
        painter.text(
            Pos2::new(full.left() + 4.0, y),
            Align2::LEFT_CENTER,
            format!("{v:.1}"),
            theme::numbers(),
            theme::TEXT_SECONDARY,
        );
    }
    // Current frame.
    let cx = map.pos(s.frame as f32, 0.0).x;
    painter.line_segment(
        [Pos2::new(cx, full.top()), Pos2::new(cx, full.bottom())],
        Stroke::new(1.5, theme::ACCENT_TIME),
    );

    // Curves, then keys and handles on top.
    for p in &shown {
        let v = p.get(peg).cloned().unwrap_or(Value::Constant(p.rest()));
        let steps = (rect.width() / 2.0).max(20.0) as usize;
        let pts: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let f = 1.0 + (len - 1.0) * i as f32 / steps as f32;
                map.pos(f, v.at_f(f))
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, p.colour())));
        for k in v.keys() {
            let c = map.pos(k.frame as f32, k.value);
            let selected = st.selected == Some((*p, k.frame));
            if selected && (k.interp == Interp::Bezier || prev_is_bezier(&v, k.frame)) {
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
            painter.rect_filled(
                r,
                1.0,
                if selected {
                    theme::ACCENT_SELECT
                } else {
                    p.colour()
                },
            );
            painter.rect_stroke(
                r,
                1.0,
                Stroke::new(1.0, theme::BG_BASE),
                egui::StrokeKind::Outside,
            );
        }
    }

    // Input.
    let selected_now = st.selected;
    let hit = |pos: Pos2| -> Option<(Param, u32, Grab)> {
        let mut best: Option<(f32, (Param, u32, Grab))> = None;
        for p in &shown {
            let Some(v) = p.get(peg) else { continue };
            for k in v.keys() {
                let c = map.pos(k.frame as f32, k.value);
                let mut cands = vec![(c, Grab::Key)];
                if selected_now == Some((*p, k.frame)) {
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
                        best = Some((d, (*p, k.frame, g)));
                    }
                }
            }
        }
        best.map(|b| b.1)
    };
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.drag_started() || resp.clicked() {
            match hit(pos) {
                Some((p, f, g)) => {
                    st.selected = Some((p, f));
                    st.drag = resp.drag_started().then_some((p, f, g));
                }
                None => {
                    st.selected = None;
                    s.frame = (map.frame(pos.x).round() as u32).clamp(1, s.length());
                }
            }
        }
        if resp.double_clicked() && hit(pos).is_none() {
            // Add a key on the nearest shown curve at that frame.
            let f = (map.frame(pos.x).round() as u32).clamp(1, s.length());
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
                s.set_peg(layer, np);
                s.end_gesture();
                st.selected = Some((p, f));
            }
        }
        if resp.dragged() {
            if let Some((p, f, g)) = st.drag {
                let mut np = peg.clone();
                let v = p.get_mut(&mut np);
                match g {
                    Grab::Key => {
                        let to = (map.frame(pos.x).round() as i64).clamp(1, 100_000) as u32;
                        let val = map.value(pos.y);
                        if v.move_key(f, to, val) {
                            st.drag = Some((p, to, g));
                            st.selected = Some((p, to));
                        }
                    }
                    Grab::LeftHandle | Grab::RightHandle => {
                        if let Some(k) = v.key_mut(f) {
                            let off = [map.frame(pos.x) - f as f32, map.value(pos.y) - k.value];
                            let right = g == Grab::RightHandle;
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
                            // Continuity: Smooth mirrors angle and length,
                            // Straight keeps the other handle's length.
                            let other_len =
                                (other[0] * other[0] + (other[1] / map.per_point()).powi(2)).sqrt();
                            match k.continuity {
                                Continuity::Smooth => *other = [-off[0], -off[1]],
                                Continuity::Straight => {
                                    let len = (off[0] * off[0]
                                        + (off[1] / map.per_point()).powi(2))
                                    .sqrt()
                                    .max(1e-6);
                                    let scale = other_len / len;
                                    *other = [-off[0] * scale, -off[1] * scale];
                                }
                                Continuity::Corner => {}
                            }
                        }
                    }
                }
                s.set_peg(layer, np);
            }
        }
    }
    if resp.drag_stopped() {
        st.drag = None;
        s.end_gesture();
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

/// Is the segment that ends at `frame` a Bézier one (so the key's left
/// handle matters)?
fn prev_is_bezier(v: &Value, frame: u32) -> bool {
    let keys = v.keys();
    keys.iter()
        .position(|k| k.frame == frame)
        .and_then(|i| i.checked_sub(1))
        .is_some_and(|i| keys[i].interp == Interp::Bezier)
}
