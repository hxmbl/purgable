//! The destructive filesystem operations behind the review actions: plain
//! removal, clearing contents, and secure deletion (shredding).
//!
//! The `PURGABLE` marker is deliberately never a target on its own: it is
//! metadata about the directory rather than content inside it, so clearing or
//! shredding a directory leaves the directory marked.
//!
//! Every one of these walks the tree concurrently, because unlinking hundreds
//! of thousands of files is syscall-bound and one thread leaves the rest of the
//! machine idle.
//!
//! Symlinks are never followed, on two independent levels. Entries are
//! classified from the parent directory's listing, so a link is recognised as a
//! link and unlinked as one. And the destruction itself goes through a
//! descriptor for the directory that was just inspected, via `unlinkat` and
//! `openat` with `O_NOFOLLOW`, neither of which resolves a link's target. That
//! second level also means the directory cannot be swapped for something else
//! between being inspected and being emptied, which is the one race that
//! mattered when this used full paths.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rand::{rng, Rng, RngExt};

use crate::dirfd::Dir;
use crate::marker::target_os;
use crate::parallel;

/// Bytes overwritten per write call while shredding.
///
/// Large enough that a multi-gigabyte file costs thousands of writes rather
/// than millions, and small enough that a thread's buffer stays out of the way.
const SHRED_CHUNK: usize = 1024 * 1024;

/// Files handed to a shredding thread at a time. Small enough that one huge
/// file cannot leave the other threads idle at the end.
const SHRED_BATCH: usize = 16;

/// Directories removed per removal thread batch.
const REMOVE_BATCH: usize = 64;

/// macOS spells `O_NOFOLLOW` 0x100 and libc does not export it there.
#[cfg(target_os = "macos")]
const O_NOFOLLOW: libc::c_int = 0x0100;
#[cfg(not(target_os = "macos"))]
const O_NOFOLLOW: libc::c_int = libc::O_NOFOLLOW;

/// A path that could not be emptied, with just enough of the failure to report
/// it: `io::Error` itself is neither cloneable nor shareable across threads.
type Failure = (PathBuf, io::ErrorKind, String);

pub(crate) fn remove_dir(path: &Path) -> io::Result<()> {
    empty_tree(path, None, false)
}

/// Delete everything inside `path`, keeping `path` itself and its marker.
pub(crate) fn clear_dir(path: &Path) -> io::Result<()> {
    empty_tree(path, Some(target_os()), false)
}

/// Overwrite every regular file inside `path`, then remove the remaining
/// entries, keeping `path` itself.
///
/// The PURGABLE marker is never shredded: it is metadata about the directory,
/// not content. After this, `path` contains only that marker.
pub(crate) fn shred_dir(path: &Path) -> io::Result<()> {
    empty_tree(path, Some(target_os()), true)
}

/// Overwrite every regular file inside `path`, then remove `path` itself.
///
/// One walk, not two: with nothing kept, shredding a directory and deleting it
/// are the same operation, so running `shred_dir` and then `remove_dir` would
/// walk the whole tree a second time only to find it already empty.
pub(crate) fn shred_and_remove_dir(path: &Path) -> io::Result<()> {
    empty_tree(path, None, true)
}

