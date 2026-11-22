//! Saving and opening projects (ENGINEERING E7).
//!
//! A project is a folder, `Name.symphony/`:
//!
//! ```text
//! Name.symphony/
//!   project.json          manifest: format version, scenes
//!   scenes/main/scene.json
//!   media/                imported images and sound, named by content (media.rs)
//!   .backups/             the last 5 saves of each scene file
//! ```
//!
//! Drawings are stored inside scene.json for now. Binary drawing files
//! (`drawings/<id>.svd`) come later, as a format migration (E16).
//!
//! Every write is atomic (E7.3): write a temporary file, flush it to disk,
//! then rename it over the old one, so a crash mid-save never damages the
//! previous save.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use symphony_core::doc::Scene;

/// Bump when the on-disk format changes, and add a migration (E7.2).
///
/// - 1: the first format.
/// - 2: drawings can show an imported image, scenes can have sound
///   tracks, and the image and sound files live in `media/`. A version 1
///   file reads unchanged: it has neither. The bump keeps an older
///   Symphony from opening a version 2 file, dropping its images and
///   sound without a word, and saving over it.
pub const FORMAT_VERSION: u32 = 2;
pub const EXTENSION: &str = "symphony";
const KEEP_BACKUPS: usize = 5;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format_version: u32,
    app_version: String,
    scenes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SceneFile {
    format_version: u32,
    scene: Scene,
}

/// Errors in plain words, ready for the user (DESIGN D10.3).
#[derive(Debug)]
pub struct IoError(pub String);

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IoError {}

pub(crate) fn err(what: &str, path: &Path, e: impl std::fmt::Display) -> IoError {
    IoError(format!("Couldn't {what} {} ({e}).", path.display()))
}

/// Make sure a chosen path ends in `.symphony`.
pub fn with_extension(path: &Path) -> PathBuf {
    if path.extension().and_then(|e| e.to_str()) == Some(EXTENSION) {
        path.to_path_buf()
    } else {
        let mut p = path.as_os_str().to_owned();
        p.push(".");
        p.push(EXTENSION);
        PathBuf::from(p)
    }
}

/// The project's display name: the folder name without `.symphony`.
pub fn project_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".into())
}

/// Write `bytes` to `path` atomically (E7.3).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), IoError> {
    let dir = path
        .parent()
        .ok_or_else(|| err("write", path, "no folder"))?;
    fs::create_dir_all(dir).map_err(|e| err("create", dir, e))?;
    let tmp = path.with_extension("tmp");
    let mut f = File::create(&tmp).map_err(|e| err("write", &tmp, e))?;
    f.write_all(bytes).map_err(|e| err("write", &tmp, e))?;
    f.sync_all().map_err(|e| err("write", &tmp, e))?;
    drop(f);
    fs::rename(&tmp, path).map_err(|e| err("save", path, e))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Copy the current file into `.backups/` and keep only the newest few (E7.4).
fn backup(project: &Path, file: &Path) {
    if !file.exists() {
        return;
    }
    let dir = project.join(".backups");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let _ = fs::copy(file, dir.join(format!("{stem}-{stamp:024}.json")));
    let mut olds: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(&stem))
                })
                .collect()
        })
        .unwrap_or_default();
    olds.sort();
    while olds.len() > KEEP_BACKUPS {
        let _ = fs::remove_file(olds.remove(0));
    }
}

/// Save `scene` as the project at `path` (a `.symphony` folder).
pub fn save(scene: &Scene, path: &Path) -> Result<(), IoError> {
    let path = with_extension(path);
    let scene_path = path.join("scenes").join("main").join("scene.json");
    backup(&path, &scene_path);
    let mut scene = scene.clone();
    media::write_media(&mut scene, &path.join(media::MEDIA))?;
    let file = SceneFile {
        format_version: FORMAT_VERSION,
        scene,
    };
    let bytes = serde_json::to_vec(&file).map_err(|e| err("save", &scene_path, e))?;
    write_atomic(&scene_path, &bytes)?;
    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        scenes: vec!["main".into()],
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| err("save", &path, e))?;
    write_atomic(&path.join("project.json"), &bytes)
}

/// Open the project at `path`. Bad files give a readable error, never a
/// panic (E7.6).
pub fn open(path: &Path) -> Result<Scene, IoError> {
    open_with_report(path).map(|(scene, _)| scene)
}

