//! Installing, keeping and launching Symphony versions (ENGINEERING E13.4).
//!
//! Layout inside the OS data folder (`~/Library/Application Support` on
//! macOS, `%APPDATA%` on Windows, `~/.local/share` on Linux):
//!
//! ```text
//! Symphony/
//!   launcher.json          channel + selected version
//!   versions/0.1.0/        one folder per installed version
//!   downloads/             partial downloads (deleted after install)
//! ```
//!
//! A version folder only counts as installed once its `.complete` marker
//! exists, so an interrupted install is never launched.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::release::{expected_sha256, Channel, Release};

/// Refuse downloads bigger than this (a hostile or broken server).
const MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;
const MARKER: &str = ".complete";

pub const EXE: &str = if cfg!(windows) {
    "symphony.exe"
} else {
    "symphony"
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub channel: Channel,
    /// Version to launch. `None` means the newest installed.
    pub selected: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn default_location() -> Option<Self> {
        Some(Paths {
            root: dirs::data_dir()?.join("Symphony"),
        })
    }
    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }
    fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }
    fn state_file(&self) -> PathBuf {
        self.root.join("launcher.json")
    }
    pub fn version_dir(&self, v: &Version) -> PathBuf {
        self.versions().join(v.to_string())
    }
    pub fn exe(&self, v: &Version) -> PathBuf {
        self.version_dir(v).join(EXE)
    }

    /// Bad or missing state falls back to defaults (E9.2).
    pub fn load_state(&self) -> State {
        fs::read(self.state_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save_state(&self, s: &State) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let tmp = self.state_file().with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(&tmp, self.state_file()).map_err(|e| e.to_string())
    }

    /// Installed versions, newest first.
    pub fn installed(&self) -> Vec<Version> {
        let Ok(entries) = fs::read_dir(self.versions()) else {
            return Vec::new();
        };
        let mut v: Vec<Version> = entries
            .flatten()
            .filter(|e| e.path().join(MARKER).exists())
            .filter_map(|e| Version::parse(&e.file_name().to_string_lossy()).ok())
            .collect();
        v.sort_by(|a, b| b.cmp(a));
        v
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Downloading { done: u64, total: u64 },
    Verifying,
    Installing,
}

pub fn sha256_hex(mut r: impl Read) -> std::io::Result<String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    crate::release::agent()
        .get(url)
        .header("User-Agent", "symphony-launcher")
        .call()
        .map_err(|e| {
            format!("Couldn't download from GitHub ({e}). Check your connection and try again.")
        })
}

/// Download, verify and install `release`. Runs on a background thread
/// (E5.2) and reports through `progress`.
pub fn install(
    paths: &Paths,
    release: &Release,
    progress: &dyn Fn(Progress),
) -> Result<Version, String> {
    let version = release
        .version()
        .ok_or("This release has an unreadable version number.")?;
    let archive = release
        .archive()
        .ok_or("This release has no build for this computer.")?;
    let sums_asset = release
        .checksums()
        .ok_or("This release has no checksum file, so it can't be checked.")?;

    let mut sums = String::new();
    get(&sums_asset.browser_download_url)?
        .into_body()
        .into_reader()
        .take(1024 * 1024)
        .read_to_string(&mut sums)
        .map_err(|e| format!("Couldn't read the checksum file ({e})."))?;
    let expected = expected_sha256(&sums, &archive.name)
        .ok_or("The checksum file doesn't list this build, so it can't be checked.")?
        .to_ascii_lowercase();

    fs::create_dir_all(paths.downloads()).map_err(|e| e.to_string())?;
    let part = paths.downloads().join(format!("{}.part", archive.name));
    let total = archive.size;
    {
        let mut out =
            File::create(&part).map_err(|e| format!("Couldn't write the download ({e})."))?;
        let mut body = get(&archive.browser_download_url)?
            .into_body()
            .into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            let n = body
                .read(&mut buf)
                .map_err(|e| format!("The download stopped ({e}). Try again."))?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > MAX_DOWNLOAD {
                let _ = fs::remove_file(&part);
                return Err("The download is far bigger than expected, so it was stopped.".into());
            }
            out.write_all(&buf[..n])
                .map_err(|e| format!("Couldn't write the download ({e})."))?;
            progress(Progress::Downloading { done, total });
        }
        out.sync_all().map_err(|e| e.to_string())?;
    }

    progress(Progress::Verifying);
    let actual = sha256_hex(BufReader::new(
        File::open(&part).map_err(|e| e.to_string())?,
    ))
    .map_err(|e| e.to_string())?;
    if actual != expected {
        let _ = fs::remove_file(&part);
        return Err("The download was damaged (its checksum didn't match). Try again.".into());
    }

    progress(Progress::Installing);
    let result = install_archive(paths, &part, &version, archive.name.ends_with(".zip"));
    let _ = fs::remove_file(&part);
    result.map(|_| version)
}