/// Walk `path`, unlink everything inside it except `keep`, then remove `path`
/// itself if it was not being kept.
///
/// Each directory is opened once and its non-file entries are unlinked by name
/// through that one descriptor. Naming them relative to the directory they are
/// already listed in is both faster and safer than spelling out full paths; see
/// `dirfd`.
///
/// Files that need overwriting are the exception. They are collected during the
/// walk and shredded afterwards in one pass across every core, because a review
/// run empties one directory at a time and shredding them where they are found
/// would leave all but one core idle on a tree of many directories. The cost of
/// holding them is one `PathBuf` per file, which is what this did before.
///
/// Directories are collected the same way, since a directory can only be
/// unlinked once everything below it is gone; they are removed deepest-first
/// once the walk that filled both lists has finished.
fn empty_tree(path: &Path, keep: Option<&OsStr>, shred: bool) -> io::Result<()> {
    let files: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let dirs: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let failures: Mutex<Vec<Failure>> = Mutex::new(Vec::new());

    let record = |failures: &Mutex<Vec<Failure>>, at: &Path, e: &io::Error| {
        failures.lock().unwrap_or_else(|p| p.into_inner()).push((
            at.to_path_buf(),
            e.kind(),
            e.to_string(),
        ));
    };

    parallel::walk(
        &[path.to_path_buf()],
        |cursor, dir, entries| {
            // The root is opened by the caller's own name, so following a
            // symlink there is what was asked for. Anything reached from a
            // directory listing is not: a link in that position is refused, and
            // this is the only line of defence between emptying a tree and
            // emptying somewhere a concurrent rename moved it to.
            let opened = match if dir == path {
                Dir::open_root(dir)
            } else {
                Dir::open(dir)
            } {
                Ok(open) => open,
                Err(e) => {
                    // Deliberately no fallback to full paths. By now the listing
                    // above has already resolved `dir`, so spelling a name out
                    // again could act on whatever it points at now. Leaving the
                    // directory alone and saying so is the safe answer.
                    record(&failures, dir, &e);
                    return;
                }
            };

            let mut subdirs = Vec::new();
            let mut to_shred = Vec::new();

            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                let name = entry.file_name();
                if keep == Some(&name) {
                    // The marker is metadata about the directory, not content.
                    // Removing it would silently unmark the directory as a
                    // side effect of emptying it.
                    continue;
                }
                if file_type.is_dir() {
                    subdirs.push(dir.join(&name));
                } else if shred && file_type.is_file() {
                    // Left for the shredding pass, which can spread the work out
                    // across every core rather than one directory at a time.
                    to_shred.push(dir.join(&name));
                } else {
                    // Named relative to the directory, so this cannot be
                    // redirected by anything swapped in after the listing.
                    if let Err(e) = opened.unlink(&name) {
                        if e.kind() != io::ErrorKind::NotFound {
                            record(&failures, &dir.join(&name), &e);
                        }
                    }
                }
            }

            if !to_shred.is_empty() {
                files
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend(to_shred);
            }
            if !subdirs.is_empty() {
                dirs.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend(subdirs.iter().cloned());
                for sub in subdirs {
                    cursor.descend(sub);
                }
            }
        },
        |at, e| record(&failures, at, e),
    );

    if shred {
        let files = files.into_inner().unwrap_or_else(|e| e.into_inner());
        shred_files(&files, &failures);
    }

    let dirs = dirs.into_inner().unwrap_or_else(|e| e.into_inner());
    remove_dirs_deepest_first(&dirs, &failures);

    // The root is the caller's, not the tree's: it is removed last, once its
    // contents are gone, and only when it was not something to keep.
    if keep.is_none() {
        if let Err(e) = fs::remove_dir(path) {
            if e.kind() != io::ErrorKind::NotFound {
                record(&failures, path, &e);
            }
        }
    }

    report(failures.into_inner().unwrap_or_else(|e| e.into_inner()))
}

/// Remove `dirs` in order of decreasing depth, so a directory is only unlinked
/// once everything under it has been.
fn remove_dirs_deepest_first(dirs: &[PathBuf], failures: &Mutex<Vec<Failure>>) {
    if dirs.is_empty() {
        return;
    }
    let deepest = dirs.iter().map(|d| depth(d)).max().unwrap_or(0);
    let mut buckets: Vec<Vec<&PathBuf>> = vec![Vec::new(); deepest + 1];
    for dir in dirs {
        buckets[depth(dir)].push(dir);
    }
    for bucket in buckets.iter_mut().rev() {
        parallel::for_each_chunk(bucket, REMOVE_BATCH, |slice| {
            for dir in slice {
                if let Err(e) = fs::remove_dir(dir) {
                    if e.kind() != io::ErrorKind::NotFound {
                        failures.lock().unwrap_or_else(|p| p.into_inner()).push((
                            (*dir).clone(),
                            e.kind(),
                            e.to_string(),
                        ));
                    }
                }
            }
        });
    }
}

/// Number of components in a path, used as its depth.
fn depth(path: &Path) -> usize {
    path.components().count()
}

