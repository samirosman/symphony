//! CPU rendering (ENGINEERING E10.2): a scene frame becomes an RGBA image.
//! Camera, export, and render tests share this graph and projection.
//!
//! Export runs on a background thread with progress and cancel (E5.2).

mod effects;
pub mod movie;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use image::{Delay, Frame, RgbaImage};
use symphony_core::doc::Scene;
use symphony_core::graph::{RenderCommand, RenderItem};
use symphony_core::id::Id;
use symphony_core::math::Affine;
use symphony_core::stroke::{ArtLayer, StrokeKind};
use symphony_vector::tessellate;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Transform};

/// Render `frame` at `width` x `height` pixels (the scene's aspect is
/// kept; the stage fills the image). Background is white paper.
pub fn render_frame(scene: &Scene, frame: u32, width: u32, height: u32) -> Option<RgbaImage> {
    let pixmap = render_display(scene, frame, width, height, true, None)?;
    RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.take())
}

/// Temporary drawing-space transform for the Select tool's live preview.
#[derive(Clone, Copy)]
pub struct StrokePreview<'a> {
    pub drawing: Id,
    pub strokes: &'a [Id],
    pub transform: Affine,
}

/// Render the Display graph as a premultiplied RGBA stage with transparent paper.
pub fn render_camera(
    scene: &Scene,
    frame: u32,
    width: u32,
    height: u32,
    preview: Option<StrokePreview<'_>>,
) -> Option<Pixmap> {
    render_display(scene, frame, width, height, false, preview)
}

