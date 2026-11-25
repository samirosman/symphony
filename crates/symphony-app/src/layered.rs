//! Files that hold several pictures: layers (Photoshop PSD, OpenRaster
//! ORA) or frames (animated GIF, PNG and WebP), and Krita files (their
//! flattened picture). Each part comes in as its own layer (layers) or
//! as a timed sequence on one layer (frames), as pictures or, when
//! chosen, vectorized.

use std::io::Read;

use image::{AnimationDecoder, RgbaImage};

/// One layer's worth: its name, and its frames with how many scene
/// frames each is held for.
pub struct Part {
    pub name: String,
    pub frames: Vec<(RgbaImage, u32)>,
}

/// What a file holds, or None when it's an ordinary single picture.
/// `layered`: the parts are a file's layers, bottom first (they stack in
/// that order and keep their place, rather than a full-frame picture
/// going to the bottom).
pub struct Many {
    pub parts: Vec<Part>,
    pub layered: bool,
}

/// Read a file that holds several pictures. `fps` times animations.
pub fn read(stem: &str, bytes: &[u8], ext: &str, fps: f32) -> Result<Option<Many>, String> {
    let fps = if fps > 0.0 { fps } else { 24.0 };
    match ext {
        "psd" => psd(stem, bytes).map(Some),
        "ora" => ora(stem, bytes).map(Some),
        "kra" => kra(stem, bytes).map(Some),
        "gif" | "png" | "apng" | "webp" => Ok(animation(stem, bytes, ext, fps)),
        _ => Ok(None),
    }
}

fn psd(stem: &str, bytes: &[u8]) -> Result<Many, String> {
    // The PSD reader can panic on damaged files: keep that from taking
    // the import with it.
    let parsed = crate::crash::guarded(|| {
        let psd = psd::Psd::from_bytes(bytes).map_err(|e| e.to_string())?;
        let (w, h) = (psd.width(), psd.height());
        if w == 0 || h == 0 || w > 16_384 || h > 16_384 {
            return Err("its size isn't one Symphony can open".to_owned());
        }
        let mut parts = Vec::new();
        for layer in psd.layers() {
            // The reader calls flag bit 1 "visible", but Photoshop sets it
            // for hidden layers (both layers of a plain two-layer file
            // read as "not visible").
            if layer.visible() {
                continue;
            }
            let mut px = layer.rgba();
            if px.len() != (w * h * 4) as usize {
                continue;
            }
            let opacity = layer.opacity() as u32;
            if opacity < 255 {
                for a in px.iter_mut().skip(3).step_by(4) {
                    *a = (*a as u32 * opacity / 255) as u8;
                }
            }
            if px.iter().skip(3).step_by(4).all(|a| *a == 0) {
                continue;
            }
            let Some(img) = RgbaImage::from_raw(w, h, px) else {
                continue;
            };
            parts.push(Part {
                name: format!("{stem} - {}", layer.name()),
                frames: vec![(img, 1)],
            });
        }
        // The reader lists layers top first (its own sample's top layer is
        // the colour of the flattened picture); stack bottom first.
        parts.reverse();
        if parts.is_empty() {
            // No separate layers: the flattened picture.
            if let Some(img) = RgbaImage::from_raw(w, h, psd.rgba()) {
                parts.push(Part {
                    name: stem.to_owned(),
                    frames: vec![(img, 1)],
                });
            }
        }
        Ok(parts)
    })
    .ok_or_else(|| "the Photoshop file looks damaged".to_owned())??;
    if parsed.is_empty() {
        return Err("the Photoshop file has nothing visible in it".into());
    }
    Ok(Many {
        parts: parsed,
        layered: true,
    })
}

fn zip_file(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
    let mut f = archive.by_name(name).ok()?;
    // Pictures in these files are at most a few hundred MB.
    if f.size() > 1 << 30 {
        return None;
    }
    let mut out = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut out).ok()?;
    Some(out)
}

