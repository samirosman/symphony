//! Decoding sound files to 32-bit float samples, and the check a file
//! passes on import. WAV, AIFF and MP3, through Symphonia.
//!
//! Every file is untrusted (E7.6). The size, rate, channel count and
//! length are checked against the limits in [`symphony_core::sound`]
//! while decoding, so a hostile file can't make the decoded samples grow
//! past them. Damaged packets are skipped, as players do.

use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphony_core::sound::{self, SoundFile, SoundFormat};

/// Decoded sound: interleaved samples, one per channel per frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Samples {
    pub rate: u32,
    pub channels: u16,
    pub data: Vec<f32>,
}

impl Samples {
    /// Length in samples per channel.
    pub fn len(&self) -> usize {
        self.data.len() / usize::from(self.channels.max(1))
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Lets Symphonia read shared file bytes without copying them.
struct Shared(Arc<Vec<u8>>);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Decode a whole file. Errors are plain words for the user (D10.3).
pub fn decode(bytes: &Arc<Vec<u8>>) -> Result<Samples, String> {
    // Symphonia is not built to be panic-free on hostile input, and a
    // loader must never take the app down (E7.6).
    catch_unwind(AssertUnwindSafe(|| decode_inner(bytes)))
        .unwrap_or_else(|_| Err("it is damaged".into()))
}

fn decode_inner(bytes: &Arc<Vec<u8>>) -> Result<Samples, String> {
    if bytes.len() as u64 > sound::MAX_FILE_BYTES {
        return Err("the file is larger than 256 MB".into());
    }
    let source = Cursor::new(Shared(Arc::clone(bytes)));
    let mss = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(format) = SoundFormat::sniff(bytes) {
        hint.with_extension(format.extension());
    }
    let unreadable = |e: Error| format!("it couldn't be read ({e})");
    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| match e {
            Error::Unsupported(_) => "it isn't a WAV, AIFF or MP3 file".to_string(),
            e => unreadable(e),
        })?;
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or("it has no sound in it")?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("it has no sound in it")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|_| "its sound is in a kind of encoding Symphony can't read".to_string())?;

    let mut out: Option<Samples> = None;
    let mut chunk = Vec::new();
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(p)) => p,
            // The end, or a file cut short: keep what was read.
            Ok(None) | Err(Error::IoError(_)) => break,
            Err(e) => return Err(unreadable(e)),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            // A damaged packet: skip it.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(unreadable(e)),
        };
        append(&mut out, &buf, &mut chunk)?;
    }
    let out = out.ok_or("it has no sound in it")?;
    if out.is_empty() {
        return Err("it has no sound in it".into());
    }
    Ok(out)
}

/// Add one decoded packet, checking it against the limits and against
/// the packets before it.
fn append(
    out: &mut Option<Samples>,
    buf: &GenericAudioBufferRef<'_>,
    chunk: &mut Vec<f32>,
) -> Result<(), String> {
    let rate = buf.spec().rate();
    let channels = u16::try_from(buf.spec().channels().count()).unwrap_or(u16::MAX);
    if !(1..=sound::MAX_RATE).contains(&rate) {
        return Err(format!("its sample rate ({rate} Hz) is above 192 kHz"));
    }
    if !(1..=sound::MAX_CHANNELS).contains(&channels) {
        return Err(format!("it has {channels} channels; the limit is 8"));
    }
    let s = out.get_or_insert_with(|| Samples {
        rate,
        channels,
        data: Vec::new(),
    });
    if (s.rate, s.channels) != (rate, channels) {
        return Err("its sample rate or channel count changes part way".into());
    }
    let limit = sound::MAX_SECONDS * u64::from(rate) * u64::from(channels);
    if (s.data.len() + buf.samples_interleaved()) as u64 > limit {
        return Err("it is longer than 10 minutes".into());
    }
    buf.copy_to_vec_interleaved(chunk);
    s.data.extend_from_slice(chunk);
    Ok(())
}

/// Check a file the user chose to import, and read what the scene keeps
/// about it. Returns its content key, the [`SoundFile`] for
/// `Scene::sound_files`, and the decoded samples, so the caller doesn't
/// decode it again. The format comes from the content, not the name.
pub fn import_bytes(bytes: Vec<u8>) -> Result<(u64, SoundFile, Samples), String> {
    if bytes.len() as u64 > sound::MAX_FILE_BYTES {
        return Err("the file is larger than 256 MB".into());
    }
    let format = SoundFormat::sniff(&bytes).ok_or("it isn't a WAV, AIFF or MP3 file")?;
    let bytes = Arc::new(bytes);
    let samples = decode(&bytes)?;
    let key = sound::content_key(&bytes);
    let file = SoundFile {
        format,
        rate: samples.rate,
        channels: samples.channels,
        samples: samples.len() as u64,
        bytes,
    };
    Ok((key, file, samples))
}

/// [`import_bytes`] for a file on disk, with the file named in the error.
pub fn import(path: &Path) -> Result<(u64, SoundFile, Samples), String> {
    let why = |w: String| format!("Couldn't import {}: {w}.", path.display());
    let size = std::fs::metadata(path)
        .map_err(|e| why(e.to_string()))?
        .len();
    if size > sound::MAX_FILE_BYTES {
        return Err(why("the file is larger than 256 MB".into()));
    }
    let bytes = std::fs::read(path).map_err(|e| why(e.to_string()))?;
    import_bytes(bytes).map_err(why)
}

/// A 16-bit PCM WAV file, for tests.
#[cfg(test)]
pub(crate) fn wav(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_wav_to_float() {
        let bytes = wav(&[0, 16_384, -16_384, 32_767], 2, 8_000);
        let (key, file, s) = import_bytes(bytes.clone()).unwrap();
        assert_eq!(key, sound::content_key(&bytes));
        assert_eq!(
            (file.format, file.rate, file.channels),
            (SoundFormat::Wav, 8_000, 2)
        );
        assert_eq!(file.samples, 2);
        assert_eq!(s.len(), 2);
        assert_eq!(s.data[0], 0.0);
        assert!((s.data[1] - 0.5).abs() < 1e-4);
        assert!((s.data[2] + 0.5).abs() < 1e-4);
    }

    #[test]
    fn refuses_what_is_not_sound() {
        assert!(import_bytes(b"\x89PNG\r\n\x1a\n....".to_vec()).is_err());
        assert!(import_bytes(Vec::new()).is_err());
        // A WAV header with no samples.
        assert!(import_bytes(wav(&[], 1, 8_000)).is_err());
    }

    #[test]
    fn damaged_files_give_an_error_not_a_panic() {
        let good = wav(&[1, 2, 3, 4, 5, 6, 7, 8], 1, 8_000);
        for cut in 0..good.len() {
            let _ = import_bytes(good[..cut].to_vec());
        }
        // Flip each byte of the header in turn.
        for i in 0..44 {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = import_bytes(bad);
        }
    }

    #[test]
    fn refuses_a_rate_above_the_limit() {
        let err = import_bytes(wav(&[1, 2], 1, 384_000)).unwrap_err();
        assert!(err.contains("192 kHz"), "{err}");
    }
}
