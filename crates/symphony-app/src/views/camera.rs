//! Camera and Drawing views: one canvas, two modes (DESIGN D3.6).
//! - Camera renders the Display node, including compositing effects.
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
use symphony_vector::{pressure_curve, scale_of, tessellate, Smoother};

use crate::keymap::{self, Action, Profile};
use crate::session::{apply_stencil, CanvasColours, Session, Stencil, Tool};
use crate::views::select::{self, SelectState};

#[path = "camera_render.rs"]
mod camera_render;

/// Eraser radius, in screen points.
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 32.0;

pub struct CanvasState {
    pan: Vec2,
    /// Screen points per scene unit: absolute, as in Harmony, so 100% is
    /// actual size and resizing the view keeps the drawing's scale (and
    /// re-stamps nothing). Fitted to the view when first shown and on Fit.
    zoom: f32,
    framing: f32,
    /// Fit the stage to the view on the next frame.
    fit_pending: bool,
    /// Degrees, clockwise.
    rotation: f32,
    active: Option<Active>,
    /// An eraser drag in progress.
    erasing: Option<Erasing>,
    /// A Line Editor drag in progress.
    line_edit: Option<LineDrag>,
    /// Screen -> scene for the last frame, used by input.
    last_view: Affine,
    pub select: SelectState,
    /// Stamped textured strokes, reused while only panning.
    strokes: super::textured::Cache,
    display: camera_render::Cache,
}

impl Default for CanvasState {
    fn default() -> Self {
        CanvasState {
            pan: Vec2::ZERO,
            zoom: 1.0,
            framing: 1.0,
            fit_pending: true,
            rotation: 0.0,
            active: None,
            erasing: None,
            line_edit: None,
            last_view: Affine::IDENTITY,
            select: SelectState::default(),
            strokes: Default::default(),
            display: Default::default(),
        }
    }
}

/// What a Line Editor drag does: push lines, Shift-paint a point
/// selection, or move the selected points.
#[derive(Clone, Copy, PartialEq)]
enum LineMode {
    Push,
    Select,
    MoveSelected,
}

struct LineDrag {
    drawing: Id,
    to_local: Affine,
    /// Last position, drawing space.
    last: [f32; 2],
    mode: LineMode,
    moved: bool,
    /// Drawing units per screen point, for tolerances.
    local_px: f32,
}

/// One step of a Line Editor drag to scene point `to`; `radius` is in
/// drawing units.
fn line_step(s: &mut Session, le: &mut LineDrag, to: (f32, f32), radius: f32) {
    let (x, y) = le.to_local.apply(to.0, to.1);
    let delta = [x - le.last[0], y - le.last[1]];
    if delta[0] == 0.0 && delta[1] == 0.0 {
        return;
    }
    le.moved = true;
    match le.mode {
        LineMode::Push => s.line_push(le.drawing, le.last, radius, delta),
        LineMode::Select => s.line_select(le.drawing, [x, y], radius),
        LineMode::MoveSelected => s.line_move_selected(le.drawing, delta),
    }
    le.last = [x, y];
}

/// What one eraser drag erases: drawings with their scene -> drawing
/// transforms, and the last stamp (scene space) so the path is continuous.
struct Erasing {
    targets: Vec<(Id, Affine)>,
    last: Option<symphony_vector::cut::Disc>,
}

struct Active {
    layer: usize,
    frame: u32,
    /// Scene -> drawing-local.
    to_local: Affine,
    smoother: Smoother,
    min_step: f32,
    stroke: Stroke,
    /// Pencil thickness stencil, if the preset has one.
    stencil: Option<Stencil>,
    /// Stamping state while a textured brush stroke is being drawn.
    live: Option<super::textured::Live>,
}

impl CanvasState {
    /// Where scene point (`x`, `y`) was on screen last frame (UI tests).
    #[cfg(test)]
    pub fn to_screen(&self, x: f32, y: f32) -> Pos2 {
        let m = self.last_view.inverse().unwrap_or(Affine::IDENTITY);
        let (sx, sy) = m.apply(x, y);
        Pos2::new(sx, sy)
    }

