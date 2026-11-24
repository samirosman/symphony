//! File > Export > Soundtrack: the scene's sound tracks as WAV files, for
//! an editing program. One merged file, or one file per track, over a
//! frame range, at a chosen rate, sample size and channel count.
//!
//! Only tracks that play are exported: a muted track is left out, in both
//! modes. Each track lands where playback puts it (the same `Clip` timing
//! as `app/sound`), so the file lines up with a PNG sequence exported over
//! the same frames.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use symphony_core::doc::Scene;

use crate::decode::decode_path;
use crate::play::{prepare, Clip};

/// Rates offered in the export dialog, in Hz.
pub const RATES: [u32; 3] = [22_050, 44_100, 48_000];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleSize {
    /// 8-bit unsigned PCM.
    Eight,
    /// 16-bit signed PCM.
    Sixteen,
}

#[derive(Clone, Debug)]
pub struct SoundtrackJob {
    /// The merged file, or the name the per-track files start with.
    pub path: PathBuf,
    /// Scene frames, 1-based and inclusive (E2.4).
    pub first: u32,
    pub last: u32,
    pub rate: u32,
    pub size: SampleSize,
    /// 1 (mono) or 2 (stereo).
    pub channels: u16,
    /// One merged file (true), or one file per track (false).
    pub merge: bool,
}

/// Run an export. `progress` gets (tracks done, tracks in all). Returns
/// the files written. Errors are plain sentences (DESIGN D10.3).
pub fn export(
    scene: &Scene,
    job: &SoundtrackJob,
    cancel: &AtomicBool,
    progress: &dyn Fn(u32, u32),
) -> Result<Vec<PathBuf>, String> {
    let tracks: Vec<_> = scene
        .sounds
        .iter()
        .filter(|t| !t.muted && scene.media.contains_key(&t.media))
        .collect();
    if tracks.is_empty() {
        return Err("The scene has no sound to export. Muted tracks are left out.".into());
    }
    let (rate, channels) = (job.rate.max(1), job.channels.clamp(1, 2));
    let ch = usize::from(channels);
    let fps = f64::from(scene.settings.fps.max(1.0));
    let (first, last) = (job.first.max(1), job.last.max(job.first.max(1)));
    let at = |frame: u32| (f64::from(frame - 1) / fps * f64::from(rate)).round() as u64;
    let from = at(first);
    let frames = (at(last + 1) - from) as usize;

    let total = tracks.len() as u32;
    let mut merged = vec![0.0f32; if job.merge { frames * ch } else { 0 }];
    let mut written = Vec::new();
    let mut names = Vec::new();
    for (i, t) in tracks.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err("Export cancelled.".into());
        }
        let file = &scene.media[&t.media];
        let fail = |why: String| format!("Couldn't read the sound \"{}\": {why}.", t.name);
        let path = scene
            .media_path(t.media)
            .ok_or_else(|| fail("the file is missing".into()))?;
        let samples = decode_path(&path).map_err(fail)?;
        let pcm = prepare(&samples, rate, channels).map_err(fail)?;
        let start = f64::from(t.start.saturating_sub(1)) / fps;
        let clip = Clip::new(pcm, start, true, rate).trimmed(t.offset(file), t.seconds(file), rate);
        if job.merge {
            add(&mut merged, &clip, from, ch);
        } else {
            let mut own = vec![0.0f32; frames * ch];
            add(&mut own, &clip, from, ch);
            let path = track_path(&job.path, &t.name, &mut names);
            write(&path, &own, channels, rate, job.size)?;
            written.push(path);
        }
        progress(i as u32 + 1, total);
    }
    if job.merge {
        write(&job.path, &merged, channels, rate, job.size)?;
        written.push(job.path.clone());
    }
    Ok(written)
}

/// Add the part of `clip` that falls in `out` (which starts at device
/// frame `from`), clipping to full scale as the mixer does.
fn add(out: &mut [f32], clip: &Clip, from: u64, ch: usize) {
    let frames = (out.len() / ch) as u64;
    let available = (clip.samples.len() / ch) as u64;
    let length = clip.frames.min(available.saturating_sub(clip.offset));
    let begin = clip.start.max(from);
    let end = clip.start.saturating_add(length).min(from + frames);
    for p in begin..end {
        let src = (clip.offset + p - clip.start) as usize * ch;
        let dst = (p - from) as usize * ch;
        for c in 0..ch {
            let o = &mut out[dst + c];
            *o = (*o + clip.samples[src + c]).clamp(-1.0, 1.0);
        }
    }
}