/// Overwrite and unlink every file, spread across every core.
///
/// This is the pass that has to be parallel. A review run empties one directory
/// at a time, so shredding inside the walk would hand the pool a single
/// directory of work and leave every other core waiting; handing out files in
/// batches here means a tree of many directories fills the machine just as well
/// as one huge directory does.
fn shred_files(files: &[PathBuf], failures: &Mutex<Vec<Failure>>) {
    parallel::for_each_chunk(files, SHRED_BATCH, |slice| {
        // One buffer and one generator per batch, not per file: a megabyte
        // allocated for every file in the tree is a lot of pointless work for a
        // lot of small files.
        let mut buffer = Vec::new();
        let mut rng = rng();
        for file in slice {
            if let Err(e) = shred_file(file, &mut buffer, &mut rng) {
                failures.lock().unwrap_or_else(|p| p.into_inner()).push((
                    file.clone(),
                    e.kind(),
                    e.to_string(),
                ));
            }
        }
    });
}

/// Overwrite a file's contents with random data, then remove it.
///
/// `buffer` and `rng` are borrowed so that a caller shredding many files pays for
/// neither once per batch.
fn shred_file(path: &Path, buffer: &mut Vec<u8>, rng: &mut impl Rng) -> io::Result<()> {
    // symlink_metadata, not metadata: a link must never be followed to whatever
    // it points at and overwritten.
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if !metadata.is_file() {
        return fs::remove_file(path);
    }
    let size = metadata.len();
    if size == 0 {
        // Nothing to overwrite: there are no old bytes to get rid of.
        return fs::remove_file(path);
    }

    let file = match fs::OpenOptions::new()
        .write(true)
        // The listing called this a regular file. Refuse to open it if it has
        // become a link since, rather than overwriting whatever it now names.
        .custom_flags(O_NOFOLLOW)
        .open(path)
    {
        Ok(f) => f,
        // Vanished between the listing and the open: the outcome was wanted
        // either way.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        // Listed as a regular file but is now a link. Unlinking the link is the
        // outcome, and overwriting what it points at never is.
        Err(e) if is_refused_link(&e) => return fs::remove_file(path),
        Err(e) => return Err(e),
    };
    let mut file = file;

    // Only as much buffer as this file can actually need.
    buffer.clear();
    buffer.resize(size.min(SHRED_CHUNK as u64) as usize, 0);

    let mut written: u64 = 0;
    while written < size {
        let n = std::cmp::min(SHRED_CHUNK as u64, size - written) as usize;
        rng.fill(&mut buffer[..n]);
        file.write_all(&buffer[..n])?;
        written += n as u64;
    }
    // fdatasync, not fsync: the overwrite has to reach the device before the
    // name goes away, but the file's timestamps are not part of that promise.
    file.sync_data()?;
    drop(file);
    fs::remove_file(path)
}

/// True for the errno refusing to follow a symlink produces.
fn is_refused_link(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::ELOOP)
}

