//! UI tests: drive the real Camera view with simulated pen input, the way
//! an artist would, and check the scene that results. These catch bugs in
//! the wiring between input, tools and the document that unit tests of
//! each part can't see.

use eframe::egui::{self, Pos2, Vec2};
use egui_kittest::Harness;
use symphony_core::stroke::{ArtLayer, StrokeKind};
use symphony_pen::{PenSample, Phase, Tool as PenTool};

use crate::keymap::Profile;
use crate::session::{Session, Tool};
use crate::views::camera::{self, CanvasState};

pub struct Rig {
    pub s: Session,
    pub canvas: CanvasState,
    samples: Vec<PenSample>,
}

pub fn rig() -> Harness<'static, Rig> {
    let mut s = Session::new_scene();
    s.animate = false;
    let rig = Rig {
        s,
        canvas: CanvasState::default(),
        samples: Vec::new(),
    };
    let mut h = Harness::builder()
        .with_size(Vec2::new(1000.0, 700.0))
        .build_ui_state(
            |ui, r: &mut Rig| {
                let samples = std::mem::take(&mut r.samples);
                camera::ui(
                    ui,
                    &mut r.s,
                    &mut r.canvas,
                    &samples,
                    false,
                    Profile::Harmony,
                );
            },
            rig,
        );
    h.run_steps(2);
    h
}

fn sample(p: Pos2, phase: Phase, pressure: f32, tool: PenTool) -> PenSample {
    PenSample {
        x: p.x as f64,
        y: p.y as f64,
        pressure,
        tilt_x: 0.0,
        tilt_y: 0.0,
        rotation: 0.0,
        tangential: 0.0,
        phase,
        tool,
        buttons: 0,
        time: 0.0,
    }
}

/// Drag through scene points with the current tool (or the pen's eraser
/// end when `tool` is `PenTool::Eraser`), one frame per point.
pub fn drag(h: &mut Harness<'static, Rig>, scene: &[(f32, f32)], pressure: f32, tool: PenTool) {
    let n = scene.len();
    for (i, &(x, y)) in scene.iter().enumerate() {
        let p = h.state().canvas.to_screen(x, y);
        let phase = match i {
            0 => Phase::Down,
            _ if i + 1 == n => Phase::Up,
            _ => Phase::Move,
        };
        h.input_mut().events.push(egui::Event::PointerMoved(p));
        h.state_mut().samples.push(sample(p, phase, pressure, tool));
        h.step();
    }
    h.run_steps(2);
}

/// A straight line of `steps` points from a to b.
pub fn line(a: (f32, f32), b: (f32, f32), steps: usize) -> Vec<(f32, f32)> {
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        })
        .collect()
}

pub fn click(h: &mut Harness<'static, Rig>, x: f32, y: f32) {
    drag(h, &[(x, y), (x, y)], 1.0, PenTool::Pen);
}

/// The strokes of the drawing at frame 1 of layer `layer`.
pub fn strokes(h: &Harness<'static, Rig>, layer: usize) -> Vec<symphony_core::stroke::Stroke> {
    let s = &h.state().s;
    s.doc
        .exposed(layer, 1)
        .and_then(|d| s.doc.drawings.get(&d))
        .map(|d| d.strokes.clone())
        .unwrap_or_default()
}

pub fn fills(h: &Harness<'static, Rig>, layer: usize) -> Vec<symphony_core::stroke::Fill> {
    let s = &h.state().s;
    s.doc
        .exposed(layer, 1)
        .and_then(|d| s.doc.drawings.get(&d))
        .map(|d| d.fills.clone())
        .unwrap_or_default()
}

/// A closed square drawn with the brush, from (x0,y0) to (x1,y1).
pub fn draw_square(h: &mut Harness<'static, Rig>, x0: f32, y0: f32, x1: f32, y1: f32) {
    h.state_mut().s.tool = Tool::Brush;
    let mut pts = line((x0, y0), (x1, y0), 8);
    pts.extend(line((x1, y0), (x1, y1), 8));
    pts.extend(line((x1, y1), (x0, y1), 8));
    pts.extend(line((x0, y1), (x0, y0), 8));
    drag(h, &pts, 1.0, PenTool::Pen);
}

