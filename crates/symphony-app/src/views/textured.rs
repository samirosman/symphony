//! Textured vector strokes on the canvas. Each stroke's coverage comes
//! from `symphony_vector::stamp` (the same code export uses) and is cached
//! as a texture, so panning moves it without stamping it again. Zooming,
//! rotating or editing the line re-stamps it, re-sampling the texture the
//! way Harmony does for textured vector brushes.
//!
//! Stamping can take tens of milliseconds for a big brush, so it never
//! blocks a frame (DESIGN §17). Finished strokes are re-stamped on a
//! background thread, and the last image is shown moved to the new
//! position until then. The stroke being drawn is stamped a piece at a
//! time ([`Live`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, Pos2, Rect};
use symphony_core::id::Id;
use symphony_core::math::Affine;
use symphony_core::stroke::{Stroke, StrokeTexture, Texture};
use symphony_vector::stamp::{Coverage, Stamper};

/// A stamped stroke on the GPU, and the drawing -> pixel transform it was
/// stamped with (so it can be shown moved while a new one is made).
#[derive(Clone)]
struct Image {
    handle: egui::TextureHandle,
    /// Pixel rectangle [x, y, w, h] in the space of `px`.
    rect: [f32; 4],
    px: Affine,
}

impl Image {
    fn upload(ctx: &egui::Context, c: &Coverage, px: Affine, name: &str) -> Self {
        let pixels: Vec<Color32> = c
            .data
            .iter()
            .map(|&a| Color32::from_white_alpha(a))
            .collect();
        let image = egui::ColorImage {
            size: [c.width as usize, c.height as usize],
            source_size: egui::vec2(c.width as f32, c.height as f32),
            pixels,
        };
        Image {
            handle: ctx.load_texture(name, image, egui::TextureOptions::LINEAR),
            rect: [c.x as f32, c.y as f32, c.width as f32, c.height as f32],
            px,
        }
    }

    /// Draw it for the transform `now` (drawing -> pixels): exact when
    /// `now` is the transform it was made with, moved otherwise.
    fn draw(&self, painter: &egui::Painter, now: Affine, ppp: f32, colour: Color32) {
        let Some(back) = self.px.inverse() else {
            return;
        };
        let map = now.compose(back);
        let [x, y, w, h] = self.rect;
        let corner = |cx: f32, cy: f32| {
            let (px, py) = map.apply(cx, cy);
            Pos2::new(px / ppp, py / ppp)
        };
        let mut mesh = egui::Mesh::with_texture(self.handle.id());
        for (cx, cy, u, v) in [
            (x, y, 0.0, 0.0),
            (x + w, y, 1.0, 0.0),
            (x + w, y + h, 1.0, 1.0),
            (x, y + h, 0.0, 1.0),
        ] {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: corner(cx, cy),
                uv: Pos2::new(u, v),
                color: colour,
            });
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        painter.add(egui::Shape::mesh(mesh));
    }
}

struct Job {
    key: u64,
    stroke: Stroke,
    tex: StrokeTexture,
    tip: Option<Arc<Texture>>,
    grain: Option<Arc<Texture>>,
    local: Affine,
    clip: [i32; 4],
}

struct Done {
    key: u64,
    id: Id,
    coverage: Option<Coverage>,
    local: Affine,
}

struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// The newest key wanted per stroke; older queued jobs are skipped.
    latest: Arc<Mutex<HashMap<Id, u64>>>,
}

