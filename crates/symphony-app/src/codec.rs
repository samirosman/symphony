//! Cisco's OpenH264 codec, for movie export (docs/CODECS.md).
//!
//! Like Harmony, Symphony never ships the codec. When the user asks for it,
//! Symphony downloads Cisco's own binary straight from Cisco onto the user's
//! computer, checks it against Cisco's published SHA-256, and keeps it in
//! the app data folder. The user can turn it off, back on, or remove it in
//! Preferences > Video, where Cisco's required notice is shown. These are the
//! conditions of Cisco's binary licence (assets/OPENH264_BINARY_LICENSE.txt).

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// Cisco's required notice, shown wherever the codec can be controlled.
pub const NOTICE: &str = "OpenH264 Video Codec provided by Cisco Systems, Inc.";
pub const LICENSE: &str = include_str!("../assets/OPENH264_BINARY_LICENSE.txt");
pub const VERSION: &str = "2.7.0";
const BASE: &str = "https://ciscobinary.openh264.org";

/// Cisco's file name for this computer, if Cisco builds one.
pub fn file_name() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "libopenh264-2.7.0-mac-arm64.dylib",
        ("macos", "x86_64") => "libopenh264-2.7.0-mac-x64.dylib",
        ("linux", "x86_64") => "libopenh264-2.7.0-linux64.8.so",
        ("linux", "aarch64") => "libopenh264-2.7.0-linux-arm64.8.so",
        ("windows", "x86_64") => "openh264-2.7.0-win64.dll",
        ("windows", "aarch64") => "openh264-2.7.0-win-arm64.dll",
        _ => return None,
    })
}

fn dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Symphony").join("codecs"))
}

/// The downloaded codec, if it's there.
pub fn installed() -> Option<PathBuf> {
    let p = dir()?.join(file_name()?);
    p.is_file().then_some(p)
}

pub fn remove() {
    if let Some(p) = installed() {
        let _ = fs::remove_file(p);
    }
}

/// Download progress for the UI: bytes done and total, when known.
pub type Progress = dyn Fn(u64, Option<u64>) + Send;

/// Download Cisco's binary for this computer, check it, and install it.
/// Runs on a background thread (ENGINEERING E5.2).
pub fn download(progress: &Progress) -> Result<PathBuf, String> {
    let name = file_name().ok_or("Cisco doesn't publish OpenH264 for this kind of computer.")?;
    let dir = dir().ok_or("Couldn't find a folder for the codec.")?;
    fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {} ({e}).", dir.display()))?;
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(10 * 60)))
            .build(),
    );
    let get = |url: String| {
        agent
            .get(&url)
            .header("User-Agent", "Symphony")
            .call()
            .map_err(|e| format!("Couldn't reach Cisco's download server ({e}). Check your connection and try again."))
    };

    // Cisco's checksum file: "SHA256 (<name>) = <hex>".
    let mut sums = String::new();
    get(format!("{BASE}/{name}.signed.sha256.txt"))?
        .into_body()
        .into_reader()
        .take(64 * 1024)
        .read_to_string(&mut sums)
        .map_err(|e| format!("Couldn't read Cisco's checksum ({e})."))?;
    let expected = sums
        .split('=')
        .nth(1)
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| h.len() == 64)
        .ok_or("Cisco's checksum file wasn't in the expected format.")?;

    let resp = get(format!("{BASE}/{name}.bz2"))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse().ok());
    let mut compressed = Vec::new();
    let mut body = resp.into_body().into_reader().take(64 * 1024 * 1024);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = body
            .read(&mut buf)
            .map_err(|e| format!("The download stopped ({e}). Try again."))?;
        if n == 0 {
            break;
        }
        compressed.extend_from_slice(&buf[..n]);
        progress(compressed.len() as u64, total);
    }
    let mut binary = Vec::new();
    bzip2::read::BzDecoder::new(compressed.as_slice())
        .read_to_end(&mut binary)
        .map_err(|e| format!("The download was damaged ({e}). Try again."))?;
    let actual: String = Sha256::digest(&binary)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err(
            "The download didn't match Cisco's checksum, so it wasn't installed. Try again.".into(),
        );
    }
    let dest = dir.join(name);
    let tmp = dir.join(format!("{name}.tmp"));
    fs::write(&tmp, &binary).map_err(|e| format!("Couldn't save the codec ({e})."))?;
    fs::rename(&tmp, &dest).map_err(|e| format!("Couldn't install the codec ({e})."))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    /// Real download from Cisco. Opt-in: SYMPHONY_TEST_NETWORK=1.
    #[test]
    fn downloads_and_verifies_from_cisco() {
        if std::env::var_os("SYMPHONY_TEST_NETWORK").is_none() || super::file_name().is_none() {
            return;
        }
        let path = super::download(&|_, _| {}).expect("download");
        assert!(path.is_file());
        // The loader's own list of known Cisco builds must accept it too.
        openh264::OpenH264API::from_blob_path(&path).expect("known Cisco build");
    }

    #[test]
    fn notice_and_license_are_present() {
        assert!(super::LICENSE.contains(super::NOTICE));
        assert!(super::LICENSE.contains("separately downloaded"));
    }
}