#[test]
fn drawing_a_stroke_adds_one_brush_stroke() {
    let mut h = rig();
    h.state_mut().s.tool = Tool::Brush;
    drag(
        &mut h,
        &line((-100.0, 0.0), (100.0, 0.0), 20),
        1.0,
        PenTool::Pen,
    );
    let st = strokes(&h, 0);
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].kind, StrokeKind::Brush);
    assert_eq!(st[0].art, ArtLayer::LineArt);
    assert!(st[0].points.len() >= 2);
    // The line ends where the pen lifted (it used to stop short).
    let end = st[0].points.last().unwrap();
    assert!((end.x - 100.0).abs() < 0.5 && end.y.abs() < 0.5, "{end:?}");
}

#[test]
fn brush_size_is_fixed_to_the_drawing_by_default() {
    let mut h = rig();
    let size = h.state().s.brush.max_width;
    h.state_mut().canvas.set_zoom(2.0);
    h.run_steps(2);
    h.state_mut().s.tool = Tool::Brush;
    drag(
        &mut h,
        &line((-50.0, 0.0), (50.0, 0.0), 10),
        1.0,
        PenTool::Pen,
    );
    let st = strokes(&h, 0);
    // Width in drawing units equals the setting, whatever the zoom.
    assert!(
        (st[0].max_width - size).abs() < 0.01,
        "{} vs {size}",
        st[0].max_width
    );

    // With "Brush size follows zoom", sizes are screen points: the line's
    // width in the drawing is the size divided by the on-screen scale.
    h.state_mut().s.size_follows_zoom = true;
    let c = &h.state().canvas;
    let screen_per_scene = c.to_screen(100.0, 0.0).distance(c.to_screen(0.0, 0.0)) / 100.0;
    drag(
        &mut h,
        &line((-50.0, 40.0), (50.0, 40.0), 10),
        1.0,
        PenTool::Pen,
    );
    let want = size / screen_per_scene;
    let st = strokes(&h, 0);
    assert!(
        (st[1].max_width - want).abs() < 0.05,
        "{} vs {want}",
        st[1].max_width
    );
    // Zooming in further makes newly drawn lines finer in the drawing.
    h.state_mut().canvas.set_zoom(4.0);
    h.run_steps(2);
    drag(
        &mut h,
        &line((-30.0, 10.0), (30.0, 10.0), 10),
        1.0,
        PenTool::Pen,
    );
    let st = strokes(&h, 0);
    assert!(
        (st[2].max_width - want / 2.0).abs() < 0.05,
        "{} vs {}",
        st[2].max_width,
        want / 2.0
    );
}

#[test]
fn painting_inside_a_closed_shape_fills_it() {
    let mut h = rig();
    draw_square(&mut h, -100.0, -100.0, 100.0, 100.0);
    h.state_mut().s.tool = Tool::Paint;
    click(&mut h, 0.0, 0.0);
    let f = fills(&h, 0);
    let st = strokes(&h, 0);
    assert_eq!(
        f.len(),
        1,
        "one painted zone; notice {:?}; ends {:?} {:?}",
        h.state().s.notice,
        st.first().map(|s| s.points[0]),
        st.first().and_then(|s| s.points.last().copied())
    );
    assert!(f[0].contains(0.0, 0.0));
    assert!(!f[0].contains(150.0, 0.0));
}

/// Drag the eraser (the Eraser tool, full pressure) along a line.
fn erase_line(h: &mut Harness<'static, Rig>, a: (f32, f32), b: (f32, f32)) {
    h.state_mut().s.tool = Tool::Eraser;
    drag(h, &line(a, b, 6), 1.0, PenTool::Pen);
}

fn brush_line(h: &mut Harness<'static, Rig>, a: (f32, f32), b: (f32, f32)) {
    h.state_mut().s.tool = Tool::Brush;
    drag(h, &line(a, b, 20), 1.0, PenTool::Pen);
}

#[test]
fn erasing_across_a_brush_line_splits_it_with_the_eraser_shape() {
    let mut h = rig();
    brush_line(&mut h, (-200.0, 0.0), (200.0, 0.0));
    erase_line(&mut h, (0.0, -60.0), (0.0, 60.0));
    let st = strokes(&h, 0);
    assert_eq!(st.len(), 2, "cut in two");
    for piece in &st {
        assert!(!piece.shape.is_empty(), "brush pieces are outlines");
        assert!(
            !symphony_vector::hits(piece, 0.0, 0.0, 0.0),
            "nothing left under the eraser"
        );
    }
    // The eraser end of the pen erases too, whatever tool is active.
    h.state_mut().s.tool = Tool::Brush;
    drag(
        &mut h,
        &line((-150.0, -60.0), (-150.0, 60.0), 6),
        1.0,
        PenTool::Eraser,
    );
    assert_eq!(strokes(&h, 0).len(), 3, "the pen's eraser end cut again");
}

