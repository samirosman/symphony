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
use symphony_core::stroke::{ArtLayer, Point, Stroke, StrokeKind};
use symphony_pen::{PenSample, Phase, Tool as PenTool};
use symphony_ui::icons::{self, Icon};
use symphony_ui::theme;
use symphony_vector::{hits, pressure_curve, scale_of, tessellate, Smoother};

use crate::keymap::{self, Action, Profile};
use crate::session::{Session, Tool};
use crate::views::select::{self, SelectState};

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
    pub select: SelectState,
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
            select: SelectState::default(),
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
        self.active.is_some() || self.erasing.is_some() || self.select.busy()
    }

    pub fn reset_view(&mut self) {
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.rotation = 0.0;
    }

    pub fn rotate_by(&mut self, degrees: f32) {
        self.rotation = (self.rotation + degrees).rem_euclid(360.0);
    }

    /// Run a View action on this canvas (Harmony's View menu).
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::ZoomIn => self.zoom = (self.zoom * 1.25).min(MAX_ZOOM),
            Action::ZoomOut => self.zoom = (self.zoom / 1.25).max(MIN_ZOOM),
            Action::RotateCw => self.rotate_by(ROTATE_STEP),
            Action::RotateCcw => self.rotate_by(-ROTATE_STEP),
            Action::ResetZoom => self.zoom = 1.0,
            Action::ResetRotation => self.rotation = 0.0,
            Action::ResetPan => self.pan = Vec2::ZERO,
            Action::ResetView => self.reset_view(),
            _ => {}
        }
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

fn paint_mesh(painter: &egui::Painter, mesh: symphony_vector::Mesh, colour: Color32) {
    if mesh.indices.is_empty() {
        return;
    }
    let mut out = egui::Mesh::default();
    for v in &mesh.vertices {
        out.colored_vertex(Pos2::new(v[0], v[1]), colour);
    }
    out.indices = mesh.indices;
    painter.add(egui::Shape::mesh(out));
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

/// Rotate View steps 30 degrees, like an animation table (D3.5a).
pub const ROTATE_STEP: f32 = 30.0;

fn dense_label(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .font(theme::dense())
            .color(theme::TEXT_SECONDARY),
    );
}

/// Top toolbar (D3.5a): display options, here onion skin.
fn top_toolbar(ui: &mut egui::Ui, s: &mut Session, profile: Profile, drawing_only: bool) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(theme::CARD_PAD, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let tip = keymap::tooltip(profile, Action::ToggleOnionSkin);
                if icons::button(ui, Icon::Onion, s.onion.on, true, &tip).clicked() {
                    s.onion.on = !s.onion.on;
                }
                ui.add_space(8.0);
                let counts: [(&str, Action, Action, bool); 2] = [
                    (
                        "Previous",
                        Action::OnionLessPrevious,
                        Action::OnionMorePrevious,
                        true,
                    ),
                    ("Next", Action::OnionLessNext, Action::OnionMoreNext, false),
                ];
                for (label, less, more, before) in counts {
                    dense_label(ui, label);
                    let n = if before {
                        s.onion.before
                    } else {
                        s.onion.after
                    };
                    if icons::button(
                        ui,
                        Icon::Minus,
                        false,
                        n > 0,
                        &keymap::tooltip(profile, less),
                    )
                    .clicked()
                    {
                        s.onion_step(before, -1);
                    }
                    dense_label(ui, n.to_string());
                    if icons::button(
                        ui,
                        Icon::Plus,
                        false,
                        n < 9,
                        &keymap::tooltip(profile, more),
                    )
                    .clicked()
                    {
                        s.onion_step(before, 1);
                    }
                    ui.add_space(10.0);
                }
                if drawing_only {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dense_label(ui, "Selected drawing only, without pegs");
                    });
                }
            });
        });
}

