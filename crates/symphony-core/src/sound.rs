//! Sound clips: imported soundtracks and dialogue that play with the
//! picture (PLAN §4.6, §4.11). A clip is not a layer. It has no column,
//! node, peg or drawing, and the Node view doesn't show it, as in Harmony.
//! Clips are listed top to bottom in `Scene::sounds`; an import goes to
//! the bottom.
//!
//! Core holds what a file says about itself (sample rate, channels,
//! length) in `Scene::media`, never decoded samples: decoding needs an
//! audio library, which core doesn't depend on (E1.2). `symphony-audio`
//! checks and decodes files. The files stay in the media folders.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::doc::{Media, MediaKind, Scene, SoundClip};
use crate::history::Edit;
use crate::id::Id;

/// Largest sound file accepted on import and open (E7.6).
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Longest sound accepted: 10 minutes. Decoded to 32-bit float at 48 kHz
/// in stereo, that is 230 MB.
pub const MAX_SECONDS: u64 = 10 * 60;
/// Highest sample rate accepted.
pub const MAX_RATE: u32 = 192_000;
/// Most channels accepted; playback mixes them down to the device's.
pub const MAX_CHANNELS: u16 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundFormat {
    Wav,
    Aiff,
    Mp3,
    Flac,
    /// Ogg Vorbis.
    Ogg,
    /// An MP4 container (M4A, MP4, MOV): AAC or ALAC sound.
    Mp4,
    /// Raw AAC in ADTS frames.
    Aac,
}

impl SoundFormat {
    /// File extension used in the media folders.
    pub fn extension(self) -> &'static str {
        match self {
            SoundFormat::Wav => "wav",
            SoundFormat::Aiff => "aiff",
            SoundFormat::Mp3 => "mp3",
            SoundFormat::Flac => "flac",
            SoundFormat::Ogg => "ogg",
            SoundFormat::Mp4 => "m4a",
            SoundFormat::Aac => "aac",
        }
    }

    /// The format from the first bytes of a file, not from its name.
    pub fn sniff(bytes: &[u8]) -> Option<SoundFormat> {
        let riff = |kind: &[u8]| bytes.len() >= 12 && &bytes[8..12] == kind;
        let sync = bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0;
        if (bytes.starts_with(b"RIFF") || bytes.starts_with(b"RF64")) && riff(b"WAVE") {
            Some(SoundFormat::Wav)
        } else if bytes.starts_with(b"FORM") && (riff(b"AIFF") || riff(b"AIFC")) {
            Some(SoundFormat::Aiff)
        } else if bytes.starts_with(b"fLaC") {
            Some(SoundFormat::Flac)
        } else if bytes.starts_with(b"OggS") {
            Some(SoundFormat::Ogg)
        } else if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
            Some(SoundFormat::Mp4)
        } else if sync && bytes[1] & 0x06 == 0 {
            // An ADTS frame: the MPEG sync with layer bits 00, which MP3
            // never uses (ISO/IEC 13818-7).
            Some(SoundFormat::Aac)
        } else if bytes.starts_with(b"ID3") || sync {
            // An ID3 tag, or an MPEG audio frame sync.
            Some(SoundFormat::Mp3)
        } else {
            None
        }
    }
}

/// A sound file read for import: what its header says, and the original
/// bytes, unchanged. The scene keeps only [`SoundFile::media`].
#[derive(Clone, Debug)]
pub struct SoundFile {
    pub format: SoundFormat,
    /// Samples a second, per channel.
    pub rate: u32,
    pub channels: u16,
    /// Length in samples per channel.
    pub samples: u64,
    pub bytes: Arc<Vec<u8>>,
}

impl SoundFile {
    /// The name of the file in the project's `media/` folder.
    pub fn file_name(&self, key: u64) -> String {
        format!("{key:016x}.{}", self.format.extension())
    }

    /// Length in seconds.
    pub fn seconds(&self) -> f64 {
        self.samples as f64 / f64::from(self.rate.max(1))
    }

    /// The scene's record of this file, stored as `<key>.<ext>` in the
    /// media folders.
    pub fn media(&self, key: u64, name: &str) -> Media {
        Media {
            kind: MediaKind::Sound,
            file: self.file_name(key),
            name: name.into(),
            size: [0, 0],
            seconds: self.seconds() as f32,
            rate: self.rate,
            channels: self.channels,
            samples: self.samples,
        }
    }

