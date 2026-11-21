//! Finding releases on GitHub and picking the right file for this
//! computer (ENGINEERING E13.2–E13.3).
//!
//! Release files are named `symphony-<version>-<target>.<ext>`, where
//! `<ext>` is `tar.gz` on macOS and Linux and `zip` on Windows. Each
//! release also has a `SHA256SUMS` file.

use semver::Version;
use serde::{Deserialize, Serialize};

pub const REPO: &str = "tdemers218/symphony";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    /// Tagged releases only.
    #[default]
    Stable,
    /// Pre-releases too, for testers.
    Preview,
}

impl Channel {
    pub fn title(self) -> &'static str {
        match self {
            Channel::Stable => "Stable",
            Channel::Preview => "Preview",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn version(&self) -> Option<Version> {
        parse_version(&self.tag_name)
    }

    /// The app archive for this computer, if the release has one.
    pub fn archive(&self) -> Option<&Asset> {
        let v = self.version()?;
        let want = archive_name(&v.to_string(), current_target()?);
        self.assets.iter().find(|a| a.name == want)
    }

    pub fn checksums(&self) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == "SHA256SUMS")
    }
}

/// `v0.2.0-preview.1` -> 0.2.0-preview.1
pub fn parse_version(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

/// Rust target triple for the running launcher, if we ship builds for it.
pub fn current_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

pub fn archive_name(version: &str, target: &str) -> String {
    let ext = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("symphony-{version}-{target}.{ext}")
}

/// Newest usable release on `channel`: not a draft, has a build for this
/// computer and a checksum file.
pub fn newest(releases: &[Release], channel: Channel) -> Option<&Release> {
    releases
        .iter()
        .filter(|r| !r.draft && (channel == Channel::Preview || !r.prerelease))
        .filter(|r| r.archive().is_some() && r.checksums().is_some())
        .max_by_key(|r| r.version())
}

/// Look up `file` in a SHA256SUMS listing (`<hex>  <name>` per line).
pub fn expected_sha256<'a>(sums: &'a str, file: &str) -> Option<&'a str> {
    sums.lines().find_map(|l| {
        let (hash, name) = l.split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == file).then_some(hash.trim())
    })
}

pub fn fetch_releases() -> Result<Vec<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
    ureq::get(&url)
        .header("User-Agent", "symphony-launcher")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| {
            format!("Couldn't reach GitHub ({e}). You can still launch installed versions.")
        })?
        .body_mut()
        .read_json::<Vec<Release>>()
        .map_err(|e| format!("GitHub sent a reply the launcher couldn't read ({e})."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, pre: bool, with_build: bool) -> Release {
        let v = parse_version(tag).unwrap().to_string();
        let mut assets = vec![Asset {
            name: "SHA256SUMS".into(),
            browser_download_url: String::new(),
            size: 1,
        }];
        if with_build {
            assets.push(Asset {
                name: archive_name(&v, current_target().unwrap()),
                browser_download_url: String::new(),
                size: 1,
            });
        }
        Release {
            tag_name: tag.into(),
            body: None,
            prerelease: pre,
            draft: false,
            assets,
        }
    }

    #[test]
    fn picks_newest_for_channel() {
        let list = [
            rel("v0.1.0", false, true),
            rel("v0.2.0-preview.1", true, true),
            rel("v0.1.1", false, true),
        ];
        assert_eq!(newest(&list, Channel::Stable).unwrap().tag_name, "v0.1.1");
        assert_eq!(
            newest(&list, Channel::Preview).unwrap().tag_name,
            "v0.2.0-preview.1"
        );
    }

    #[test]
    fn skips_releases_without_a_build_for_this_computer() {
        let list = [rel("v0.1.0", false, true), rel("v0.3.0", false, false)];
        assert_eq!(newest(&list, Channel::Stable).unwrap().tag_name, "v0.1.0");
    }

    #[test]
    fn preview_orders_after_its_stable() {
        assert!(parse_version("v0.2.0-preview.1") < parse_version("v0.2.0"));
    }

    #[test]
    fn reads_sha256sums() {
        let sums = "abc123  symphony-0.1.0-x.tar.gz\ndef456 *symphony-0.1.0-y.zip\n";
        assert_eq!(
            expected_sha256(sums, "symphony-0.1.0-x.tar.gz"),
            Some("abc123")
        );
        assert_eq!(
            expected_sha256(sums, "symphony-0.1.0-y.zip"),
            Some("def456")
        );
        assert_eq!(expected_sha256(sums, "missing"), None);
    }
}
