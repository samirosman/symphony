//! Camera and Drawing views: one canvas, two modes (DESIGN D3.6).
//! - Camera shows the composited scene (every visible layer, with pegs).
//! - Drawing shows only the selected layer's drawing, untransformed.
//!
//! Drawing on an empty cell creates the next numbered drawing, like Harmony.

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke as EStroke, StrokeKind, Vec2};
use symphony_core::math::Affine;
use symphony_pen::{PenSample, Phase, Tool};

use crate::scene::{Scene, Stroke, StrokePoint, Tool as SceneTool, STAGE_H, STAGE_W};
use crate::theme;

/// Eraser radius, in screen points.
const ERASER_RADIUS: f32 = 10.0;

#[derive(Default)]
pub struct CanvasState {
    active: Option<Active>,
    erasing: Option<((String, String), Affine)>,
}

struct Active {
    key: (String, String),
    /// Scene -> drawing-local.
    to_local: Affine,
    stroke: Stroke,
}

impl CanvasState {
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.erasing.is_some()
    }
}

/// Where the stage sits inside `rect`, and scene-units -> points scale.
fn fit_stage(rect: Rect) -> (Rect, f32) {
    let s = ((rect.width() - 24.0) / STAGE_W)
        .min((rect.height() - 24.0) / STAGE_H)
        .max(0.01);
    (
        Rect::from_center_size(rect.center(), Vec2::new(STAGE_W * s, STAGE_H * s)),
        s,
    )
}

fn to_screen(stage: Rect, s: f32, (x, y): (f32, f32)) -> Pos2 {
    stage.center() + Vec2::new(x * s, y * s)
}

fn to_scene(stage: Rect, s: f32, p: Pos2) -> (f32, f32) {
    let d = (p - stage.center()) / s;
    (d.x, d.y)
}

/// Rough uniform scale of a transform, for line widths.
fn scale_of(m: Affine) -> f32 {
    (m.a * m.d - m.b * m.c).abs().sqrt()
}

fn draw_stroke(
    painter: &egui::Painter,
    stroke: &Stroke,
    colour: Color32,
    m: Affine,
    stage: Rect,
    s: f32,
) {
    let k = scale_of(m) * s;
    let width = |p: &StrokePoint| {
        (stroke.min_width + (stroke.max_width - stroke.min_width) * p.pressure) * k
    };
    let pts: Vec<Pos2> = stroke
        .points
        .iter()
        .map(|p| to_screen(stage, s, m.apply(p.x, p.y)))
        .collect();
    if let [only] = pts.as_slice() {
        painter.circle_filled(*only, width(&stroke.points[0]) * 0.5, colour);
        return;
    }
    for (i, w) in pts.windows(2).enumerate() {
        let wd = (width(&stroke.points[i]) + width(&stroke.points[i + 1])) * 0.5;
        painter.line_segment([w[0], w[1]], EStroke::new(wd, colour));
        painter.circle_filled(w[1], wd * 0.5, colour);
    }
}

/// When no native pen backend exists, turn egui's pointer into samples (E4.2).
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
            tool: Tool::Mouse,
            buttons: 0,
            time: i.time,
        }]
    })
}