    /// The header values are within the limits (hostile files, E7.6).
    pub fn is_valid(&self) -> bool {
        (1..=MAX_RATE).contains(&self.rate)
            && (1..=MAX_CHANNELS).contains(&self.channels)
            && self.samples <= MAX_SECONDS * u64::from(self.rate)
    }
}

/// Key of a file by its content (64-bit FNV-1a), so the same file imported
/// twice is stored once.
pub fn content_key(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

impl Media {
    /// A sound's length in seconds, to the sample.
    pub fn sound_seconds(&self) -> f64 {
        self.samples as f64 / f64::from(self.rate.max(1))
    }
}

impl SoundClip {
    /// Samples of `file` the track plays, after its trims.
    pub fn played(&self, file: &Media) -> u64 {
        file.samples
            .saturating_sub(self.trim_start)
            .saturating_sub(self.trim_end)
    }

    /// Length the track plays, in seconds.
    pub fn seconds(&self, file: &Media) -> f64 {
        self.played(file) as f64 / f64::from(file.rate.max(1))
    }

    /// Where in the file the track starts playing, in seconds.
    pub fn offset(&self, file: &Media) -> f64 {
        self.trim_start as f64 / f64::from(file.rate.max(1))
    }

    /// The track moved to start at `start` (never before frame 1).
    pub fn moved_to(&self, start: u32) -> SoundClip {
        SoundClip {
            start: start.max(1),
            ..self.clone()
        }
    }

    /// The track with its start trimmed by `frames` scene frames: positive
    /// cuts sound away, negative brings it back. The sound that stays
    /// keeps its place in the scene, so the start frame moves with the
    /// trim. At least one frame of sound stays, and nothing starts before
    /// frame 1.
    pub fn trimmed_start(&self, file: &Media, fps: f32, frames: i64) -> SoundClip {
        let per = samples_per_frame(file, fps);
        // Frames the trim can take back (file start, scene start) and give.
        let back = (self.trim_start as f64 / per).floor() as i64;
        let back = back.min(i64::from(self.start) - 1);
        let ahead = (self.played(file) as f64 / per).ceil() as i64 - 1;
        let frames = frames.clamp(-back, ahead.max(0));
        let trim = (self.trim_start as f64 + frames as f64 * per).round();
        SoundClip {
            trim_start: (trim.max(0.0) as u64).min(file.samples.saturating_sub(self.trim_end)),
            start: (i64::from(self.start) + frames).max(1) as u32,
            ..self.clone()
        }
    }

    /// The track with its end trimmed by `frames` scene frames: negative
    /// cuts sound away, positive brings it back, up to the end of the
    /// file. At least one frame of sound stays.
    pub fn trimmed_end(&self, file: &Media, fps: f32, frames: i64) -> SoundClip {
        let per = samples_per_frame(file, fps);
        let back = (self.trim_end as f64 / per).ceil() as i64;
        let cut = (self.played(file) as f64 / per).ceil() as i64 - 1;
        let frames = frames.clamp(-cut.max(0), back);
        let trim = (self.trim_end as f64 - frames as f64 * per).round();
        SoundClip {
            trim_end: (trim.max(0.0) as u64).min(file.samples.saturating_sub(self.trim_start)),
            ..self.clone()
        }
    }
}

impl SoundClip {
    /// The track with exactly `samples` left out at the start of the file,
    /// as the Sound editor trims: the start frame stays where it is, so
    /// the sound after the cut begins there. At least one frame of sound
    /// stays.
    pub fn with_trim_start(&self, file: &Media, fps: f32, samples: u64) -> SoundClip {
        let keep = samples_per_frame(file, fps).ceil() as u64;
        let most = file
            .samples
            .saturating_sub(self.trim_end)
            .saturating_sub(keep);
        SoundClip {
            trim_start: samples.min(most),
            ..self.clone()
        }
    }

    /// The track with exactly `samples` left out at the end of the file.
    /// At least one frame of sound stays.
    pub fn with_trim_end(&self, file: &Media, fps: f32, samples: u64) -> SoundClip {
        let keep = samples_per_frame(file, fps).ceil() as u64;
        let most = file
            .samples
            .saturating_sub(self.trim_start)
            .saturating_sub(keep);
        SoundClip {
            trim_end: samples.min(most),
            ..self.clone()
        }
    }
}

/// Samples of `file` in one scene frame at `fps`.
fn samples_per_frame(file: &Media, fps: f32) -> f64 {
    f64::from(file.rate.max(1)) / f64::from(fps.max(1.0))
}

/// Seconds into a sound that starts at frame `start` where scene frame
/// `frame` begins. Negative before the sound starts.
pub fn seconds_at(start: u32, frame: u32, fps: f32) -> f64 {
    (f64::from(frame) - f64::from(start)) / f64::from(fps.max(1.0))
}

/// The last scene frame that `track` reaches. A sound longer than the
/// scene doesn't make the scene longer; callers clip to the scene length
/// themselves.
pub fn last_frame(track: &SoundClip, file: &Media, fps: f32) -> u32 {
    let frames = (track.seconds(file) * f64::from(fps.max(1.0))).ceil() as u32;
    track.start.saturating_add(frames.max(1) - 1)
}

/// The edit that adds a clip at the bottom of the list, playing the file
/// under `key`. The file must already be in `Scene::media`.
pub fn add_track(scene: &mut Scene, name: &str, key: u64) -> Edit {
    Edit::AddSound {
        index: scene.sounds.len(),
        track: SoundClip {
            id: scene.ids.fresh(),
            media: key,
            name: name.into(),
            start: 1,
            muted: false,
            trim_start: 0,
            trim_end: 0,
        },
    }
}

/// The edit that removes clip `id`.
pub fn remove_track(scene: &Scene, id: Id) -> Option<Edit> {
    let index = scene.sounds.iter().position(|t| t.id == id)?;
    Some(Edit::RemoveSound {
        index,
        track: scene.sounds[index].clone(),
    })
}

/// The edit that changes clip `id` with `change` (rename, mute). `None`
/// when the track doesn't exist or nothing changes.
pub fn change_track(scene: &Scene, id: Id, change: impl FnOnce(&mut SoundClip)) -> Option<Edit> {
    let before = scene.sounds.iter().find(|t| t.id == id)?.clone();
    let mut after = before.clone();
    change(&mut after);
    (after != before).then_some(Edit::SetSound { before, after })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Settings;
    use crate::history::History;

    fn probed(rate: u32, samples: u64) -> SoundFile {
        SoundFile {
            format: SoundFormat::Wav,
            rate,
            channels: 2,
            samples,
            bytes: Arc::new(Vec::new()),
        }
    }

    fn file(rate: u32, samples: u64) -> Media {
        probed(rate, samples).media(1, "A")
    }

    fn track(start: u32) -> SoundClip {
        SoundClip {
            id: Id(1),
            media: 1,
            name: "A".into(),
            start,
            muted: false,
            trim_start: 0,
            trim_end: 0,
        }
    }

    #[test]
    fn trims_snap_to_frames_and_keep_the_sound_in_place() {
        // 2 s at 24 fps, 2000 samples a frame.
        let f = file(48_000, 96_000);
        let t = track(10);
        let a = t.trimmed_start(&f, 24.0, 3);
        assert_eq!((a.start, a.trim_start), (13, 6_000));
        assert_eq!(last_frame(&a, &f, 24.0), last_frame(&t, &f, 24.0));
        // Back, but not past the file's start.
        let b = a.trimmed_start(&f, 24.0, -10);
        assert_eq!((b.start, b.trim_start), (10, 0));
        // Not before frame 1 either.
        let late = SoundClip {
            trim_start: 20_000,
            ..track(2)
        };
        let c = late.trimmed_start(&f, 24.0, -6);
        assert_eq!((c.start, c.trim_start), (1, 18_000));
        // At least one frame stays.
        let d = t.trimmed_start(&f, 24.0, 1_000);
        assert_eq!(d.played(&f), 2_000);

        let e = t.trimmed_end(&f, 24.0, -4);
        assert_eq!((e.start, e.trim_end), (10, 8_000));
        assert_eq!(last_frame(&e, &f, 24.0), last_frame(&t, &f, 24.0) - 4);
        // Back to the end of the file, no further.
        assert_eq!(e.trimmed_end(&f, 24.0, 99).trim_end, 0);
        assert_eq!(t.trimmed_end(&f, 24.0, -1_000).played(&f), 2_000);
        assert_eq!(t.moved_to(0).start, 1);
    }

    #[test]
    fn exact_trims_keep_the_start_frame() {
        // 2 s at 24 fps, 2000 samples a frame.
        let f = file(48_000, 96_000);
        let t = track(10);
        let a = t.with_trim_start(&f, 24.0, 2_800);
        assert_eq!((a.start, a.trim_start), (10, 2_800));
        assert_eq!(a.with_trim_start(&f, 24.0, 0).trim_start, 0);
        // At least one frame stays, at both ends.
        assert_eq!(t.with_trim_start(&f, 24.0, 99_999).played(&f), 2_000);
        let e = t.with_trim_end(&f, 24.0, 1_234);
        assert_eq!((e.start, e.trim_end), (10, 1_234));
        assert_eq!(t.with_trim_end(&f, 24.0, 99_999).played(&f), 2_000);
    }

    #[test]
    fn frames_and_seconds() {
        assert_eq!(seconds_at(1, 1, 24.0), 0.0);
        assert_eq!(seconds_at(1, 25, 24.0), 1.0);
        assert!(seconds_at(10, 1, 24.0) < 0.0);
        let at = |start| track(start);
        // One second at 24 fps covers frames 1 to 24.
        assert_eq!(last_frame(&at(1), &file(48_000, 48_000), 24.0), 24);
        // A little more reaches into frame 25.
        assert_eq!(last_frame(&at(1), &file(48_000, 48_001), 24.0), 25);
        assert_eq!(last_frame(&at(5), &file(48_000, 0), 24.0), 5);
    }

    #[test]
    fn limits() {
        assert!(probed(48_000, 48_000).is_valid());
        assert!(!probed(0, 1).is_valid());
        assert!(!probed(MAX_RATE + 1, 1).is_valid());
        assert!(!probed(8_000, MAX_SECONDS * 8_000 + 1).is_valid());
        let mut f = probed(48_000, 1);
        f.channels = 0;
        assert!(!f.is_valid());
    }

    #[test]
    fn sniffs_the_format_from_the_content() {
        assert_eq!(
            SoundFormat::sniff(b"RIFF\0\0\0\0WAVEfmt "),
            Some(SoundFormat::Wav)
        );
        assert_eq!(
            SoundFormat::sniff(b"FORM\0\0\0\0AIFFCOMM"),
            Some(SoundFormat::Aiff)
        );
        assert_eq!(SoundFormat::sniff(b"ID3\x04"), Some(SoundFormat::Mp3));
        assert_eq!(
            SoundFormat::sniff(&[0xFF, 0xFB, 0x90]),
            Some(SoundFormat::Mp3)
        );
        assert_eq!(SoundFormat::sniff(b"fLaC\0"), Some(SoundFormat::Flac));
        assert_eq!(SoundFormat::sniff(b"OggS\0"), Some(SoundFormat::Ogg));
        assert_eq!(
            SoundFormat::sniff(b"\0\0\0\x20ftypM4A "),
            Some(SoundFormat::Mp4)
        );
        assert_eq!(
            SoundFormat::sniff(&[0xFF, 0xF1, 0x50]),
            Some(SoundFormat::Aac)
        );
        assert_eq!(SoundFormat::sniff(b"RIFF\0\0\0\0AVI "), None);
        assert_eq!(SoundFormat::sniff(b"\x89PNG"), None);
    }

    #[test]
    fn add_mute_rename_remove_undo() {
        let mut s = Scene::new(Settings::default());
        let mut h = History::default();
        let add = add_track(&mut s, "Dialogue", 7);
        h.apply(&mut s, "Import Sound", vec![add]);
        let id = s.sounds[0].id;
        let add2 = add_track(&mut s, "Music", 8);
        h.apply(&mut s, "Import Sound", vec![add2]);
        assert_eq!(s.sounds[1].name, "Music");

        let mute = change_track(&s, id, |t| t.muted = true).unwrap();
        h.apply(&mut s, "Mute Sound", vec![mute]);
        assert!(s.sounds[0].muted);
        assert!(change_track(&s, id, |t| t.muted = true).is_none());

        let rename = change_track(&s, id, |t| t.name = "Lines".into()).unwrap();
        h.apply(&mut s, "Rename Sound", vec![rename]);
        let remove = remove_track(&s, id).unwrap();
        h.apply(&mut s, "Delete Sound", vec![remove]);
        assert_eq!(s.sounds.len(), 1);

        // Undo puts the track back in its place, then undoes each change.
        h.undo(&mut s);
        assert_eq!(s.sounds[0].name, "Lines");
        h.undo(&mut s);
        assert_eq!(s.sounds[0].name, "Dialogue");
        h.undo(&mut s);
        assert!(!s.sounds[0].muted);
        h.undo(&mut s);
        h.undo(&mut s);
        assert!(s.sounds.is_empty());
        h.redo(&mut s);
        assert_eq!(s.sounds[0].id, id);
    }
}
