//! Camera and Drawing views: one canvas, two modes (DESIGN D3.6).
//! - Camera shows the composited scene (every visible layer, with pegs).
//! - Drawing shows only the selected layer's drawing, untransformed.
//!
//! Navigation: Space+drag pans, Shift+Space+drag rotates, pinch or
//! Cmd/Ctrl+scroll zooms, scroll pans. The Hand and Zoom tools do the same
//! with the pen. Drawing on an empty cell creates the next drawing.

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke as EStroke, Vec2};
use symphony_core::id::Id;
use symphony_core::math::Affine;
use symphony_core::stroke::{Point, Stroke, StrokeKind};
use symphony_pen::{PenSample, Phase, Tool as PenTool};
use symphony_ui::icons::{self, Icon};
use symphony_ui::theme;
use symphony_vector::{hits, pressure_curve, scale_of, tessellate, Smoother};

use crate::keymap::{self, Action, Profile};
use crate::session::{Session, Tool};

/// Eraser radius, in screen points.
const ERASER_RADIUS: f32 = 10.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 32.0;

pub struct CanvasState {
    pan: Vec2,
    zoom: f32,
    /// Degrees, clockwise.
    rotation: f32,
    active: Option<Active>,
    erasing: Option<(Id, Affine)>,
    /// Screen -> scene for the last frame, used by input.
    last_view: Affine,
}

impl Default for CanvasState {
    fn default() -> Self {
        CanvasState {
            pan: Vec2::ZERO,
            zoom: 1.0,
            rotation: 0.0,
            active: None,
            erasing: None,
            last_view: Affine::IDENTITY,
        }
    }
}

struct Active {
    layer: usize,
    frame: u32,
    /// Scene -> drawing-local.
    to_local: Affine,
    smoother: Smoother,
    min_step: f32,
    stroke: Stroke,
}

impl CanvasState {
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.erasing.is_some()
    }

    pub fn reset_view(&mut self) {
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.rotation = 0.0;
    }

    pub fn rotate_by(&mut self, degrees: f32) {
        self.rotation = (self.rotation + degrees).rem_euclid(360.0);
    }
}

/// Scene units -> screen points for this canvas rect.
fn view_transform(state: &CanvasState, rect: Rect, w: f32, h: f32) -> Affine {
    let fit = ((rect.width() - 64.0) / w)
        .min((rect.height() - 64.0) / h)
        .max(0.01);
    let c = rect.center() + state.pan;
    Affine::translate(c.x, c.y)
        .compose(Affine::rotate_deg(state.rotation))
        .compose(Affine::scale(fit * state.zoom, fit * state.zoom))
}

fn to_pos((x, y): (f32, f32)) -> Pos2 {
    Pos2::new(x, y)
}

fn paint_stroke(painter: &egui::Painter, stroke: &Stroke, colour: Color32, m: Affine) {
    let mesh = tessellate(stroke, m, 0.6);
    if mesh.indices.is_empty() {
        return;
    }
    let mut out = egui::Mesh::default();
    out.vertices.reserve(mesh.vertices.len());
    for v in &mesh.vertices {
        out.colored_vertex(Pos2::new(v[0], v[1]), colour);
    }
    out.indices = mesh.indices;
    painter.add(egui::Shape::mesh(out));
}

/// When there's no native pen backend, egui's pointer becomes pen samples (E4.2).
pub fn pointer_samples(ctx: &egui::Context) -> Vec<PenSample> {
    ctx.input(|i| {
        let Some(pos) = i.pointer.latest_pos() else {
            return Vec::new();
        };
        let phase = if i.pointer.primary_pressed() {
            Phase::Down
        } else if i.pointer.primary_released() {
            Phase::Up
        } else if i.pointer.primary_down() {
            Phase::Move
        } else {
            Phase::Hover
        };
        vec![PenSample {
            x: pos.x as f64,
            y: pos.y as f64,
            pressure: if phase == Phase::Hover { 0.0 } else { 1.0 },
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            tangential: 0.0,
            phase,
            tool: PenTool::Mouse,
            buttons: 0,
            time: i.time,
        }]
    })
}

