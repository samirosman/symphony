//! How long imported images take to decode and draw:
//! `cargo run --release -p symphony-render --example bitmap_bench -- gen`
//! then `... -- decode <file>` or `... -- frames <file> <layers>`.
//! Files go to `target/bitmap-bench/`. Peak memory: run under
//! `/usr/bin/time -v` and read "Maximum resident set size".

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use symphony_core::bitmap::{layer_edits, Alignment, ImageFile, ImageFormat};
use symphony_core::doc::{Scene, Settings};
use symphony_core::history::History;

fn dir() -> PathBuf {
    let d = PathBuf::from("target/bitmap-bench");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A smooth gradient with a little noise, so files compress like a
/// painted background rather than like flat colour or pure noise.
fn value(x: u32, y: u32, w: u32, h: u32, c: u32) -> f32 {
    let g = match c {
        0 => x as f32 / w as f32,
        1 => y as f32 / h as f32,
        _ => 1.0 - (x + y) as f32 / (w + h) as f32,
    };
    let n = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ c) % 17) as f32 / 255.0;
    (g + n).min(1.0)
}

fn gen() {
    for (name, w, h) in [
        ("8mp", 3840, 2160),
        ("16mp", 5120, 3200),
        ("64mp", 8192, 8192),
    ] {
        let rgba = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([0, 1, 2, 3].map(|c| {
                if c == 3 {
                    255
                } else {
                    (value(x, y, w, h, c) * 255.0) as u8
                }
            }))
        });
        let p = dir().join(format!("{name}.png"));
        rgba.save(&p).unwrap();
        let rgb = image::DynamicImage::ImageRgba8(rgba).into_rgb8();
        let j = dir().join(format!("{name}.jpg"));
        let mut f = std::fs::File::create(&j).unwrap();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 90)
            .encode_image(&rgb)
            .unwrap();
        drop(rgb);
        exr_file(name, w, h, true, true);
        exr_file(name, w, h, true, false);
        exr_file(name, w, h, false, true);
        eprintln!("wrote {name}");
    }
}

/// `half`: f16 or f32 samples. `alpha`: RGBA or RGB. ZIP compression.
fn exr_file(name: &str, w: u32, h: u32, half: bool, alpha: bool) {
    use exr::prelude::*;
    let (w, h) = (w as usize, h as usize);
    let lin = move |p: Vec2<usize>, c: u32| {
        let s = value(p.0 as u32, p.1 as u32, w as u32, h as u32, c);
        s.powf(2.2)
    };
    let tag = format!(
        "{name}-{}-{}.exr",
        if half { "half" } else { "float" },
        if alpha { "rgba" } else { "rgb" }
    );
    let path = dir().join(tag);
    let enc = Encoding::SMALL_LOSSLESS;
    match (half, alpha) {
        (true, true) => {
            let ch = SpecificChannels::rgba(move |p: Vec2<usize>| {
                (
                    f16::from_f32(lin(p, 0)),
                    f16::from_f32(lin(p, 1)),
                    f16::from_f32(lin(p, 2)),
                    f16::from_f32(1.0),
                )
            });
            Image::from_encoded_channels((w, h), enc, ch)
                .write()
                .to_file(&path)
                .unwrap();
        }
        (true, false) => {
            let ch = SpecificChannels::rgb(move |p: Vec2<usize>| {
                (
                    f16::from_f32(lin(p, 0)),
                    f16::from_f32(lin(p, 1)),
                    f16::from_f32(lin(p, 2)),
                )
            });
            Image::from_encoded_channels((w, h), enc, ch)
                .write()
                .to_file(&path)
                .unwrap();
        }
        (false, _) => {
            let ch = SpecificChannels::rgba(move |p: Vec2<usize>| {
                (lin(p, 0), lin(p, 1), lin(p, 2), 1.0f32)
            });
            Image::from_encoded_channels((w, h), enc, ch)
                .write()
                .to_file(&path)
                .unwrap();
        }
    }
}

fn load(path: &Path) -> ImageFile {
    let bytes = std::fs::read(path).unwrap();
    let format = match path.extension().unwrap().to_str().unwrap() {
        "png" => ImageFormat::Png,
        "jpg" => ImageFormat::Jpeg,
        _ => ImageFormat::Exr,
    };
    let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .unwrap()
        .into_dimensions()
        .unwrap();
    ImageFile {
        format,
        width,
        height,
        bytes: Arc::new(bytes),
    }
}

fn decode(path: &Path) {
    let file = load(path);
    let alpha = file.format.usual_alpha();
    let t = Instant::now();
    let p = symphony_render::bitmap::decode(&file, alpha);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{}\t{} MB file\t{}\t{ms:.0} ms",
        path.file_name().unwrap().to_string_lossy(),
        file.bytes.len() / 1_000_000,
        if p.is_some() { "ok" } else { "FAILED" },
    );
}

/// `layers` copies of the image, each with its own key (so each decodes
/// and caches on its own), rendered at 1920 x 1080 for four frames.
fn frames(path: &Path, layers: u64) {
    let file = load(path);
    let alpha = file.format.usual_alpha();
    let mut s = Scene::new(Settings {
        width: 1920,
        height: 1080,
        fps: 24.0,
        length: 4,
    });
    let mut h = History::default();
    for k in 0..layers {
        s.images.insert(k + 1, file.clone());
        let edits = layer_edits(&mut s, "BG", 0, &[k + 1], 1, Alignment::Fit, alpha);
        h.apply(&mut s, "Import image", edits);
    }
    for f in 1..=4 {
        let t = Instant::now();
        let img = symphony_render::render_frame(&s, f, 1920, 1080);
        assert!(img.is_some());
        println!(
            "frame {f}\t{layers} layers\t{:.0} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["gen"] => gen(),
        ["decode", p] => decode(Path::new(p)),
        ["exrparts", p] => {
            let bytes = std::fs::read(p).unwrap();
            let t = Instant::now();
            let mut r = image::ImageReader::with_format(
                Cursor::new(&bytes[..]),
                image::ImageFormat::OpenExr,
            );
            r.no_limits();
            let d = r.decode().unwrap();
            let a = t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            let f = d.into_rgba32f();
            let b = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "{p}\timage decode {a:.0} ms\tto rgba32f {b:.0} ms\t{}",
                f.len()
            );
        }
        ["frames", p, n] => frames(Path::new(p), n.parse().unwrap()),
        _ => eprintln!("usage: gen | decode <file> | frames <file> <layers>"),
    }
}