impl Worker {
    fn start(ctx: &egui::Context) -> Self {
        let (jobs, rx) = channel::<Job>();
        let (tx, done) = channel();
        let latest: Arc<Mutex<HashMap<Id, u64>>> = Arc::default();
        let wanted = latest.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("symphony-stamp".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let current = wanted
                        .lock()
                        .map(|m| m.get(&job.stroke.id) == Some(&job.key))
                        .unwrap_or(true);
                    if !current {
                        continue;
                    }
                    let coverage = symphony_vector::stamp::rasterize(
                        &job.stroke,
                        &job.tex,
                        job.tip.as_deref(),
                        job.grain.as_deref(),
                        job.local,
                        job.clip,
                    );
                    let id = job.stroke.id;
                    if tx
                        .send(Done {
                            key: job.key,
                            id,
                            coverage,
                            local: job.local,
                        })
                        .is_err()
                    {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok();
        Worker { jobs, done, latest }
    }
}

struct Entry {
    image: Option<Image>,
    used: u64,
}

#[derive(Default)]
pub struct Cache {
    entries: HashMap<u64, Entry>,
    /// The last image shown for each stroke, to stand in while re-stamping.
    last: HashMap<Id, (Image, u64)>,
    pending: HashSet<u64>,
    textures: HashMap<u64, Arc<Texture>>,
    worker: Option<Worker>,
    frame: u64,
    /// Plain vector lines' cached triangles (they draw through here too).
    pub vectors: super::vector_cache::VectorMeshes,
}

/// Stamped strokes are rebuilt per this many pixels of clip window, so
/// small pans reuse them.
const CLIP_GRID: f32 = 512.0;
/// Most cached strokes; old ones are dropped first.
const MAX_ENTRIES: usize = 512;
/// Strokes cheaper than this (stamped pixels, roughly) are stamped at once
/// the first time they appear, so they never pop in.
const SYNC_BUDGET: f32 = 2_000_000.0;

impl Cache {
    /// Call once per frame before drawing: collect finished stamping and
    /// forget strokes not drawn lately.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        self.vectors.begin_frame();
        if let Some(w) = &self.worker {
            while let Ok(d) = w.done.try_recv() {
                self.pending.remove(&d.key);
                let image = d
                    .coverage
                    .as_ref()
                    .map(|c| Image::upload(ctx, c, d.local, "stroke"));
                // Ignore results for strokes that have changed again since.
                let current = w
                    .latest
                    .lock()
                    .map(|l| l.get(&d.id) == Some(&d.key))
                    .unwrap_or(true);
                if current {
                    self.entries.insert(
                        d.key,
                        Entry {
                            image,
                            used: self.frame,
                        },
                    );
                }
            }
        }
        let keep_after = self.frame.saturating_sub(2);
        self.entries.retain(|_, e| e.used >= keep_after);
        self.last.retain(|_, (_, used)| *used >= keep_after);
        if self.entries.len() > MAX_ENTRIES {
            let mut ages: Vec<u64> = self.entries.values().map(|e| e.used).collect();
            ages.sort_unstable();
            let cut = ages[ages.len() - MAX_ENTRIES];
            self.entries.retain(|_, e| e.used >= cut);
        }
    }

    fn texture(
        &mut self,
        textures: &BTreeMap<u64, Texture>,
        key: Option<u64>,
    ) -> Option<Arc<Texture>> {
        let key = key?;
        if let Some(t) = self.textures.get(&key) {
            return Some(t.clone());
        }
        let t = Arc::new(textures.get(&key)?.clone());
        self.textures.insert(key, t.clone());
        Some(t)
    }

    /// Hand over the image of a stroke that was just drawn, so it shows
    /// without a gap once it's committed.
    pub fn adopt(&mut self, id: Id, live: Live) {
        if let Some(image) = live.image {
            self.last.insert(id, (image, self.frame));
        }
    }

    /// Draw `stroke` with its texture under `m` (drawing -> screen points).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        textures: &BTreeMap<u64, Texture>,
        stroke: &Stroke,
        tex: &StrokeTexture,
        colour: Color32,
        m: Affine,
    ) {
        let ppp = ctx.pixels_per_point();
        // Drawing -> physical pixels, split into a whole-pixel offset and a
        // remainder, so a pan only moves the cached image.
        let px = Affine::scale(ppp, ppp).compose(m);
        let (ox, oy) = (px.tx.floor(), px.ty.floor());
        let local = Affine {
            tx: px.tx - ox,
            ty: px.ty - oy,
            ..px
        };
        let visible = painter.clip_rect();
        let snap = |v: f32, up: bool| {
            let g = (v / CLIP_GRID).floor() + if up { 2.0 } else { -1.0 };
            (g * CLIP_GRID) as i32
        };
        let clip = [
            snap(visible.left() * ppp - ox, false),
            snap(visible.top() * ppp - oy, false),
            snap(visible.right() * ppp - ox, true),
            snap(visible.bottom() * ppp - oy, true),
        ];
        let key = key(stroke, tex, local, clip);
        let frame = self.frame;
        if let Some(e) = self.entries.get_mut(&key) {
            e.used = frame;
            if let Some(img) = &e.image {
                img.draw(painter, px, ppp, colour);
                self.last.insert(stroke.id, (img.clone(), frame));
            }
            return;
        }
        let tip = self.texture(textures, tex.tip);
        let grain = self.texture(textures, tex.grain);
        let stale = self.last.get_mut(&stroke.id);
        let cost = stroke.points.len() as f32
            * (stroke.max_width * symphony_vector::scale_of(px)).powi(2)
            / (tex.spacing.max(0.01) * 4.0);
        // Small strokes are stamped now, whether new or just edited (the
        // Line Editor reshaping a line), so they never lag behind.
        if cost < SYNC_BUDGET {
            let c = symphony_vector::stamp::rasterize(
                stroke,
                tex,
                tip.as_deref(),
                grain.as_deref(),
                local,
                clip,
            );
            let image = c.as_ref().map(|c| Image::upload(ctx, c, local, "stroke"));
            if let Some(img) = &image {
                img.draw(painter, px, ppp, colour);
                self.last.insert(stroke.id, (img.clone(), frame));
            }
            self.entries.insert(key, Entry { image, used: frame });
            return;
        }
        // Otherwise stamp in the background and show the last image moved
        // into place (or the plain vector shape) until it's ready.
        match stale {
            Some((img, used)) => {
                *used = frame;
                img.draw(painter, px, ppp, colour);
            }
            None => {
                let mesh = symphony_vector::tessellate(stroke, m, 0.6);
                let mut out = egui::Mesh::default();
                for v in &mesh.vertices {
                    out.colored_vertex(Pos2::new(v[0], v[1]), colour.gamma_multiply(0.35));
                }
                out.indices = mesh.indices;
                painter.add(egui::Shape::mesh(out));
            }
        }
        if self.pending.insert(key) {
            let worker = self.worker.get_or_insert_with(|| Worker::start(ctx));
            if let Ok(mut l) = worker.latest.lock() {
                l.insert(stroke.id, key);
            }
            let _ = worker.jobs.send(Job {
                key,
                stroke: stroke.clone(),
                tex: *tex,
                tip,
                grain,
                local,
                clip,
            });
        }
    }
}