/// View header (D3.2): onion skin, zoom, rotation and fit.
fn header(
    ui: &mut egui::Ui,
    s: &mut Session,
    state: &mut CanvasState,
    profile: Profile,
    drawing_only: bool,
) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(theme::CARD_PAD, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icons::button(
                    ui,
                    Icon::Onion,
                    s.onion.on,
                    true,
                    &keymap::tooltip(profile, Action::ToggleOnionSkin),
                )
                .clicked()
                {
                    s.onion.on = !s.onion.on;
                }
                if s.onion.on {
                    ui.add_space(6.0);
                    ui.add(
                        egui::DragValue::new(&mut s.onion.before)
                            .range(0..=5)
                            .prefix("Before "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut s.onion.after)
                            .range(0..=5)
                            .prefix("After "),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if icons::button(
                        ui,
                        Icon::Fit,
                        false,
                        true,
                        &keymap::tooltip(profile, Action::ResetView),
                    )
                    .clicked()
                    {
                        state.reset_view();
                    }
                    if icons::button(ui, Icon::RotateRight, false, true, "Rotate view clockwise")
                        .clicked()
                    {
                        state.rotate_by(15.0);
                    }
                    if icons::button(
                        ui,
                        Icon::RotateLeft,
                        false,
                        true,
                        "Rotate view counter-clockwise",
                    )
                    .clicked()
                    {
                        state.rotate_by(-15.0);
                    }
                    ui.add_space(8.0);
                    let label = format!("{:.0}%", state.zoom * 100.0);
                    ui.label(
                        egui::RichText::new(label)
                            .font(theme::dense())
                            .color(theme::TEXT_SECONDARY),
                    );
                    if state.rotation != 0.0 {
                        let r = format!("{:.0} deg", state.rotation);
                        ui.label(
                            egui::RichText::new(r)
                                .font(theme::dense())
                                .color(theme::TEXT_SECONDARY),
                        );
                    }
                    let mode = if drawing_only {
                        "Drawing view: selected drawing only, without pegs"
                    } else {
                        ""
                    };
                    if !mode.is_empty() {
                        ui.label(
                            egui::RichText::new(mode)
                                .font(theme::dense())
                                .color(theme::TEXT_SECONDARY),
                        );
                    }
                });
            });
        });
}