/// OpenRaster (Krita, MyPaint, GIMP and others can save it): a zip with
/// `stack.xml` listing PNG layers, top first.
fn ora(stem: &str, bytes: &[u8]) -> Result<Many, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "not an OpenRaster file Symphony can read")?;
    let stack = zip_file(&mut archive, "stack.xml").ok_or("it has no layer list")?;
    let stack = String::from_utf8_lossy(&stack).into_owned();
    let attr = |tag: &str, name: &str| -> Option<String> {
        let key = format!(" {name}=\"");
        let at = tag.find(&key)? + key.len();
        let end = tag[at..].find('"')? + at;
        Some(unescape(&tag[at..end]))
    };
    let image_tag = stack
        .split('<')
        .find(|t| t.starts_with("image"))
        .ok_or("its layer list is empty")?;
    let (w, h): (u32, u32) = (
        attr(image_tag, "w")
            .and_then(|v| v.parse().ok())
            .ok_or("it has no size")?,
        attr(image_tag, "h")
            .and_then(|v| v.parse().ok())
            .ok_or("it has no size")?,
    );
    if w == 0 || h == 0 || w > 16_384 || h > 16_384 {
        return Err("its size isn't one Symphony can open".into());
    }
    let mut parts = Vec::new();
    for tag in stack.split('<').filter(|t| t.starts_with("layer ")) {
        if attr(tag, "visibility").is_some_and(|v| v == "hidden") {
            continue;
        }
        let Some(src) = attr(tag, "src") else {
            continue;
        };
        let Some(png) = zip_file(&mut archive, &src) else {
            continue;
        };
        let Ok(layer) = image::load_from_memory(&png) else {
            continue;
        };
        let (x, y): (i64, i64) = (
            attr(tag, "x").and_then(|v| v.parse().ok()).unwrap_or(0),
            attr(tag, "y").and_then(|v| v.parse().ok()).unwrap_or(0),
        );
        let opacity: f32 = attr(tag, "opacity")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0f32)
            .clamp(0.0, 1.0);
        let mut canvas = RgbaImage::new(w, h);
        image::imageops::overlay(&mut canvas, &layer.into_rgba8(), x, y);
        if opacity < 1.0 {
            for p in canvas.pixels_mut() {
                p.0[3] = (p.0[3] as f32 * opacity).round() as u8;
            }
        }
        let name = attr(tag, "name").unwrap_or_else(|| "Layer".into());
        parts.push(Part {
            name: format!("{stem} - {name}"),
            frames: vec![(canvas, 1)],
        });
    }
    if parts.is_empty() {
        return Err("it has no visible layers".into());
    }
    // Listed top first; stack bottom first.
    parts.reverse();
    Ok(Many {
        parts,
        layered: true,
    })
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Krita files keep their layers in Krita's own tiled format; the
/// flattened picture they also hold comes in.
fn kra(stem: &str, bytes: &[u8]) -> Result<Many, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "not a Krita file Symphony can read")?;
    let png = zip_file(&mut archive, "mergedimage.png")
        .ok_or("it has no flattened picture (save it again in Krita)")?;
    let img = image::load_from_memory(&png)
        .map_err(|_| "its picture couldn't be read")?
        .into_rgba8();
    Ok(Many {
        parts: vec![Part {
            name: stem.to_owned(),
            frames: vec![(img, 1)],
        }],
        layered: false,
    })
}

