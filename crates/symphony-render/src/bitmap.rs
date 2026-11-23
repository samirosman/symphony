//! Drawing imported images ([`symphony_core::bitmap`]): decode once, keep
//! the premultiplied pixels in a small cache, draw them through the
//! drawing's transform. The Camera view can share [`decode`] for its GPU
//! textures.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use symphony_core::bitmap::{self, Alpha, ImageFile, ImageFormat, ImageRef};
use symphony_core::doc::Scene;
use symphony_core::math::Affine;
use tiny_skia::{
    FilterQuality, IntSize, Paint, Pattern, Pixmap, Point, Rect, SpreadMode, Transform,
};

/// Decoded pixels kept between frames, so export decodes each image once.
const CACHE_BYTES: usize = 1024 * 1024 * 1024;

struct Entry {
    pixmap: Arc<Pixmap>,
    /// The [`FRAME`] count when it was last drawn.
    used: u64,
}

type Cache = HashMap<(u64, Alpha), Entry>;

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Counts rendered frames, so the cache knows which images the frame
/// being drawn already holds.
static FRAME: AtomicU64 = AtomicU64::new(0);

/// Call once before each frame is drawn.
pub(crate) fn next_frame() {
    FRAME.fetch_add(1, Ordering::Relaxed);
}

/// Decode `file` to premultiplied RGBA, undoing the file's own alpha
/// encoding. `None` if the bytes don't decode; never panics (E7.6).
pub fn decode(file: &ImageFile, alpha: Alpha) -> Option<Pixmap> {
    if !file.size_ok() {
        return None;
    }
    let format = match file.format {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Exr => return decode_exr(&file.bytes, alpha),
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(&file.bytes[..]), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(bitmap::MAX_SIDE);
    limits.max_image_height = Some(bitmap::MAX_SIDE);
    limits.max_alloc = Some(bitmap::MAX_PIXELS * 8);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let rgba = decoded.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut px = rgba.into_raw();
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3];
        for c in &mut p[..3] {
            *c = match alpha {
                // tiny-skia wants colour premultiplied with black.
                Alpha::Straight => ((*c as u16 * a as u16 + 127) / 255) as u8,
                // Already premultiplied with black; clamp stray values.
                Alpha::PremultipliedBlack => (*c).min(a),
                // stored = colour * a + 255 * (1 - a), so the black-based
                // value is stored - (255 - a).
                Alpha::PremultipliedWhite => c.saturating_sub(255 - a),
            };
        }
    }
    Pixmap::from_vec(px, IntSize::from_wh(w, h)?)
}

/// Decode OpenEXR (linear light, half or full float) straight to
/// premultiplied sRGB bytes, the same form [`decode`] gives for PNG and
/// JPEG. Each pixel is converted as the `exr` crate hands it over, so no
/// float copy of the image is ever held: through the `image` crate a
/// 64-megapixel RGBA file needed two 1 GiB float buffers. The
/// premultiplication is undone in linear light, where it was made. Values
/// above 1.0 are clipped (no tone mapping); NaN reads as 0.
///
/// It shows the same part and the same area as `image` 0.25 did: the
/// first flat part with R, G and B channels, placed in the display window.
/// Pixels outside the data window stay as `image` left them, zero: clear
/// for a file with alpha, black for one without.
fn decode_exr(bytes: &[u8], alpha: Alpha) -> Option<Pixmap> {
    use exr::prelude::*;
    let reader = exr::block::read(Cursor::new(bytes), false).ok()?;
    let part = reader.headers().iter().find(|h| {
        !h.deep
            && ["R", "G", "B"]
                .iter()
                .all(|c| h.channels.find_index_of_channel(&Text::from(*c)).is_some())
    })?;
    // The header, not the scene's copy of the size, decides the buffer.
    let display = part.shared_attributes.display_window;
    let w = u32::try_from(display.size.width()).ok()?;
    let h = u32::try_from(display.size.height()).ok()?;
    if !bitmap::size_ok(w, h) {
        return None;
    }
    let offset = part.own_attributes.layer_position - display.position;
    let has_alpha = part
        .channels
        .find_index_of_channel(&Text::from("A"))
        .is_some();
    let blank = if has_alpha { [0; 4] } else { [0, 0, 0, 255] };
    let (row, rows) = (w as usize, h as usize);
    let image = read()
        .no_deep_data()
        .largest_resolution_level()
        .rgba_channels(
            move |_, _| vec![blank; row * rows],
            move |out: &mut Vec<[u8; 4]>, at: Vec2<usize>, (r, g, b, a): (f32, f32, f32, f32)| {
                let p = at.to_i32() + offset;
                if (0..row as i32).contains(&p.x()) && (0..rows as i32).contains(&p.y()) {
                    let i = p.y() as usize * row + p.x() as usize;
                    linear_pixel(&[r, g, b, a], &mut out[i], alpha);
                }
            },
        )
        .first_valid_layer()
        .all_attributes()
        .from_chunks(reader)
        .ok()?;
    let px = image.layer_data.channel_data.pixels;
    Pixmap::from_vec(px.into_flattened(), IntSize::from_wh(w, h)?)
}