pub fn ui(
    ui: &mut egui::Ui,
    s: &mut Session,
    state: &mut CanvasState,
    samples: &[PenSample],
    drawing_only: bool,
    profile: Profile,
) {
    header(ui, s, state, profile, drawing_only);
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;
    let painter = painter.with_clip_rect(rect);
    let (w, h) = (s.doc.settings.width as f32, s.doc.settings.height as f32);

    // Navigation (before drawing, so this frame already shows the result).
    let (space, shift, zoom_delta, scroll, alt) = ui.input(|i| {
        (
            i.key_down(egui::Key::Space),
            i.modifiers.shift,
            i.zoom_delta(),
            i.smooth_scroll_delta,
            i.modifiers.alt,
        )
    });
    let navigating = space || s.tool == Tool::Hand;
    if response.hovered() || response.dragged() {
        if let Some(p) = response.hover_pos() {
            if zoom_delta != 1.0 {
                zoom_at(state, rect, w, h, p, zoom_delta);
            }
        }
        if scroll != Vec2::ZERO && zoom_delta == 1.0 {
            state.pan += scroll;
        }
    }
    if response.dragged() && navigating {
        if space && shift {
            // Rotate around the view centre by the angle the pointer swept.
            if let Some(p) = response.interact_pointer_pos() {
                let c = rect.center() + state.pan;
                let (a, b) = (p - response.drag_delta() - c, p - c);
                state.rotate_by((b.y.atan2(b.x) - a.y.atan2(a.x)).to_degrees());
            }
        } else {
            state.pan += response.drag_delta();
        }
    }
    if s.tool == Tool::Zoom && response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            zoom_at(state, rect, w, h, p, if alt { 0.8 } else { 1.25 });
        }
    }

    let view = view_transform(state, rect, w, h);
    state.last_view = view.inverse().unwrap_or(Affine::IDENTITY);
    let px = 1.0 / scale_of(view).max(1e-6);

    // D4.2: neutral surround. D3.10: the stage is paper with a soft shadow.
    painter.rect_filled(rect, 0.0, theme::CANVAS_SURROUND);
    let corners = [
        (-w / 2.0, -h / 2.0),
        (w / 2.0, -h / 2.0),
        (w / 2.0, h / 2.0),
        (-w / 2.0, h / 2.0),
    ]
    .map(|(x, y)| to_pos(view.apply(x, y)));
    if state.rotation == 0.0 {
        let stage = Rect::from_two_pos(corners[0], corners[2]);
        let shadow = egui::Shadow {
            offset: [0, 10],
            blur: 36,
            spread: 0,
            color: theme::SHADOW,
        };
        painter.add(shadow.as_shape(stage, 0));
    }
    painter.add(egui::Shape::convex_polygon(
        corners.to_vec(),
        theme::CANVAS_STAGE,
        EStroke::NONE,
    ));

    let colour = |s: &Session, swatch: Id| {
        let [r, g, b] = s.doc.swatch_rgb(swatch);
        Color32::from_rgb(r, g, b)
    };

    // The selected layer's transform, for onion skin and new strokes.
    let layer_world = if drawing_only {
        Affine::IDENTITY
    } else {
        s.doc
            .layers
            .get(s.selected)
            .and_then(|l| s.doc.graph.world_transform(l.node, s.frame).ok())
            .unwrap_or(Affine::IDENTITY)
    };

    // Onion skin: neighbouring drawings of the selected layer, tinted.
    if s.onion.on {
        let (before, after) = s.onion_drawings();
        for (list, tint) in [(before, theme::ONION_BEFORE), (after, theme::ONION_AFTER)] {
            for (k, d) in list.iter().enumerate().rev() {
                let alpha = 0.45 / (k as f32 + 1.0);
                if let Some(dr) = s.doc.drawings.get(d) {
                    for st in &dr.strokes {
                        paint_stroke(
                            &painter,
                            st,
                            tint.gamma_multiply(alpha),
                            view.compose(layer_world),
                        );
                    }
                }
            }
        }
    }

    // The artwork, back to front.
    let items: Vec<(Id, Affine)> = if drawing_only {
        s.doc
            .exposed(s.selected, s.frame)
            .map(|d| vec![(d, Affine::IDENTITY)])
            .unwrap_or_default()
    } else {
        s.doc
            .graph
            .evaluate(s.doc.display, &s.doc.xsheet, s.frame)
            .map(|v| v.into_iter().map(|i| (i.drawing, i.transform)).collect())
            .unwrap_or_default()
    };
    for (d, m) in &items {
        if let Some(dr) = s.doc.drawings.get(d) {
            for st in &dr.strokes {
                paint_stroke(&painter, st, colour(s, st.swatch), view.compose(*m));
            }
        }
    }
    if let Some(a) = &state.active {
        if let Some(back) = a.to_local.inverse() {
            paint_stroke(
                &painter,
                &a.stroke,
                colour(s, a.stroke.swatch),
                view.compose(back),
            );
        }
    }

    // Pen input.
    let zoom_factor = ui.ctx().zoom_factor();
    let draws = matches!(s.tool, Tool::Brush | Tool::Pencil | Tool::Eraser) && !space;
    let locked = s.doc.column(s.selected).is_some_and(|c| c.locked);
    for sm in samples {
        let pos = Pos2::new(sm.x as f32 / zoom_factor, sm.y as f32 / zoom_factor);
        let (sx, sy) = state.last_view.apply(pos.x, pos.y);
        let erasing = sm.tool == PenTool::Eraser || s.tool == Tool::Eraser;
        match sm.phase {
            Phase::Down
                if draws
                    && !locked
                    && (response.hovered() || response.contains_pointer())
                    && rect.contains(pos) =>
            {
                let Some(to_local) = layer_world.inverse() else {
                    continue;
                };
                let local_px = px / scale_of(layer_world).max(1e-6);
                if erasing {
                    if let Some(d) = s.doc.exposed(s.selected, s.frame) {
                        state.erasing = Some((d, to_local));
                        erase_at(s, state, (sx, sy), ERASER_RADIUS * local_px);
                    }
                } else {
                    let settings = if s.tool == Tool::Pencil {
                        s.pencil
                    } else {
                        s.brush
                    };
                    let kind = if s.tool == Tool::Pencil {
                        StrokeKind::Pencil
                    } else {
                        StrokeKind::Brush
                    };
                    let (x, y) = to_local.apply(sx, sy);
                    let p = Point {
                        x,
                        y,
                        pressure: pressure_curve(sm.pressure, s.pressure_gamma),
                    };
                    let mut smoother = Smoother::new(settings.smoothing);
                    let first = smoother.push(p, 0.0).unwrap_or(p);
                    // Widths are in screen points, so a stroke looks the same at any zoom when drawn.
                    let k = px / scale_of(layer_world).max(1e-6);
                    state.active = Some(Active {
                        layer: s.selected,
                        frame: s.frame,
                        to_local,
                        smoother,
                        min_step: 0.5 * local_px,
                        stroke: Stroke {
                            id: s.doc.ids.fresh(),
                            kind,
                            art: Default::default(),
                            swatch: s.swatch,
                            min_width: settings.min_width * k,
                            max_width: settings.max_width * k,
                            points: vec![first],
                        },
                    });
                }
            }
            Phase::Move => {
                if let Some(a) = state.active.as_mut() {
                    let (x, y) = a.to_local.apply(sx, sy);
                    let raw = Point {
                        x,
                        y,
                        pressure: pressure_curve(sm.pressure, s.pressure_gamma),
                    };
                    if let Some(p) = a.smoother.push(raw, a.min_step) {
                        a.stroke.points.push(p);
                    }
                } else if state.erasing.is_some() {
                    let local_px = px / scale_of(layer_world).max(1e-6);
                    erase_at(s, state, (sx, sy), ERASER_RADIUS * local_px);
                }
            }
            Phase::Up => {
                if let Some(mut a) = state.active.take() {
                    if let Some(p) = a.smoother.finish() {
                        a.stroke.points.push(p);
                    }
                    s.commit_stroke(a.layer, a.frame, a.stroke);
                }
                if state.erasing.take().is_some() {
                    s.end_gesture();
                }
            }
            _ => {}
        }
    }

    // Eraser cursor (D7.3: destructive tools tint red while in use).
    if let Some(last) = samples.last() {
        if last.tool == PenTool::Eraser || s.tool == Tool::Eraser {
            let p = Pos2::new(last.x as f32 / zoom_factor, last.y as f32 / zoom_factor);
            if rect.contains(p) {
                painter.circle_stroke(p, ERASER_RADIUS, EStroke::new(1.0, theme::STATE_DANGER));
            }
        }
    }

    // D10.4: an empty cell invites drawing. On the surround for contrast (D4.4).
    let hint = if locked {
        Some(format!("{} is locked", s.layer_name(s.selected)))
    } else if s.doc.exposed(s.selected, s.frame).is_none() {
        Some(format!(
            "Draw here to create a drawing on {}",
            s.layer_name(s.selected)
        ))
    } else {
        None
    };
    if let Some(hint) = hint {
        painter.text(
            rect.left_top() + Vec2::new(10.0, 8.0),
            Align2::LEFT_TOP,
            hint,
            theme::dense(),
            theme::TEXT_PRIMARY,
        );
    }
}

fn zoom_at(state: &mut CanvasState, rect: Rect, w: f32, h: f32, p: Pos2, factor: f32) {
    let before = view_transform(state, rect, w, h)
        .inverse()
        .map(|inv| inv.apply(p.x, p.y));
    state.zoom = (state.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    if let Some((x, y)) = before {
        let after = view_transform(state, rect, w, h).apply(x, y);
        state.pan += p - to_pos(after);
    }
}

/// Erase whole strokes under the eraser. (Cutting strokes in two comes
/// with the vector editing tools.)
fn erase_at(s: &mut Session, state: &CanvasState, scene_pt: (f32, f32), radius: f32) {
    let Some((d, to_local)) = state.erasing else {
        return;
    };
    let (x, y) = to_local.apply(scene_pt.0, scene_pt.1);
    let ids: Vec<Id> = s
        .doc
        .drawings
        .get(&d)
        .map(|dr| {
            dr.strokes
                .iter()
                .filter(|st| hits(st, x, y, radius))
                .map(|st| st.id)
                .collect()
        })
        .unwrap_or_default();
    if !ids.is_empty() {
        s.erase(d, &ids);
    }
}