#[test]
fn erasing_paint_leaves_a_hole() {
    let mut h = rig();
    draw_square(&mut h, -150.0, -150.0, 150.0, 150.0);
    h.state_mut().s.tool = Tool::Paint;
    click(&mut h, 0.0, 0.0);
    assert!(fills(&h, 0)[0].contains(0.0, 0.0));
    erase_line(&mut h, (-10.0, 0.0), (10.0, 0.0));
    let f = fills(&h, 0);
    assert!(!f.is_empty(), "the rest of the paint stays");
    assert!(
        !f.iter().any(|f| f.contains(0.0, 0.0)),
        "a hole where the eraser went"
    );
    assert!(f.iter().any(|f| f.contains(100.0, 100.0)));
}

#[test]
fn erase_all_layers_reaches_through_every_visible_layer() {
    let mut h = rig();
    brush_line(&mut h, (-200.0, 0.0), (200.0, 0.0));
    h.state_mut().s.add_layer();
    let top = h.state().s.selected;
    brush_line(&mut h, (-200.0, 0.0), (200.0, 0.0));
    let bottom = if top == 0 { 1 } else { 0 };
    // Only the selected layer by default.
    erase_line(&mut h, (-100.0, -60.0), (-100.0, 60.0));
    assert_eq!(strokes(&h, top).len(), 2);
    assert_eq!(strokes(&h, bottom).len(), 1);
    // Every visible layer with the option on.
    h.state_mut().s.erase_all_layers = true;
    erase_line(&mut h, (100.0, -60.0), (100.0, 60.0));
    assert_eq!(strokes(&h, top).len(), 3);
    assert_eq!(strokes(&h, bottom).len(), 2);
}

#[test]
fn erase_all_frames_reaches_every_drawing_of_the_layer() {
    let mut h = rig();
    brush_line(&mut h, (-200.0, 0.0), (200.0, 0.0));
    h.state_mut().s.frame = 2;
    brush_line(&mut h, (-200.0, 0.0), (200.0, 0.0));
    let s = &h.state().s;
    let (d1, d2) = (s.doc.exposed(0, 1).unwrap(), s.doc.exposed(0, 2).unwrap());
    assert_ne!(d1, d2, "two drawings");
    let count = |h: &Harness<'static, Rig>, d| h.state().s.doc.drawings[&d].strokes.len();
    h.state_mut().s.erase_all_frames = true;
    erase_line(&mut h, (0.0, -60.0), (0.0, 60.0));
    assert_eq!(count(&h, d1), 2);
    assert_eq!(count(&h, d2), 2);
    // One drag is one undo, across all the drawings it touched.
    h.state_mut().s.undo();
    assert_eq!(count(&h, d1), 1);
    assert_eq!(count(&h, d2), 1);
}

/// Renders an erased line to a PNG for a human look (not a pass/fail
/// check): `SYMPHONY_UI_SHOT=path cargo test -p symphony-app erased_line_look`.
#[test]
fn erased_line_look() {
    let Ok(path) = std::env::var("SYMPHONY_UI_SHOT") else {
        return;
    };
    let mut h = rig();
    h.state_mut().s.brush.max_width = 40.0;
    h.state_mut().s.brush.min_width = 40.0;
    brush_line(&mut h, (-300.0, 0.0), (300.0, 0.0));
    h.state_mut().s.eraser.max_width = 50.0;
    erase_line(&mut h, (-120.0, -80.0), (-120.0, 80.0));
    erase_line(&mut h, (60.0, -40.0), (140.0, -10.0));
    let s = &h.state().s;
    let img = symphony_render::render_frame(&s.doc, 1, 960, 540).unwrap();
    img.save(path).unwrap();
}

#[test]
fn resizing_the_view_keeps_the_drawing_scale() {
    let mut h = rig();
    let scale = |h: &Harness<'static, Rig>| {
        let c = &h.state().canvas;
        c.to_screen(100.0, 0.0).distance(c.to_screen(0.0, 0.0)) / 100.0
    };
    let fitted = scale(&h);
    assert!(
        fitted > 0.0 && fitted < 1.0,
        "the 1920-wide stage was fitted into 1000 px: {fitted}"
    );
    // Dragging a panel edge: the view shrinks, the drawing doesn't.
    for w in [900.0, 760.0, 640.0] {
        h.set_size(Vec2::new(w, 700.0));
        h.run_steps(2);
        assert!(
            (scale(&h) - fitted).abs() < 1e-4,
            "scale changed at width {w}"
        );
    }
    // 100% is actual size: one scene unit per screen point.
    h.state_mut().canvas.apply(crate::keymap::Action::ResetZoom);
    h.run_steps(2);
    assert!((scale(&h) - 1.0).abs() < 1e-4);
    // Reset View fits the stage to the (now smaller) view again.
    h.state_mut().canvas.apply(crate::keymap::Action::ResetView);
    h.run_steps(2);
    assert!(scale(&h) < fitted, "refitted to the narrower view");
}

