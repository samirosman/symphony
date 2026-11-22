//! Drawing imported images ([`symphony_core::bitmap`]): decode once, keep
//! the premultiplied pixels in a small cache, draw them through the
//! drawing's transform. The Camera view can share [`decode`] for its GPU
//! textures.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use rayon::prelude::*;
use symphony_core::bitmap::{self, Alpha, ImageFile, ImageFormat, ImageRef};
use symphony_core::doc::Scene;
use symphony_core::math::Affine;
use tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};

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
        ImageFormat::Exr => image::ImageFormat::OpenExr,
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(&file.bytes[..]), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(bitmap::MAX_SIDE);
    limits.max_image_height = Some(bitmap::MAX_SIDE);
    // EXR decodes to four f32 channels: 16 bytes a pixel.
    let per_pixel = if file.format == ImageFormat::Exr {
        16
    } else {
        8
    };
    limits.max_alloc = Some(bitmap::MAX_PIXELS * per_pixel);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    if file.format == ImageFormat::Exr {
        let (w, h) = (decoded.width(), decoded.height());
        return match decoded {
            // No alpha channel: read the RGB samples as they are, rather
            // than copy them into a second, four-channel buffer.
            image::DynamicImage::ImageRgb32F(img) => linear_to_pixmap(w, h, &img, 3, alpha),
            other => linear_to_pixmap(w, h, &other.into_rgba32f(), 4, alpha),
        };
    }
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

/// Linear-light float pixels (OpenEXR) to premultiplied sRGB bytes, the
/// same form [`decode`] gives for PNG and JPEG. `samples` holds RGB (an
/// opaque image) or RGBA, per `channels`. The premultiplication is undone
/// in linear light, where it was made. Values above 1.0 are clipped (no
/// tone mapping); NaN reads as 0.
fn linear_to_pixmap(
    w: u32,
    h: u32,
    samples: &[f32],
    channels: usize,
    alpha: Alpha,
) -> Option<Pixmap> {
    let row = w as usize;
    let mut out = vec![0; row * h as usize * 4];
    // Rows in parallel: a 64-megapixel image has 200 million samples.
    samples
        .par_chunks(row * channels)
        .zip(out.par_chunks_mut(row * 4))
        .for_each(|(src, dst)| {
            for (p, o) in src.chunks_exact(channels).zip(dst.as_chunks_mut::<4>().0) {
                linear_pixel(p, o, alpha);
            }
        });
    Pixmap::from_vec(out, IntSize::from_wh(w, h)?)
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

fn cached(scene: &Scene, r: ImageRef) -> Option<Arc<Pixmap>> {
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
    let place = m.compose(Affine::translate(x0, y0)).compose(Affine::scale(
        (x1 - x0) / img.width() as f32,
        (y1 - y0) / img.height() as f32,
    ));
    let t = Transform::from_row(place.a, place.b, place.c, place.d, place.tx, place.ty);
    let paint = PixmapPaint {
        quality: FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    pix.draw_pixmap(0, 0, (*img).as_ref(), &paint, t, None);
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

    #[test]
    fn opaque_rgb_exr_reads_like_rgba() {
        let rgb = linear_to_pixmap(1, 1, &[0.2158, 0.0, 1.0], 3, Alpha::PremultipliedBlack);
        let rgba = linear_to_pixmap(1, 1, &[0.2158, 0.0, 1.0, 1.0], 4, Alpha::PremultipliedBlack);
        assert_eq!(rgb.unwrap().data(), rgba.unwrap().data());
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

    #[test]
    fn garbage_bytes_decode_to_nothing() {
        let mut f = file([0, 0, 0, 255]);
        f.bytes = Arc::new(b"\x89PNG\r\n\x1a\nbroken".to_vec());
        assert!(decode(&f, Alpha::Straight).is_none());
    }
}
