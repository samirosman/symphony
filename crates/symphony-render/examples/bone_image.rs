//! An imported image of an arm bent by a two-bone chain, to judge the
//! mesh warp by eye and time it:
//! `cargo run --release -p symphony-render --example bone_image [out_dir]`
//!
//! Writes, to `out_dir` (default `target/bone-image`):
//! - `bent.png`: rows are mesh detail (default, coarse, fine), columns
//!   are bend angle.
//! - `mesh.png`: the same, with the triangles drawn over the image.
//!
//! Then times a full-HD image bent at 60 degrees for each mesh detail.

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Instant;

use image::{Rgba, RgbaImage};
use symphony_core::bitmap::{layer_edits, one_to_one, Alpha};
use symphony_core::deform::{BonePose, ChainPose, Influence, MeshDetail};
use symphony_core::doc::{Media, MediaKind};
use symphony_core::doc::{Scene, Settings};
use symphony_core::history::History;
use symphony_core::math::Affine;
use symphony_core::stroke::PlacedImage;
use symphony_render::warp::{paint_image_bent, Mesh};
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

const W: u32 = 480;
const H: u32 = 360;
const BONE: f32 = 120.0;
const HALF: f32 = 22.0;
const ANGLES: [f32; 5] = [0.0, -30.0, -60.0, -90.0, -135.0];
const GAP: u32 = 8;

fn details() -> [(&'static str, MeshDetail); 3] {
    [
        ("default", MeshDetail::default()),
        (
            "coarse",
            MeshDetail {
                largest: 128.0,
                smallest: 4.0,
                tolerance: 1.0,
            },
        ),
        (
            "fine",
            MeshDetail {
                largest: 32.0,
                smallest: 1.0,
                tolerance: 0.1,
            },
        ),
    ]
}

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/bone-image"), PathBuf::from);
    std::fs::create_dir_all(&out).expect("create output folder");

    let (scene, r) = image_scene(W, H, arm_image(BONE, HALF));
    let view = Affine::translate(W as f32 / 2.0, H as f32 / 2.0);
    let mut plain = Vec::new();
    let mut wired = Vec::new();
    for (_, detail) in details() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for &angle in &ANGLES {
            let pose = arm_pose(BONE, angle);
            let mut pix = white(W, H);
            let mesh = paint_image_bent(&mut pix, &scene, r, view, |p| pose.map(p), &detail);
            a.push(to_image(pix.clone()));
            if let Some(mesh) = mesh {
                wireframe(&mut pix, &mesh);
            }
            b.push(to_image(pix));
        }
        plain.push(a);
        wired.push(b);
    }
    sheet(&plain)
        .save(out.join("bent.png"))
        .expect("write bent.png");
    sheet(&wired)
        .save(out.join("mesh.png"))
        .expect("write mesh.png");
    println!("Wrote {}", out.display());
    println!("rows: mesh detail default, coarse, fine; columns: bend {ANGLES:?} degrees");

    // Full HD: a 1600 x 300 arm on a 1920 x 1080 stage.
    let (big, br) = image_scene(1920, 1080, arm_image(800.0, 150.0));
    let view = Affine::translate(960.0, 540.0);
    let pose = arm_pose(800.0, -60.0);
    for (name, detail) in details() {
        let mut pix = white(1920, 1080);
        // The first call decodes the image into the cache.
        let mesh =
            paint_image_bent(&mut pix, &big, br, view, |p| pose.map(p), &detail).expect("bent");
        let runs = 5;
        let t = Instant::now();
        for _ in 0..runs {
            let mut pix = white(1920, 1080);
            paint_image_bent(&mut pix, &big, br, view, |p| pose.map(p), &detail);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / runs as f64;
        println!(
            "full HD, 60 degrees, {name:7}: {:6} triangles, {ms:6.1} ms a frame",
            mesh.triangles.len()
        );
    }
}