/// Frame cost of a busy scene, still and while the view is being resized.
/// Not a pass/fail test: `cargo test --release -p symphony-app
/// frame_cost -- --ignored --nocapture`.
#[test]
#[ignore]
fn frame_cost() {
    use symphony_core::stroke::{Point, Stroke};
    let mut h = rig();
    // 600 pressure strokes of 60 points each, like a dense character page.
    {
        let s = &mut h.state_mut().s;
        let swatch = s.swatch;
        for i in 0..600 {
            let y = -400.0 + (i % 60) as f32 * 13.0;
            let x0 = -700.0 + (i / 60) as f32 * 140.0;
            let st = Stroke {
                id: s.doc.ids.fresh(),
                kind: StrokeKind::Brush,
                art: Default::default(),
                swatch,
                min_width: 1.0,
                max_width: 6.0,
                points: (0..60)
                    .map(|k| {
                        let t = k as f32 / 59.0;
                        Point {
                            x: x0 + t * 120.0,
                            y: y + (t * 9.0).sin() * 5.0,
                            pressure: (t * 3.1).sin(),
                        }
                    })
                    .collect(),
                tips: Default::default(),
                texture: None,
                shape: Vec::new(),
            };
            s.commit_stroke(0, 1, st);
        }
    }
    h.run_steps(3);
    let time = |h: &mut Harness<'static, Rig>, resize: bool| {
        let n = 30;
        let start = std::time::Instant::now();
        for i in 0..n {
            if resize {
                h.set_size(Vec2::new(1000.0 - (i % 15) as f32 * 8.0, 700.0));
            }
            h.step();
        }
        start.elapsed() / n
    };
    let still = time(&mut h, false);
    let resizing = time(&mut h, true);
    println!("frame_cost: still {still:?} per frame, resizing {resizing:?} per frame");
}

#[test]
fn drawn_lines_keep_few_points_and_follow_the_stroke_detail_setting() {
    // A gentle curve drawn with many pen samples.
    let curve: Vec<(f32, f32)> = (0..=120)
        .map(|i| {
            let t = i as f32 / 120.0;
            (-200.0 + t * 400.0, (t * std::f32::consts::PI).sin() * 80.0)
        })
        .collect();
    let mut h = rig();
    h.state_mut().s.tool = Tool::Brush;
    h.state_mut().s.brush.smoothing = 0.0;
    h.state_mut().s.stroke_detail = 1.0;
    drag(&mut h, &curve, 1.0, PenTool::Pen);
    let full = strokes(&h, 0)[0].points.len();
    h.state_mut().s.stroke_detail = 0.3;
    drag(&mut h, &curve, 1.0, PenTool::Pen);
    let light = strokes(&h, 0)[1].points.len();
    assert!(full > 100, "full detail keeps every sample: {full}");
    assert!(
        light * 4 < full,
        "lower detail keeps far fewer: {light} of {full}"
    );
}

/// Renders a lightly kept curve big, for a human look at its smoothness:
/// `SYMPHONY_UI_SHOT=path cargo test -p symphony-app curve_look`.
#[test]
fn curve_look() {
    let Ok(path) = std::env::var("SYMPHONY_UI_SHOT") else {
        return;
    };
    let mut h = rig();
    h.state_mut().s.tool = Tool::Brush;
    h.state_mut().s.brush.max_width = 30.0;
    h.state_mut().s.brush.min_width = 4.0;
    h.state_mut().s.stroke_detail = 0.3;
    let curve: Vec<(f32, f32)> = (0..=150)
        .map(|i| {
            let t = i as f32 / 150.0;
            (-600.0 + t * 1200.0, (t * 9.0).sin() * 250.0)
        })
        .collect();
    drag(&mut h, &curve, 1.0, PenTool::Pen);
    let s = &h.state().s;
    eprintln!(
        "kept {} of {} points",
        strokes(&h, 0)[0].points.len(),
        curve.len()
    );
    symphony_render::render_frame(&s.doc, 1, 1920, 1080)
        .unwrap()
        .save(path)
        .unwrap();
}