/// `<stem>_<track>.wav` next to `path`. Characters a file name can't hold
/// become `_`; a name already used gets a number.
fn track_path(path: &Path, track: &str, used: &mut Vec<String>) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Soundtrack");
    let clean: String = track
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let base = format!("{stem}_{}", if clean.is_empty() { "Sound" } else { &clean });
    let mut name = base.clone();
    let mut n = 2;
    while used.iter().any(|u| u.eq_ignore_ascii_case(&name)) {
        name = format!("{base} {n}");
        n += 1;
    }
    used.push(name.clone());
    path.with_file_name(format!("{name}.wav"))
}

fn write(
    path: &Path,
    samples: &[f32],
    channels: u16,
    rate: u32,
    size: SampleSize,
) -> Result<(), String> {
    let bytes = wav(samples, channels, rate, size)
        .ok_or("The soundtrack is too long for a WAV file (4 GB at most).")?;
    std::fs::write(path, bytes).map_err(|e| format!("Couldn't write {} ({e}).", path.display()))
}

/// A PCM WAV file. `None` when the data passes the format's 4 GB limit.
pub fn wav(samples: &[f32], channels: u16, rate: u32, size: SampleSize) -> Option<Vec<u8>> {
    let width: u16 = match size {
        SampleSize::Eight => 1,
        SampleSize::Sixteen => 2,
    };
    let data = u32::try_from(samples.len() * usize::from(width)).ok()?;
    let riff = data.checked_add(36 + (data & 1))?;
    let mut b = Vec::with_capacity(44 + data as usize + 1);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&riff.to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * u32::from(channels * width)).to_le_bytes());
    b.extend_from_slice(&(channels * width).to_le_bytes());
    b.extend_from_slice(&(width * 8).to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for &s in samples {
        let s = s.clamp(-1.0, 1.0);
        match size {
            // 8-bit WAV is unsigned, with silence at 128.
            SampleSize::Eight => b.push((s * 127.0).round() as i16 as u8 ^ 0x80),
            SampleSize::Sixteen => {
                b.extend_from_slice(&((s * 32_767.0).round() as i16).to_le_bytes())
            }
        }
    }
    if data & 1 == 1 {
        b.push(0); // RIFF chunks are padded to an even length.
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::import_bytes;
    use symphony_core::doc::Settings;
    use symphony_core::history::History;
    use symphony_core::sound::add_track;

    fn add(scene: &mut Scene, key: u64) {
        let edit = add_track(scene, "Dialogue", key);
        History::default().apply(scene, "Import Sound", vec![edit]);
    }

    /// A scene at `fps` with one mono 8 kHz track of `samples`.
    fn scene(fps: f32, samples: &[i16]) -> Scene {
        let mut scene = Scene::new(Settings {
            fps,
            ..Settings::default()
        });
        let bytes = crate::decode::wav(samples, 1, 8_000);
        let (key, file, _) = import_bytes(bytes.clone()).unwrap();
        let media = file.media(key, "Dialogue");
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = temp(&format!("media-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(dir.join(&media.file), &bytes).unwrap();
        scene.media_dirs = vec![dir];
        scene.media.insert(key, media);
        add(&mut scene, key);
        scene
    }

    fn job(path: PathBuf, first: u32, last: u32) -> SoundtrackJob {
        SoundtrackJob {
            path,
            first,
            last,
            rate: 8_000,
            size: SampleSize::Sixteen,
            channels: 1,
            merge: true,
        }
    }

    fn run(scene: &Scene, job: &SoundtrackJob) -> Result<Vec<PathBuf>, String> {
        export(scene, job, &AtomicBool::new(false), &|_, _| {})
    }

    fn read(path: &Path) -> Vec<f32> {
        let (_, _, s) = import_bytes(std::fs::read(path).unwrap()).unwrap();
        s.data
    }

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("symphony-soundtrack-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wav_round_trips_and_pads_odd_data() {
        let s = [0.0, 0.5, -0.5, 1.0, -1.0];
        let b = wav(&s, 1, 8_000, SampleSize::Sixteen).unwrap();
        assert_eq!(b.len(), 44 + 10);
        let (_, file, d) = import_bytes(b).unwrap();
        assert_eq!((file.rate, file.channels, file.samples), (8_000, 1, 5));
        for (a, b) in s.iter().zip(&d.data) {
            assert!((a - b).abs() < 1e-3, "{a} {b}");
        }
        let b = wav(&s, 1, 8_000, SampleSize::Eight).unwrap();
        assert_eq!(b.len(), 44 + 5 + 1, "odd data gets a pad byte");
        assert_eq!(&b[44..49], &[128, 192, 64, 255, 1]);
        let (_, file, _) = import_bytes(b).unwrap();
        assert_eq!(file.samples, 5);
    }

    #[test]
    fn the_range_and_the_track_start_line_up_with_frames() {
        // 8 fps at 8 kHz: 1 000 samples a frame. The sound is frame-long
        // steps: 0.1 in its first frame, 0.2 in the second.
        let mut pcm = vec![3_277i16; 1_000];
        pcm.extend(vec![6_554i16; 1_000]);
        let mut sc = scene(8.0, &pcm);
        sc.sounds[0].start = 3;
        let dir = temp("range");
        let p = dir.join("Scene.wav");
        run(&sc, &job(p.clone(), 2, 5)).unwrap();
        let d = read(&p);
        assert_eq!(d.len(), 4_000, "frames 2 to 5");
        assert!(
            d[..1_000].iter().all(|&x| x == 0.0),
            "frame 2 is before the sound"
        );
        assert!((d[1_000] - 0.1).abs() < 1e-3 && (d[1_999] - 0.1).abs() < 1e-3);
        assert!((d[2_000] - 0.2).abs() < 1e-3);
        assert!(d[3_000..].iter().all(|&x| x == 0.0), "the sound has ended");
    }

    #[test]
    fn trims_and_mutes_are_respected() {
        let pcm: Vec<i16> = (0..2_000)
            .map(|i| if i < 1_000 { 3_277 } else { 6_554 })
            .collect();
        let mut sc = scene(8.0, &pcm);
        sc.sounds[0].trim_start = 1_000;
        let dir = temp("trim");
        let p = dir.join("Scene.wav");
        run(&sc, &job(p.clone(), 1, 2)).unwrap();
        let d = read(&p);
        assert!((d[0] - 0.2).abs() < 1e-3, "the trimmed start is left out");
        assert!(d[1_000..].iter().all(|&x| x == 0.0));

        sc.sounds[0].muted = true;
        let err = run(&sc, &job(p, 1, 2)).unwrap_err();
        assert!(err.contains("no sound"), "{err}");
    }

    #[test]
    fn merging_adds_and_clips_while_individual_files_stay_apart() {
        let mut sc = scene(8.0, &vec![24_000i16; 1_000]);
        let key = sc.sounds[0].media;
        add(&mut sc, key);
        let dir = temp("merge");
        let p = dir.join("Scene.wav");
        run(&sc, &job(p.clone(), 1, 1)).unwrap();
        assert!(
            read(&p).iter().all(|&x| x > 0.99),
            "two loud tracks clip at full scale"
        );

        let mut j = job(p, 1, 1);
        j.merge = false;
        let files = run(&sc, &j).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["Scene_Dialogue.wav", "Scene_Dialogue 2.wav"]);
        for f in &files {
            assert!(read(f)
                .iter()
                .all(|&x| (x - 24_000.0 / 32_768.0).abs() < 1e-3));
        }
    }

    #[test]
    fn stereo_output_and_resampling() {
        let sc = scene(8.0, &vec![16_384i16; 8_000]);
        let dir = temp("stereo");
        let p = dir.join("Scene.wav");
        let mut j = job(p.clone(), 1, 8);
        (j.rate, j.channels) = (48_000, 2);
        run(&sc, &j).unwrap();
        let (_, file, d) = import_bytes(std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(
            (file.rate, file.channels, file.samples),
            (48_000, 2, 48_000)
        );
        let mid = d.data[48_000];
        assert!((mid - 0.5).abs() < 1e-2, "{mid}");
    }

    #[test]
    fn file_names_are_made_safe() {
        let mut used = Vec::new();
        let p = Path::new("/out/Scene.wav");
        assert_eq!(
            track_path(p, "a/b: c?", &mut used),
            Path::new("/out/Scene_a_b_ c_.wav")
        );
        assert_eq!(
            track_path(p, "  ", &mut used),
            Path::new("/out/Scene_Sound.wav")
        );
    }
}