/// An animated GIF, PNG or WebP as a timed sequence; None when it's a
/// still picture.
fn animation(stem: &str, bytes: &[u8], ext: &str, fps: f32) -> Option<Many> {
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
    let cursor = || std::io::Cursor::new(bytes);
    let frames: Vec<image::Frame> = match ext {
        "gif" => GifDecoder::new(cursor())
            .ok()?
            .into_frames()
            .take(5000)
            .collect::<Result<_, _>>()
            .ok()?,
        "png" | "apng" => {
            let d = PngDecoder::new(cursor()).ok()?;
            if !d.is_apng().ok()? {
                return None;
            }
            d.apng()
                .ok()?
                .into_frames()
                .take(5000)
                .collect::<Result<_, _>>()
                .ok()?
        }
        "webp" => {
            let d = WebPDecoder::new(cursor()).ok()?;
            if !d.has_animation() {
                return None;
            }
            d.into_frames().take(5000).collect::<Result<_, _>>().ok()?
        }
        _ => return None,
    };
    if frames.len() < 2 {
        return None;
    }
    // Each frame is held for as many scene frames as its delay lasts
    // (at least one), so the timing matches the original.
    let mut owed = 0.0f32;
    let frames = frames
        .into_iter()
        .map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            let ms = if d == 0 { 100.0 } else { n as f32 / d as f32 };
            // Browsers show 0-10 ms delays as 100 ms.
            let ms = if ms <= 10.0 { 100.0 } else { ms };
            owed += ms / 1000.0 * fps;
            let hold = owed.round().max(1.0);
            owed -= hold;
            (f.into_buffer(), hold as u32)
        })
        .collect();
    Some(Many {
        parts: vec![Part {
            name: stem.to_owned(),
            frames,
        }],
        layered: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(img: &RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn openraster_layers_come_in_bottom_first_and_placed() {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("mimetype", o).unwrap();
            z.write_all(b"image/openraster").unwrap();
            z.start_file("stack.xml", o).unwrap();
            z.write_all(
                br#"<?xml version="1.0"?><image w="40" h="20"><stack>
                <layer name="Ink &amp; line" src="data/top.png" x="10" y="5" opacity="0.5"/>
                <layer name="Hidden" src="data/top.png" visibility="hidden"/>
                <layer name="Paper" src="data/bottom.png"/>
                </stack></image>"#,
            )
            .unwrap();
            z.start_file("data/top.png", o).unwrap();
            z.write_all(&png(&RgbaImage::from_pixel(
                4,
                4,
                image::Rgba([0, 0, 0, 255]),
            )))
            .unwrap();
            z.start_file("data/bottom.png", o).unwrap();
            z.write_all(&png(&RgbaImage::from_pixel(
                40,
                20,
                image::Rgba([255, 255, 255, 255]),
            )))
            .unwrap();
            z.finish().unwrap();
        }
        let many = read("sheet", &buf, "ora", 24.0).unwrap().unwrap();
        assert!(many.layered);
        let names: Vec<_> = many.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["sheet - Paper", "sheet - Ink & line"]);
        let ink = &many.parts[1].frames[0].0;
        assert_eq!((ink.width(), ink.height()), (40, 20));
        assert_eq!(
            ink.get_pixel(11, 6).0,
            [0, 0, 0, 128],
            "placed, at half opacity"
        );
        assert_eq!(ink.get_pixel(0, 0).0[3], 0);
    }

    #[test]
    fn animated_gifs_become_timed_sequences_and_stills_stay_pictures() {
        use image::codecs::gif::GifEncoder;
        use image::{Delay, Frame};
        let mut buf = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut buf);
            for (c, ms) in [(0u8, 125u32), (120, 250), (240, 125)] {
                let img = RgbaImage::from_pixel(8, 8, image::Rgba([c, 0, 0, 255]));
                enc.encode_frame(Frame::from_parts(
                    img,
                    0,
                    0,
                    Delay::from_numer_denom_ms(ms, 1),
                ))
                .unwrap();
            }
        }
        let many = read("walk", &buf, "gif", 24.0).unwrap().unwrap();
        let holds: Vec<u32> = many.parts[0].frames.iter().map(|f| f.1).collect();
        assert_eq!(holds, [3, 6, 3], "125 ms is 3 frames at 24 fps");
        let still = png(&RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])));
        assert!(read("still", &still, "png", 24.0).unwrap().is_none());
    }

    #[test]
    fn photoshop_layers_come_in_as_layers() {
        let bytes = include_bytes!("../tests/fixtures/two-layers-red-green-1x1.psd");
        let many = read("rg", bytes, "psd", 24.0).unwrap().unwrap();
        assert!(many.layered);
        let summary: Vec<(String, [u8; 4])> = many
            .parts
            .iter()
            .map(|p| (p.name.clone(), p.frames[0].0.get_pixel(0, 0).0))
            .collect();
        // Bottom first: green under red, as in the file.
        assert_eq!(
            summary,
            [
                ("rg - Green".to_owned(), [0, 255, 0, 255]),
                ("rg - Red".to_owned(), [255, 0, 0, 255])
            ]
        );
    }

    #[test]
    fn damaged_photoshop_files_are_refused_not_crashed() {
        let mut junk = b"8BPS".to_vec();
        junk.extend(std::iter::repeat_n(0xffu8, 64));
        assert!(read("bad", &junk, "psd", 24.0).is_err());
    }
}
