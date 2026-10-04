//! Locating what to purge: validating the root, finding marked directories by
//! marker, and finding policy matches to mark in the first place.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::config::Policy;
use crate::marker::target_os;
use crate::parallel;
use crate::size::seq_dir_size;

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
    let matches: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let warnings: Mutex<Vec<String>> = Mutex::new(Vec::new());

    parallel::walk(
        &[PathBuf::from(root)],
        |cursor, _, entries| {
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                if file_type.is_dir() {
                    // Any directory could hold a marker, so keep walking.
                    cursor.descend(path);
                } else if file_type.is_file() && path.file_name() == Some(target_os()) {
                    // The marker names the directory that contains it, so the
                    // parent of this entry is what the caller wants.
                    if let Some(parent) = path.parent() {
                        matches
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(parent.to_path_buf());
                    }
                }
            }
        },
        |path, e| {
            warnings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(format!(
                    "warning: skipping {}: IO error for operation on {}: {}",
                    path.display(),
                    path.display(),
                    e
                ));
        },
    );

    let mut messages = warnings.into_inner().unwrap_or_else(|e| e.into_inner());
    // The walk is concurrent, so report in a fixed order rather than in
    // whatever order the threads happened to finish.
    messages.sort();
    for message in messages {
        writeln!(warn, "{}", message)?;
    }

    let mut matches = matches.into_inner().unwrap_or_else(|e| e.into_inner());
    matches.sort();
    matches.dedup();
    Ok(matches)
}

/// A directory that matched a policy, with the size that justified the match.
pub(crate) struct Candidate<'a> {
    pub(crate) path: PathBuf,
    pub(crate) policy: &'a str,
    pub(crate) size: u64,
}

/// The policy set, indexed by directory name.
///
/// A scan visits every directory in the tree and each policy may claim several
/// names, so comparing every policy against every name turns a large tree into
/// tens of millions of string comparisons. Indexing the names once turns the
/// same scan into one lookup per directory.
pub(crate) struct Matcher<'a> {
    policies: Vec<&'a Policy>,
    /// Directory name -> indices of the policies that claim it, in file order.
    by_name: HashMap<&'a str, Vec<u32>>,
}

impl<'a> Matcher<'a> {
    pub(crate) fn new(policies: &[&'a Policy]) -> Self {
        let enabled: Vec<&'a Policy> = policies
            .iter()
            .copied()
            .filter(|p| p.is_enabled())
            .collect();
        let mut by_name: HashMap<&'a str, Vec<u32>> = HashMap::new();
        for (index, policy) in enabled.iter().enumerate() {
            let index = index as u32;
            if let Some(name) = policy.dir_name.as_deref() {
                by_name.entry(name).or_default().push(index);
            }
            for name in &policy.dir_name_any {
                by_name.entry(name.as_str()).or_default().push(index);
            }
        }
        for indices in by_name.values_mut() {
            // First match wins, so the file order of the policies has to be
            // preserved inside each bucket.
            indices.sort_unstable();
        }
        Matcher {
            policies: enabled,
            by_name,
        }
    }

    /// Test one directory against the policies, returning the first match.
    pub(crate) fn match_dir(&self, dir: &Path, default_min: Option<u64>) -> Option<Candidate<'a>> {
        let name = dir.file_name()?.to_str()?;
        let candidates = self.by_name.get(name)?;
        let parent = dir.parent()?;
        // Measured at most once: every policy is looking at the same bytes.
        let mut measured: Option<u64> = None;
        for index in candidates {
            let policy = &self.policies[*index as usize];
            if !policy.parent_ok(parent) || !policy.child_ok(dir) {
                continue;
            }
            let size = match measured {
                Some(size) => size,
                None => *measured.insert(seq_dir_size(dir)),
            };
            if let Some(min) = policy.effective_min(default_min) {
                if size < min {
                    continue;
                }
            }
            return Some(Candidate {
                path: dir.to_path_buf(),
                policy: policy.name.as_str(),
                size,
            });
        }
        None
    }
}

/// Walk `dir` collecting policy matches. Matched directories are not descended
/// into, so a build tree never yields nested candidates.
pub(crate) fn scan_for_policies<'a>(
    dir: &Path,
    matcher: &Matcher<'a>,
    default_min: Option<u64>,
    found: &mut Vec<Candidate<'a>>,
    warn: &mut impl io::Write,
) {
    let matches: Mutex<Vec<Candidate<'a>>> = Mutex::new(Vec::new());
    let warnings: Mutex<Vec<String>> = Mutex::new(Vec::new());

    parallel::walk(
        &[dir.to_path_buf()],
        |cursor, _, entries| {
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
                match matcher.match_dir(&path, default_min) {
                    Some(candidate) => {
                        matches
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(candidate);
                    }
                    // Only descend when the directory is not itself a match.
                    None => cursor.descend(path),
                }
            }
        },
        |path, e| {
            // This warning predates the parallel walker and came from a bare
            // fs::read_dir, so it quotes the errno on its own.
            warnings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(format!("warning: cannot read {}: {}", path.display(), e));
        },
    );

    let mut messages = warnings.into_inner().unwrap_or_else(|e| e.into_inner());
    messages.sort();
    for message in messages {
        let _ = writeln!(warn, "{}", message);
    }

    found.extend(matches.into_inner().unwrap_or_else(|e| e.into_inner()));
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
    fn test_find_normalises_a_trailing_slash_root() {
        let dir = tempfile::tempdir().unwrap();
        write_dir_with_content(dir.path(), &[("PURGABLE", "")]);
        let root = format!("{}/", dir.path().display());

        let matches = find(&root, &mut io::stderr()).unwrap();
        assert_eq!(matches, vec![dir.path().to_path_buf()]);
    }

    #[test]
    fn test_find_discovers_nested_markers_inside_marked_directories() {
        // A marked directory may itself contain marked directories; both are
        // reported, which is what `list` and `unmark` need.
        let dir = tempfile::tempdir().unwrap();
        write_dir_with_content(
            dir.path(),
            &[("outer/PURGABLE", ""), ("outer/inner/PURGABLE", "")],
        );

        let matches = find(dir.path().to_str().unwrap(), &mut io::stderr()).unwrap();
        assert_eq!(matches.len(), 2);
        assert!(matches.contains(&dir.path().join("outer")));
        assert!(matches.contains(&dir.path().join("outer/inner")));
    }

    #[test]
    fn test_validate_root_rejects_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("file.txt");
        fs::write(&f, "x").unwrap();
        assert!(validate_root(f.to_str().unwrap()).is_err());
    }
}
