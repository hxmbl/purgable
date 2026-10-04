//! Size accounting and human-readable formatting of sizes, ages, and
//! timestamps.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::parallel;

const SIZE_UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];

/// Recursively sum the size of one directory.
///
/// Commands measure every candidate in a single pass with `dir_sizes`; this is
/// the single-tree form, for the places that only ever have one tree to measure.
#[cfg(test)]
pub(crate) fn dir_size(path: &Path) -> u64 {
    let sizes = dir_sizes(std::slice::from_ref(&path.to_path_buf()));
    sizes.first().copied().unwrap_or(0)
}

/// Recursively sum the size of every directory in `paths`, in parallel.
///
/// One traversal pass handles all of them, so measuring a tree of twenty build
/// directories costs one thread pool rather than twenty walks.
///
/// Symlinks are never followed and never counted, so a link pointing at a large
/// tree cannot inflate the figure. Hardlinks are counted once per directory
/// entry, which can overstate a tree containing many links to one file.
pub(crate) fn dir_sizes(paths: &[PathBuf]) -> Vec<u64> {
    let totals: Vec<AtomicU64> = paths.iter().map(|_| AtomicU64::new(0)).collect();
    parallel::walk(
        paths,
        |cursor, _, entries| {
            let mut total = 0u64;
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    cursor.descend(entry.path());
                } else if file_type.is_file() {
                    // DirEntry::metadata stats through the open directory
                    // handle, so no path has to be spelled out per file.
                    total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
                }
            }
            totals[cursor.task()].fetch_add(total, Ordering::Relaxed);
        },
        |_, _| {},
    );
    totals.iter().map(|t| t.load(Ordering::Relaxed)).collect()
}

/// Single-threaded size of one directory, for use from inside a walk that is
/// already running across every core.
pub(crate) fn seq_dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
            }
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

    #[test]
    fn test_dir_sizes_measures_each_root_independently() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small");
        let large = dir.path().join("large");
        write_file(&small.join("a.bin"), &"x".repeat(100));
        write_file(&large.join("b.bin"), &"y".repeat(5000));
        write_file(&large.join("nested/c.bin"), &"z".repeat(20));

        let sizes = dir_sizes(&[small.clone(), large.clone()]);
        assert_eq!(sizes[0], 100);
        assert_eq!(sizes[1], 5020);
    }

    #[test]
    fn test_dir_sizes_on_no_roots_is_empty() {
        assert!(dir_sizes(&[]).is_empty());
    }

    #[test]
    fn test_seq_dir_size_matches_parallel_size() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("a/big.bin"), &"x".repeat(2048));
        write_file(&root.join("a/b/c/small.bin"), &"y".repeat(1024));
        symlink(root.join("a"), root.join("link")).unwrap();
        assert_eq!(seq_dir_size(root), dir_size(root));
    }
}