/// The stroke being drawn, stamped a piece at a time.
pub struct Live {
    px: Affine,
    stamper: Stamper,
    image: Option<Image>,
    points: usize,
}

impl Live {
    /// Stamp what's new in `stroke` and draw it. Starts over if the view
    /// changed mid-stroke.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        slot: &mut Option<Live>,
        ctx: &egui::Context,
        painter: &egui::Painter,
        textures: &BTreeMap<u64, Texture>,
        stroke: &Stroke,
        tex: &StrokeTexture,
        colour: Color32,
        m: Affine,
    ) {
        let ppp = ctx.pixels_per_point();
        let px = Affine::scale(ppp, ppp).compose(m);
        let visible = painter.clip_rect();
        let clip = [
            (visible.left() * ppp).floor() as i32,
            (visible.top() * ppp).floor() as i32,
            (visible.right() * ppp).ceil() as i32,
            (visible.bottom() * ppp).ceil() as i32,
        ];
        let restart = slot
            .as_ref()
            .is_none_or(|l| l.px != px || stroke.points.len() < l.points);
        if restart {
            *slot = Some(Live {
                px,
                stamper: Stamper::new(*tex, px, clip),
                image: None,
                points: 0,
            });
        }
        let Some(live) = slot.as_mut() else { return };
        let tip = tex.tip.and_then(|k| textures.get(&k));
        let grain = tex.grain.and_then(|k| textures.get(&k));
        if stroke.points.len() != live.points || live.image.is_none() {
            live.stamper.extend(stroke, tip, grain);
            live.points = stroke.points.len();
            if let Some(c) = live.stamper.coverage() {
                live.image = Some(Image::upload(ctx, &c, px, "live-stroke"));
            }
        }
        if let Some(img) = &live.image {
            img.draw(painter, px, ppp, colour);
        }
    }
}

