//! Imported media files (ENGINEERING E7.1 `media/`): images and sound.
//!
//! A scene names each file by content key; the file itself is copied,
//! unchanged, to `media/<key>.<ext>` next to the scenes. Files are written
//! once and never rewritten, because the name is the content. Every file
//! read here is untrusted (E7.6): its size, type and content key are
//! checked, and a bad one is left out with its name reported.
//!
//! Images are checked here, from their header. Checking the audio inside
//! a sound file needs a decoder, which lives in `symphony-audio`; it
//! checks files on import and decodes them for playback. Here a sound
//! file only has to be the one the scene names.

use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use symphony_core::bitmap::{self, ImageFile, ImageFormat};
use symphony_core::doc::Scene;
use symphony_core::sound::{self, SoundFormat};

use crate::{err, read_project_file, write_atomic, IoError};

/// Folder name inside a project, and inside a recovery entry.
pub const MEDIA: &str = "media";

/// Write each used image and sound file that isn't in `dir` yet, and drop
/// unused ones from the saved scene.
pub fn write_media(scene: &mut Scene, dir: &Path) -> Result<(), IoError> {
    let used = used_image_keys(scene);
    scene.images.retain(|k, _| used.contains(k));
    for (key, img) in &scene.images {
        write_file(dir, &img.file_name(*key), &img.bytes, "image")?;
    }
    let used = used_sound_keys(scene);
    scene.sound_files.retain(|k, _| used.contains(k));
    for (key, file) in &scene.sound_files {
        write_file(dir, &file.file_name(*key), &file.bytes, "sound")?;
    }
    Ok(())
}

/// Load every image and sound file the scene lists from `dir` inside
/// `root`. A missing or damaged file loses only that file: its drawings
/// show nothing, or its track plays nothing, until it's imported again.
/// Returns the names of the files that were left out.
pub fn read_media(scene: &mut Scene, root: &Path, dir: &Path) -> Vec<String> {
    let mut dropped = read_images(scene, root, dir);
    dropped.extend(read_sounds(scene, root, dir));
    dropped
}

fn write_file(dir: &Path, name: &str, bytes: &[u8], what: &str) -> Result<(), IoError> {
    let path = dir.join(name);
    if path.is_file() {
        return Ok(());
    }
    if bytes.is_empty() {
        return Err(IoError(format!(
            "Couldn't save the {what} {name}: its data is missing."
        )));
    }
    write_atomic(&path, bytes)
}

/// Read a file inside `root`, or nothing if it's missing, unreadable or
/// larger than `max` bytes. The size is checked before reading, so a huge
/// file is never loaded.
fn read_small(root: &Path, path: &Path, max: u64) -> Option<Vec<u8>> {
    let small = fs::metadata(path).is_ok_and(|m| m.len() <= max);
    small.then(|| read_project_file(root, path).ok()).flatten()
}

// Images.

fn sniff_image(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(&[0x76, 0x2F, 0x31, 0x01]) {
        Some(ImageFormat::Exr)
    } else {
        None
    }
}

/// Refuse an OpenEXR file the decoder can't read, from its header alone,
/// so it fails on import instead of drawing nothing later. The `image`
/// crate (0.25) shows the first part that is flat (not deep) and has R, G
/// and B channels; the `exr` crate (1.74.3) under it decodes every
/// compression but HTJ2K.
fn exr_readable(bytes: &[u8]) -> Result<(), String> {
    use exr::compression::Compression;
    use exr::meta::attribute::Text;
    let meta = exr::meta::MetaData::read_from_buffered(Cursor::new(bytes), false)
        .map_err(|e| format!("it couldn't be read ({e})"))?;
    let shown = meta.headers.iter().find(|h| {
        !h.deep
            && ["R", "G", "B"]
                .iter()
                .all(|c| h.channels.find_index_of_channel(&Text::from(*c)).is_some())
    });
    let Some(part) = shown else {
        return Err("it has no flat R, G and B channels to show".into());
    };
    match part.compression {
        Compression::HTJ2K32 | Compression::HTJ2K256 => Err(format!(
            "it uses {}, which Symphony can't read yet; \
             save it again with ZIP, PIZ or DWAA compression",
            part.compression
        )),
        _ => Ok(()),
    }
}

/// Check image bytes and read their size from the header. The format comes
/// from the content, not the file name.
pub fn image_from_bytes(bytes: Vec<u8>) -> Result<(u64, ImageFile), String> {
    if bytes.len() as u64 > bitmap::MAX_FILE_BYTES {
        return Err("the file is larger than 256 MB".into());
    }
    let format = sniff_image(&bytes).ok_or("it isn't a PNG, JPEG or OpenEXR image")?;
    if format == ImageFormat::Exr {
        exr_readable(&bytes)?;
    }
    let reader = image::ImageReader::with_format(
        Cursor::new(&bytes[..]),
        match format {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            ImageFormat::Exr => image::ImageFormat::OpenExr,
        },
    );
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| format!("it couldn't be read ({e})"))?;
    if !bitmap::size_ok(width, height) {
        return Err(format!(
            "it is {width} x {height} pixels; the limit is {} pixels a side and 64 megapixels",
            bitmap::MAX_SIDE
        ));
    }
    let key = bitmap::content_key(&bytes);
    Ok((
        key,
        ImageFile {
            format,
            width,
            height,
            bytes: Arc::new(bytes),
        },
    ))
}

