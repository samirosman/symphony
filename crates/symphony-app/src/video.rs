//! Video import: an MP4/MOV with H.264 video becomes an image-sequence
//! layer (a drawing per frame, sampled at the scene's frame rate) and, if
//! it has sound, a sound clip.
//!
//! Decoding uses Cisco's OpenH264 binary, the same one movie export uses,
//! which the user turns on in Preferences > Video (Symphony never ships it,
//! docs/CODECS.md). Frames are written as JPEGs sized to the camera into
//! the media cache, on a background thread.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use crate::import::{content_key, ImageGroup, Staged, StagedSound};

/// The longest video imported, in seconds (frames add up quickly).
const MAX_SECONDS: f64 = 300.0;

pub struct VideoOutcome {
    pub frames: Option<ImageGroup>,
    pub sound: Option<StagedSound>,
    pub problems: Vec<String>,
}

pub struct VideoImport {
    rx: Receiver<VideoOutcome>,
    pub name: String,
}

impl VideoImport {
    pub fn start(
        path: PathBuf,
        codec: PathBuf,
        fps: f32,
        camera: [u32; 2],
        ctx: &eframe::egui::Context,
    ) -> Self {
        let (tx, rx) = channel();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let out = match crate::import::cache_dir() {
                Some(cache) => import_into(&path, &codec, fps, camera, &cache),
                None => VideoOutcome {
                    frames: None,
                    sound: None,
                    problems: vec![
                        "There's nowhere to keep imported files on this computer.".into()
                    ],
                },
            };
            let _ = tx.send(out);
            ctx.request_repaint();
        });
        VideoImport { rx, name }
    }

    pub fn poll(&self) -> Option<VideoOutcome> {
        self.rx.try_recv().ok()
    }
}

/// Decode `path` into frames (JPEGs in `cache`) at `fps`, plus its sound.
pub fn import_into(
    path: &Path,
    codec: &Path,
    fps: f32,
    camera: [u32; 2],
    cache: &Path,
) -> VideoOutcome {
    let shown = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = VideoOutcome {
        frames: None,
        sound: None,
        problems: Vec::new(),
    };
    match decode_frames(path, codec, fps, camera, cache, &stem) {
        Ok(frames) if !frames.is_empty() => {
            out.frames = Some(ImageGroup {
                name: stem.clone(),
                frames,
            });
        }
        Ok(_) => out.problems.push(format!("{shown}: no video frames in it")),
        Err(e) => out.problems.push(format!("{shown}: {e}")),
    }
    // Its sound, if it has any (silently skipped if not).
    let sound = crate::import::stage_sounds_into(&[path.to_path_buf()], cache);
    out.sound = sound.sounds.into_iter().next();
    out
}

/// Length-prefixed NAL units (MP4) to an Annex B stream for the decoder.
fn annex_b(sample: &[u8], length_size: usize, out: &mut Vec<u8>) {
    let mut i = 0;
    while i + length_size <= sample.len() {
        let mut n = 0usize;
        for k in 0..length_size {
            n = (n << 8) | sample[i + k] as usize;
        }
        i += length_size;
        if i + n > sample.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&sample[i..i + n]);
        i += n;
    }
}

