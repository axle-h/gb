//! A text the model rewrites, kept as every version of it: one JSON object per line, appended and
//! never rewritten, so the current text is the last line.
//! ```text
//! {"version":3,"at":1760000000000,"turn":41,"text":"1. beat Brock\n"}
//! ```

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::published::now_ms;
use crate::run::transcript::RevLines;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Version {
    /// 1 for the first and one more for each after it, which is what a page steps and pages by.
    pub version: u64,
    /// Unix milliseconds.
    pub at: u64,
    /// The turn that wrote it; absent for a clear and for a file carried over from an older run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    pub text: String,
}

/// The newest version, read from the end; a line torn by a crash is skipped.
pub fn last(path: &Path) -> Option<Version> {
    let file = std::fs::File::open(path).ok()?;
    RevLines::new(file).find_map(|line| serde_json::from_str(&line).ok())
}

/// The newest `limit` versions older than `before`, or than none, oldest first.
pub fn page(path: &Path, limit: usize, before: Option<u64>) -> Vec<Version> {
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    let mut versions: Vec<Version> = RevLines::new(file)
        .filter_map(|line| serde_json::from_str::<Version>(&line).ok())
        .filter(|version| before.is_none_or(|before| version.version < before))
        .take(limit)
        .collect();
    versions.reverse();
    versions
}

/// One more line. A last line torn by a crash is ended first, so it stays the only line lost.
pub fn append(path: &Path, version: &Version) -> Result<(), String> {
    let mut line = serde_json::to_string(version).map_err(|e| format!("could not encode a version: {e}"))?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let mut last = [0u8; 1];
    let torn = file.seek(SeekFrom::End(-1)).is_ok() && file.read_exact(&mut last).is_ok() && last[0] != b'\n';
    if torn {
        line.insert(0, '\n');
    }
    file.write_all(line.as_bytes()).map_err(|e| format!("could not append to {}: {e}", path.display()))
}

/// When a file from an older run was last written, for the version it becomes.
pub fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(now_ms, |since| since.as_millis() as u64)
}

/// A version log and the version it ends on.
pub struct Versions {
    /// `None` for a run with no directory, which counts versions and writes nothing.
    path: Option<PathBuf>,
    current: Option<Version>,
}

impl Versions {
    pub fn open(path: Option<PathBuf>) -> Self {
        let current = path.as_deref().and_then(last);
        Self { path, current }
    }

    pub fn text(&self) -> &str {
        self.current.as_ref().map_or("", |version| version.text.as_str())
    }

    /// 0 before the first.
    pub fn version(&self) -> u64 {
        self.current.as_ref().map_or(0, |version| version.version)
    }

    /// Append `text` as the next version, unless it is the current one; before the first, the
    /// current one is empty. A failed write is
    /// reported and the text is current anyway, as it would be with no directory.
    pub fn record(&mut self, text: &str, turn: Option<u64>) {
        if self.text() == text {
            return;
        }
        let next = Version { version: self.version() + 1, at: now_ms(), turn, text: text.to_string() };
        if let Some(path) = self.path.as_deref() {
            if let Err(failure) = append(path, &next) {
                eprintln!("{failure}");
            }
        }
        self.current = Some(next);
    }
}

/// Delete a file an older run kept, if it is there.
pub fn remove_legacy(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("could not delete {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Scratch;

    #[test]
    fn every_version_is_a_line_and_the_last_is_current() {
        let scratch = Scratch::new("versions");
        let path = scratch.0.join("plan-history.jsonl");
        let mut versions = Versions::open(Some(path.clone()));
        assert_eq!((versions.version(), versions.text()), (0, ""));
        versions.record("a", Some(3));
        versions.record("a", Some(4));
        versions.record("b\nc", None);
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2, "a repeat is not a version");

        let reopened = Versions::open(Some(path.clone()));
        assert_eq!((reopened.version(), reopened.text()), (2, "b\nc"));
        let all = page(&path, 10, None);
        assert_eq!(all.iter().map(|v| (v.version, v.turn)).collect::<Vec<_>>(), [(1, Some(3)), (2, None)]);
    }

    #[test]
    fn a_page_is_the_newest_before_a_version_oldest_first() {
        let scratch = Scratch::new("versions-page");
        let path = scratch.0.join("plan-history.jsonl");
        let mut versions = Versions::open(Some(path.clone()));
        for n in 1..=7 {
            versions.record(&format!("v{n}"), None);
        }
        let numbers = |page: Vec<Version>| page.into_iter().map(|v| v.version).collect::<Vec<_>>();
        assert_eq!(numbers(page(&path, 3, None)), [5, 6, 7]);
        assert_eq!(numbers(page(&path, 3, Some(5))), [2, 3, 4]);
        assert_eq!(numbers(page(&path, 3, Some(2))), [1]);
        assert!(page(&scratch.0.join("absent.jsonl"), 3, None).is_empty());
    }

    /// A crash mid-append loses that line and no other.
    #[test]
    fn a_torn_last_line_costs_only_itself() {
        let scratch = Scratch::new("versions-torn");
        let path = scratch.0.join("plan-history.jsonl");
        let mut versions = Versions::open(Some(path.clone()));
        versions.record("whole", None);
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(br#"{"version":2,"at":1,"te"#).unwrap();

        let mut reopened = Versions::open(Some(path.clone()));
        assert_eq!((reopened.version(), reopened.text()), (1, "whole"));
        reopened.record("next", None);
        assert_eq!(last(&path).map(|v| (v.version, v.text)), Some((2, "next".to_string())));
        assert_eq!(page(&path, 10, None).len(), 2);
    }
}
