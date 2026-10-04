//! The destructive filesystem operations behind the review actions: plain
//! removal, clearing contents, and secure deletion (shredding).
//!
//! The `PURGABLE` marker is deliberately never a target on its own: it is
//! metadata about the directory rather than content inside it, so clearing or
//! shredding a directory leaves the directory marked.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use walkdir::WalkDir;

use rand::{rng, RngExt};

use crate::discovery::is_regular_file;
use crate::marker::TARGET;

/// Size of the random overwrite buffer used per file.
const SHRED_BUF_SIZE: usize = 64 * 1024;

pub(crate) fn remove_dir(path: &Path) -> io::Result<()> {
    fs::remove_dir_all(path)
}

/// Delete everything inside `path`, keeping `path` itself and its marker.
pub(crate) fn clear_dir(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        // The marker is metadata about the directory, not content. Removing it
        // would silently unmark the directory as a side effect of clearing it.
        if child.file_name() == Some(OsStr::new(TARGET)) {
            continue;
        }
        remove_any(&child)?;
    }
    Ok(())
}

/// Remove a file, symlink, or directory. Unlike `remove_dir_all`, this works on
/// non-directories, and it never follows a symlink out of the tree.
pub(crate) fn remove_any(path: &Path) -> io::Result<()> {
    // symlink_metadata, not metadata: a symlink to a directory must be unlinked,
    // not recursed into.
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Overwrite every regular file inside `path`, then remove the remaining
/// entries, keeping `path` itself.
///
/// The PURGABLE marker is never shredded: it is metadata about the directory,
/// not content. After this, `path` contains only that marker.
pub(crate) fn shred_dir(path: &Path) -> io::Result<()> {
    // Collect first, then act. Removing entries while a WalkDir iterator is
    // still descending through them is not safe: the iterator would walk into
    // directories that no longer exist.
    let mut files = Vec::new();
    for entry in WalkDir::new(path).follow_links(false) {
        let entry = entry?;
        let p = entry.path();
        if p == Path::new(path) {
            continue;
        }
        if entry.file_name() == OsStr::new(TARGET) {
            continue;
        }
        if is_regular_file(&entry) {
            files.push(p.to_path_buf());
        }
    }

    for file in &files {
        shred_file(file)?;
    }
    // Directories hold no data of their own; once their files are gone they are
    // just empty husks. Removing them is what makes this action actually clear
    // the directory rather than leave a skeleton behind.
    clear_dir(path)
}

pub(crate) fn shred_file(path: &Path) -> io::Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if !metadata.is_file() {
        return Ok(());
    }
    let size = metadata.len();
    if size == 0 {
        return fs::remove_file(path);
    }

    let mut file = match fs::OpenOptions::new().write(true).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };

    let mut buf = vec![0u8; SHRED_BUF_SIZE];
    let mut written: u64 = 0;
    let mut rng = rng();

    while written < size {
        let n = std::cmp::min(SHRED_BUF_SIZE as u64, size - written) as usize;
        rng.fill(&mut buf[..n]);
        file.write_all(&buf[..n])?;
        written += n as u64;
    }
    file.sync_all()?;
    drop(file);
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::TARGET;
    use crate::test_support::write_dir_with_content;
    use std::fs;

    #[test]
    fn test_shred_file_overwrites_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("secret.bin");
        let body = "top secret contents";
        fs::write(&f, body).unwrap();

        shred_file(&f).unwrap();
        assert!(!f.exists());
    }

    #[test]
    fn test_shred_file_disappeared() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("vanish.txt");
        fs::write(&f, "secret").unwrap();
        fs::remove_file(&f).unwrap();
        assert!(shred_file(&f).is_ok());
    }

    #[test]
    fn test_shred_skips_marker() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[(TARGET, ""), ("data.txt", "secret")]);
        shred_dir(&sub).unwrap();
        assert!(sub.join(TARGET).exists(), "marker must never be shredded");
        assert!(!sub.join("data.txt").exists());
    }
}