fn key(stroke: &Stroke, tex: &StrokeTexture, m: Affine, clip: [i32; 4]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in &stroke.points {
        (p.x.to_bits(), p.y.to_bits(), p.pressure.to_bits()).hash(&mut h);
    }
    // An erased stroke's outline changes what shows.
    for q in stroke.shape.iter().flatten() {
        (q[0].to_bits(), q[1].to_bits()).hash(&mut h);
    }
    (stroke.min_width.to_bits(), stroke.max_width.to_bits()).hash(&mut h);
    (
        tex.tip,
        tex.grain,
        tex.spacing.to_bits(),
        tex.opacity.to_bits(),
    )
        .hash(&mut h);
    (tex.angle.to_bits(), tex.follow, tex.grain_scale.to_bits()).hash(&mut h);
    (tex.flow.to_bits(), tex.hardness.to_bits()).hash(&mut h);
    for v in tex.opacity_by_pressure {
        v.to_bits().hash(&mut h);
    }
    for v in [m.a, m.b, m.c, m.d, m.tx, m.ty] {
        v.to_bits().hash(&mut h);
    }
    clip.hash(&mut h);
    h.finish()
}

/// A sample stroke drawn with a textured brush, for Tool Properties and
/// the preset list. Uses the library textures, so it works before any
/// stroke is drawn.
pub fn sample(
    painter: &egui::Painter,
    rect: Rect,
    tex: &StrokeTexture,
    min_width: f32,
    max_width: f32,
    ink: Color32,
) {
    use symphony_core::stroke::{Point, StrokeKind};
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let n = 48;
    let max_width = max_width.min(rect.height() - 6.0).max(0.5);
    let min_width = min_width.min(max_width);
    let wave = (rect.height() * 0.15).min(8.0);
    let points: Vec<Point> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            Point {
                x: 10.0 + t * (rect.width() - 20.0),
                y: rect.height() * 0.5 + (t * std::f32::consts::TAU).sin() * wave,
                pressure: (t * std::f32::consts::PI).sin(),
            }
        })
        .collect();
    let stroke = Stroke {
        id: symphony_core::id::Id(0),
        kind: StrokeKind::Brush,
        art: Default::default(),
        swatch: symphony_core::id::Id(0),
        min_width,
        max_width,
        points,
        tips: Default::default(),
        texture: Some(*tex),
        shape: Vec::new(),
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key(&stroke, tex, Affine::scale(ppp, ppp), [0, 0, 0, 0]).hash(&mut h);
    (rect.width().to_bits(), rect.height().to_bits()).hash(&mut h);
    let id = egui::Id::unique(("brush-sample", h.finish()));
    let handle = ctx
        .data(|d| d.get_temp::<egui::TextureHandle>(id))
        .or_else(|| {
            let tip = tex.tip.and_then(crate::brush_import::library_texture);
            let grain = tex.grain.and_then(crate::brush_import::library_texture);
            let clip = [
                0,
                0,
                (rect.width() * ppp) as i32,
                (rect.height() * ppp) as i32,
            ];
            let c = symphony_vector::stamp::rasterize(
                &stroke,
                tex,
                tip.as_deref(),
                grain.as_deref(),
                Affine::scale(ppp, ppp),
                clip,
            )?;
            let mut pixels = vec![Color32::TRANSPARENT; (clip[2] * clip[3]).max(0) as usize];
            for row in 0..c.height as i32 {
                for col in 0..c.width as i32 {
                    let (x, y) = (c.x + col, c.y + row);
                    if x < clip[2] && y < clip[3] {
                        pixels[(y * clip[2] + x) as usize] = Color32::from_white_alpha(
                            c.data[(row * c.width as i32 + col) as usize],
                        );
                    }
                }
            }
            let image = egui::ColorImage {
                size: [clip[2] as usize, clip[3] as usize],
                source_size: egui::vec2(clip[2] as f32, clip[3] as f32),
                pixels,
            };
            let handle = ctx.load_texture("brush-sample", image, egui::TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
            Some(handle)
        });
    if let Some(handle) = handle {
        painter.image(
            handle.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            ink,
        );
    }
}