fn linear_pixel(p: &[f32], out: &mut [u8; 4], alpha: Alpha) {
    // Not `clamp`, which keeps NaN; a NaN alpha reads as 0.
    let a = match p.get(3) {
        None => 1.0,
        Some(&a) if a > 0.0 => a.min(1.0),
        Some(_) => 0.0,
    };
    let a8 = (a * 255.0).round() as u8;
    for (o, &c) in out.iter_mut().zip(&p[..3]) {
        // Colour premultiplied with black, in linear light.
        let black = match alpha {
            Alpha::Straight => c * a,
            Alpha::PremultipliedBlack => c,
            Alpha::PremultipliedWhite => c - (1.0 - a),
        };
        let straight = if a > 0.0 { black / a } else { 0.0 };
        let s = srgb8(straight) as u16;
        *o = ((s * a8 as u16 + 127) / 255) as u8;
    }
    out[3] = a8;
}

/// Buckets over linear [0, 1) for [`srgb8`]. A bucket (1/8192 wide) is
/// narrower than the closest two thresholds (1/3295 apart, in the linear
/// segment near black), so a bucket holds at most one threshold.
const BUCKETS: usize = 8192;

struct SrgbTable {
    /// The linear value where each code ends: halfway to the next code in
    /// sRGB (IEC 61966-2-1).
    upper: [f32; 255],
    /// The code at the start of each bucket.
    first: Vec<u8>,
}

/// The sRGB code for a linear value: the first code whose upper threshold
/// is above it. The thresholds sit halfway between codes in sRGB, so this
/// matches rounding the exact curve. A table look-up gives the code at the
/// start of the value's bucket; at most one threshold follows inside it.
fn srgb8(linear: f32) -> u8 {
    static TABLE: OnceLock<SrgbTable> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let upper = std::array::from_fn(|i| {
            let s = (i as f32 + 0.5) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        });
        let first = (0..BUCKETS)
            .map(|b| {
                let x = b as f32 / BUCKETS as f32;
                upper.partition_point(|&t| t <= x) as u8
            })
            .collect();
        SrgbTable { upper, first }
    });
    // `max` maps NaN to 0. Every threshold is below 1.0.
    let x = linear.max(0.0);
    if x >= 1.0 {
        return 255;
    }
    let mut code = t.first[(x * BUCKETS as f32) as usize];
    while code < 255 && t.upper[code as usize] <= x {
        code += 1;
    }
    code
}

pub(crate) fn cached(scene: &Scene, r: ImageRef) -> Option<Arc<Pixmap>> {
    let file = scene.images.get(&r.key)?;
    let frame = FRAME.load(Ordering::Relaxed);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(e) = cache.get_mut(&(r.key, r.alpha)) {
        e.used = frame;
        return Some(e.pixmap.clone());
    }
    let p = Arc::new(decode(file, r.alpha)?);
    store(cache, (r.key, r.alpha), p.clone(), frame, CACHE_BYTES);
    Some(p)
}

/// Keep `p` if it fits in `budget` bytes once the least recently drawn
/// images are dropped. Images this frame has drawn stay: when a frame
/// needs more than the budget, dropping them would decode every image
/// again on every frame, so the extra one is drawn and not kept.
fn store(cache: &mut Cache, key: (u64, Alpha), p: Arc<Pixmap>, frame: u64, budget: usize) {
    let size = p.data().len();
    let mut used: usize = cache.values().map(|e| e.pixmap.data().len()).sum();
    while used + size > budget {
        let oldest = cache
            .iter()
            .filter(|(_, e)| e.used != frame)
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| *k);
        let Some(k) = oldest else {
            return;
        };
        if let Some(e) = cache.remove(&k) {
            used -= e.pixmap.data().len();
        }
    }
    cache.insert(
        key,
        Entry {
            pixmap: p,
            used: frame,
        },
    );
}

