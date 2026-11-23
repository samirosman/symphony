//! Sound tracks: imported soundtracks and dialogue that play with the
//! picture (PLAN §4.6, §4.11). A sound track is not a layer. It has no
//! column, node, peg or drawing, and the Node view doesn't show it, as in
//! Harmony. Tracks are listed top to bottom; an import goes to the bottom.
//!
//! Core holds the original file bytes and what the file says about itself
//! (sample rate, channels, length), never decoded samples: decoding needs
//! an audio library, which core doesn't depend on (E1.2). `symphony-audio`
//! checks and decodes files; `symphony-io` copies them into the project's
//! `media/` folder (E7.1).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::doc::Scene;
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
}

impl SoundFormat {
    /// File extension used in `media/`.
    pub fn extension(self) -> &'static str {
        match self {
            SoundFormat::Wav => "wav",
            SoundFormat::Aiff => "aiff",
            SoundFormat::Mp3 => "mp3",
        }
    }

    /// The format from the first bytes of a file, not from its name.
    pub fn sniff(bytes: &[u8]) -> Option<SoundFormat> {
        let riff = |kind: &[u8]| bytes.len() >= 12 && &bytes[8..12] == kind;
        if (bytes.starts_with(b"RIFF") || bytes.starts_with(b"RF64")) && riff(b"WAVE") {
            Some(SoundFormat::Wav)
        } else if bytes.starts_with(b"FORM") && (riff(b"AIFF") || riff(b"AIFC")) {
            Some(SoundFormat::Aiff)
        } else if bytes.starts_with(b"ID3")
            || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
        {
            // An ID3 tag, or an MPEG audio frame sync.
            Some(SoundFormat::Mp3)
        } else {
            None
        }
    }
}

/// One sound file used by the scene, stored by content key in
/// `Scene::sound_files`. The bytes are the original file, unchanged.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SoundFile {
    pub format: SoundFormat,
    /// Samples a second, per channel.
    pub rate: u32,
    pub channels: u16,
    /// Length in samples per channel.
    pub samples: u64,
    /// Loaded from `media/` on open; not part of `scene.json`.
    #[serde(skip)]
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

/// A track in the Timeline that plays one sound file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundTrack {
    pub id: Id,
    /// A label (E2.2); the file name without its extension on import.
    pub name: String,
    /// Key in `Scene::sound_files`.
    pub key: u64,
    /// Off: the track is muted.
    #[serde(default = "on")]
    pub enabled: bool,
    /// The scene frame (1-based, E2.4) where the sound starts.
    #[serde(default = "first_frame")]
    pub start: u32,
    /// Samples (per channel, at the file's rate) left out at the start of
    /// the file. The file itself is never changed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub trim_start: u64,
    /// Samples left out at the end of the file.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub trim_end: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl SoundTrack {
    /// Samples of `file` the track plays, after its trims.
    pub fn played(&self, file: &SoundFile) -> u64 {
        file.samples
            .saturating_sub(self.trim_start)
            .saturating_sub(self.trim_end)
    }

    /// Length the track plays, in seconds.
    pub fn seconds(&self, file: &SoundFile) -> f64 {
        self.played(file) as f64 / f64::from(file.rate.max(1))
    }

    /// Where in the file the track starts playing, in seconds.
    pub fn offset(&self, file: &SoundFile) -> f64 {
        self.trim_start as f64 / f64::from(file.rate.max(1))
    }

    /// The track moved to start at `start` (never before frame 1).
    pub fn moved_to(&self, start: u32) -> SoundTrack {
        SoundTrack {
            start: start.max(1),
            ..self.clone()
        }
    }

    /// The track with its start trimmed by `frames` scene frames: positive
    /// cuts sound away, negative brings it back. The sound that stays
    /// keeps its place in the scene, so the start frame moves with the
    /// trim. At least one frame of sound stays, and nothing starts before
    /// frame 1.
    pub fn trimmed_start(&self, file: &SoundFile, fps: f32, frames: i64) -> SoundTrack {
        let per = samples_per_frame(file, fps);
        // Frames the trim can take back (file start, scene start) and give.
        let back = (self.trim_start as f64 / per).floor() as i64;
        let back = back.min(i64::from(self.start) - 1);
        let ahead = (self.played(file) as f64 / per).ceil() as i64 - 1;
        let frames = frames.clamp(-back, ahead.max(0));
        let trim = (self.trim_start as f64 + frames as f64 * per).round();
        SoundTrack {
            trim_start: (trim.max(0.0) as u64).min(file.samples.saturating_sub(self.trim_end)),
            start: (i64::from(self.start) + frames).max(1) as u32,
            ..self.clone()
        }
    }

    /// The track with its end trimmed by `frames` scene frames: negative
    /// cuts sound away, positive brings it back, up to the end of the
    /// file. At least one frame of sound stays.
    pub fn trimmed_end(&self, file: &SoundFile, fps: f32, frames: i64) -> SoundTrack {
        let per = samples_per_frame(file, fps);
        let back = (self.trim_end as f64 / per).ceil() as i64;
        let cut = (self.played(file) as f64 / per).ceil() as i64 - 1;
        let frames = frames.clamp(-cut.max(0), back);
        let trim = (self.trim_end as f64 - frames as f64 * per).round();
        SoundTrack {
            trim_end: (trim.max(0.0) as u64).min(file.samples.saturating_sub(self.trim_start)),
            ..self.clone()
        }
    }
}