/// Two bones of `bone` along +x, the joint at the stage centre.
fn arm_pose(bone: f32, angle: f32) -> ChainPose {
    let mut root = BonePose::new(0.0, bone, bone);
    root.x = -bone;
    let fore = BonePose::new(0.0, bone, bone);
    let rest = [root, fore];
    let mut anim = rest;
    anim[1].orientation = angle;
    ChainPose::new(&rest, &anim, Influence::Infinite)
}

/// A straight arm with round ends over two bones of `bone`: a checker of
/// two skin tones with dark lines every 24 pixels, clear outside the arm,
/// with a soft one-pixel edge in its alpha.
fn arm_image(bone: f32, half: f32) -> RgbaImage {
    let (w, h) = ((2.0 * (bone + half)) as u32, (2.0 * half) as u32);
    RgbaImage::from_fn(w, h, |x, y| {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let cx = px.clamp(half, w as f32 - half);
        let d = (px - cx).hypot(py - half);
        let a = (half - d + 0.5).clamp(0.0, 1.0);
        let line = x % 24 < 2 || y % 24 < 2;
        let check = (x / 12 + y / 12) % 2 == 0;
        let [r, g, b] = if line {
            [60, 50, 70]
        } else if check {
            [240, 196, 160]
        } else {
            [215, 160, 125]
        };
        Rgba([r, g, b, (a * 255.0).round() as u8])
    })
}

/// A scene holding `img` at one image pixel per scene pixel, centred.
fn image_scene(w: u32, h: u32, img: RgbaImage) -> (Scene, PlacedImage) {
    let mut s = Scene::new(Settings {
        width: w,
        height: h,
        fps: 24.0,
        length: 1,
        ..Settings::default()
    });
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("encode PNG");
    let dir = std::env::temp_dir().join(format!("symphony-bone-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("media folder");
    let file = format!("{w}x{h}-{}.png", img.width());
    std::fs::write(dir.join(&file), &bytes).expect("write PNG");
    s.media_dirs = vec![dir];
    s.media.insert(
        1,
        Media {
            kind: MediaKind::Image,
            file,
            name: "Arm".into(),
            size: [img.width(), img.height()],
            seconds: 0.0,
            rate: 0,
            channels: 0,
            samples: 0,
        },
    );
    let placed = PlacedImage {
        media: 1,
        rect: one_to_one([img.width(), img.height()]),
        alpha: Alpha::Straight,
    };
    let edits = layer_edits(&mut s, "Arm", 0, &[placed], 1);
    History::default().apply(&mut s, "Import image", edits);
    let d = s.exposed(0, 1).expect("exposed drawing");
    let r = s.drawings[&d].image.expect("image drawing");
    (s, r)
}

fn white(w: u32, h: u32) -> Pixmap {
    let mut p = Pixmap::new(w, h).expect("pixmap");
    p.fill(tiny_skia::Color::WHITE);
    p
}

fn wireframe(pix: &mut Pixmap, mesh: &Mesh) {
    let mut pb = PathBuilder::new();
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.points[i as usize]);
        pb.move_to(a[0], a[1]);
        pb.line_to(b[0], b[1]);
        pb.line_to(c[0], c[1]);
        pb.close();
    }
    let Some(path) = pb.finish() else { return };
    let mut paint = Paint::default();
    paint.set_color_rgba8(20, 40, 160, 140);
    paint.anti_alias = true;
    let stroke = Stroke {
        width: 0.6,
        ..Stroke::default()
    };
    pix.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

/// The stage is opaque white, so premultiplied and straight agree.
fn to_image(pix: Pixmap) -> RgbaImage {
    RgbaImage::from_raw(pix.width(), pix.height(), pix.take()).expect("image")
}

fn sheet(rows: &[Vec<RgbaImage>]) -> RgbaImage {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0) as u32;
    let mut img = RgbaImage::from_pixel(
        GAP + cols * (W + GAP),
        GAP + rows.len() as u32 * (H + GAP),
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