/// Like [`open`], and also names the files in `media/` that were missing
/// or damaged and so were left out, for the user to import again.
pub fn open_with_report(path: &Path) -> Result<(Scene, Vec<String>), IoError> {
    let manifest_path = path.join("project.json");
    let bytes = read_project_file(path, &manifest_path)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
        IoError(format!(
            "{} isn't a Symphony project, or it's damaged ({e}).",
            path.display()
        ))
    })?;
    if manifest.format_version > FORMAT_VERSION {
        return Err(IoError(format!(
            "{} was saved by a newer Symphony ({}). Update Symphony to open it.",
            path.display(),
            manifest.app_version
        )));
    }
    let scene_name = manifest
        .scenes
        .first()
        .ok_or_else(|| IoError("The project has no scenes.".into()))?;
    if scene_name.contains(['/', '\\']) || scene_name.contains("..") {
        return Err(IoError(
            "The project file points outside its folder, so it wasn't opened.".into(),
        ));
    }
    let scene_path = path.join("scenes").join(scene_name).join("scene.json");
    let mut scene = read_scene(path, &scene_path)?;
    let dropped = media::read_media(&mut scene, path, &path.join(media::MEDIA));
    Ok((scene, dropped))
}

/// Resolve links before reading so a project can only reference its own files.
pub(crate) fn read_project_file(project: &Path, path: &Path) -> Result<Vec<u8>, IoError> {
    let root = project
        .canonicalize()
        .map_err(|e| err("open", project, e))?;
    let file = path.canonicalize().map_err(|e| err("open", path, e))?;
    if !file.starts_with(root) || !file.is_file() {
        return Err(IoError(
            "The project needs a regular file inside its folder.".into(),
        ));
    }
    fs::read(&file).map_err(|e| err("open", path, e))
}

fn read_scene(project: &Path, path: &Path) -> Result<Scene, IoError> {
    let bytes = read_project_file(project, path)?;
    let file: SceneFile = serde_json::from_slice(&bytes).map_err(|e| err("read", path, e))?;
    if file.format_version > FORMAT_VERSION {
        return Err(IoError(
            "The scene needs a newer version of Symphony.".into(),
        ));
    }
    validate(file.scene)
}

/// Reject scenes whose references don't line up, instead of crashing later.
fn validate(mut scene: Scene) -> Result<Scene, IoError> {
    // A damaged brush texture only loses its texture: its strokes draw
    // with a plain round tip instead of refusing the whole scene.
    scene.textures.retain(|_, t| t.is_valid());
    let bad = |what: &str| Err(IoError(format!("The scene is damaged: {what}.")));
    if scene.graph.node(scene.composite).is_none() || scene.graph.node(scene.display).is_none() {
        return bad("its output nodes are missing");
    }
    for l in &scene.layers {
        if scene.xsheet.column(l.column).is_none()
            || scene.graph.node(l.node).is_none()
            || scene.graph.node(l.peg).is_none()
        {
            return bad("a layer is missing its column or nodes");
        }
    }
    Ok(scene)
}

/// Crash recovery (E7.5): autosaves live in the app data folder, one file
/// per open scene, until the scene is saved normally.
pub struct Recovery {
    dir: PathBuf,
}

impl Recovery {
    pub fn new(dir: PathBuf) -> Self {
        Recovery { dir }
    }

    fn file(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// Media of one autosave, since an unsaved scene has no project folder.
    fn media(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.{}", media::MEDIA))
    }

    pub fn write(&self, key: &str, scene: &Scene, origin: Option<&Path>) -> Result<(), IoError> {
        #[derive(Serialize)]
        struct Rec<'a> {
            format_version: u32,
            origin: Option<&'a Path>,
            saved_at: u64,
            scene: &'a Scene,
        }
        let mut scene = scene.clone();
        media::write_media(&mut scene, &self.media(key))?;
        let rec = Rec {
            format_version: FORMAT_VERSION,
            origin,
            saved_at: now_secs(),
            scene: &scene,
        };
        let bytes = serde_json::to_vec(&rec).map_err(|e| err("autosave", &self.dir, e))?;
        write_atomic(&self.file(key), &bytes)
    }

    pub fn clear(&self, key: &str) {
        let _ = fs::remove_file(self.file(key));
        let _ = fs::remove_dir_all(self.media(key));
    }

