//! Size accounting and human-readable formatting of sizes, ages, and
//! timestamps.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use crate::discovery::is_regular_file;

const SIZE_UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];

/// Recursively sum the size of regular files under `path`.
///
/// Symlinks are never followed and never counted, so a link pointing at a large
/// tree cannot inflate the figure. Hardlinks are counted once per directory
/// entry, which can overstate a tree containing many links to one file.
pub(crate) fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    for entry in WalkDir::new(path).follow_links(false).into_iter().flatten() {
        if entry.path() == path {
            continue;
        }
        if is_regular_file(&entry) {
            total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    total
}

pub(crate) fn human_size(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < SIZE_UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{}{}", bytes, SIZE_UNITS[0])
    } else {
        format!("{:.1}{}", value, SIZE_UNITS[unit])
    }
}

/// Parse a human size such as `500M`, `1.5G`, `2TiB`. A bare number is bytes.
pub(crate) fn parse_size(input: &str) -> Option<u64> {
    let text = input.trim().to_ascii_uppercase();
    if text.is_empty() {
        return None;
    }
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    // Accept "M", "MB", "MiB" and friends: drop a trailing "B", then "I".
    let suffix = unit.trim().trim_end_matches('B').trim_end_matches('I');
    let multiplier: u64 = match suffix {
        "" => 1,
        "K" => 1024,
        "M" => 1024 * 1024,
        "G" => 1024 * 1024 * 1024,
        "T" => 1024u64 * 1024 * 1024 * 1024,
        _ => return None,
    };
    let value: f64 = number.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * multiplier as f64) as u64)
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn human_age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{}s ago", s),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::write_file;
    use std::os::unix::fs::symlink;

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("500M"), Some(500 * 1024 * 1024));
        assert_eq!(parse_size("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size("1.5G"), Some(1536 * 1024 * 1024));
        assert_eq!(parse_size("2TiB"), Some(2 * 1024 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("12Q"), None);
    }

    #[test]
    fn test_human_size() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(999), "999B");
        assert_eq!(human_size(1024), "1.0K");
        assert_eq!(human_size(500 * 1024 * 1024), "500.0M");
        assert_eq!(human_size(7 * 1024 * 1024 * 1024), "7.0G");
    }

    #[test]
    fn test_dir_size_counts_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("a/big.bin"), &"x".repeat(2048));
        write_file(&root.join("a/b/small.bin"), &"y".repeat(1024));
        assert_eq!(dir_size(root), 3072);
    }

    #[test]
    fn test_dir_size_ignores_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("real.bin"), &"x".repeat(4096));
        symlink(root.join("real.bin"), root.join("link.bin")).unwrap();
        assert_eq!(dir_size(root), 4096);
    }
}
