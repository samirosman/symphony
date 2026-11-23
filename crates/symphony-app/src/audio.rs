//! Sound: decoding imported files (symphonia, pure Rust) and playing the
//! scene's sound clips in step with the animation (cpal).
//!
//! Files are decoded once, on a background thread, into stereo samples
//! plus a rough waveform for the Timeline. The audio output only opens
//! when a scene has sound. While playing, the mixer follows the playhead
//! (re-syncing on loops and jumps); when you step or scrub, it plays one
//! frame's slice, which is what lip sync needs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Waveform bins per second.
pub const PEAKS_PER_SECOND: f32 = 100.0;

/// A decoded sound: interleaved stereo samples and a waveform.
pub struct Decoded {
    pub rate: u32,
    pub samples: Vec<f32>,
    /// Loudest sample per 1/100 s, 0..=1.
    pub peaks: Vec<f32>,
}

impl Decoded {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / 2.0 / self.rate.max(1) as f32
    }
}

/// Decode a whole sound file (or a video's sound track).
pub fn decode(path: &Path) -> Result<Decoded, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| format!("couldn't open it ({e})"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| "not a sound format Symphony can read".to_owned())?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("it has no sound")?;
    let id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(48_000);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| "its sound is in a format Symphony can't decode".to_owned())?;
    let mut samples: Vec<f32> = Vec::new();
    // At most an hour, so a huge file can't exhaust memory.
    let limit = rate as usize * 2 * 3600;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };
        if packet.track_id() != id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                let channels = spec.channels.count().max(1);
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(buf);
                for frame in sb.samples().chunks(channels) {
                    let l = frame[0];
                    let r = if channels > 1 { frame[1] } else { l };
                    samples.push(l);
                    samples.push(r);
                }
                if samples.len() > limit {
                    break;
                }
            }
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }
    if samples.is_empty() {
        return Err("it has no sound".into());
    }
    let per_bin = (rate as f32 / PEAKS_PER_SECOND).max(1.0) as usize * 2;
    let peaks = samples
        .chunks(per_bin)
        .map(|c| c.iter().fold(0.0f32, |m, s| m.max(s.abs())).min(1.0))
        .collect();
    Ok(Decoded {
        rate,
        samples,
        peaks,
    })
}

enum Slot {
    Loading,
    Ready(Arc<Decoded>),
    Failed,
}

static DECODED: Mutex<Option<HashMap<PathBuf, Slot>>> = Mutex::new(None);

/// A decoded sound, or None while it decodes in the background (asked for
/// once; `ctx` repaints when it's ready).
pub fn sound(path: &Path, ctx: &eframe::egui::Context) -> Option<Arc<Decoded>> {
    let mut guard = DECODED.lock().ok()?;
    let map = guard.get_or_insert_with(HashMap::new);
    match map.get(path) {
        Some(Slot::Ready(d)) => return Some(d.clone()),
        Some(_) => return None,
        None => {}
    }
    map.insert(path.to_path_buf(), Slot::Loading);
    let (path, ctx) = (path.to_path_buf(), ctx.clone());
    std::thread::spawn(move || {
        let slot = match decode(&path) {
            Ok(d) => Slot::Ready(Arc::new(d)),
            Err(_) => Slot::Failed,
        };
        if let Ok(mut g) = DECODED.lock() {
            g.get_or_insert_with(HashMap::new).insert(path, slot);
        }
        ctx.request_repaint();
    });
    None
}

/// Hand a sound decoded elsewhere (at import) to the cache.
pub fn remember(path: &Path, decoded: Arc<Decoded>) {
    if let Ok(mut g) = DECODED.lock() {
        g.get_or_insert_with(HashMap::new)
            .insert(path.to_path_buf(), Slot::Ready(decoded));
    }
}

struct Clip {
    data: Arc<Decoded>,
    start: f64,
}

#[derive(Default)]
struct Mix {
    clips: Vec<Clip>,
    /// Seconds into the scene.
    pos: f64,
    playing: bool,
    /// Scrubbing: stop here.
    stop_at: Option<f64>,
    out_rate: f64,
}

impl Mix {
    fn fill(&mut self, out: &mut [f32], channels: usize) {
        let step = 1.0 / self.out_rate.max(1.0);
        for frame in out.chunks_mut(channels.max(1)) {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            if self.playing {
                for c in &self.clips {
                    let t = self.pos - c.start;
                    if t >= 0.0 {
                        let i = (t * f64::from(c.data.rate)) as usize * 2;
                        if i + 1 < c.data.samples.len() {
                            l += c.data.samples[i];
                            r += c.data.samples[i + 1];
                        }
                    }
                }
                self.pos += step;
                if self.stop_at.is_some_and(|end| self.pos >= end) {
                    self.playing = false;
                    self.stop_at = None;
                }
            }
            for (k, s) in frame.iter_mut().enumerate() {
                *s = (if k % 2 == 0 { l } else { r }).clamp(-1.0, 1.0);
            }
        }
    }
}

/// The scene's sound, kept in step with the playhead.
#[derive(Default)]
pub struct Player {
    stream: Option<cpal::Stream>,
    mix: Arc<Mutex<Mix>>,
    /// What the mixer holds (media key, start frame) and the last state.
    loaded: Vec<(u64, u32)>,
    was_playing: bool,
    last_frame: u32,
    failed: bool,
}

