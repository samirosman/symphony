//! Stamp the build with where it came from, so testers can tell a release
//! from a dev build of `main` or of someone's branch.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn main() {
    let hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some();
    let stamp = match branch.as_str() {
        "" | "HEAD" => hash,
        b => format!("{b} {hash}{}", if dirty { "+" } else { "" }),
    };
    println!("cargo:rustc-env=SYMPHONY_BUILD={stamp}");
    // Rebuild the stamp when the checked-out commit changes.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={dir}/{r}");
        }
    }
}