impl SoundTrack {
    /// The track with exactly `samples` left out at the start of the file,
    /// as the Sound editor trims: the start frame stays where it is, so
    /// the sound after the cut begins there. At least one frame of sound
    /// stays.
    pub fn with_trim_start(&self, file: &SoundFile, fps: f32, samples: u64) -> SoundTrack {
        let keep = samples_per_frame(file, fps).ceil() as u64;
        let most = file
            .samples
            .saturating_sub(self.trim_end)
            .saturating_sub(keep);
        SoundTrack {
            trim_start: samples.min(most),
            ..self.clone()
        }
    }

    /// The track with exactly `samples` left out at the end of the file.
    /// At least one frame of sound stays.
    pub fn with_trim_end(&self, file: &SoundFile, fps: f32, samples: u64) -> SoundTrack {
        let keep = samples_per_frame(file, fps).ceil() as u64;
        let most = file
            .samples
            .saturating_sub(self.trim_start)
            .saturating_sub(keep);
        SoundTrack {
            trim_end: samples.min(most),
            ..self.clone()
        }
    }
}

/// Samples of `file` in one scene frame at `fps`.
fn samples_per_frame(file: &SoundFile, fps: f32) -> f64 {
    f64::from(file.rate.max(1)) / f64::from(fps.max(1.0))
}

fn on() -> bool {
    true
}

fn first_frame() -> u32 {
    1
}

/// Seconds into a sound that starts at frame `start` where scene frame
/// `frame` begins. Negative before the sound starts.
pub fn seconds_at(start: u32, frame: u32, fps: f32) -> f64 {
    (f64::from(frame) - f64::from(start)) / f64::from(fps.max(1.0))
}

/// The last scene frame that `track` reaches. A sound longer than the
/// scene doesn't make the scene longer; callers clip to the scene length
/// themselves.
pub fn last_frame(track: &SoundTrack, file: &SoundFile, fps: f32) -> u32 {
    let frames = (track.seconds(file) * f64::from(fps.max(1.0))).ceil() as u32;
    track.start.saturating_add(frames.max(1) - 1)
}

/// The edit that adds a track at the bottom of the list, playing the file
/// under `key`. The file must already be in `Scene::sound_files`.
pub fn add_track(scene: &mut Scene, name: &str, key: u64) -> Edit {
    Edit::AddSound {
        index: scene.sounds.len(),
        track: SoundTrack {
            id: scene.ids.fresh(),
            name: name.into(),
            key,
            enabled: true,
            start: 1,
            trim_start: 0,
            trim_end: 0,
        },
    }
}

/// The edit that removes track `id`.
pub fn remove_track(scene: &Scene, id: Id) -> Option<Edit> {
    let index = scene.sounds.iter().position(|t| t.id == id)?;
    Some(Edit::RemoveSound {
        index,
        track: scene.sounds[index].clone(),
    })
}

/// The edit that changes track `id` with `change` (rename, mute). `None`
/// when the track doesn't exist or nothing changes.
pub fn change_track(scene: &Scene, id: Id, change: impl FnOnce(&mut SoundTrack)) -> Option<Edit> {
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

    fn file(rate: u32, samples: u64) -> SoundFile {
        SoundFile {
            format: SoundFormat::Wav,
            rate,
            channels: 2,
            samples,
            bytes: Arc::new(Vec::new()),
        }
    }

    fn track(start: u32) -> SoundTrack {
        SoundTrack {
            id: Id(1),
            name: "A".into(),
            key: 1,
            enabled: true,
            start,
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
        let late = SoundTrack {
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
        assert!(file(48_000, 48_000).is_valid());
        assert!(!file(0, 1).is_valid());
        assert!(!file(MAX_RATE + 1, 1).is_valid());
        assert!(!file(8_000, MAX_SECONDS * 8_000 + 1).is_valid());
        let mut f = file(48_000, 1);
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

        let mute = change_track(&s, id, |t| t.enabled = false).unwrap();
        h.apply(&mut s, "Mute Sound", vec![mute]);
        assert!(!s.sounds[0].enabled);
        assert!(change_track(&s, id, |t| t.enabled = false).is_none());

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
        assert!(s.sounds[0].enabled);
        h.undo(&mut s);
        h.undo(&mut s);
        assert!(s.sounds.is_empty());
        h.redo(&mut s);
        assert_eq!(s.sounds[0].id, id);
    }

    #[test]
    fn old_scene_files_have_no_sound() {
        let s = Scene::new(Settings::default());
        let mut v = serde_json::to_value(&s).unwrap();
        v.as_object_mut().unwrap().remove("sounds");
        v.as_object_mut().unwrap().remove("sound_files");
        let back: Scene = serde_json::from_value(v).unwrap();
        assert!(back.sounds.is_empty() && back.sound_files.is_empty());
    }

    #[test]
    fn a_track_without_enabled_or_start_reads_as_on_from_frame_1() {
        let t: SoundTrack = serde_json::from_str(r#"{"id":3,"name":"A","key":9}"#).unwrap();
        assert!(t.enabled);
        assert_eq!(t.start, 1);
        assert_eq!((t.trim_start, t.trim_end), (0, 0));
    }
}