/// Draw the drawing's image into `pix` with `m` (drawing space to pixels).
pub fn paint_image(pix: &mut Pixmap, scene: &Scene, r: ImageRef, m: Affine) {
    let Some(file) = scene.images.get(&r.key) else {
        return;
    };
    let Some(img) = cached(scene, r) else {
        return;
    };
    let [x0, y0, x1, y1] = r.rect(file, &scene.settings);
    let m = cap_zoom(m, [x1 - x0, y1 - y0], pix);
    let place = m.compose(Affine::translate(x0, y0)).compose(Affine::scale(
        (x1 - x0) / img.width() as f32,
        (y1 - y0) / img.height() as f32,
    ));
    let t = Transform::from_row(place.a, place.b, place.c, place.d, place.tx, place.ty);
    draw(pix, &img, t);
}

/// The largest size, in output pixels, an image may be drawn at. Pegs
/// can scale without limit, but past this the image's coordinates leave
/// the range f32 holds to a fraction of a pixel: at 2^20 px an f32 step
/// is 1/8 px. For the largest image (16384 px a side) that is a zoom of
/// 64 output pixels to one image pixel.
pub const MAX_EXTENT: f32 = 1_048_576.0;

/// `m` (drawing space to `pix`) scaled down about the centre of `pix`
/// so that an image `size` drawing units large is drawn no larger than
/// [`MAX_EXTENT`] in any direction.
pub fn cap_zoom(m: Affine, size: [f32; 2], pix: &Pixmap) -> Affine {
    // The largest stretch of the image's size: the largest singular
    // value of the image-to-output matrix.
    let [w, h] = size;
    let (a, b, c, d) = (m.a * w, m.b * w, m.c * h, m.d * h);
    let sum = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let extent = ((sum + (sum * sum - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt();
    // A NaN is left for the draw to refuse.
    if extent.is_nan() || extent <= MAX_EXTENT {
        return m;
    }
    let f = MAX_EXTENT / extent;
    let (cx, cy) = (pix.width() as f32 / 2.0, pix.height() as f32 / 2.0);
    Affine::translate(cx, cy)
        .compose(Affine::scale(f, f))
        .compose(Affine::translate(-cx, -cy))
        .compose(m)
}

/// Larger output coordinates than this leave the edge hard, as the bone
/// warp does. Anti-aliased, tiny-skia 0.11 drew correctly up to 1e8 and
/// failed at 1e9: nothing drawn, or a debug assertion in its scanner.
const MAX_EDGE: f32 = 1e6;

/// Draw `img` source-over onto `pix` through `t` (image pixels to `pix`
/// pixels), with an anti-aliased edge. `Pixmap::draw_pixmap` fills the
/// same rectangle with anti-aliasing off, so a still image had a hard
/// edge while the same image bent by a bone had a soft one.
fn draw(pix: &mut Pixmap, img: &Pixmap, t: Transform) {
    let Some(rect) = Rect::from_xywh(0.0, 0.0, img.width() as f32, img.height() as f32) else {
        return;
    };
    let mut corners = [
        Point::from_xy(rect.left(), rect.top()),
        Point::from_xy(rect.right(), rect.top()),
        Point::from_xy(rect.right(), rect.bottom()),
        Point::from_xy(rect.left(), rect.bottom()),
    ];
    t.map_points(&mut corners);
    let anti_alias = corners
        .iter()
        .all(|p| p.x.abs() <= MAX_EDGE && p.y.abs() <= MAX_EDGE);
    let paint = Paint {
        shader: Pattern::new(
            img.as_ref(),
            SpreadMode::Pad,
            FilterQuality::Bilinear,
            1.0,
            Transform::identity(),
        ),
        anti_alias,
        ..Paint::default()
    };
    pix.fill_rect(rect, &paint, t, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(px: [u8; 4]) -> ImageFile {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba(px));
        let mut bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        ImageFile {
            format: ImageFormat::Png,
            width: 1,
            height: 1,
            bytes: Arc::new(bytes),
        }
    }

    fn one(px: [u8; 4], alpha: Alpha) -> [u8; 4] {
        let p = decode(&file(px), alpha).unwrap();
        p.data().try_into().unwrap()
    }

    #[test]
    fn undoes_each_alpha_encoding() {
        // Half-transparent pure red, stored three ways.
        assert_eq!(one([255, 0, 0, 128], Alpha::Straight), [128, 0, 0, 128]);
        assert_eq!(
            one([128, 0, 0, 128], Alpha::PremultipliedBlack),
            [128, 0, 0, 128]
        );
        // Over white: red stays 255, green and blue fade to 127.
        assert_eq!(
            one([255, 127, 127, 128], Alpha::PremultipliedWhite),
            [128, 0, 0, 128]
        );
    }

    /// One pixel of a half-float OpenEXR file.
    fn exr(px: [f32; 4]) -> ImageFile {
        use exr::prelude::*;
        let h = px.map(f16::from_f32);
        let ch = SpecificChannels::rgba(move |_: Vec2<usize>| (h[0], h[1], h[2], h[3]));
        let mut bytes = Vec::new();
        Image::from_channels((1, 1), ch)
            .write()
            .to_buffered(Cursor::new(&mut bytes))
            .unwrap();
        ImageFile {
            format: ImageFormat::Exr,
            width: 1,
            height: 1,
            bytes: Arc::new(bytes),
        }
    }

    fn one_exr(px: [f32; 4], alpha: Alpha) -> [u8; 4] {
        let p = decode(&exr(px), alpha).unwrap();
        p.data().try_into().unwrap()
    }

    #[test]
    fn exr_linear_light_becomes_srgb() {
        // Linear 0.2158 is sRGB 128 (0.502).
        assert_eq!(
            one_exr([0.2158, 0.0, 1.0, 1.0], Alpha::PremultipliedBlack),
            [128, 0, 255, 255]
        );
        // Half alpha, premultiplied in linear light: unpremultiplied there
        // to 0.2158 (sRGB 128), then premultiplied again in sRGB.
        assert_eq!(
            one_exr([0.1079, 0.0, 0.5, 0.5], Alpha::PremultipliedBlack),
            [64, 0, 128, 128]
        );
        assert_eq!(
            one_exr([0.2158, 0.0, 1.0, 0.5], Alpha::Straight),
            [64, 0, 128, 128]
        );
        // Over white in linear light: stored = colour * a + (1 - a).
        assert_eq!(
            one_exr([0.6079, 0.5, 1.0, 0.5], Alpha::PremultipliedWhite),
            [64, 0, 128, 128]
        );
    }

    #[test]
    fn exr_clips_bright_and_bad_values() {
        assert_eq!(
            one_exr([4.0, f32::NAN, -1.0, 1.0], Alpha::PremultipliedBlack),
            [255, 0, 0, 255]
        );
        assert_eq!(
            one_exr([1.0, 1.0, 1.0, 0.0], Alpha::PremultipliedBlack),
            [0, 0, 0, 0]
        );
        assert_eq!(
            one_exr([1.0, 1.0, 1.0, f32::NAN], Alpha::PremultipliedBlack),
            [0, 0, 0, 0]
        );
    }

    #[test]
    fn srgb_codes_match_the_exact_curve() {
        for code in 0..=255u8 {
            let s = code as f32 / 255.0;
            let lin = if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            };
            assert_eq!(srgb8(lin), code);
        }
        assert_eq!(srgb8(f32::INFINITY), 255);
    }

    #[test]
    fn srgb_table_matches_a_search_of_the_thresholds() {
        let upper: [f32; 255] = std::array::from_fn(|i| {
            let s = (i as f32 + 0.5) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        });
        let search = |x: f32| upper.partition_point(|&t| t <= x) as u8;
        // Every value a half-float file can hold in [0, 1], then each
        // threshold and its neighbours.
        for bits in 0..=0x3c00u16 {
            let x = exr::prelude::f16::from_bits(bits).to_f32();
            assert_eq!(srgb8(x), search(x), "{x}");
        }
        for &t in &upper {
            for x in [t.next_down(), t, t.next_up()] {
                assert_eq!(srgb8(x), search(x), "{x}");
            }
        }
    }

    /// Write an `exr` image to an [`ImageFile`] of its display size.
    macro_rules! exr_file {
        ($image:expr) => {{
            let image = $image;
            let size = image.attributes.display_window.size;
            let mut bytes = Vec::new();
            image.write().to_buffered(Cursor::new(&mut bytes)).unwrap();
            ImageFile {
                format: ImageFormat::Exr,
                width: size.width() as u32,
                height: size.height() as u32,
                bytes: Arc::new(bytes),
            }
        }};
    }

    #[test]
    fn opaque_rgb_exr_reads_like_rgba() {
        use exr::prelude::*;
        let ch = SpecificChannels::rgb(|_: Vec2<usize>| (0.2158_f32, 0.0_f32, 1.0_f32));
        let f = exr_file!(Image::from_channels((1, 1), ch));
        let p = decode(&f, Alpha::PremultipliedBlack).unwrap();
        assert_eq!(
            p.data(),
            one_exr([0.2158, 0.0, 1.0, 1.0], Alpha::PremultipliedBlack)
        );
    }

    #[test]
    fn exr_data_window_sits_in_the_display_window() {
        use exr::prelude::*;
        // A one-pixel data window at x = 1 of a 3 x 1 display window.
        let ch = SpecificChannels::rgba(|_: Vec2<usize>| (1.0_f32, 1.0_f32, 1.0_f32, 1.0_f32));
        let mut layer = Layer::new(
            (1, 1),
            LayerAttributes::default(),
            Encoding::UNCOMPRESSED,
            ch,
        );
        layer.attributes.layer_position = Vec2(1, 0);
        let mut image = Image::from_layer(layer);
        image.attributes.display_window = IntegerBounds::new((0, 0), (3, 1));
        let p = decode(&exr_file!(image), Alpha::PremultipliedBlack).unwrap();
        assert_eq!((p.width(), p.height()), (3, 1));
        assert_eq!(p.data(), [0, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn exr_size_comes_from_the_file_header() {
        // The scene says 1 x 1; the file is 2 x 1. The pixmap follows the
        // file, which is what the bytes hold.
        use exr::prelude::*;
        let ch = SpecificChannels::rgba(|_: Vec2<usize>| (0.0_f32, 0.0_f32, 0.0_f32, 1.0_f32));
        let mut f = exr_file!(Image::from_channels((2, 1), ch));
        f.width = 1;
        let p = decode(&f, Alpha::PremultipliedBlack).unwrap();
        assert_eq!((p.width(), p.height()), (2, 1));
    }

    #[test]
    fn garbage_exr_decodes_to_nothing() {
        let mut f = exr([0.0; 4]);
        let mut cut = f.bytes.to_vec();
        cut.truncate(cut.len() / 2);
        f.bytes = Arc::new(cut);
        assert!(decode(&f, Alpha::Straight).is_none());
    }

    #[test]
    fn a_full_cache_drops_old_images_and_keeps_this_frames() {
        let px = |n: u32| Arc::new(Pixmap::new(n, 1).unwrap());
        let key = |k: u64| (k, Alpha::Straight);
        let mut c = Cache::new();
        // Budget: three 4-byte images.
        store(&mut c, key(1), px(1), 1, 12);
        store(&mut c, key(2), px(1), 2, 12);
        store(&mut c, key(3), px(1), 3, 12);
        // Frame 3 needs a fourth: image 1, drawn longest ago, goes.
        store(&mut c, key(4), px(1), 3, 12);
        assert!(!c.contains_key(&key(1)));
        assert_eq!(c.len(), 3);
        // Frame 4 draws 2, 3 and 4, then needs a fifth: nothing of this
        // frame is dropped, and the fifth is not kept.
        for k in 2..=4 {
            c.get_mut(&key(k)).unwrap().used = 4;
        }
        store(&mut c, key(5), px(1), 4, 12);
        assert!(!c.contains_key(&key(5)));
        assert_eq!(c.len(), 3);
    }

    /// An opaque red `n` by `n` image, drawn through `t` onto 8 by 8.
    fn drawn(n: u32, t: Transform) -> Pixmap {
        let mut img = Pixmap::new(n, n).unwrap();
        img.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        let mut pix = Pixmap::new(8, 8).unwrap();
        draw(&mut pix, &img, t);
        pix
    }

    fn alpha(p: &Pixmap, x: u32, y: u32) -> u8 {
        p.pixel(x, y).unwrap().alpha()
    }

    #[test]
    fn the_edge_is_anti_aliased() {
        // Left and top edges half a pixel into column and row 2.
        let p = drawn(4, Transform::from_translate(2.5, 2.5));
        let edge = alpha(&p, 2, 4);
        assert!((100..=155).contains(&edge), "edge alpha {edge}");
        assert!(alpha(&p, 2, 2) < edge, "corner is a quarter covered");
        assert_eq!(alpha(&p, 4, 4), 255);
        assert_eq!(alpha(&p, 1, 4), 0);
    }

    #[test]
    fn an_edge_on_the_pixel_grid_stays_sharp() {
        let p = drawn(4, Transform::from_translate(2.0, 2.0));
        for i in 2..6 {
            assert_eq!(alpha(&p, 2, i), 255);
            assert_eq!(alpha(&p, 5, i), 255);
            assert_eq!(alpha(&p, 1, i), 0);
            assert_eq!(alpha(&p, 6, i), 0);
        }
    }

    #[test]
    fn a_huge_zoom_falls_back_to_a_hard_edge() {
        // Three corners far past MAX_EDGE. Anti-aliased, tiny-skia 0.11
        // draws nothing here.
        let t = Transform::from_row(1e9, 0.0, 0.0, 1e9, 3.3, 3.3);
        let p = drawn(4, t);
        assert_eq!(alpha(&p, 4, 4), 255);
        assert_eq!(alpha(&p, 2, 2), 0);
    }

    /// A 16 by 16 opaque checker of 4-pixel red and blue squares.
    fn checker() -> Pixmap {
        let mut img = Pixmap::new(16, 16).unwrap();
        let [red, blue] = [[255, 0, 0, 255], [0, 0, 255, 255]];
        for (i, px) in img.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % 16 / 4, i / 16 / 4);
            *px = if (x + y) % 2 == 0 { red } else { blue };
        }
        img
    }

    /// The checker drawn through `t` onto a clear 32 by 32 pixmap.
    fn golden(name: &str, t: Transform) {
        let mut pix = Pixmap::new(32, 32).unwrap();
        draw(&mut pix, &checker(), t);
        crate::golden::check(name, &pix);
    }

    #[test]
    fn golden_edges_of_a_still_image() {
        golden("still-on-grid", Transform::from_translate(8.0, 8.0));
        golden("still-half-pixel", Transform::from_translate(8.5, 8.25));
        golden(
            "still-turned-30",
            Transform::from_rotate_at(30.0, 8.0, 8.0).post_translate(8.0, 8.0),
        );
        golden(
            "still-scaled",
            Transform::from_scale(1.37, 1.37).post_translate(4.3, 5.6),
        );
    }

    #[test]
    fn golden_hard_edge_at_a_huge_zoom() {
        // One checker pixel is 1e9 px wide: the frame shows its corner.
        golden(
            "still-huge-zoom",
            Transform::from_row(1e9, 0.0, 0.0, 1e9, 10.3, 10.3),
        );
    }

    /// The largest size of a `w` by `h` image under `m`, measured on a
    /// circle of directions.
    fn extent(m: Affine, [w, h]: [f32; 2]) -> f32 {
        let (zx, zy) = m.apply(0.0, 0.0);
        (0..360)
            .map(|deg| {
                let r = (deg as f32).to_radians();
                let (x, y) = m.apply(r.cos() * w, r.sin() * h);
                (x - zx).hypot(y - zy)
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_zoom_is_capped_about_the_frame_centre() {
        let pix = Pixmap::new(200, 100).unwrap();
        // Turned and squashed, then zoomed far past the cap.
        let m = Affine::translate(30.0, 40.0)
            .compose(Affine::rotate_deg(30.0))
            .compose(Affine::scale(1e6, 3e5));
        let size = [40.0, 160.0];
        let capped = cap_zoom(m, size, &pix);
        let e = extent(capped, size);
        assert!((e - MAX_EXTENT).abs() < 0.01 * MAX_EXTENT, "extent {e}");
        // The frame centre keeps the same drawing point.
        let inv = |m: Affine| m.inverse().unwrap().apply(100.0, 50.0);
        let ((ax, ay), (bx, by)) = (inv(m), inv(capped));
        assert!((ax - bx).abs() < 1e-3 && (ay - by).abs() < 1e-3);
        // The turn stays.
        let dir = |m: Affine| m.b.atan2(m.a);
        assert!((dir(m) - dir(capped)).abs() < 1e-5);
    }

    #[test]
    fn a_zoom_below_the_cap_is_unchanged() {
        let pix = Pixmap::new(200, 100).unwrap();
        // A tiny image filling the frame is far below the cap.
        let m = Affine::translate(3.0, 4.0).compose(Affine::scale(100.0, -100.0));
        assert_eq!(cap_zoom(m, [2.0, 1.0], &pix), m);
        let bad = Affine::scale(f32::NAN, 1.0);
        assert!(cap_zoom(bad, [1.0, 1.0], &pix).a.is_nan());
    }

    #[test]
    fn garbage_bytes_decode_to_nothing() {
        let mut f = file([0, 0, 0, 255]);
        f.bytes = Arc::new(b"\x89PNG\r\n\x1a\nbroken".to_vec());
        assert!(decode(&f, Alpha::Straight).is_none());
    }
}
