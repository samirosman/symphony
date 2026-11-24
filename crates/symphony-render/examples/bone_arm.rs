//! An arm bent by a two-bone chain, rendered to PNG so the fold can be
//! judged by eye:
//! `cargo run --release -p symphony-render --example bone_arm [out_dir]`
//!
//! Writes, to `out_dir` (default `target/bone-arm`):
//! - `bias.png`: rows are joint bias, columns are bend angle.
//! - `radius.png`: rows are joint radius (half a bone at most), columns
//!   are bend angle, at the default bias.
//! - `bend/0001.png` ...: the arm bending at the default joint.

use std::path::PathBuf;

use image::{Rgba, RgbaImage};
use symphony_core::deform::{BonePose, ChainPose, Influence, DEFAULT_BIAS, MAX_BIAS};
use symphony_core::doc::{Scene, Settings};
use symphony_core::history::{Edit, History};
use symphony_core::id::Id;
use symphony_core::stroke::{Drawing, Fill, Point, Stroke, StrokeKind, Tip, Tips};
use symphony_render::deform::deform_drawing;
use symphony_render::render_frame;

const W: u32 = 480;
const H: u32 = 360;
/// Bone length and half the arm's width, in scene units (pixels here).
const BONE: f32 = 120.0;
const HALF: f32 = 22.0;
/// Where the chain starts.
const ROOT: [f32; 2] = [-150.0, 80.0];
/// Bend angles; negative turns the forearm upwards on screen.
const ANGLES: [f32; 5] = [0.0, -30.0, -60.0, -90.0, -135.0];
const GAP: u32 = 8;

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/bone-arm"), PathBuf::from);
    std::fs::create_dir_all(out.join("bend")).expect("create output folder");
    let (mut scene, id) = arm_scene();
    let art = scene.drawings[&id].clone();

    let biases = [0.0, 0.2, DEFAULT_BIAS, 0.7, MAX_BIAS];
    let rows: Vec<Vec<RgbaImage>> = biases
        .iter()
        .map(|&bias| {
            ANGLES
                .iter()
                .map(|&a| cell(&mut scene, id, &art, 0.3, bias, a))
                .collect()
        })
        .collect();
    sheet(&rows)
        .save(out.join("bias.png"))
        .expect("write bias.png");

    let shares = [0.15, 0.3, 0.4, 0.5];
    let rows: Vec<Vec<RgbaImage>> = shares
        .iter()
        .map(|&share| {
            ANGLES
                .iter()
                .map(|&a| cell(&mut scene, id, &art, share, DEFAULT_BIAS, a))
                .collect()
        })
        .collect();
    sheet(&rows)
        .save(out.join("radius.png"))
        .expect("write radius.png");

    let frames = 24;
    for f in 0..frames {
        let a = -120.0 * f as f32 / (frames - 1) as f32;
        cell(&mut scene, id, &art, 0.3, DEFAULT_BIAS, a)
            .save(out.join("bend").join(format!("{:04}.png", f + 1)))
            .expect("write frame");
    }

    println!("Wrote {}", out.display());
    println!("bias.png   rows: bias {biases:.2?}; columns: bend {ANGLES:?} degrees");
    println!("radius.png rows: radius {shares:?} of a bone; columns: as above");
    println!("bend/      {frames} frames, 0 to -120 degrees, default joint");
}

/// The arm rendered with a joint of `share` times the bone length, the
/// given bias, and the forearm turned by `angle` degrees.
fn cell(scene: &mut Scene, id: Id, art: &Drawing, share: f32, bias: f32, angle: f32) -> RgbaImage {
    let mut root = BonePose::new(0.0, BONE, BONE);
    (root.x, root.y) = (ROOT[0], ROOT[1]);
    let mut fore = BonePose::new(0.0, BONE, BONE);
    fore.radius = share * BONE;
    fore.bias = bias;
    let rest = [root, fore];
    let mut anim = rest;
    anim[1].orientation = angle;
    let pose = ChainPose::new(&rest, &anim, Influence::Infinite);
    scene.drawings.insert(id, deform_drawing(art, &pose));
    render_frame(scene, 1, W, H).expect("render")
}