/// Read an image the user chose to import, for `Scene::images`.
pub fn import_image(path: &Path) -> Result<(u64, ImageFile), IoError> {
    let size = fs::metadata(path).map_err(|e| err("open", path, e))?.len();
    if size > bitmap::MAX_FILE_BYTES {
        return Err(IoError(format!(
            "Couldn't import {}: the file is larger than 256 MB.",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|e| err("open", path, e))?;
    image_from_bytes(bytes)
        .map_err(|why| IoError(format!("Couldn't import {}: {why}.", path.display())))
}

/// Keys of the images some drawing shows. Others are left over from undone
/// imports and are not saved.
pub fn used_image_keys(scene: &Scene) -> BTreeSet<u64> {
    scene
        .drawings
        .values()
        .filter_map(|d| d.image.map(|i| i.key))
        .collect()
}

fn read_images(scene: &mut Scene, root: &Path, dir: &Path) -> Vec<String> {
    let mut dropped = Vec::new();
    let keys: Vec<u64> = scene.images.keys().copied().collect();
    for key in keys {
        let img = &scene.images[&key];
        let name = img.file_name(key);
        let checked = read_small(root, &dir.join(&name), bitmap::MAX_FILE_BYTES)
            .and_then(|b| image_from_bytes(b).ok());
        match checked {
            Some((k, file))
                if k == key
                    && file.format == img.format
                    && (file.width, file.height) == (img.width, img.height) =>
            {
                scene.images.insert(key, file);
            }
            _ => {
                scene.images.remove(&key);
                dropped.push(name);
            }
        }
    }
    dropped
}

// Sound.

/// Keys of the sound files some track plays. Others are left over from
/// undone imports and are not saved.
pub fn used_sound_keys(scene: &Scene) -> BTreeSet<u64> {
    scene.sounds.iter().map(|t| t.key).collect()
}

fn read_sounds(scene: &mut Scene, root: &Path, dir: &Path) -> Vec<String> {
    let mut dropped = Vec::new();
    let keys: Vec<u64> = scene.sound_files.keys().copied().collect();
    for key in keys {
        let file = &scene.sound_files[&key];
        let name = file.file_name(key);
        let bytes = read_small(root, &dir.join(&name), sound::MAX_FILE_BYTES).filter(|b| {
            file.is_valid()
                && SoundFormat::sniff(b) == Some(file.format)
                && sound::content_key(b) == key
        });
        match bytes {
            Some(b) => {
                if let Some(f) = scene.sound_files.get_mut(&key) {
                    f.bytes = b.into();
                }
            }
            None => {
                scene.sound_files.remove(&key);
                dropped.push(name);
            }
        }
    }
    dropped
}

#[cfg(test)]
pub(crate) fn png(width: u32, height: u32, seed: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([seed, x as u8, y as u8, 255])
    });
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A short 16-bit PCM WAV file, for tests.
#[cfg(test)]
pub(crate) fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// A half-float RGBA OpenEXR file with the default (lossless) encoding.
#[cfg(test)]
pub(crate) fn exr_half(width: usize, height: usize) -> Vec<u8> {
    use exr::prelude::*;
    let px = SpecificChannels::rgba(|Vec2(x, _y)| {
        (f16::from_f32(x as f32), f16::ZERO, f16::ONE, f16::ONE)
    });
    let mut out = Vec::new();
    Image::from_channels((width, height), px)
        .write()
        .to_buffered(Cursor::new(&mut out))
        .unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_half_float_exr_size() {
        let (key, f) = image_from_bytes(exr_half(5, 3)).unwrap();
        assert_eq!((f.width, f.height, f.format), (5, 3, ImageFormat::Exr));
        assert_eq!(f.file_name(key), format!("{key:016x}.exr"));
    }

    #[test]
    fn refuses_exr_compression_the_decoder_lacks() {
        // The exr crate can't write HTJ2K, so set the compression byte of
        // a lossless file to HTJ2K256 (10 in the OpenEXR header).
        let mut bytes = exr_half(2, 2);
        let name = b"compression\0compression\0";
        let at = bytes.windows(name.len()).position(|w| w == name).unwrap();
        bytes[at + name.len() + 4] = 10;
        let why = image_from_bytes(bytes).unwrap_err();
        assert!(why.contains("j2k"), "{why}");
    }

    #[test]
    fn reads_png_size_and_refuses_other_files() {
        let (key, f) = image_from_bytes(png(3, 2, 0)).unwrap();
        assert_eq!((f.width, f.height, f.format), (3, 2, ImageFormat::Png));
        assert_eq!(key, bitmap::content_key(&f.bytes));
        assert!(image_from_bytes(b"GIF89a....".to_vec()).is_err());
        assert!(image_from_bytes(Vec::new()).is_err());
        // A PNG signature with a broken header.
        assert!(image_from_bytes(b"\x89PNG\r\n\x1a\nbroken".to_vec()).is_err());
    }

    #[test]
    fn refuses_images_over_the_size_limit() {
        // A header that claims 20000 x 1 pixels (IHDR width at byte 16).
        // The edit also breaks the chunk checksum, so either check may
        // refuse it; the point is that it never reaches a decoder.
        let mut big = png(1, 1, 0);
        big[16..20].copy_from_slice(&20_000u32.to_be_bytes());
        assert!(image_from_bytes(big).is_err());
    }
}
