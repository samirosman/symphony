//! Imported media files (ENGINEERING E7.1 `media/`): sound for now.
//!
//! A scene names each file by content key; the file itself is copied,
//! unchanged, to `media/<key>.<ext>` next to the scenes. Files are written
//! once and never rewritten, because the name is the content. Every file
//! read here is untrusted (E7.6): its size, type and content key are
//! checked, and a bad one is left out with its name reported.
//!
//! Checking the audio inside a file needs a decoder, which lives in
//! `symphony-audio`; it checks files on import and decodes them for
//! playback. Here a file only has to be the one the scene names.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use symphony_core::doc::Scene;
use symphony_core::sound::{self, SoundFormat};

use crate::{read_project_file, write_atomic, IoError};

/// Folder name inside a project, and inside a recovery entry.
pub const MEDIA: &str = "media";

/// Keys of the sound files some track plays. Others are left over from
/// undone imports and are not saved.
pub fn used_sound_keys(scene: &Scene) -> BTreeSet<u64> {
    scene.sounds.iter().map(|t| t.key).collect()
}

/// Write each used file that isn't in `dir` yet, and drop unused ones from
/// the saved scene.
pub fn write_media(scene: &mut Scene, dir: &Path) -> Result<(), IoError> {
    let used = used_sound_keys(scene);
    scene.sound_files.retain(|k, _| used.contains(k));
    for (key, file) in &scene.sound_files {
        let path = dir.join(file.file_name(*key));
        if path.is_file() {
            continue;
        }
        if file.bytes.is_empty() {
            return Err(IoError(format!(
                "Couldn't save the sound {}: its data is missing.",
                file.file_name(*key)
            )));
        }
        write_atomic(&path, &file.bytes)?;
    }
    Ok(())
}

/// Load every sound file the scene lists from `dir` inside `root`. A
/// missing or damaged file loses only that sound: its track stays and
/// plays nothing until the file is imported again. Returns the names of
/// the files that were left out.
pub fn read_media(scene: &mut Scene, root: &Path, dir: &Path) -> Vec<String> {
    let mut dropped = Vec::new();
    let keys: Vec<u64> = scene.sound_files.keys().copied().collect();
    for key in keys {
        let file = &scene.sound_files[&key];
        let name = file.file_name(key);
        let path = dir.join(&name);
        // Check the size before reading, so a huge file is never loaded.
        let small = fs::metadata(&path).is_ok_and(|m| m.len() <= sound::MAX_FILE_BYTES);
        let bytes = small
            .then(|| read_project_file(root, &path).ok())
            .flatten()
            .filter(|b| {
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