    /// Set the zoom (screen points per scene unit), e.g. from search.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.fit_pending = false;
    }

    pub fn busy(&self) -> bool {
        self.active.is_some()
            || self.erasing.is_some()
            || self.line_edit.is_some()
            || self.select.busy()
    }

    /// Harmony's Reset View: centred, upright, and the stage fitted to the view.
    pub fn reset_view(&mut self) {
        self.pan = Vec2::ZERO;
        self.fit_pending = true;
        self.rotation = 0.0;
        self.display = Default::default();
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
            // 100%: one scene unit per screen point.
            Action::ResetZoom => self.zoom = 1.0,
            Action::ResetRotation => self.rotation = 0.0,
            Action::ResetPan => self.pan = Vec2::ZERO,
            Action::ResetView => self.reset_view(),
            _ => {}
        }
    }
}

/// The zoom that fits a `w` x `h` stage in `rect`, with a margin.
fn fit_zoom(rect: Rect, w: f32, h: f32) -> f32 {
    ((rect.width() - 64.0) / w)
        .min((rect.height() - 64.0) / h)
        .clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Scene units -> screen points for this canvas rect. The zoom is
/// absolute: the rect only places the centre.
fn view_transform(state: &CanvasState, rect: Rect) -> Affine {
    let c = rect.center() + state.pan;
    let scale = state.zoom * state.framing;
    Affine::translate(c.x, c.y)
        .compose(Affine::rotate_deg(state.rotation))
        .compose(Affine::scale(scale, scale))
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

fn paint_stroke(
    painter: &egui::Painter,
    cache: &mut super::textured::Cache,
    textures: &std::collections::BTreeMap<u64, symphony_core::stroke::Texture>,
    stroke: &Stroke,
    colour: Color32,
    m: Affine,
) {
    if let Some(tex) = &stroke.texture {
        cache.paint(painter.ctx(), painter, textures, stroke, tex, colour, m);
        return;
    }
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
                art_layer_toolbar(ui, s, profile);
                if drawing_only {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dense_label(ui, "Selected drawing only, without pegs");
                    });
                }
            });
        });
}