fn decode_frames(
    path: &Path,
    codec: &Path,
    fps: f32,
    camera: [u32; 2],
    cache: &Path,
    stem: &str,
) -> Result<Vec<Staged>, String> {
    use openh264::decoder::{Decoder, DecoderConfig};
    use openh264::formats::YUVSource;
    let file = std::fs::File::open(path).map_err(|e| format!("couldn't open it ({e})"))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), size)
        .map_err(|_| "not a video Symphony can read (MP4 or MOV with H.264)".to_owned())?;
    let (track_id, length_size, params, video_fps, samples) = {
        let track = mp4
            .tracks()
            .values()
            .find(|t| matches!(t.media_type(), Ok(mp4::MediaType::H264)))
            .ok_or("its video isn't H.264, which is the kind Symphony can read")?;
        let avc1 = track
            .trak
            .mdia
            .minf
            .stbl
            .stsd
            .avc1
            .as_ref()
            .ok_or("its video settings are missing")?;
        let mut params = Vec::new();
        for nal in avc1
            .avcc
            .sequence_parameter_sets
            .iter()
            .chain(&avc1.avcc.picture_parameter_sets)
        {
            params.extend_from_slice(&[0, 0, 0, 1]);
            params.extend_from_slice(&nal.bytes);
        }
        let rate = track.frame_rate();
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            24.0
        };
        (
            track.track_id(),
            usize::from(avc1.avcc.length_size_minus_one & 3) + 1,
            params,
            rate,
            track.sample_count(),
        )
    };
    let api = openh264::OpenH264API::from_blob_path(codec)
        .map_err(|_| "the OpenH264 codec couldn't be loaded".to_owned())?;
    let mut decoder = Decoder::with_api_config(api, DecoderConfig::new())
        .map_err(|_| "the video decoder couldn't start".to_owned())?;
    let max_frames = (MAX_SECONDS * video_fps) as u32;
    // Decoded pictures in order, kept only when a scene frame needs them.
    let scene_fps = f64::from(fps.max(1.0));
    let mut next_time = 0.0f64;
    let mut decoded_index = 0u32;
    let mut frames: Vec<Staged> = Vec::new();
    let mut rgb: Vec<u8> = Vec::new();
    let mut stream = params;
    for id in 1..=samples.min(max_frames) {
        let Ok(Some(sample)) = mp4.read_sample(track_id, id) else {
            continue;
        };
        annex_b(&sample.bytes, length_size, &mut stream);
        let picture = match decoder.decode(&stream) {
            Ok(p) => p,
            Err(_) => {
                stream.clear();
                continue;
            }
        };
        stream.clear();
        let Some(yuv) = picture else { continue };
        let t = f64::from(decoded_index) / video_fps;
        decoded_index += 1;
        // Scene frames whose time this picture covers.
        let mut uses = 0;
        while next_time <= t + 0.5 / video_fps {
            uses += 1;
            next_time = frames.len() as f64 / scene_fps + f64::from(uses) / scene_fps;
        }
        if uses == 0 {
            continue;
        }
        let (w, h) = yuv.dimensions();
        rgb.resize(w * h * 3, 0);
        yuv.write_rgb8(&mut rgb);
        let Some(img) = image::RgbImage::from_raw(w as u32, h as u32, rgb.clone()) else {
            continue;
        };
        // Sized to the camera: no bigger than needed.
        let k = (f64::from(camera[0]) / w as f64)
            .min(f64::from(camera[1]) / h as f64)
            .min(1.0);
        let img = if k < 0.999 {
            image::imageops::resize(
                &img,
                ((w as f64 * k) as u32).max(1),
                ((h as f64 * k) as u32).max(1),
                image::imageops::FilterType::Triangle,
            )
        } else {
            img
        };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 88)
            .encode_image(&img)
            .map_err(|e| format!("couldn't save a frame ({e})"))?;
        let key = content_key(&jpeg);
        let file = format!("{key:016x}.jpg");
        let dest = cache.join(&file);
        if !dest.is_file() {
            std::fs::create_dir_all(cache).map_err(|e| format!("couldn't save frames ({e})"))?;
            std::fs::write(&dest, &jpeg).map_err(|e| format!("couldn't save a frame ({e})"))?;
        }
        let staged = Staged {
            key,
            file,
            name: format!("{stem}_{:04}", frames.len() + 1),
            size: [img.width(), img.height()],
        };
        for _ in 0..uses {
            frames.push(staged.clone());
        }
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefixed_units_become_annex_b() {
        let sample = [0, 0, 0, 2, 0xAA, 0xBB, 0, 0, 0, 1, 0xCC];
        let mut out = Vec::new();
        annex_b(&sample, 4, &mut out);
        assert_eq!(out, [0, 0, 0, 1, 0xAA, 0xBB, 0, 0, 0, 1, 0xCC]);
        // A truncated unit is dropped, never read past the end.
        let mut out = Vec::new();
        annex_b(&[0, 0, 0, 9, 1], 4, &mut out);
        assert!(out.is_empty());
    }
}