/// Extract `archive` into the version folder, atomically: unpack into a
/// temporary folder, check it, mark it complete, then rename into place.
pub fn install_archive(
    paths: &Paths,
    archive: &Path,
    version: &Version,
    is_zip: bool,
) -> Result<(), String> {
    let tmp = paths.versions().join(format!(".tmp-{version}"));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let file = File::open(archive).map_err(|e| e.to_string())?;
    // Both extractors refuse paths that escape the folder (E7.6).
    if is_zip {
        zip::ZipArchive::new(file)
            .and_then(|mut z| z.extract(&tmp))
            .map_err(|e| format!("Couldn't unpack the download ({e})."))?;
    } else {
        let mut t = tar::Archive::new(flate2::read::GzDecoder::new(file));
        t.set_preserve_permissions(true);
        t.unpack(&tmp)
            .map_err(|e| format!("Couldn't unpack the download ({e})."))?;
    }
    if !tmp.join(EXE).is_file() {
        let _ = fs::remove_dir_all(&tmp);
        return Err("The download doesn't contain the Symphony app.".into());
    }
    File::create(tmp.join(MARKER)).map_err(|e| e.to_string())?;
    let dest = paths.version_dir(version);
    let _ = fs::remove_dir_all(&dest);
    fs::rename(&tmp, &dest).map_err(|e| e.to_string())
}

/// Delete installed versions except `keep`. Never touches a running one (E13.4).
pub fn prune(paths: &Paths, keep: &[Version], running: &[Version]) {
    for v in paths.installed() {
        if !keep.contains(&v) && !running.contains(&v) {
            let _ = fs::remove_dir_all(paths.version_dir(&v));
        }
    }
}

pub fn launch(paths: &Paths, v: &Version) -> Result<Child, String> {
    Command::new(paths.exe(v))
        .current_dir(paths.version_dir(v))
        .spawn()
        .map_err(|e| format!("Couldn't start Symphony {v} ({e}). Try reinstalling it."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> Paths {
        let root = std::env::temp_dir().join(format!(
            "symphony-launcher-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        Paths { root }
    }

    fn make_tar_gz(path: &Path, files: &[(&str, &[u8])]) {
        let gz =
            flate2::write::GzEncoder::new(File::create(path).unwrap(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            tar.append_data(&mut h, name, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn sha256_known_value() {
        assert_eq!(
            sha256_hex(&b"abc"[..]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn installs_marks_complete_and_prunes() {
        let paths = temp_root("install");
        fs::create_dir_all(&paths.root).unwrap();
        let archive = paths.root.join("a.tar.gz");
        make_tar_gz(&archive, &[(EXE, b"app"), ("LICENSE", b"gpl")]);

        let v1 = Version::parse("0.1.0").unwrap();
        let v2 = Version::parse("0.2.0").unwrap();
        install_archive(&paths, &archive, &v1, false).unwrap();
        install_archive(&paths, &archive, &v2, false).unwrap();
        assert_eq!(paths.installed(), vec![v2.clone(), v1.clone()]);
        assert!(paths.exe(&v1).is_file());

        prune(&paths, std::slice::from_ref(&v2), &[]);
        assert_eq!(paths.installed(), vec![v2]);
        let _ = fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn rejects_archive_without_app() {
        let paths = temp_root("noapp");
        fs::create_dir_all(&paths.root).unwrap();
        let archive = paths.root.join("b.tar.gz");
        make_tar_gz(&archive, &[("README", b"hi")]);
        let v = Version::parse("0.1.0").unwrap();
        assert!(install_archive(&paths, &archive, &v, false).is_err());
        assert!(paths.installed().is_empty());
        let _ = fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn state_round_trips_and_survives_garbage() {
        let paths = temp_root("state");
        let s = State {
            channel: Channel::Preview,
            selected: Some("0.1.0".into()),
        };
        paths.save_state(&s).unwrap();
        assert_eq!(paths.load_state().channel, Channel::Preview);
        fs::write(paths.root.join("launcher.json"), b"not json").unwrap();
        assert_eq!(paths.load_state().channel, Channel::Stable);
        let _ = fs::remove_dir_all(&paths.root);
    }
}