#[test]
fn erasing_a_textured_line_keeps_its_texture_and_cuts_its_shape() {
    use symphony_core::stroke::StrokeTexture;
    let mut h = rig();
    h.state_mut().s.brush.texture = Some(StrokeTexture {
        tip: None,
        grain: None,
        spacing: 0.1,
        opacity: 1.0,
        angle: 0.0,
        follow: false,
        grain_scale: 1.0,
        flow: 1.0,
        hardness: 1.0,
        opacity_by_pressure: [1.0; 4],
        ..Default::default()
    });
    h.state_mut().s.brush.min_width = 20.0;
    h.state_mut().s.brush.max_width = 20.0;
    brush_line(&mut h, (-200.0, 0.0), (200.0, 0.0));
    erase_line(&mut h, (0.0, -60.0), (0.0, 60.0));
    let st = strokes(&h, 0);
    assert_eq!(st.len(), 2, "cut in two");
    for piece in &st {
        assert!(piece.texture.is_some(), "still textured");
        assert!(!piece.shape.is_empty(), "clipped to an outline");
        // Nothing of it is stamped where the eraser went.
        let c = symphony_vector::stamp::rasterize(
            piece,
            piece.texture.as_ref().unwrap(),
            None,
            None,
            symphony_core::math::Affine::IDENTITY,
            [-1000, -1000, 1000, 1000],
        )
        .unwrap();
        let (cx, cy) = (0 - c.x, 0 - c.y);
        if cx >= 0 && cy >= 0 && cx < c.width as i32 && cy < c.height as i32 {
            assert_eq!(c.data[(cy * c.width as i32 + cx) as usize], 0);
        }
    }
}

#[test]
fn line_editor_pushes_lines_and_moves_shift_picked_points() {
    let mut h = rig();
    h.state_mut().s.tool = Tool::Pencil;
    h.state_mut().s.pencil.smoothing = 0.0;
    drag(
        &mut h,
        &line((-200.0, 0.0), (200.0, 0.0), 20),
        1.0,
        PenTool::Pen,
    );
    let before = strokes(&h, 0)[0].clone();

    // Push the middle of the line downwards.
    h.state_mut().s.tool = Tool::LineEditor;
    h.state_mut().s.line_edit_radius = 60.0;
    drag(&mut h, &line((0.0, 0.0), (0.0, 30.0), 6), 1.0, PenTool::Pen);
    let pushed = strokes(&h, 0)[0].clone();
    let y_near = |st: &symphony_core::stroke::Stroke, x: f32| {
        st.points
            .iter()
            .min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()))
            .map(|p| p.y)
            .unwrap()
    };
    assert!(
        y_near(&pushed, 0.0) > 20.0,
        "the middle moved: {}",
        y_near(&pushed, 0.0)
    );
    assert!(y_near(&pushed, -200.0).abs() < 0.5, "the ends stayed");
    // One drag, one undo.
    h.state_mut().s.undo();
    assert_eq!(strokes(&h, 0)[0], before);

    // Shift-drag over the right end picks its points...
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    drag(
        &mut h,
        &line((190.0, 0.0), (200.0, 0.0), 2),
        1.0,
        PenTool::Pen,
    );
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    assert!(!h.state().s.line_points.is_empty(), "points picked");
    // ...then a plain drag moves only those.
    drag(&mut h, &line((0.0, 0.0), (0.0, 50.0), 4), 1.0, PenTool::Pen);
    let moved = strokes(&h, 0)[0].clone();
    assert!(y_near(&moved, 200.0) > 40.0, "the picked end moved");
    assert!(y_near(&moved, -200.0).abs() < 0.5, "the other end didn't");
}

/// Cost of Line Editor pushes on a busy page. Not pass/fail:
/// `cargo test --release -p symphony-app line_edit_cost -- --ignored --nocapture`.
#[test]
#[ignore]
fn line_edit_cost() {
    use symphony_core::stroke::{Point, Stroke};
    let mut s = crate::session::Session::new_scene();
    let swatch = s.swatch;
    for i in 0..600 {
        let y = -400.0 + (i % 60) as f32 * 13.0;
        let x0 = -700.0 + (i / 60) as f32 * 140.0;
        let st = Stroke {
            id: s.doc.ids.fresh(),
            kind: StrokeKind::Pencil,
            art: Default::default(),
            swatch,
            min_width: 2.0,
            max_width: 2.0,
            points: (0..60)
                .map(|k| Point {
                    x: x0 + k as f32 * 2.0,
                    y,
                    pressure: 1.0,
                })
                .collect(),
            tips: Default::default(),
            texture: None,
            shape: Vec::new(),
        };
        s.commit_stroke(0, 1, st);
    }
    let d = s.doc.exposed(0, 1).unwrap();
    let start = std::time::Instant::now();
    for k in 0..60 {
        s.line_push(d, [-650.0 + k as f32, -400.0], 30.0, [1.0, 0.5]);
    }
    s.line_edit_finish(d, 0.5);
    println!("line_edit_cost: {:?} per push step", start.elapsed() / 60);
}

