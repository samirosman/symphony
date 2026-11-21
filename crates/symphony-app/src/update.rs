//! Hand off to a separate process so the editor can close before an update.
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

fn source_root(exe: &Path) -> Option<PathBuf> {
    let target = exe.parent()?.parent()?;
    let repo = target.parent()?;
    (target.file_name()? == "target" && repo.join("crates/symphony-app/Cargo.toml").is_file())
        .then(|| repo.to_owned())
}

pub fn start() -> std::io::Result<Child> {
    let exe = std::env::current_exe()?;
    let repo = std::env::var_os("SYMPHONY_DEV_REPO")
        .map(PathBuf::from)
        .or_else(|| source_root(&exe));
    if let Some(repo) = repo {
        let launcher = repo
            .join("target/release")
            .join(format!("symphony-launcher{}", std::env::consts::EXE_SUFFIX));
        let mut cmd = if launcher.is_file() {
            Command::new(launcher)
        } else {
            let mut cargo = Command::new("cargo");
            cargo.args(["run", "--release", "-p", "symphony-launcher", "--"]);
            cargo
        };
        return cmd.current_dir(&repo).arg("--dev").arg(&repo).spawn();
    }
    Command::new(exe).arg("--update").spawn()
}