    /// Autosaves left behind by a crash: (key, original path, scene).
    pub fn pending(&self) -> Vec<(String, Option<PathBuf>, Scene)> {
        #[derive(Deserialize)]
        struct Rec {
            format_version: u32,
            origin: Option<PathBuf>,
            scene: Scene,
        }
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if !e.file_type().ok()?.is_file() || p.extension()?.to_str()? != "json" {
                    return None;
                }
                let key = p.file_stem()?.to_string_lossy().into_owned();
                let rec: Rec = serde_json::from_slice(&fs::read(&p).ok()?).ok()?;
                (rec.format_version <= FORMAT_VERSION).then_some(())?;
                let mut scene = validate(rec.scene).ok()?;
                media::read_media(&mut scene, &self.dir, &self.media(&key));
                Some((key, rec.origin, scene))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphony_core::doc::Settings;
    use symphony_core::history::{Edit, History};

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("symphony-io-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    fn sample() -> Scene {
        let mut s = Scene::new(Settings::default());
        let snap = s.make_layer("Drawing", 0);
        History::default().apply(&mut s, "Add layer", vec![Edit::AddLayer(Box::new(snap))]);
        s
    }

    #[test]
    fn save_and_open_round_trip() {
        let dir = temp("roundtrip");
        let path = dir.join("Walk");
        let s = sample();
        save(&s, &path).unwrap();
        let p = with_extension(&path);
        assert!(p.join("project.json").is_file());
        let back = open(&p).unwrap();
        assert_eq!(back.layers, s.layers);
        assert_eq!(project_name(&p), "Walk");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn textures_survive_saving_and_bad_ones_are_dropped() {
        let dir = temp("textures");
        let path = dir.join("Tex");
        let mut s = sample();
        let good = symphony_core::stroke::Texture {
            width: 2,
            height: 2,
            pixels: vec![0, 64, 128, 255],
        };
        s.textures.insert(7, good.clone());
        s.textures.insert(
            8,
            symphony_core::stroke::Texture {
                width: 9,
                height: 9,
                pixels: vec![1],
            },
        );
        save(&s, &path).unwrap();
        let back = open(&with_extension(&path)).unwrap();
        assert_eq!(back.textures.get(&7), Some(&good));
        assert!(!back.textures.contains_key(&8));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_five_backups() {
        let dir = temp("backups");
        let path = dir.join("B.symphony");
        let s = sample();
        for _ in 0..8 {
            save(&s, &path).unwrap();
        }
        let n = fs::read_dir(path.join(".backups")).unwrap().count();
        assert_eq!(n, KEEP_BACKUPS);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_newer_formats_and_garbage() {
        let dir = temp("newer");
        let path = dir.join("N.symphony");
        save(&sample(), &path).unwrap();
        let manifest = r#"{"format_version": 999, "app_version": "9.0.0", "scenes": ["main"]}"#;
        fs::write(path.join("project.json"), manifest).unwrap();
        assert!(open(&path).unwrap_err().0.contains("newer Symphony"));
        fs::write(path.join("project.json"), b"nonsense").unwrap();
        assert!(open(&path).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_paths_outside_the_project() {
        let dir = temp("escape");
        let path = dir.join("E.symphony");
        save(&sample(), &path).unwrap();
        let manifest = r#"{"format_version": 1, "app_version": "0", "scenes": ["../../etc"]}"#;
        fs::write(path.join("project.json"), manifest).unwrap();
        assert!(open(&path).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn damaged_scenes_are_rejected_and_good_autosaves_still_recover() {
        let dir = temp("damaged-scenes");
        let project = dir.join("Test.symphony");
        let recovery_dir = dir.join("recovery");
        let recovery = Recovery::new(recovery_dir.clone());
        let scene = sample();
        save(&scene, &project).unwrap();
        recovery.write("good", &scene, None).unwrap();
        let file = project.join("scenes/main/scene.json");
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        let mut missing_node = original.clone();
        missing_node["scene"]["composite"] = serde_json::json!(u64::MAX);
        let mut future = original.clone();
        future["format_version"] = serde_json::json!(FORMAT_VERSION + 1);
        let mut truncated = serde_json::to_vec(&original).unwrap();
        truncated.pop();
        for bytes in [
            truncated,
            b"[]".to_vec(),
            br#"{"format_version":1e9999,"scene":{}}"#.to_vec(),
            serde_json::to_vec(&missing_node).unwrap(),
            serde_json::to_vec(&future).unwrap(),
        ] {
            fs::write(&file, &bytes).unwrap();
            fs::write(recovery_dir.join("damaged.json"), &bytes).unwrap();
            assert!(open(&project).is_err());
            let pending = recovery.pending();
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].0, "good");
        }
        fs::copy(
            recovery_dir.join("good.json"),
            recovery_dir.join("partial.tmp"),
        )
        .unwrap();
        assert_eq!(recovery.pending().len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_scene_link_outside_project() {
        let dir = temp("linked-scene");
        let project = dir.join("Test.symphony");
        save(&sample(), &project).unwrap();
        let file = project.join("scenes/main/scene.json");
        let external = dir.join("external.json");
        fs::rename(&file, &external).unwrap();
        std::os::unix::fs::symlink(external, file).unwrap();
        assert!(open(&project).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recovery_round_trip() {
        let dir = temp("recovery");
        let r = Recovery::new(dir.clone());
        r.write("abc", &sample(), None).unwrap();
        assert_eq!(r.pending().len(), 1);
        r.clear("abc");
        assert!(r.pending().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    fn with_sound(seed: i16) -> (Scene, u64) {
        use symphony_core::sound::{add_track, content_key, SoundFile, SoundFormat};
        let mut s = sample();
        let bytes = media::wav(&[seed, -seed, seed], 8_000);
        let key = content_key(&bytes);
        let file = SoundFile {
            format: SoundFormat::Wav,
            rate: 8_000,
            channels: 1,
            samples: 3,
            bytes: bytes.into(),
        };
        s.sound_files.insert(key, file);
        let edit = add_track(&mut s, "Dialogue", key);
        History::default().apply(&mut s, "Import Sound", vec![edit]);
        (s, key)
    }

    #[test]
    fn sound_files_are_copied_to_media_and_read_back() {
        let dir = temp("sound");
        let path = dir.join("S.symphony");
        let (mut s, key) = with_sound(100);
        // A file left over from an undone import isn't saved.
        let (spare, file) = {
            let (t, k) = with_sound(7);
            (k, t.sound_files[&k].clone())
        };
        s.sound_files.insert(spare, file);
        save(&s, &path).unwrap();
        let name = s.sound_files[&key].file_name(key);
        let on_disk = fs::read(path.join("media").join(&name)).unwrap();
        assert_eq!(on_disk, *s.sound_files[&key].bytes);
        assert_eq!(fs::read_dir(path.join("media")).unwrap().count(), 1);
        let (back, dropped) = open_with_report(&path).unwrap();
        assert!(dropped.is_empty());
        assert_eq!(back.sounds, s.sounds);
        assert_eq!(back.sound_files[&key].bytes, s.sound_files[&key].bytes);
        assert_eq!(back.sound_files[&key].rate, 8_000);
        assert!(!back.sound_files.contains_key(&spare));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_or_missing_sound_is_left_out_and_reported() {
        let dir = temp("sound-bad");
        let path = dir.join("D.symphony");
        let (s, key) = with_sound(200);
        save(&s, &path).unwrap();
        let name = s.sound_files[&key].file_name(key);
        let file = path.join("media").join(&name);
        // Another valid WAV under this name: the content key doesn't match.
        fs::write(&file, media::wav(&[1, 2, 3], 8_000)).unwrap();
        let (back, dropped) = open_with_report(&path).unwrap();
        assert_eq!(dropped, std::slice::from_ref(&name));
        assert!(back.sound_files.is_empty());
        // The track stays, so the user sees what to import again.
        assert_eq!(back.sounds.len(), 1);
        fs::remove_file(&file).unwrap();
        assert_eq!(open_with_report(&path).unwrap().1, [name]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sound_with_bad_header_values_is_left_out() {
        let dir = temp("sound-header");
        let path = dir.join("H.symphony");
        let (s, key) = with_sound(300);
        save(&s, &path).unwrap();
        let scene = path.join("scenes/main/scene.json");
        let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&scene).unwrap()).unwrap();
        v["scene"]["sound_files"][key.to_string()]["rate"] = serde_json::json!(0);
        fs::write(&scene, serde_json::to_vec(&v).unwrap()).unwrap();
        let (back, dropped) = open_with_report(&path).unwrap();
        assert_eq!(dropped.len(), 1);
        assert!(back.sound_files.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_sound_link_outside_the_project() {
        let dir = temp("sound-link");
        let path = dir.join("L.symphony");
        let (s, key) = with_sound(400);
        save(&s, &path).unwrap();
        let file = path.join("media").join(s.sound_files[&key].file_name(key));
        let outside = dir.join("outside.wav");
        fs::rename(&file, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &file).unwrap();
        assert!(open(&path).unwrap().sound_files.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recovery_keeps_the_sound_of_unsaved_scenes() {
        let dir = temp("recovery-sound");
        let r = Recovery::new(dir.clone());
        let (s, key) = with_sound(500);
        r.write("k", &s, None).unwrap();
        let pending = r.pending();
        assert_eq!(
            pending[0].2.sound_files[&key].bytes,
            s.sound_files[&key].bytes
        );
        r.clear("k");
        assert!(!dir.join("k.media").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn with_image(seed: u8) -> (Scene, u64) {
        use symphony_core::bitmap::{layer_edits, Alignment, Alpha};
        let mut s = sample();
        let (key, img) = media::image_from_bytes(media::png(4, 3, seed)).unwrap();
        s.images.insert(key, img);
        let edits = layer_edits(&mut s, "BG", 1, &[key], 1, Alignment::Fit, Alpha::Straight);
        History::default().apply(&mut s, "Import image", edits);
        (s, key)
    }

    #[test]
    fn images_are_copied_to_media_and_read_back() {
        let dir = temp("media");
        let path = dir.join("M.symphony");
        let (mut s, key) = with_image(1);
        // An image left over from an undone import isn't saved.
        let (spare, img) = media::image_from_bytes(media::png(2, 2, 9)).unwrap();
        s.images.insert(spare, img);
        save(&s, &path).unwrap();
        let file = path.join("media").join(s.images[&key].file_name(key));
        assert_eq!(fs::read(&file).unwrap(), *s.images[&key].bytes);
        assert_eq!(fs::read_dir(path.join("media")).unwrap().count(), 1);
        let json = fs::read_to_string(path.join("scenes/main/scene.json")).unwrap();
        assert!(!json.contains(&format!("{spare}")));
        let (back, dropped) = open_with_report(&path).unwrap();
        assert!(dropped.is_empty());
        assert_eq!(back.images[&key].bytes, s.images[&key].bytes);
        assert_eq!((back.images[&key].width, back.images[&key].height), (4, 3));
        assert!(!back.images.contains_key(&spare));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_or_missing_image_is_dropped_and_reported() {
        let dir = temp("media-bad");
        let path = dir.join("D.symphony");
        let (s, key) = with_image(2);
        save(&s, &path).unwrap();
        let name = s.images[&key].file_name(key);
        let file = path.join("media").join(&name);
        // Another valid PNG under this name: the content key doesn't match.
        fs::write(&file, media::png(4, 3, 3)).unwrap();
        let (back, dropped) = open_with_report(&path).unwrap();
        assert_eq!(dropped, std::slice::from_ref(&name));
        assert!(back.images.is_empty());
        assert_eq!(back.layers.len(), 2);
        fs::remove_file(&file).unwrap();
        assert_eq!(open_with_report(&path).unwrap().1, [name]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_an_image_link_outside_the_project() {
        let dir = temp("media-link");
        let path = dir.join("L.symphony");
        let (s, key) = with_image(4);
        save(&s, &path).unwrap();
        let file = path.join("media").join(s.images[&key].file_name(key));
        let outside = dir.join("outside.png");
        fs::rename(&file, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &file).unwrap();
        assert!(open(&path).unwrap().images.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recovery_keeps_images_of_unsaved_scenes() {
        let dir = temp("recovery-media");
        let r = Recovery::new(dir.clone());
        let (s, key) = with_image(5);
        r.write("k", &s, None).unwrap();
        let pending = r.pending();
        assert_eq!(pending[0].2.images[&key].bytes, s.images[&key].bytes);
        r.clear("k");
        assert!(!dir.join("k.media").exists());
        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    fn images_and_sound_share_the_media_folder() {
        let dir = temp("media-both");
        let path = dir.join("B.symphony");
        let (mut s, image) = with_image(6);
        let (t, sound) = with_sound(600);
        s.sounds = t.sounds;
        s.sound_files = t.sound_files;
        save(&s, &path).unwrap();
        assert_eq!(fs::read_dir(path.join("media")).unwrap().count(), 2);
        let image_file = path.join("media").join(s.images[&image].file_name(image));
        fs::remove_file(&image_file).unwrap();
        // A lost image leaves the sound alone.
        let (back, dropped) = open_with_report(&path).unwrap();
        assert_eq!(dropped, [s.images[&image].file_name(image)]);
        assert!(back.images.is_empty());
        assert_eq!(back.sound_files[&sound].bytes, s.sound_files[&sound].bytes);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_1_files_still_open() {
        let dir = temp("v1");
        let path = dir.join("Old.symphony");
        save(&sample(), &path).unwrap();
        for file in ["project.json", "scenes/main/scene.json"] {
            let p = path.join(file);
            let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
            v["format_version"] = serde_json::json!(1);
            fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
        }
        let back = open(&path).unwrap();
        assert_eq!(back.layers.len(), 1);
        assert!(back.sounds.is_empty());
        assert!(back.images.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

pub mod harmony;
pub mod media;
