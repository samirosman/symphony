//! Drawing imported images ([`symphony_core::bitmap`]): decode once, keep
//! the premultiplied pixels in a small cache, draw them through the
//! drawing's transform. The Camera view can share [`decode`] for its GPU
//! textures.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use symphony_core::bitmap::{self, Alpha, ImageFile, ImageFormat, ImageRef};
use symphony_core::doc::Scene;
use symphony_core::math::Affine;
use tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};

/// Decoded pixels kept between frames, so export decodes each image once.
const CACHE_BYTES: usize = 1024 * 1024 * 1024;

type Cache = HashMap<(u64, Alpha), Arc<Pixmap>>;

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Decode `file` to premultiplied RGBA, undoing the file's own alpha
/// encoding. `None` if the bytes don't decode; never panics (E7.6).
pub fn decode(file: &ImageFile, alpha: Alpha) -> Option<Pixmap> {
    if !file.size_ok() {
        return None;
    }
    let format = match file.format {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(&file.bytes[..]), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(bitmap::MAX_SIDE);
    limits.max_image_height = Some(bitmap::MAX_SIDE);
    limits.max_alloc = Some(bitmap::MAX_PIXELS * 4 * 2);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.into_rgba8();
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

fn cached(scene: &Scene, r: ImageRef) -> Option<Arc<Pixmap>> {
    let file = scene.images.get(&r.key)?;
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(p) = cache.get(&(r.key, r.alpha)) {
        return Some(p.clone());
    }
    let p = Arc::new(decode(file, r.alpha)?);
    let used: usize = cache.values().map(|p| p.data().len()).sum();
    if used + p.data().len() > CACHE_BYTES {
        cache.clear();
    }
    cache.insert((r.key, r.alpha), p.clone());
    Some(p)
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

    #[test]
    fn garbage_bytes_decode_to_nothing() {
        let mut f = file([0, 0, 0, 255]);
        f.bytes = Arc::new(b"\x89PNG\r\n\x1a\nbroken".to_vec());
        assert!(decode(&f, Alpha::Straight).is_none());
    }
}
