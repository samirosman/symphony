//! Crash reports. If Symphony panics, a report (version, system, what went
//! wrong and where, a backtrace) is written to the crashes folder. On the
//! next launch the app offers to send it: that opens a pre-filled GitHub
//! issue in the browser for the tester to read and submit, so nothing is
//! ever sent without them seeing it.

use std::path::PathBuf;

const ISSUES: &str = "https://github.com/tdemers218/symphony/issues/new";

pub fn dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Symphony").join("crashes"))
}

/// Write a report on any panic, then let the default hook print it too.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let report = format!(
            "Symphony {} ({})\nSystem: {} {}\nThread: {}\n\n{}\n\nBacktrace:\n{}\n",
            env!("CARGO_PKG_VERSION"),
            env!("SYMPHONY_BUILD"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            thread.name().unwrap_or("unnamed"),
            info,
            backtrace,
        );
        if let Some(dir) = dir() {
            let _ = std::fs::create_dir_all(&dir);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let _ = std::fs::write(dir.join(format!("crash-{stamp}.txt")), report);
        }
        previous(info);
    }));
}

/// The newest report not yet sent or dismissed.
pub fn pending() -> Option<(PathBuf, String)> {
    let dir = dir()?;
    let mut reports: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    reports.sort();
    let path = reports.pop()?;
    let text = std::fs::read_to_string(&path).ok()?;
    Some((path, text))
}

/// Done with a report (sent or dismissed): it moves to `handled/`, so it's
/// kept but not offered again.
pub fn handled(path: &std::path::Path) {
    if let (Some(dir), Some(name)) = (dir(), path.file_name()) {
        let done = dir.join("handled");
        let _ = std::fs::create_dir_all(&done);
        let _ = std::fs::rename(path, done.join(name));
    }
}

/// A new-issue link with the report filled in (cut to fit a URL).
pub fn issue_url(report: &str) -> String {
    let first = report
        .lines()
        .find(|l| l.contains("panicked"))
        .unwrap_or("Symphony closed unexpectedly");
    let title = format!("Crash: {}", first.chars().take(90).collect::<String>());
    let mut body = String::from(
        "**What were you doing when it closed?**\n\n\n**Report** (written by Symphony):\n```\n",
    );
    body.push_str(&report.chars().take(5500).collect::<String>());
    body.push_str("\n```\n");
    format!(
        "{ISSUES}?title={}&labels=crash&body={}",
        encode(&title),
        encode(&body)
    )
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_issue_link_carries_the_report_safely() {
        let report = "Symphony 0.1.0 (main abc)\nSystem: macos aarch64\n\npanicked at src/x.rs:1:1: boom & bust\n";
        let url = issue_url(report);
        assert!(url.starts_with(ISSUES));
        assert!(url.contains("title=Crash%3A%20panicked"));
        assert!(!url.contains(' ') && !url.contains('&') || url.matches('&').count() == 2);
        assert!(url.len() < 20_000);
    }
}