/// Symphony profile: Brush and Pencil are modes of one Pen tool, switched
/// at the top of Tool Properties; each mode keeps its own settings.
#[test]
fn pen_tool_switches_between_brush_and_pencil_modes() {
    use egui_kittest::kittest::Queryable;
    let mut s = Session::new_scene();
    s.pen_modes = true;
    s.tool = Tool::Brush;
    s.pencil.max_width = 7.0;
    s.brush.max_width = 19.0;
    // The size sliders' value boxes have no accessible name yet (a known
    // gap, tracked separately), so this test skips that check.
    let mut h = Harness::builder()
        .with_size(Vec2::new(360.0, 700.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, st: &mut (Session, crate::presets::Presets)| {
                crate::views::panels::tool_properties(ui, &mut st.0, &mut st.1, None);
            },
            (s, crate::presets::Presets::default()),
        );
    h.run_steps(2);
    // One tool, named Pen, with both modes on offer.
    h.get_by_label("Pen");
    h.get_by_label("Pencil").click();
    h.run_steps(2);
    assert_eq!(h.state().0.tool, Tool::Pencil);
    h.get_by_label("Brush").click();
    h.run_steps(2);
    assert_eq!(h.state().0.tool, Tool::Brush);
    // Switching modes never mixes their settings.
    assert_eq!(h.state().0.pencil.max_width, 7.0);
    assert_eq!(h.state().0.brush.max_width, 19.0);
}

/// Searching a blend mode adds a Blend Modes node straight onto the
/// selected layer: wired in, visible in the render, one undo removes it.
#[test]
fn search_puts_a_blend_mode_on_the_selected_layer() {
    use symphony_core::graph::{BlendMode, Effect, NodeKind};
    use symphony_core::stroke::{Point, Stroke};
    let mut s = Session::new_scene();
    // The same thick line on two layers, so the top one has something
    // behind it to blend with.
    s.add_layer();
    let swatch = s.swatch;
    for l in 0..2 {
        let st = Stroke {
            id: s.doc.ids.fresh(),
            kind: StrokeKind::Pencil,
            art: Default::default(),
            swatch,
            min_width: 40.0,
            max_width: 40.0,
            points: [(-300.0, 0.0), (300.0, 0.0)]
                .iter()
                .map(|&(x, y)| Point {
                    x,
                    y,
                    pressure: 1.0,
                })
                .collect(),
            tips: Default::default(),
            texture: None,
            shape: Vec::new(),
        };
        s.commit_stroke(l, s.frame, st);
    }
    s.selected = 0;
    let layer = s.doc.layers[0];
    let links = s.doc.graph.links().to_vec();
    let before = symphony_render::render_camera(&s.doc, s.frame, 160, 90, None).unwrap();
    let mut view = crate::views::node::NodeViewState::default();
    crate::views::node::add_from_search(
        &mut s,
        &mut view,
        NodeKind::Effect(Effect::Blend {
            mode: BlendMode::Difference,
            amount: 1.0,
        }),
    );
    let blend = s
        .doc
        .graph
        .nodes()
        .find(|(_, n)| matches!(n.kind, NodeKind::Effect(Effect::Blend { .. })))
        .map(|(id, _)| id)
        .expect("the node was added");
    assert_eq!(
        s.doc.graph.input(blend, 0),
        Some(layer.node),
        "fed by the layer"
    );
    assert!(
        s.doc.graph.links().iter().any(|l| l.from == blend),
        "and feeds where the layer went"
    );
    let after = symphony_render::render_camera(&s.doc, s.frame, 160, 90, None).unwrap();
    assert!(before.data() != after.data(), "the render changes");
    s.undo();
    assert_eq!(s.doc.graph.links(), &links[..], "one undo rewires back");
    assert!(s.doc.graph.node(blend).is_none());
}