/// Harmony's Art Layer toolbar: preview all, then the art layers back to
/// front as Harmony lists them (Overlay and Underlay only when enabled).
fn art_layer_toolbar(ui: &mut egui::Ui, s: &mut Session, profile: Profile) {
    use crate::session::art_name;
    // Turning the preference off leaves nothing on a hidden art layer.
    if !s.overlay_underlay && matches!(s.art, ArtLayer::Overlay | ArtLayer::Underlay) {
        s.set_art(ArtLayer::LineArt);
    }
    ui.separator();
    let tip = keymap::tooltip(profile, Action::PreviewArtLayers);
    if icons::button(ui, Icon::ArtPreview, s.art_preview, true, &tip).clicked() {
        s.art_preview = !s.art_preview;
    }
    let mut layers = vec![
        (ArtLayer::LineArt, Icon::ArtLine),
        (ArtLayer::ColourArt, Icon::ArtColour),
    ];
    if s.overlay_underlay {
        layers.insert(0, (ArtLayer::Overlay, Icon::ArtOverlay));
        layers.push((ArtLayer::Underlay, Icon::ArtUnderlay));
    }
    for (art, icon) in layers {
        let key = match art {
            ArtLayer::LineArt | ArtLayer::ColourArt => Action::SwitchLineColourArt,
            _ => Action::SwitchOverlayUnderlay,
        };
        let shortcut = keymap::label(profile, key);
        let tip = if shortcut.is_empty() {
            art_name(art).to_owned()
        } else {
            format!("{} ({shortcut} switches)", art_name(art))
        };
        if icons::button(ui, icon, s.art == art, true, &tip).clicked() {
            s.set_art(art);
        }
    }
    ui.add_space(10.0);
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
                    })
                    .response
                    // Screen readers announce it as "Zoom".
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Zoom"));
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
    state.strokes.begin_frame(ui.ctx());
    let (w, h) = (s.doc.settings.width as f32, s.doc.settings.height as f32);

    state.framing = if drawing_only {
        1.0
    } else {
        s.doc.camera_scale(s.frame)
    };

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
                zoom_at(state, rect, p, zoom_delta);
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
            zoom_at(state, rect, p, if alt { 0.8 } else { 1.25 });
        }
    }

    if state.fit_pending && rect.width() > 80.0 && rect.height() > 80.0 {
        state.zoom = fit_zoom(rect, w, h);
        state.fit_pending = false;
    }
    let view = view_transform(state, rect);
    state.last_view = view.inverse().unwrap_or(Affine::IDENTITY);
    let px = 1.0 / scale_of(view).max(1e-6);

    // D4.2: neutral surround. D3.10: the stage is paper with a soft shadow.
    let colours = s.colours;
    // The view's corners are rounded like its panel; the stage stays sharp.
    painter.rect_filled(
        rect,
        f32::from(theme::GLASS_RADIUS) - 4.0,
        if drawing_only {
            CanvasColours::get(colours.surround)
        } else {
            Color32::BLACK
        },
    );
    let (w, h) = (w / state.framing, h / state.framing);
    let corners = [
        (-w / 2.0, -h / 2.0),
        (w / 2.0, -h / 2.0),
        (w / 2.0, h / 2.0),
        (-w / 2.0, h / 2.0),
    ]
    .map(|(x, y)| to_pos(view.apply(x, y)));
    if drawing_only && state.rotation == 0.0 {
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
        CanvasColours::get(colours.stage),
        EStroke::NONE,
    ));

    let colour = |s: &Session, swatch: Id| {
        let [r, g, b, a] = s.doc.swatch_rgba(swatch);
        Color32::from_rgba_unmultiplied(r, g, b, a)
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
        let tints = [
            CanvasColours::get(colours.onion_before),
            CanvasColours::get(colours.onion_after),
        ];
        for (list, tint) in [(before, tints[0]), (after, tints[1])] {
            for (k, d) in list.iter().enumerate().rev() {
                let alpha = 0.45 / (k as f32 + 1.0);
                if let Some(dr) = s.doc.drawings.get(d) {
                    for st in &dr.strokes {
                        paint_stroke(
                            &painter,
                            &mut state.strokes,
                            &s.doc.textures,
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
    if !drawing_only && !state.display.paint(&painter, s, corners, preview) {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Couldn’t render the Display output.",
            theme::body(),
            theme::TEXT_PRIMARY,
        );
    }
    for (_, d, m) in items.iter().filter(|_| drawing_only || s.show_strokes) {
        if let Some(dr) = s.doc.drawings.get(d) {
            let moving = preview.is_some() && s.selection.drawing == Some(*d);
            // Art layers back to front; within each, paint under ink.
            for art in ArtLayer::STACK {
                // The Drawing view shows only the current art layer unless
                // Preview All Art Layers (P) is on, as in Harmony.
                if drawing_only && !s.art_preview && art != s.art {
                    continue;
                }
                if drawing_only {
                    for f in dr.fills.iter().filter(|f| f.art == art) {
                        paint_mesh(
                            &painter,
                            symphony_vector::fill::fill_mesh(&f.contours, view.compose(*m)),
                            colour(s, f.swatch),
                        );
                    }
                }
                for st in dr.strokes.iter().filter(|st| st.art == art) {
                    if !drawing_only && st.kind != StrokeKind::Invisible {
                        continue;
                    }
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
                    paint_stroke(
                        &painter,
                        &mut state.strokes,
                        &s.doc.textures,
                        st,
                        colour(s, st.swatch),
                        m,
                    );
                }
            }
        }
    }
    // Line Editor: the picked points, as dots; Esc clears them.
    if s.tool == Tool::LineEditor {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            s.line_points.clear();
        }
        if let Some(d) = s
            .doc
            .exposed(s.selected, s.frame)
            .and_then(|d| s.doc.drawings.get(&d))
        {
            let m = view.compose(layer_world);
            for (id, i) in &s.line_points {
                let Some(p) = d
                    .strokes
                    .iter()
                    .find(|st| st.id == *id)
                    .and_then(|st| st.points.get(*i))
                else {
                    continue;
                };
                let at = to_pos(m.apply(p.x, p.y));
                painter.circle_filled(at, 4.5, theme::BG_BASE);
                painter.circle_filled(at, 3.0, theme::ACCENT_SELECT);
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
        .as_mut()
        .filter(|a| a.stroke.kind != StrokeKind::Invisible)
    {
        if let Some(back) = a.to_local.inverse() {
            let m = view.compose(back);
            let ink = colour(s, a.stroke.swatch);
            match a.stroke.texture {
                // Textured: stamp only the new part each frame.
                Some(tex) => super::textured::Live::paint(
                    &mut a.live,
                    ui.ctx(),
                    &painter,
                    &s.doc.textures,
                    &a.stroke,
                    &tex,
                    ink,
                    m,
                ),
                None => paint_stroke(
                    &painter,
                    &mut state.strokes,
                    &s.doc.textures,
                    &a.stroke,
                    ink,
                    m,
                ),
            }
        }
    }

    // Pen input.
    let zoom_factor = ui.ctx().zoom_factor();
    let draws = matches!(
        s.tool,
        Tool::Brush
            | Tool::Pencil
            | Tool::Eraser
            | Tool::Paint
            | Tool::CloseGap
            | Tool::Dropper
            | Tool::LineEditor
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
                if !erasing && s.tool == Tool::LineEditor {
                    let Some(drawing) = s.doc.exposed(s.selected, s.frame) else {
                        continue;
                    };
                    let mode = if shift {
                        LineMode::Select
                    } else if !s.line_points.is_empty() {
                        LineMode::MoveSelected
                    } else {
                        LineMode::Push
                    };
                    if mode == LineMode::Select {
                        let r =
                            s.line_edit_radius * size_unit(s, px) / scale_of(layer_world).max(1e-6);
                        s.line_select(drawing, [lx, ly], r);
                    }
                    state.line_edit = Some(LineDrag {
                        drawing,
                        to_local,
                        last: [lx, ly],
                        mode,
                        moved: false,
                        local_px,
                    });
                    continue;
                }
                if erasing {
                    let targets = eraser_targets(s, &items, layer_world);
                    if !targets.is_empty() {
                        state.erasing = Some(Erasing {
                            targets,
                            last: None,
                        });
                        let r = eraser_radius(s, sm.pressure) * size_unit(s, px);
                        erase_at(s, state, (sx, sy), r);
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
                    // A textured brush brings its tip and grain into the
                    // scene, so the scene keeps working on any machine.
                    if let (Some(tex), StrokeKind::Brush) = (settings.texture, kind) {
                        for k in [tex.tip, tex.grain].into_iter().flatten() {
                            crate::brush_import::ensure_texture(&mut s.doc, k);
                        }
                    }
                    let (x, y) = to_local.apply(sx, sy);
                    let p = Point {
                        x,
                        y,
                        pressure: pressure_curve(sm.pressure, s.pressure_gamma),
                    };
                    let mut smoother = Smoother::new(settings.smoothing);
                    let first = smoother.push(p, 0.0).unwrap_or(p);
                    // Sizes are scene units by default, so zooming in makes
                    // the brush look bigger, as on paper. With "Brush size
                    // follows zoom" they are screen points instead.
                    let k = size_unit(s, px) / scale_of(layer_world).max(1e-6);
                    state.active = Some(Active {
                        layer: s.selected,
                        frame: s.frame,
                        to_local,
                        smoother,
                        min_step: 0.5 * local_px,
                        stroke: Stroke {
                            id: s.doc.ids.fresh(),
                            kind,
                            art: s.art,
                            swatch: s.swatch,
                            min_width: settings.min_width * k,
                            max_width: settings.max_width * k,
                            points: vec![first],
                            // Start and end tips are a pencil property.
                            tips: if kind == StrokeKind::Pencil {
                                settings.tips
                            } else {
                                Default::default()
                            },
                            texture: settings.texture.filter(|_| kind == StrokeKind::Brush),
                            shape: Vec::new(),
                        },
                        stencil: settings.stencil.filter(|_| kind == StrokeKind::Pencil),
                        live: None,
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
                        // A thickness stencil ignores pressure; show it live.
                        if let Some(st) = &a.stencil {
                            apply_stencil(&mut a.stroke.points, st);
                        }
                    }
                } else if state.erasing.is_some() {
                    let r = eraser_radius(s, sm.pressure) * size_unit(s, px);
                    erase_at(s, state, (sx, sy), r);
                } else if let Some(le) = state.line_edit.as_mut() {
                    let r = s.line_edit_radius * size_unit(s, px) / scale_of(layer_world).max(1e-6);
                    line_step(s, le, (sx, sy), r);
                }
            }
            Phase::Up => {
                if let Some(mut le) = state.line_edit.take() {
                    // The last stretch, up to where the pen lifted.
                    let r = s.line_edit_radius * size_unit(s, px) / scale_of(layer_world).max(1e-6);
                    line_step(s, &mut le, (sx, sy), r);
                    match le.mode {
                        LineMode::Push if le.moved => {
                            // Slim the pushed lines back to the detail setting.
                            let tolerance =
                                (1.0 - s.stroke_detail).clamp(0.0, 0.9) * 3.0 * le.local_px;
                            s.line_edit_finish(le.drawing, tolerance);
                        }
                        // A click without dragging clears the point selection.
                        LineMode::MoveSelected if !le.moved => s.line_points.clear(),
                        _ => s.end_gesture(),
                    }
                }
                if let Some(mut a) = state.active.take() {
                    // Where the pen lifted is part of the line: without it,
                    // every stroke lost its last stretch and drawn shapes
                    // didn't close. Lifting reports ~0 pressure, so keep
                    // the last pressure.
                    let (x, y) = a.to_local.apply(sx, sy);
                    let pressure = a.stroke.points.last().map_or(1.0, |p| p.pressure);
                    if let Some(p) = a.smoother.push(Point { x, y, pressure }, a.min_step) {
                        a.stroke.points.push(p);
                    }
                    if let Some(p) = a.smoother.finish() {
                        a.stroke.points.push(p);
                    }
                    if let Some(st) = &a.stencil {
                        apply_stencil(&mut a.stroke.points, st);
                    }
                    // Keep only the points that matter (Stroke detail): the
                    // tolerance is in screen pixels, so it means the same at
                    // any zoom, and the line is drawn as a curve through them.
                    if a.stroke.kind != StrokeKind::Invisible {
                        let tolerance_px = (1.0 - s.stroke_detail).clamp(0.0, 0.9) * 3.0;
                        let local_px = a.min_step * 2.0;
                        let span = a.stroke.max_width - a.stroke.min_width;
                        a.stroke.points = symphony_vector::curve::simplify(
                            &a.stroke.points,
                            tolerance_px * local_px,
                            span,
                        );
                    }
                    if a.stroke.kind == StrokeKind::Invisible {
                        // Close Gap: the rough stroke only says where; it isn't kept.
                        let reach = 40.0 * a.min_step / 0.5;
                        s.close_gap(a.layer, a.frame, &a.stroke.points, reach);
                    } else {
                        // The live image stands in until the committed
                        // stroke is stamped, so it never flickers.
                        if let Some(live) = a.live.take() {
                            state.strokes.adopt(a.stroke.id, live);
                        }
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
    if let Some(hint) = hint.filter(|_| drawing_only) {
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
            screen_per_size: size_unit(s, px) / px,
        },
    );
    if !drawing_only {
        camera_render::mask_outside_stage(&painter, rect, corners);
    }
    bottom_toolbar(ui, state, profile);
}

fn zoom_at(state: &mut CanvasState, rect: Rect, p: Pos2, factor: f32) {
    let before = view_transform(state, rect)
        .inverse()
        .map(|inv| inv.apply(p.x, p.y));
    state.zoom = (state.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    if let Some((x, y)) = before {
        let after = view_transform(state, rect).apply(x, y);
        state.pan += p - to_pos(after);
    }
}

/// Erase whole strokes under the eraser. (Cutting strokes in two comes
/// with the vector editing tools.)
/// Eraser radius in screen points for this pen pressure.
/// Scene units per unit of tool size: 1 (sizes are scene units), or the
/// scene size of a screen point when sizes follow the zoom.
fn size_unit(s: &Session, px: f32) -> f32 {
    if s.size_follows_zoom {
        px
    } else {
        1.0
    }
}

fn eraser_radius(s: &Session, pressure: f32) -> f32 {
    s.eraser
        .width_at_pressure(pressure_curve(pressure, s.pressure_gamma))
        * 0.5
}

/// Erase from the last stamp to this one: discs along the segment, so a
/// fast drag leaves no gaps.
/// The drawings one eraser drag works on: the selected layer, or every
/// visible layer in the view ("Erase all layers"), and the drawing at this
/// frame, or all of each layer's drawings ("Erase all frames").
fn eraser_targets(s: &Session, items: &[select::Item], layer_world: Affine) -> Vec<(Id, Affine)> {
    let layers: Vec<(usize, Affine)> = if s.erase_all_layers {
        items
            .iter()
            .filter(|(layer, _, _)| s.layer_visible(*layer))
            .map(|(layer, _, world)| (*layer, *world))
            .collect()
    } else {
        vec![(s.selected, layer_world)]
    };
    let mut out: Vec<(Id, Affine)> = Vec::new();
    for (layer, world) in layers {
        let Some(to_local) = world.inverse() else {
            continue;
        };
        let drawings: Vec<Id> = if s.erase_all_frames {
            s.element_drawings(layer).into_iter().map(|d| d.0).collect()
        } else {
            s.doc.exposed(layer, s.frame).into_iter().collect()
        };
        for d in drawings {
            if !out.iter().any(|(o, _)| *o == d) {
                out.push((d, to_local));
            }
        }
    }
    out
}

/// Erase from the last stamp to this one: discs along the segment, so a
/// fast drag leaves no gaps. `radius` is in scene units.
fn erase_at(s: &mut Session, state: &mut CanvasState, scene_pt: (f32, f32), radius: f32) {
    let Some(e) = state.erasing.as_mut() else {
        return;
    };
    let (x, y) = scene_pt;
    let here = ([x, y], radius.max(0.01));
    // In path order, from the last stamp to this one: the eraser shape
    // joins them into one smooth capsule, and pencil cutting checks along it.
    let mut discs = Vec::new();
    if let Some((p, r)) = e.last {
        let len = (x - p[0]).hypot(y - p[1]);
        let step = (radius.min(r) * 0.5).max(0.01);
        let n = ((len / step).ceil() as usize).min(2000);
        for i in 0..n {
            let t = i as f32 / n as f32;
            discs.push((
                [p[0] + (x - p[0]) * t, p[1] + (y - p[1]) * t],
                r + (radius - r) * t,
            ));
        }
    }
    discs.push(here);
    e.last = Some(here);
    let targets = e.targets.clone();
    for (d, to_local) in targets {
        let k = scale_of(to_local);
        let local: Vec<symphony_vector::cut::Disc> = discs
            .iter()
            .map(|(c, r)| {
                let (lx, ly) = to_local.apply(c[0], c[1]);
                ([lx, ly], r * k)
            })
            .collect();
        s.erase(d, &local);
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
    /// Screen points per unit of tool size, for the size circle.
    screen_per_size: f32,
}

/// Our canvas cursor (D8.11): a crosshair plus the active tool's icon, or a
/// navigation cursor while a shortcut is held. Drawn on the foreground
/// layer, white over a dark outline so it reads on paper and on grey.
/// Draw a brush tip image as the cursor: `r` is half the diameter in
/// screen points. False if the tip isn't available (fall back to a circle).
fn paint_tip_cursor(
    ui: &egui::Ui,
    painter: &egui::Painter,
    at: Pos2,
    r: f32,
    key: u64,
    angle_deg: f32,
    colour: Color32,
) -> bool {
    let id = egui::Id::unique(("tip-cursor", key));
    let handle = ui
        .ctx()
        .data(|d| d.get_temp::<egui::TextureHandle>(id))
        .or_else(|| {
            let t = crate::brush_import::library_texture(key)?;
            let pixels = t
                .pixels
                .iter()
                .map(|&a| Color32::from_white_alpha(a))
                .collect();
            let image = egui::ColorImage {
                size: [t.width as usize, t.height as usize],
                source_size: Vec2::new(t.width as f32, t.height as f32),
                pixels,
            };
            let h = ui
                .ctx()
                .load_texture("tip-cursor", image, egui::TextureOptions::LINEAR);
            ui.ctx().data_mut(|d| d.insert_temp(id, h.clone()));
            Some(h)
        });
    let Some(handle) = handle else {
        return false;
    };
    // The tip's longer side spans the diameter, as when stamping.
    let [w, h] = handle.size();
    let side = w.max(h).max(1) as f32;
    let (hw, hh) = (r * w as f32 / side, r * h as f32 / side);
    let (sin, cos) = (-angle_deg.to_radians()).sin_cos();
    let corner = |x: f32, y: f32| at + Vec2::new(x * cos - y * sin, x * sin + y * cos);
    let mut mesh = egui::Mesh::with_texture(handle.id());
    for (x, y, u, v) in [
        (-hw, -hh, 0.0, 0.0),
        (hw, -hh, 1.0, 0.0),
        (hw, hh, 1.0, 1.0),
        (-hw, hh, 0.0, 1.0),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: corner(x, y),
            uv: Pos2::new(u, v),
            color: colour.gamma_multiply(0.55),
        });
    }
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    painter.add(egui::Shape::mesh(mesh));
    true
}

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
    let light = theme::ON_ACCENT;

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
        Some((s.eraser.max_width * 0.5 * m.screen_per_size).max(1.5))
    } else if s.tool == Tool::LineEditor {
        Some((s.line_edit_radius * m.screen_per_size).max(1.5))
    } else if s.brush_size_cursor && matches!(s.tool, Tool::Brush | Tool::Pencil) {
        let b = if s.tool == Tool::Pencil {
            s.pencil
        } else {
            s.brush
        };
        Some((b.max_width * 0.5 * m.screen_per_size).max(1.5))
    } else {
        None
    };
    // A textured brush with a tip image shows the tip itself: its shape,
    // size and angle, faintly, so you see what a stamp will look like.
    let tip = match (erasing, s.tool) {
        (false, Tool::Brush) => s.brush.texture.and_then(|t| t.tip.map(|k| (k, t.angle))),
        _ => None,
    };
    // A soft shadow under a white ring keeps it visible on any surface.
    let ring_with_shadow = |r: f32, colour: Color32| {
        for (w, a, dy) in [(4.0, 0.18, 1.0), (2.5, 0.35, 0.5)] {
            painter.circle_stroke(
                p + Vec2::new(0.0, dy),
                r,
                EStroke::new(w, dark.gamma_multiply(a)),
            );
        }
        painter.circle_stroke(p, r, EStroke::new(1.25, colour));
    };
    match (radius, tip) {
        (Some(r), Some((key, angle))) => {
            if !paint_tip_cursor(ui, &painter, p, r, key, angle, light) {
                ring_with_shadow(r, light);
            }
        }
        (Some(r), None) => {
            ring_with_shadow(r, if erasing { theme::STATE_DANGER } else { light });
        }
        _ => {}
    }
    if radius.is_some() {
        // The ring is the cursor (no crosshair); a tiny dot marks the centre
        // so even a very small brush shows where it lands.
        painter.circle_filled(p, 2.0, dark.gamma_multiply(0.5));
        painter.circle_filled(p, 1.0, light);
    } else {
        // Tools without a size ring keep a crosshair for aiming.
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
            Tool::LineEditor => Icon::LineEdit,
            _ => Icon::Brush,
        }
    };
    badge(icon, p + Vec2::new(16.0, 16.0), 12.0);
}