/// Cells side by side, rows top to bottom, on a grey ground.
fn sheet(rows: &[Vec<RgbaImage>]) -> RgbaImage {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0) as u32;
    let n = rows.len() as u32;
    let mut img = RgbaImage::from_pixel(
        GAP + cols * (W + GAP),
        GAP + n * (H + GAP),
        Rgba([150, 150, 150, 255]),
    );
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let x = GAP + c as u32 * (W + GAP);
            let y = GAP + r as u32 * (H + GAP);
            image::imageops::replace(&mut img, cell, x as i64, y as i64);
        }
    }
    img
}

/// A scene with one drawing of a straight arm along the chain: skin
/// paint, an outline, cross lines every 15 units, a centre line and an
/// erased-stroke sleeve band.
fn arm_scene() -> (Scene, Id) {
    let mut s = Scene::new(Settings {
        width: W,
        height: H,
        fps: 24.0,
        length: 1,
        ..Settings::default()
    });
    let mut h = History::default();
    let snap = s.make_layer("Arm", 0);
    h.apply(&mut s, "Add layer", vec![Edit::AddLayer(Box::new(snap))]);
    let col = s.layers[0].column;
    let mut d = s.make_drawing(col);
    let sw = |i: usize| s.palette.swatches[i].id;
    let (line, skin, shirt, shadow, accent) = (sw(0), sw(1), sw(2), sw(4), sw(5));

    let outline = capsule();
    d.fills.push(Fill {
        id: s.ids.fresh(),
        swatch: skin,
        art: Default::default(),
        contours: vec![outline.clone()],
    });
    let mut ring = outline;
    ring.push(ring[0]);
    let mut strokes = vec![stroke(s.ids.fresh(), line, 3.0, &ring)];
    let (x0, x1) = (ROOT[0], ROOT[0] + 2.0 * BONE);
    let mut x = x0 + 15.0;
    while x < x1 - 5.0 {
        let p = [[x, ROOT[1] - HALF], [x, ROOT[1] + HALF]];
        strokes.push(stroke(s.ids.fresh(), shadow, 1.5, &p));
        x += 15.0;
    }
    strokes.push(stroke(
        s.ids.fresh(),
        shirt,
        2.0,
        &[[x0, ROOT[1]], [x1, ROOT[1]]],
    ));
    let mut band = stroke(s.ids.fresh(), accent, 1.0, &[[x0 + 50.0, ROOT[1]]]);
    let (bx, by) = (x0 + 60.0, ROOT[1]);
    band.shape = vec![vec![
        [bx - 8.0, by - HALF - 4.0],
        [bx + 8.0, by - HALF - 4.0],
        [bx + 8.0, by + HALF + 4.0],
        [bx - 8.0, by + HALF + 4.0],
    ]];
    strokes.push(band);
    d.strokes = strokes;

    let id = d.id;
    let cells = Edit::splice(&s, col, 1, 0, vec![Some(id)]).expect("expose drawing");
    h.apply(
        &mut s,
        "Draw arm",
        vec![Edit::AddDrawing { drawing: d }, cells],
    );
    (s, id)
}

/// The arm's outline: a rectangle along both bones with round ends.
fn capsule() -> Vec<[f32; 2]> {
    let (x0, x1, y) = (ROOT[0], ROOT[0] + 2.0 * BONE, ROOT[1]);
    let mut out = Vec::new();
    let steps = 16;
    for i in 0..=steps {
        let a = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / steps as f32;
        out.push([x0 + HALF * a.cos(), y - HALF * a.sin()]);
    }
    for i in 0..=steps {
        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / steps as f32;
        out.push([x1 + HALF * a.cos(), y - HALF * a.sin()]);
    }
    out
}

fn stroke(id: Id, swatch: Id, width: f32, points: &[[f32; 2]]) -> Stroke {
    Stroke {
        id,
        kind: StrokeKind::Brush,
        art: Default::default(),
        swatch,
        min_width: width,
        max_width: width,
        points: points
            .iter()
            .map(|p| Point {
                x: p[0],
                y: p[1],
                pressure: 1.0,
                ..Point::default()
            })
            .collect(),
        tips: Tips {
            start: Tip::Round,
            end: Tip::Round,
        },
        texture: None,
        shape: Vec::new(),
    }
}