pub fn ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    state: &mut CanvasState,
    samples: &[PenSample],
    drawing_only: bool,
) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;
    let (stage, s) = fit_stage(rect);
    let zoom = ui.ctx().zoom_factor();

    // D4.2: neutral surround, white stage.
    painter.rect_filled(rect, 0.0, theme::CANVAS_SURROUND);
    painter.rect_filled(stage, 0.0, theme::CANVAS_STAGE);
    painter.rect_stroke(
        stage,
        0.0,
        EStroke::new(1.0, theme::BG_BASE),
        StrokeKind::Outside,
    );

    // What to draw: (key, transform) back to front.
    let items: Vec<((String, String), Affine)> = if drawing_only {
        let col = scene.column_name(scene.selected).to_owned();
        let exposed = scene
            .xsheet
            .column(&col)
            .and_then(|c| c.get(scene.frame))
            .map(str::to_owned);
        exposed
            .map(|d| vec![((col, d), Affine::IDENTITY)])
            .unwrap_or_default()
    } else {
        scene.sync_visibility();
        scene
            .graph
            .evaluate(scene.display, &scene.xsheet, scene.frame)
            .map(|v| {
                v.into_iter()
                    .map(|i| ((i.column, i.drawing), i.transform))
                    .collect()
            })
            .unwrap_or_default()
    };

    // Artwork colours come from the scene's palette, not the UI theme (D4.3).
    let colour_of = |idx: usize, scene: &Scene| {
        let [r, g, b] = scene.palette.get(idx).map(|sw| sw.rgb).unwrap_or_default();
        Color32::from_rgb(r, g, b)
    };
    let clip = painter.with_clip_rect(stage);
    for (key, m) in &items {
        if let Some(strokes) = scene.drawings.get(key) {
            for st in strokes {
                draw_stroke(&clip, st, colour_of(st.colour, scene), *m, stage, s);
            }
        }
    }
    if let Some(a) = &state.active {
        if let Some(back) = a.to_local.inverse() {
            draw_stroke(
                &clip,
                &a.stroke,
                colour_of(a.stroke.colour, scene),
                back,
                stage,
                s,
            );
        }
    }

    // Input. Only working tools draw (Tools toolbar disables the rest).
    let hovered = response.hovered() || state.busy();
    let can_draw = scene.tool.planned_for().is_none();
    for sm in samples {
        let pos = Pos2::new(sm.x as f32 / zoom, sm.y as f32 / zoom);
        let scene_pt = to_scene(stage, s, pos);
        let erasing_tool = sm.tool == Tool::Eraser || scene.tool == SceneTool::Eraser;
        match sm.phase {
            Phase::Down if hovered && can_draw && rect.contains(pos) => {
                let key = scene.drawing_for_edit();
                let world = if drawing_only {
                    Affine::IDENTITY
                } else {
                    scene
                        .graph
                        .world_transform(scene.layers[scene.selected].node, scene.frame)
                        .unwrap_or(Affine::IDENTITY)
                };
                let Some(to_local) = world.inverse() else {
                    continue;
                };
                if erasing_tool {
                    state.erasing = Some((key, to_local));
                    erase(scene, state, scene_pt, ERASER_RADIUS / s);
                } else {
                    let (x, y) = to_local.apply(scene_pt.0, scene_pt.1);
                    state.active = Some(Active {
                        key,
                        to_local,
                        stroke: Stroke {
                            points: vec![StrokePoint {
                                x,
                                y,
                                pressure: sm.pressure,
                            }],
                            colour: scene.brush.colour,
                            max_width: scene.brush.max_width / scale_of(world).max(1e-3),
                            min_width: scene.brush.min_width / scale_of(world).max(1e-3),
                        },
                    });
                }
            }
            Phase::Move => {
                if let Some(a) = state.active.as_mut() {
                    let (x, y) = a.to_local.apply(scene_pt.0, scene_pt.1);
                    a.stroke.points.push(StrokePoint {
                        x,
                        y,
                        pressure: sm.pressure,
                    });
                } else if state.erasing.is_some() {
                    erase(scene, state, scene_pt, ERASER_RADIUS / s);
                }
            }
            Phase::Up => {
                if let Some(a) = state.active.take() {
                    scene
                        .drawings
                        .entry(a.key.clone())
                        .or_default()
                        .push(a.stroke);
                    scene.history.push(a.key);
                }
                state.erasing = None;
            }
            _ => {}
        }
    }

    // Eraser cursor (D7.3: destructive tools tint red while in use).
    if let Some(last) = samples.last() {
        if last.tool == Tool::Eraser || scene.tool == SceneTool::Eraser {
            let p = Pos2::new(last.x as f32 / zoom, last.y as f32 / zoom);
            if rect.contains(p) {
                painter.circle_stroke(p, ERASER_RADIUS, EStroke::new(1.0, theme::STATE_DANGER));
            }
        }
    }

    // D10.4: an empty cell invites drawing.
    let col = scene.column_name(scene.selected);
    if scene
        .xsheet
        .column(col)
        .and_then(|c| c.get(scene.frame))
        .is_none()
    {
        let hint = format!("Draw here to create a drawing on {col}");
        // On the surround, not the paper: text.primary on canvas.surround is 5.9:1 (D4.4).
        painter.text(
            rect.left_top() + Vec2::new(8.0, 4.0),
            Align2::LEFT_TOP,
            hint,
            theme::dense(),
            theme::TEXT_PRIMARY,
        );
    }
}

/// Remove whole strokes near `scene_pt` in the drawing being erased.
/// (Real vector erasing that cuts strokes comes with the M3 engine.)
fn erase(scene: &mut Scene, state: &CanvasState, scene_pt: (f32, f32), radius: f32) {
    let Some((key, to_local)) = &state.erasing else {
        return;
    };
    let (x, y) = to_local.apply(scene_pt.0, scene_pt.1);
    let r = radius * scale_of(*to_local);
    if let Some(strokes) = scene.drawings.get_mut(key) {
        strokes.retain(|st| {
            !st.points
                .iter()
                .any(|p| (p.x - x).hypot(p.y - y) <= r + st.max_width * 0.5)
        });
    }
}
