//! Locating what to purge: validating the root, finding marked directories by
//! marker, and finding policy matches to mark in the first place.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

use crate::config::Policy;
use crate::marker::TARGET;
use crate::size::dir_size;

/// Confirms that `root` exists and is a directory, returning its metadata.
pub(crate) fn validate_root(root: &str) -> io::Result<fs::Metadata> {
    let info = fs::metadata(root)?;
    if !info.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{:?} is not a directory", root),
        ));
    }
    Ok(info)
}

/// Recursively finds every directory under `root` that contains a `PURGABLE`
/// marker file.
///
/// Symlinks are never followed. Unreadable entries are reported to `warn` and
/// skipped rather than aborting the scan. The result is sorted and deduplicated
/// so that a run is deterministic across filesystems.
pub(crate) fn find(root: &str, warn: &mut impl io::Write) -> io::Result<Vec<PathBuf>> {
    let mut matches = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                let path_str = e
                    .path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<unknown>".to_string());
                writeln!(warn, "warning: skipping {}: {}", path_str, e)?;
                continue;
            }
        };
        if is_regular_file(&entry) && entry.file_name() == OsStr::new(TARGET) {
            matches.push(entry.path().parent().unwrap().to_path_buf());
        }
    }
    matches.sort();
    matches.dedup();
    Ok(matches)
}

/// True when the entry is a regular file, which a symlink never is when
/// symlinks are not being followed.
pub(crate) fn is_regular_file(entry: &DirEntry) -> bool {
    entry.file_type().is_file()
}

/// A directory that matched a policy, with the size that justified the match.
pub(crate) struct Candidate {
    pub(crate) path: PathBuf,
    pub(crate) policy: String,
    pub(crate) size: u64,
}

/// Test one directory against the policies, returning the first match.
pub(crate) fn match_policy(
    dir: &Path,
    policies: &[&Policy],
    default_min: Option<u64>,
) -> Option<Candidate> {
    let name = dir.file_name()?.to_str()?;
    for policy in policies {
        if !policy.is_enabled() || !policy.name_matches(name) {
            continue;
        }
        let parent = dir.parent()?;
        if !policy.parent_ok(parent) || !policy.child_ok(dir) {
            continue;
        }
        let size = dir_size(dir);
        if let Some(min) = policy.effective_min(default_min) {
            if size < min {
                continue;
            }
        }
        return Some(Candidate {
            path: dir.to_path_buf(),
            policy: policy.name.clone(),
            size,
        });
    }
    None
}

/// Walk `dir` collecting policy matches. Matched directories are not descended
/// into, so a build tree never yields nested candidates.
pub(crate) fn scan_for_policies(
    dir: &Path,
    policies: &[&Policy],
    default_min: Option<u64>,
    found: &mut Vec<Candidate>,
    warn: &mut impl io::Write,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            let _ = writeln!(warn, "warning: cannot read {}: {}", dir.display(), e);
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // file_type() does not traverse symlinks, so this rejects them here.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if name == ".git" {
            continue;
        }
        if let Some(candidate) = match_policy(&path, policies, default_min) {
            found.push(candidate);
            continue;
        }
        scan_for_policies(&path, policies, default_min, found, warn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_dir_with_content, write_file};
    use std::fs;
    use std::io;
    use std::os::unix::fs::symlink;

    #[test]
    fn test_exact_filename_matching() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("PURGABLE"), "x");
        write_file(&root.join("PURGABLE.txt"), "x");
        write_file(&root.join("purgable"), "x");
        write_file(&root.join("PURGABLE.old"), "x");
        let _ = symlink(root.join("purgable"), root.join("PURGABLE.link"));

        let matches = find(root.to_str().unwrap(), &mut io::stderr()).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], root);
    }

    #[test]
    fn test_recursive_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("PURGABLE"), "x");
        write_file(&root.join("a/PURGABLE"), "x");
        write_file(&root.join("a/b/c/PURGABLE"), "x");
        write_file(&root.join("a/b/other.txt"), "x");

        let matches = find(root.to_str().unwrap(), &mut io::stderr()).unwrap();
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_target_is_containing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[("PURGABLE", "")]);

        let matches = find(dir.path().to_str().unwrap(), &mut io::stderr()).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], sub);
    }

    #[test]
    fn test_validate_root_rejects_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("file.txt");
        fs::write(&f, "x").unwrap();
        assert!(validate_root(f.to_str().unwrap()).is_err());
    }
}