fn render_display(
    scene: &Scene,
    frame: u32,
    width: u32,
    height: u32,
    white_paper: bool,
    preview: Option<StrokePreview<'_>>,
) -> Option<Pixmap> {
    let (width, height) = (width.max(1), height.max(1));
    let mut paper = Pixmap::new(width, height)?;
    if white_paper {
        paper.fill(tiny_skia::Color::WHITE);
    }
    let k = width as f32 / scene.settings.width.max(1) as f32 * scene.camera_scale(frame);
    // Scene units have their origin at the stage centre, y down.
    let view =
        Affine::translate(width as f32 / 2.0, height as f32 / 2.0).compose(Affine::scale(k, k));
    let commands = scene
        .graph
        .render_commands(scene.display, &scene.xsheet, frame)
        .unwrap_or_default();
    let mut layers = vec![paper];
    for command in commands {
        let current = layers.last_mut()?;
        match command {
            RenderCommand::Drawing(item) => {
                paint_drawing(current, scene, frame, view, item, preview)
            }
            RenderCommand::Gradient(gradient) => effects::gradient(current, gradient),
            RenderCommand::BeginEffect => {
                // Bound temporary raster storage, including blur scratch space.
                let bytes = current.data().len().checked_mul(layers.len() + 2)?;
                if bytes > 512 * 1024 * 1024 {
                    return None;
                }
                layers.push(Pixmap::new(width, height)?);
            }
            RenderCommand::EndEffect(effect) => {
                let mut image = layers.pop()?;
                effects::apply(&mut image, effect, k);
                layers.last_mut()?.draw_pixmap(
                    0,
                    0,
                    image.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
    }
    layers.pop()
}

fn paint_drawing(
    pix: &mut Pixmap,
    scene: &Scene,
    frame: u32,
    view: Affine,
    item: RenderItem,
    preview: Option<StrokePreview<'_>>,
) {
    let Some(d) = scene.drawings.get(&item.drawing) else {
        return;
    };
    let Some(project) = scene.project_plane(frame, item.depth) else {
        return;
    };
    let m = view.compose(project).compose(item.transform);
    // Art layers back to front; within each, paint under ink.
    for art in ArtLayer::STACK {
        for fill in d.fills.iter().filter(|f| f.art == art) {
            let mut pb = PathBuilder::new();
            for c in &fill.contours {
                for (k, p) in c.iter().enumerate() {
                    let (x, y) = m.apply(p[0], p[1]);
                    if k == 0 {
                        pb.move_to(x, y);
                    } else {
                        pb.line_to(x, y);
                    }
                }
                pb.close();
            }
            let Some(path) = pb.finish() else { continue };
            let [r, g, b, alpha] = scene.swatch_rgba(fill.swatch);
            let mut paint = Paint::default();
            paint.set_color_rgba8(r, g, b, alpha);
            paint.anti_alias = true;
            pix.fill_path(
                &path,
                &paint,
                FillRule::EvenOdd,
                Transform::identity(),
                None,
            );
        }
        paint_strokes(
            pix,
            scene,
            d,
            art,
            m,
            preview.filter(|p| p.drawing == item.drawing),
        );
    }
}

/// Ink for one art layer. Invisible strokes only bound paint zones.
fn paint_strokes(
    pix: &mut Pixmap,
    scene: &Scene,
    d: &symphony_core::stroke::Drawing,
    art: ArtLayer,
    m: Affine,
    preview: Option<StrokePreview<'_>>,
) {
    {
        for st in d
            .strokes
            .iter()
            .filter(|s| s.art == art && s.kind != StrokeKind::Invisible)
        {
            let m = match preview {
                Some(p) if p.strokes.contains(&st.id) => m.compose(p.transform),
                _ => m,
            };
            if let Some(tex) = &st.texture {
                paint_textured(pix, scene, st, tex, m);
                continue;
            }
            let mesh = tessellate(st, m, 0.35);
            if mesh.indices.is_empty() {
                continue;
            }
            // One path per stroke with non-zero fill: overlapping triangles
            // merge into one shape, so there are no seams between them.
            let mut pb = PathBuilder::new();
            for t in mesh.indices.as_chunks::<3>().0 {
                let (a, b, c) = (
                    mesh.vertices[t[0] as usize],
                    mesh.vertices[t[1] as usize],
                    mesh.vertices[t[2] as usize],
                );
                // Wind every triangle the same way so non-zero fill unions them.
                let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                let (b, c) = if cross < 0.0 { (c, b) } else { (b, c) };
                pb.move_to(a[0], a[1]);
                pb.line_to(b[0], b[1]);
                pb.line_to(c[0], c[1]);
                pb.close();
            }
            let Some(path) = pb.finish() else { continue };
            let [r, g, b, alpha] = scene.swatch_rgba(st.swatch);
            let mut paint = Paint::default();
            paint.set_color_rgba8(r, g, b, alpha);
            paint.anti_alias = true;
            pix.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
}

/// A textured vector stroke: stamped coverage, composited source-over in
/// the stroke's swatch colour.
fn paint_textured(
    pix: &mut Pixmap,
    scene: &Scene,
    st: &symphony_core::stroke::Stroke,
    tex: &symphony_core::stroke::StrokeTexture,
    m: Affine,
) {
    let clip = [0, 0, pix.width() as i32, pix.height() as i32];
    let tip = tex.tip.and_then(|k| scene.textures.get(&k));
    let grain = tex.grain.and_then(|k| scene.textures.get(&k));
    let Some(c) = symphony_vector::stamp::rasterize(st, tex, tip, grain, m, clip) else {
        return;
    };
    let [r, g, b, alpha] = scene.swatch_rgba(st.swatch);
    let width = pix.width() as i32;
    let data = pix.data_mut();
    for row in 0..c.height as i32 {
        for col in 0..c.width as i32 {
            let coverage = c.data[(row * c.width as i32 + col) as usize] as u32;
            let a = (coverage * u32::from(alpha) + 127) / 255;
            if a == 0 {
                continue;
            }
            let (x, y) = (c.x + col, c.y + row);
            let i = ((y * width + x) * 4) as usize;
            // Premultiplied source-over.
            let inv = 255 - a;
            for (k, src) in [r, g, b].into_iter().enumerate() {
                data[i + k] = ((src as u32 * a + data[i + k] as u32 * inv) / 255) as u8;
            }
            data[i + 3] = (a + data[i + 3] as u32 * inv / 255) as u8;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// One PNG per frame: name_0001.png, ...
    PngSequence,
    AnimatedGif,
    /// H.264 in MP4, through Cisco's OpenH264 binary (`ExportJob::codec`).
    Mp4Movie,
}

#[derive(Clone, Debug)]
pub struct ExportJob {
    pub format: Format,
    /// For a PNG sequence, the first file's path (the frame number is added).
    pub path: PathBuf,
    pub first: u32,
    pub last: u32,
    /// Output size as a fraction of the scene resolution (1.0, 0.5, ...).
    pub scale: f32,
    /// Path to Cisco's OpenH264 binary, for movies.
    pub codec: Option<PathBuf>,
    /// MP4 rate override. Resamples the scene in time when set.
    pub movie_fps: Option<f32>,
}

/// Run an export. `progress` gets (done, total). Stops early, cleanly, when
/// `cancel` is set. Errors are plain sentences (DESIGN D10.3).
pub fn export(
    scene: &Scene,
    job: &ExportJob,
    cancel: &AtomicBool,
    progress: &dyn Fn(u32, u32),
) -> Result<(), String> {
    let (w, h) = (
        ((scene.settings.width as f32 * job.scale).round() as u32).max(1),
        ((scene.settings.height as f32 * job.scale).round() as u32).max(1),
    );
    let (first, last) = (job.first.max(1), job.last.max(job.first.max(1)));
    let total = last - first + 1;
    match job.format {
        Format::PngSequence => {
            for (i, f) in (first..=last).enumerate() {
                if cancel.load(Ordering::SeqCst) {
                    return Err("Export cancelled.".into());
                }
                let img = render_frame(scene, f, w, h).ok_or("Couldn't render a frame.")?;
                let out = numbered(&job.path, f);
                img.save(&out)
                    .map_err(|e| format!("Couldn't write {} ({e}).", out.display()))?;
                progress(i as u32 + 1, total);
            }
            Ok(())
        }
        Format::Mp4Movie => {
            let codec = job.codec.as_deref().ok_or(
                "Movie export needs the OpenH264 codec. Download it in Preferences > Video.",
            )?;
            movie::export_mp4(
                scene,
                codec,
                &job.path,
                first,
                last,
                w,
                h,
                job.movie_fps,
                cancel,
                progress,
            )
        }
        Format::AnimatedGif => {
            let file = std::fs::File::create(&job.path)
                .map_err(|e| format!("Couldn't create {} ({e}).", job.path.display()))?;
            let mut enc =
                image::codecs::gif::GifEncoder::new_with_speed(std::io::BufWriter::new(file), 10);
            enc.set_repeat(image::codecs::gif::Repeat::Infinite)
                .map_err(|e| e.to_string())?;
            let ms = (1000.0 / scene.settings.fps.max(1.0)).round() as u32;
            for (i, f) in (first..=last).enumerate() {
                if cancel.load(Ordering::SeqCst) {
                    return Err("Export cancelled.".into());
                }
                let img = render_frame(scene, f, w, h).ok_or("Couldn't render a frame.")?;
                enc.encode_frame(Frame::from_parts(
                    img,
                    0,
                    0,
                    Delay::from_numer_denom_ms(ms, 1),
                ))
                .map_err(|e| format!("Couldn't write the GIF ({e})."))?;
                progress(i as u32 + 1, total);
            }
            Ok(())
        }
    }
}

/// `shot.png` + frame 7 -> `shot_0007.png`.
pub fn numbered(path: &Path, frame: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "frame".into());
    let dir = path.parent().unwrap_or(Path::new("."));
    dir.join(format!("{stem}_{frame:04}.png"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphony_core::doc::Settings;
    use symphony_core::history::{Edit, History};
    use symphony_core::stroke::{Point, Stroke, StrokeKind};

    fn scene_with_line() -> Scene {
        let mut s = Scene::new(Settings {
            width: 200,
            height: 100,
            fps: 12.0,
            length: 3,
        });
        let mut h = History::default();
        let snap = s.make_layer("Drawing", 0);
        h.apply(&mut s, "Add layer", vec![Edit::AddLayer(Box::new(snap))]);
        let col = s.layers[0].column;
        let d = s.make_drawing(col);
        let did = d.id;
        let swatch = s.palette.swatches[0].id;
        let st = Stroke {
            id: s.ids.fresh(),
            kind: StrokeKind::Brush,
            art: Default::default(),
            swatch,
            min_width: 10.0,
            max_width: 10.0,
            points: vec![
                Point {
                    x: -50.0,
                    y: 0.0,
                    pressure: 1.0,
                },
                Point {
                    x: 50.0,
                    y: 0.0,
                    pressure: 1.0,
                },
            ],
            tips: Default::default(),
            texture: None,
            shape: Vec::new(),
        };
        let cells = Edit::splice(&s, col, 1, 0, vec![Some(did), Some(did), Some(did)]).unwrap();
        h.apply(
            &mut s,
            "Draw",
            vec![
                Edit::AddDrawing { drawing: d },
                cells,
                Edit::AddStroke {
                    drawing: did,
                    stroke: st,
                },
            ],
        );
        s
    }

    #[test]
    fn renders_textured_strokes_with_their_grain() {
        let mut s = scene_with_line();
        // Grain: alternating black and white columns, one pixel each.
        s.textures.insert(
            1,
            symphony_core::stroke::Texture {
                width: 2,
                height: 1,
                pixels: vec![0, 255],
            },
        );
        let d = *s.drawings.keys().next().unwrap();
        s.drawings.get_mut(&d).unwrap().strokes[0].texture =
            Some(symphony_core::stroke::StrokeTexture {
                tip: None,
                grain: Some(1),
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
        let img = render_frame(&s, 1, 200, 100).unwrap();
        let row: Vec<u8> = (90..110).map(|x| img.get_pixel(x, 50).0[0]).collect();
        let ink = s.palette.swatches[0].rgb[0];
        assert!(row.contains(&255), "paper shows through the grain: {row:?}");
        assert!(row.contains(&ink), "ink where the grain is white: {row:?}");
    }

    #[test]
    fn camera_pan_moves_foreground_faster_than_background() {
        use symphony_core::anim::Value;
        use symphony_core::graph::NodeKind;
        let mut scene = scene_with_line();
        let near = scene.layers[0];
        let drawing = scene.exposed(0, 1).unwrap();
        let near_stroke = &mut scene.drawings.get_mut(&drawing).unwrap().strokes[0];
        near_stroke.points[0].x = -10.0;
        near_stroke.points[1].x = 10.0;
        for point in &mut near_stroke.points {
            point.y = -12.0;
        }
        let mut far_stroke = near_stroke.clone();
        far_stroke.id = scene.ids.fresh();
        for point in &mut far_stroke.points {
            point.y = 48.0;
        }

        let mut history = History::default();
        let layer = scene.make_layer("Far", 1);
        history.apply(
            &mut scene,
            "Add layer",
            vec![Edit::AddLayer(Box::new(layer))],
        );
        let far = scene.layers[1];
        let mut drawing = scene.make_drawing(far.column);
        drawing.strokes.push(far_stroke);
        let cells = Edit::splice(&scene, far.column, 1, 0, vec![Some(drawing.id)]).unwrap();
        history.apply(
            &mut scene,
            "Add drawing",
            vec![Edit::AddDrawing { drawing }, cells],
        );
        for (node, z) in [(near.peg, -100.0), (far.peg, 200.0)] {
            let NodeKind::Peg(peg) = &mut scene.graph.node_mut(node).unwrap().kind else {
                unreachable!()
            };
            peg.z = Value::Constant(z);
        }

        let centre = |image: &Pixmap, row| {
            let ink: Vec<_> = (0..image.width())
                .filter(|&x| image.pixel(x, row).unwrap().alpha() > 200)
                .collect();
            (ink.first().unwrap() + ink.last().unwrap() + 1) as f32 * 0.5
        };
        let before = render_camera(&scene, 1, 200, 100, None).unwrap();
        scene.camera_motion.x = Value::Constant(20.0);
        let after = render_camera(&scene, 1, 200, 100, None).unwrap();
        assert_eq!(centre(&before, 26) - centre(&after, 26), 40.0);
        assert_eq!(centre(&before, 74) - centre(&after, 74), 10.0);

        let exported = render_frame(&scene, 1, 200, 100).unwrap();
        let ink = scene.palette.swatches[0].rgb;
        assert_eq!(&exported[(60, 26)].0[..3], &ink);
        assert_eq!(&exported[(90, 74)].0[..3], &ink);
    }

    #[test]
    fn field_of_view_changes_the_framing_without_changing_image_size() {
        let mut scene = scene_with_line();
        let standard = render_camera(&scene, 1, 200, 100, None).unwrap();
        scene.camera_fov = symphony_core::anim::Value::Constant(90.0);
        let wide = render_camera(&scene, 1, 200, 100, None).unwrap();
        assert_eq!(standard.width(), wide.width());
        assert_eq!(standard.height(), wide.height());
        assert_eq!(standard.pixel(58, 50).unwrap().alpha(), 255);
        assert_eq!(wide.pixel(58, 50).unwrap().alpha(), 0);
        assert_eq!(wide.pixel(100, 50).unwrap().alpha(), 255);
    }

    #[test]
    fn swatch_opacity_limits_textured_brush_coverage() {
        let mut scene = scene_with_line();
        let drawing = scene.drawings.values_mut().next().unwrap();
        let stroke = &mut drawing.strokes[0];
        stroke.texture = Some(symphony_core::stroke::StrokeTexture {
            tip: None,
            grain: None,
            spacing: 0.1,
            opacity: 1.0,
            angle: 0.0,
            follow: false,
            grain_scale: 1.0,
            flow: 1.0,
            hardness: 1.0,
            opacity_by_pressure: [0.0, 0.33, 0.66, 1.0],
            ..Default::default()
        });
        let brush_swatch = scene.palette.get_mut(stroke.swatch).unwrap();
        brush_swatch.opacity = 128;
        let image = render_camera(&scene, 1, 200, 100, None).unwrap();
        assert_eq!(
            image.pixels().iter().map(|pixel| pixel.alpha()).max(),
            Some(128)
        );
    }

    #[test]
    fn renders_ink_on_paper() {
        let s = scene_with_line();
        let img = render_frame(&s, 1, 200, 100).unwrap();
        let [r, g, b] = s.palette.swatches[0].rgb;
        assert_eq!(
            img.get_pixel(100, 50).0,
            [r, g, b, 255],
            "centre of the line is ink"
        );
        assert_eq!(
            img.get_pixel(100, 10).0,
            [255, 255, 255, 255],
            "above it is paper"
        );
    }

    #[test]
    fn renders_effect_chain_with_gradient_behind_the_drawing() {
        use symphony_core::graph::{Effect, Gradient, NodeKind};

        let mut scene = scene_with_line();
        let drawing = scene.layers[0].node;
        let gradient = scene.graph.add(
            "Gradient",
            NodeKind::Gradient(Gradient {
                start: [240, 20, 10, 255],
                end: [10, 20, 240, 255],
                angle: 0.0,
            }),
        );
        let opacity = scene
            .graph
            .add("Opacity", NodeKind::Effect(Effect::Opacity { amount: 0.0 }));
        let blur = scene
            .graph
            .add("Box Blur", NodeKind::Effect(Effect::Blur { radius: 16.0 }));
        let grain = scene.graph.add(
            "Grain",
            NodeKind::Effect(Effect::Grain {
                amount: 0.25,
                seed: 42,
            }),
        );
        scene.graph.connect(drawing, opacity, 0);
        scene.graph.connect(opacity, blur, 0);
        scene.graph.connect(blur, scene.composite, 0);
        scene.graph.connect(gradient, scene.composite, 1);
        let background = render_frame(&scene, 1, 200, 100).unwrap();
        assert!(background[(5, 50)][0] > background[(5, 50)][2]);
        assert!(background[(195, 50)][0] < background[(195, 50)][2]);

        scene.graph.node_mut(opacity).unwrap().enabled = false;
        let blurred = render_frame(&scene, 1, 200, 100).unwrap();
        assert_ne!(blurred[(100, 45)], background[(100, 45)]);
        assert_eq!(blurred[(100, 10)], background[(100, 10)]);

        scene.graph.connect(scene.composite, grain, 0);
        scene.graph.connect(grain, scene.display, 0);
        let textured = render_frame(&scene, 1, 200, 100).unwrap();
        assert_ne!(textured, blurred);
        assert_eq!(textured, render_frame(&scene, 1, 200, 100).unwrap());
    }

    #[test]
    fn exports_gif_and_png_sequence() {
        let s = scene_with_line();
        let dir = std::env::temp_dir().join(format!("symphony-render-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cancel = AtomicBool::new(false);
        let gif = ExportJob {
            format: Format::AnimatedGif,
            path: dir.join("a.gif"),
            first: 1,
            last: 3,
            scale: 0.5,
            codec: None,
            movie_fps: None,
        };
        export(&s, &gif, &cancel, &|_, _| {}).unwrap();
        assert!(std::fs::metadata(dir.join("a.gif")).unwrap().len() > 0);
        let png = ExportJob {
            format: Format::PngSequence,
            path: dir.join("shot.png"),
            first: 2,
            last: 3,
            scale: 1.0,
            codec: None,
            movie_fps: None,
        };
        export(&s, &png, &cancel, &|_, _| {}).unwrap();
        assert!(dir.join("shot_0002.png").is_file() && dir.join("shot_0003.png").is_file());
        // Movie export runs when a Cisco binary is available:
        // SYMPHONY_TEST_OPENH264=/path/to/libopenh264-2.7.0-<platform>.
        if let Some(codec) = std::env::var_os("SYMPHONY_TEST_OPENH264") {
            let mp4 = ExportJob {
                format: Format::Mp4Movie,
                path: dir.join("m.mp4"),
                first: 1,
                last: 3,
                scale: 1.0,
                codec: Some(codec.into()),
                movie_fps: Some(48.0),
            };
            export(&s, &mp4, &cancel, &|_, _| {}).unwrap();
            let bytes = std::fs::read(dir.join("m.mp4")).unwrap();
            assert_eq!(&bytes[4..8], b"ftyp", "an MP4 file starts with ftyp");
            let file = std::io::Cursor::new(&bytes);
            let reader = mp4::Mp4Reader::read_header(file, bytes.len() as u64).unwrap();
            let track = &reader.tracks()[&1];
            assert_eq!(track.sample_count(), 12);
            assert_eq!(track.frame_rate(), 48.0);
            assert_eq!(track.duration(), std::time::Duration::from_millis(250));
            if let Some(keep) = std::env::var_os("SYMPHONY_TEST_MP4_OUT") {
                std::fs::write(keep, &bytes).unwrap();
            }
        }
        cancel.store(true, Ordering::SeqCst);
        assert!(export(&s, &png, &cancel, &|_, _| {}).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