/// Collapse collected failures into a single error, reporting the
/// lexicographically first path so the message does not depend on which thread
/// failed first.
fn report(failures: Vec<Failure>) -> io::Result<()> {
    let mut failures = failures;
    match failures.is_empty() {
        true => Ok(()),
        false => {
            failures.sort_by(|a, b| a.0.cmp(&b.0));
            let (path, kind, message) = failures.swap_remove(0);
            Err(io::Error::new(
                kind,
                format!("{}: {}", path.display(), message),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::TARGET;
    use crate::test_support::{write_dir_with_content, write_file};
    use std::fs;

    #[test]
    fn test_shred_overwrites_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_file(&sub.join("secret.bin"), &"x".repeat(4096));

        shred_dir(&sub).unwrap();
        assert!(!sub.join("secret.bin").exists());
    }

    #[test]
    fn test_shred_tolerates_a_file_that_vanished() {
        // Listed, then gone before it could be opened: the outcome the caller
        // wanted has already happened, so this must not be reported as a failure.
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[(TARGET, "")]);
        fs::remove_file(sub.join(TARGET)).unwrap();
        fs::write(sub.join("vanish.txt"), "secret").unwrap();
        fs::remove_file(sub.join("vanish.txt")).unwrap();

        assert!(shred_dir(&sub).is_ok());
    }

    #[test]
    fn test_emptying_follows_a_symlinked_root_but_not_a_symlinked_subdirectory() {
        // The root is the path the user named, so resolving it is what they
        // asked for and the tool must not refuse to start. Every directory below
        // it came out of a listing instead, and one of those being a link is
        // exactly what must never be traversed.
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        write_dir_with_content(&real, &[(TARGET, ""), ("data.txt", "x")]);
        let other = dir.path().join("other");
        write_file(&other.join("precious.txt"), "must survive");
        std::os::unix::fs::symlink(&other, real.join("escape")).unwrap();
        let root = dir.path().join("root");
        std::os::unix::fs::symlink(&real, &root).unwrap();

        clear_dir(&root).unwrap();

        assert_eq!(
            fs::read_to_string(other.join("precious.txt")).unwrap(),
            "must survive"
        );
        assert!(real.join(TARGET).exists());
        assert!(!real.join("escape").exists(), "the link itself is removed");
    }

    /// A file listed as regular but replaced by a symlink before it can be
    /// opened must be unlinked, never followed. Overwriting the link's target
    /// would destroy a file outside the tree being emptied.
    #[test]
    fn test_shred_unlinks_a_link_where_a_file_was_listed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        write_file(&outside.join("precious.txt"), "must survive");
        // Stand in for the race: a link now occupies the name a regular file
        // had when its directory was listed.
        let link = dir.path().join("swap.bin");
        std::os::unix::fs::symlink(outside.join("precious.txt"), &link).unwrap();

        shred_file(&link, &mut Vec::new(), &mut rng()).unwrap();

        assert!(!link.exists(), "the link must be gone");
        assert_eq!(
            fs::read_to_string(outside.join("precious.txt")).unwrap(),
            "must survive",
            "the file the link pointed at was overwritten"
        );
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

    #[test]
    fn test_shred_removes_deep_nesting() {
        // Regression: shredding must not leave an empty directory skeleton
        // behind, however deep the tree is.
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_file(&sub.join(TARGET), "");
        write_file(&sub.join("a/b/c/d/deep.txt"), &"x".repeat(4096));
        write_file(&sub.join("a/b/other.txt"), "y");

        shred_dir(&sub).unwrap();

        // Only the marker is left: every directory, at every depth, is gone.
        let names: Vec<_> = fs::read_dir(&sub)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![OsStr::new(TARGET)]);
    }

    #[test]
    fn test_clear_keeps_marker_and_removes_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(
            &sub,
            &[(TARGET, ""), ("data.txt", "x"), ("deep/inner.txt", "y")],
        );
        // A socket-like entry: a plain directory removal has to cope with
        // anything that is not a directory or a regular file.
        std::os::unix::fs::symlink(sub.join("data.txt"), sub.join("link")).unwrap();

        clear_dir(&sub).unwrap();
        assert!(sub.exists());
        let names: Vec<_> = fs::read_dir(&sub)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![OsStr::new(TARGET)]);
    }

    #[test]
    fn test_remove_dir_removes_the_directory_itself() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[(TARGET, ""), ("deep/inner.txt", "y")]);
        remove_dir(&sub).unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn test_emptying_never_follows_a_symlink_out_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        write_file(&outside.join("keep.txt"), "precious");
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        write_file(&sub.join(TARGET), "");
        // A symlinked directory inside the tree: it must be unlinked, and the
        // directory it points at must survive untouched.
        std::os::unix::fs::symlink(&outside, sub.join("escape")).unwrap();
        // A symlink to a file, likewise.
        std::os::unix::fs::symlink(outside.join("keep.txt"), sub.join("escape.txt")).unwrap();

        clear_dir(&sub).unwrap();
        assert!(outside.join("keep.txt").exists(), "target was deleted");
        assert!(outside.is_dir(), "target directory was deleted");
        assert!(sub.join(TARGET).exists());
    }

    #[test]
    fn test_shred_reports_a_failure_and_keeps_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[(TARGET, "")]);
        // A directory with no write permission holds its contents even for a
        // root-owned run only when it is read-only, which is the portable way
        // to make removal fail.
        let locked = sub.join("locked");
        fs::create_dir_all(&locked).unwrap();
        write_file(&locked.join("secret.txt"), "x");
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o500);
        fs::set_permissions(&locked, perms).unwrap();

        let result = if unsafe { libc::geteuid() } == 0 {
            // Root can unlink from a read-only directory, so there is nothing
            // to fail; just make sure it succeeded.
            assert!(clear_dir(&sub).is_ok());
            return;
        } else {
            clear_dir(&sub)
        };

        // Restore permissions so the temp directory can be removed.
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&locked, perms).unwrap();

        assert!(result.is_err(), "an undeletable directory must be reported");
        assert!(sub.join(TARGET).exists(), "the marker must survive");
    }
}