/// Bottom toolbar (D3.5a): navigation.
fn bottom_toolbar(ui: &mut egui::Ui, state: &mut CanvasState, profile: Profile) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(theme::CARD_PAD, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut zoom = state.zoom;
                egui::ComboBox::from_id_salt(ui.scope_id().with("zoom"))
                    .width(72.0)
                    .selected_text(format!("{:.0}%", state.zoom * 100.0))
                    .show_ui(ui, |ui| {
                        for z in [0.25, 0.5, 1.0, 2.0, 4.0, 8.0] {
                            ui.selectable_value(&mut zoom, z, format!("{:.0}%", z * 100.0));
                        }
                    });
                state.zoom = zoom;
                ui.add_space(8.0);
                for (label, action) in [
                    ("Reset zoom", Action::ResetZoom),
                    ("Reset rotation", Action::ResetRotation),
                    ("Reset pan", Action::ResetPan),
                ] {
                    if ui
                        .small_button(label)
                        .on_hover_text(keymap::tooltip(profile, action))
                        .clicked()
                    {
                        state.apply(action);
                    }
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
                        state.apply(Action::ResetView);
                    }
                    if icons::button(
                        ui,
                        Icon::RotateRight,
                        false,
                        true,
                        &keymap::tooltip(profile, Action::RotateCw),
                    )
                    .clicked()
                    {
                        state.apply(Action::RotateCw);
                    }
                    if icons::button(
                        ui,
                        Icon::RotateLeft,
                        false,
                        true,
                        &keymap::tooltip(profile, Action::RotateCcw),
                    )
                    .clicked()
                    {
                        state.apply(Action::RotateCcw);
                    }
                    if state.rotation != 0.0 {
                        dense_label(ui, format!("{:.0} deg", state.rotation));
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
    top_toolbar(ui, s, profile, drawing_only);
    let canvas_size = ui.available_size() - Vec2::new(0.0, theme::TAB_ROW);
    let (response, painter) = ui.allocate_painter(canvas_size, Sense::click_and_drag());
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
    let items: Vec<select::Item> = if drawing_only {
        s.doc
            .exposed(s.selected, s.frame)
            .map(|d| vec![(s.selected, d, Affine::IDENTITY)])
            .unwrap_or_default()
    } else {
        let layers = &s.doc.layers;
        s.doc
            .graph
            .evaluate(s.doc.display, &s.doc.xsheet, s.frame)
            .map(|v| {
                v.into_iter()
                    .filter_map(|i| {
                        layers
                            .iter()
                            .position(|l| l.node == i.node)
                            .map(|layer| (layer, i.drawing, i.transform))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    // While the Select tool drags, the selected strokes draw through the preview.
    let preview = state.select.preview.filter(|_| s.tool == Tool::Select);
    for (_, d, m) in &items {
        if let Some(dr) = s.doc.drawings.get(d) {
            let moving = preview.is_some() && s.selection.drawing == Some(*d);
            // Art layers back to front; within each, paint under ink.
            for art in ArtLayer::STACK {
                for f in dr.fills.iter().filter(|f| f.art == art) {
                    paint_mesh(
                        &painter,
                        symphony_vector::fill::fill_mesh(&f.contours, view.compose(*m)),
                        colour(s, f.swatch),
                    );
                }
                for st in dr.strokes.iter().filter(|st| st.art == art) {
                    let m = match preview {
                        Some(l) if moving && s.selection.strokes.contains(&st.id) => {
                            view.compose(*m).compose(l)
                        }
                        _ => view.compose(*m),
                    };
                    if st.kind == StrokeKind::Invisible {
                        // Show Strokes (K): invisible strokes as thin cel-blue lines.
                        if s.show_strokes {
                            let pts: Vec<Pos2> = st
                                .points
                                .iter()
                                .map(|p| to_pos(m.apply(p.x, p.y)))
                                .collect();
                            painter.add(egui::Shape::line(
                                pts,
                                EStroke::new(1.0, theme::ACCENT_SELECT),
                            ));
                        }
                        continue;
                    }
                    paint_stroke(&painter, st, colour(s, st.swatch), m);
                }
            }
        }
    }
    let navigating_now = space || s.tool == Tool::Hand;
    if matches!(s.tool, Tool::Select | Tool::Transform) && !navigating_now {
        select::ui(ui, &response, &painter, s, &mut state.select, view, &items);
    } else {
        state.select.hover = None;
    }
    if let Some(a) = state
        .active
        .as_ref()
        .filter(|a| a.stroke.kind == StrokeKind::Invisible)
    {
        if let Some(back) = a.to_local.inverse() {
            let pts: Vec<Pos2> = a
                .stroke
                .points
                .iter()
                .map(|p| to_pos(view.compose(back).apply(p.x, p.y)))
                .collect();
            painter.add(egui::Shape::line(
                pts,
                EStroke::new(1.5, theme::ACCENT_SELECT),
            ));
        }
    }
    if let Some(a) = state
        .active
        .as_ref()
        .filter(|a| a.stroke.kind != StrokeKind::Invisible)
    {
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
    let draws = matches!(
        s.tool,
        Tool::Brush | Tool::Pencil | Tool::Eraser | Tool::Paint | Tool::CloseGap | Tool::Dropper
    ) && !space;
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
                let (lx, ly) = to_local.apply(sx, sy);
                if !erasing && s.tool == Tool::Paint {
                    s.paint_at(s.selected, s.frame, lx, ly, local_px);
                    continue;
                }
                if !erasing && s.tool == Tool::Dropper {
                    s.pick_colour(s.selected, s.frame, lx, ly, local_px);
                    continue;
                }
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
                    let kind = match s.tool {
                        Tool::Pencil => StrokeKind::Pencil,
                        Tool::CloseGap => StrokeKind::Invisible,
                        _ => StrokeKind::Brush,
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
                    if a.stroke.kind == StrokeKind::Invisible {
                        // Close Gap: the rough stroke only says where; it isn't kept.
                        let reach = 40.0 * a.min_step / 0.5;
                        s.close_gap(a.layer, a.frame, &a.stroke.points, reach);
                    } else {
                        s.commit_stroke(a.layer, a.frame, a.stroke);
                    }
                }
                if state.erasing.take().is_some() {
                    s.end_gesture();
                }
            }
            _ => {}
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

    let pen_eraser = samples.last().is_some_and(|l| l.tool == PenTool::Eraser);
    draw_cursor(
        ui,
        &response,
        rect,
        s,
        Mods {
            space,
            shift,
            alt,
            pen_eraser,
            locked,
            hint: state.select.hover,
        },
    );
    bottom_toolbar(ui, state, profile);
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

#[derive(Clone, Copy)]
struct Mods {
    space: bool,
    shift: bool,
    alt: bool,
    /// The pen's eraser end is in use.
    pen_eraser: bool,
    /// The selected layer is locked.
    locked: bool,
    /// Handle hint from Select/Transform.
    hint: Option<Icon>,
}

/// Our canvas cursor (D8.11): a crosshair plus the active tool's icon, or a
/// navigation cursor while a shortcut is held. Drawn on the foreground
/// layer, white over a dark outline so it reads on paper and on grey.
fn draw_cursor(ui: &egui::Ui, response: &egui::Response, rect: Rect, s: &Session, m: Mods) {
    let (pen_eraser, locked, hint) = (m.pen_eraser, m.locked, m.hint);
    let Some(p) = ui
        .input(|i| i.pointer.latest_pos())
        .filter(|p| rect.contains(*p))
    else {
        return;
    };
    if !(response.hovered() || response.dragged() || response.contains_pointer()) {
        return;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("symphony-canvas-cursor"),
        ))
        .with_clip_rect(rect);
    let dark = theme::BG_BASE;
    let light = theme::CANVAS_STAGE;

    // A navigation cursor replaces the crosshair: the icon itself is the pointer.
    let nav = if m.space && m.shift {
        Some(Icon::RotateRight)
    } else if m.space || s.tool == Tool::Hand {
        Some(Icon::Hand)
    } else if s.tool == Tool::Zoom {
        Some(if m.alt { Icon::ZoomOut } else { Icon::Zoom })
    } else {
        None
    };
    let badge = |icon: Icon, centre: Pos2, size: f32| {
        let r = Rect::from_center_size(centre, Vec2::splat(size + 6.0));
        painter.rect_filled(r, theme::RADIUS_CONTROL as f32, dark.gamma_multiply(0.85));
        icons::paint(
            &painter,
            Rect::from_center_size(centre, Vec2::splat(size)),
            icon,
            light,
        );
    };
    if let Some(icon) = nav {
        badge(icon, p, 18.0);
        return;
    }
    // Over a Select/Transform handle: show what dragging it will do.
    if let Some(icon) = hint.filter(|_| matches!(s.tool, Tool::Select | Tool::Transform)) {
        badge(icon, p, 16.0);
        return;
    }

    let draws = matches!(s.tool, Tool::Brush | Tool::Pencil | Tool::Eraser) || pen_eraser;
    if locked && draws {
        badge(Icon::Forbidden, p, 18.0);
        return;
    }

    // Size circle: always for the eraser, and for brush tools when the
    // Brush Size Cursor preference is on (as in Harmony).
    let erasing = pen_eraser || s.tool == Tool::Eraser;
    let radius = if erasing {
        Some(ERASER_RADIUS)
    } else if s.brush_size_cursor && matches!(s.tool, Tool::Brush | Tool::Pencil) {
        let b = if s.tool == Tool::Pencil {
            s.pencil
        } else {
            s.brush
        };
        Some((b.max_width * 0.5).max(1.5))
    } else {
        None
    };
    if let Some(r) = radius {
        painter.circle_stroke(p, r, EStroke::new(3.0, dark.gamma_multiply(0.6)));
        let ring = if erasing { theme::STATE_DANGER } else { light };
        painter.circle_stroke(p, r, EStroke::new(1.0, ring));
    }

    // Crosshair with a gap at the hotspot.
    let (gap, arm) = (3.0, 9.0);
    for (a, b) in [
        (Vec2::new(-arm, 0.0), Vec2::new(-gap, 0.0)),
        (Vec2::new(gap, 0.0), Vec2::new(arm, 0.0)),
        (Vec2::new(0.0, -arm), Vec2::new(0.0, -gap)),
        (Vec2::new(0.0, gap), Vec2::new(0.0, arm)),
    ] {
        painter.line_segment([p + a, p + b], EStroke::new(3.0, dark));
        painter.line_segment([p + a, p + b], EStroke::new(1.0, light));
    }

    // Tool icon beside the crosshair, so the active tool is always clear.
    let icon = if erasing {
        Icon::Eraser
    } else {
        match s.tool {
            Tool::Select => Icon::Select,
            Tool::Transform => Icon::Transform,
            Tool::Pencil => Icon::Pencil,
            Tool::Paint => Icon::Paint,
            Tool::CloseGap => Icon::CloseGap,
            Tool::Dropper => Icon::Dropper,
            _ => Icon::Brush,
        }
    };
    badge(icon, p + Vec2::new(16.0, 16.0), 12.0);
}