impl Player {
    fn open(&mut self) {
        if self.stream.is_some() || self.failed {
            return;
        }
        let built = (|| {
            let device = cpal::default_host().default_output_device()?;
            let supported = device.default_output_config().ok()?;
            let format = supported.sample_format();
            let config: cpal::StreamConfig = supported.into();
            let channels = config.channels as usize;
            if let Ok(mut m) = self.mix.lock() {
                m.out_rate = f64::from(config.sample_rate.0);
            }
            let mix = self.mix.clone();
            let err = |e| eprintln!("Sound output: {e}");
            let stream = match format {
                cpal::SampleFormat::F32 => device.build_output_stream(
                    &config,
                    move |data: &mut [f32], _| {
                        if let Ok(mut m) = mix.lock() {
                            m.fill(data, channels);
                        }
                    },
                    err,
                    None,
                ),
                cpal::SampleFormat::I16 => {
                    let mut buf: Vec<f32> = Vec::new();
                    device.build_output_stream(
                        &config,
                        move |data: &mut [i16], _| {
                            buf.resize(data.len(), 0.0);
                            if let Ok(mut m) = mix.lock() {
                                m.fill(&mut buf, channels);
                            }
                            for (d, s) in data.iter_mut().zip(&buf) {
                                *d = (s * f32::from(i16::MAX)) as i16;
                            }
                        },
                        err,
                        None,
                    )
                }
                _ => return None,
            }
            .ok()?;
            stream.play().ok()?;
            Some(stream)
        })();
        match built {
            Some(s) => self.stream = Some(s),
            None => self.failed = true,
        }
    }

    /// Call every frame with the scene's state.
    pub fn sync(
        &mut self,
        ctx: &eframe::egui::Context,
        scene: &symphony_core::doc::Scene,
        frame: u32,
        playing: bool,
    ) {
        let fps = f64::from(scene.settings.fps.max(1.0));
        let wanted: Vec<(u64, u32)> = scene
            .sounds
            .iter()
            .filter(|c| !c.muted)
            .map(|c| (c.media, c.start))
            .collect();
        if wanted.is_empty() && self.stream.is_none() {
            return;
        }
        self.open();
        let mut clips = Vec::new();
        let mut all_ready = true;
        for (key, start) in &wanted {
            match scene.media_path(*key).and_then(|p| sound(&p, ctx)) {
                Some(data) => clips.push(Clip {
                    data,
                    start: f64::from(start.saturating_sub(1)) / fps,
                }),
                None => all_ready = false,
            }
        }
        let now = f64::from(frame.saturating_sub(1)) / fps;
        let Ok(mut m) = self.mix.lock() else { return };
        if self.loaded != wanted || !all_ready {
            m.clips = clips;
            if all_ready {
                self.loaded = wanted;
            }
        }
        if playing {
            if !self.was_playing || (m.pos - now).abs() > 0.12 {
                m.pos = now;
            }
            m.playing = true;
            m.stop_at = None;
        } else {
            if self.was_playing {
                m.playing = false;
            }
            // Stepping or scrubbing: play this frame's slice.
            if frame != self.last_frame {
                m.pos = now;
                m.stop_at = Some(now + 1.0 / fps + 0.02);
                m.playing = true;
            }
        }
        self.was_playing = playing;
        self.last_frame = frame;
    }
}

/// Tests: a 440 Hz tone at 8 kHz, `seconds` long, as a WAV file.
#[cfg(test)]
pub fn write_tone(path: &Path, seconds: f32) {
    {
        let rate = 8000u32;
        let n = (rate as f32 * seconds) as usize;
        let mut data = Vec::new();
        for i in 0..n {
            let s =
                ((i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 12000.0) as i16;
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(path, wav).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_decode_with_a_waveform() {
        let path = std::env::temp_dir().join(format!("symphony-tone-{}.wav", std::process::id()));
        write_tone(&path, 1.0);
        let d = decode(&path).unwrap();
        assert_eq!(d.rate, 8000);
        assert!((d.seconds() - 1.0).abs() < 0.01, "{}", d.seconds());
        assert!(
            d.peaks.iter().all(|p| (0.3..0.45).contains(p)),
            "steady tone"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_mixer_plays_from_the_playhead_and_stops_after_a_scrub() {
        let d = Arc::new(Decoded {
            rate: 10,
            samples: (0..40).map(|i| i as f32 / 100.0).collect(),
            peaks: Vec::new(),
        });
        let mut m = Mix {
            clips: vec![Clip {
                data: d,
                start: 0.0,
            }],
            pos: 1.0,
            playing: true,
            stop_at: Some(1.2),
            out_rate: 10.0,
        };
        let mut out = [0.0f32; 8];
        m.fill(&mut out, 2);
        // At 1 s: stereo sample 10 (values 0.20, 0.21), then 11.
        assert!((out[0] - 0.20).abs() < 1e-6 && (out[1] - 0.21).abs() < 1e-6);
        assert!((out[2] - 0.22).abs() < 1e-6);
        // The scrub slice ended: silence after.
        assert_eq!(&out[4..], &[0.0; 4]);
    }
}
