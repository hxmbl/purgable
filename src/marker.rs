//! The on-disk `PURGABLE` marker: its name, its format, and how it records
//! where it came from.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::size::human_age;

/// The exact, case-sensitive name of the marker file. A directory is purgable
/// when it directly contains a regular file with this name.
pub(crate) const TARGET: &str = "PURGABLE";

/// `TARGET` as an `OsStr`, so a directory entry's name can be compared against
/// the marker name without a lossy conversion per entry.
///
/// Built once and cached: `OsStr::new` is not const-callable on stable, and this
/// comparison sits in the innermost loop of every scan.
pub(crate) fn target_os() -> &'static OsStr {
    static TARGET_OS: OnceLock<OsString> = OnceLock::new();
    TARGET_OS.get_or_init(|| OsString::from(TARGET)).as_os_str()
}

/// First line of a marker written by this tool, distinguishing it from a
/// hand-placed empty file.
const MARKER_MAGIC: &str = "purgable:v1";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Marker {
    pub(crate) policy: Option<String>,
    pub(crate) marked_at: Option<u64>,
    pub(crate) size: Option<u64>,
}

impl Marker {
    /// True when this marker was written by `purgable mark`, as opposed to a
    /// marker a human created by hand (which is just an empty file).
    pub(crate) fn is_ours(&self) -> bool {
        self.policy.is_some()
    }

    /// Human-readable provenance for the review prompt.
    pub(crate) fn describe(&self, now: u64) -> String {
        match (&self.policy, self.marked_at) {
            (Some(policy), Some(at)) => {
                let age = now.saturating_sub(at);
                format!("{}, {}", policy, human_age(age))
            }
            (Some(policy), None) => policy.clone(),
            _ => "manual".to_string(),
        }
    }
}

pub(crate) fn marker_path(dir: &Path) -> PathBuf {
    dir.join(TARGET)
}

/// Read the marker in `dir`.
///
/// A missing or empty file yields a default `Marker`, which is how a
/// hand-placed marker is represented. Only files carrying the magic first line
/// are treated as tool-written.
pub(crate) fn read_marker(dir: &Path) -> Marker {
    let mut marker = Marker::default();
    let Ok(text) = fs::read_to_string(marker_path(dir)) else {
        return marker;
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MARKER_MAGIC) {
        return marker;
    }
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "policy" => marker.policy = Some(value.to_string()),
            "marked_at" => marker.marked_at = value.parse().ok(),
            "size" => marker.size = value.parse().ok(),
            _ => {}
        }
    }
    marker
}

pub(crate) fn write_marker(dir: &Path, policy: &str, size: u64, at: u64) -> io::Result<()> {
    let body = format!(
        "{}\npolicy={}\nmarked_at={}\nsize={}\n",
        MARKER_MAGIC, policy, at, size
    );
    fs::write(marker_path(dir), body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_write_and_read_marker_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_marker(root, "cargo-target", 4096, 1_700_000_000).unwrap();

        let marker = read_marker(root);
        assert!(marker.is_ours());
        assert_eq!(marker.policy.as_deref(), Some("cargo-target"));
        assert_eq!(marker.size, Some(4096));
        assert_eq!(marker.marked_at, Some(1_700_000_000));
    }

    #[test]
    fn test_hand_written_marker_is_not_ours() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(TARGET), "").unwrap();
        let marker = read_marker(root);
        assert!(!marker.is_ours());
        assert_eq!(marker.describe(0), "manual");
    }

    #[test]
    fn test_marker_missing_reads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_marker(dir.path()), Marker::default());
    }
}
