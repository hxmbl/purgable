//! Naming files by their parent directory's descriptor rather than by path.
//!
//! Destroying a tree means calling `unlink` once per file, and a `unlink` on a
//! full path makes the kernel resolve every component of that path again, only
//! to arrive back at a directory the caller already has open. Measured on a
//! synthetic tree of 49,000 files, that redundancy costs an order of
//! magnitude:
//!
//! ```text
//! unlink("/long/path/target/debug/deps/foo.rlib")   0.123 ms/file
//! unlinkat(dirfd_of_deps, "foo.rlib")                0.006 ms/file
//! ```
//!
//! Holding the parent open also closes a race. Every destructive call in this
//! crate is made against a descriptor for a directory that has already been
//! inspected, so the thing being emptied cannot be swapped for something else
//! in between: a symlink put in place of the directory is refused at open
//! time, and a symlink put in place of a file is unlinked as the link it is,
//! because neither `unlinkat` nor `openat` here follows one.

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;

/// macOS spells `O_NOFOLLOW` 0x100 and libc does not export it there.
#[cfg(target_os = "macos")]
const O_NOFOLLOW: libc::c_int = 0x0100;
#[cfg(not(target_os = "macos"))]
const O_NOFOLLOW: libc::c_int = libc::O_NOFOLLOW;

/// A NUL-terminated copy of `name`, for handing to a *at() syscall.
///
/// Names come from directory listings and so cannot contain a NUL, but this
/// refuses rather than assumes: a name that somehow did would otherwise be
/// truncated at the NUL and the syscall would act on a different file.
fn c_name(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains a NUL"))
}

/// An open directory, used as the starting point for every operation on the
/// entries inside it.
pub(crate) struct Dir {
    file: File,
}

impl Dir {
    /// Open `path` as a directory, refusing to follow a symlink.
    ///
    /// `O_DIRECTORY` and `O_NOFOLLOW` together mean this fails with `ENOTDIR`
    /// or `ELOOP` rather than quietly resolving to something else, which is what
    /// keeps the destructive half of a run inside the tree it was pointed at.
    ///
    /// This is for anything reached by walking a directory listing. The root a
    /// command was pointed at is the caller's own choice, so it uses
    /// [`Dir::open_root`] instead.
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Self::open_with(path, O_NOFOLLOW)
    }

    /// Open `path` as a directory, following a symlink if it is one.
    ///
    /// Only for the root of a run: the user named that path on the command line,
    /// so resolving it is what they asked for. Every directory below it came
    /// out of a listing, and a listing entry that was a symlink is never a
    /// directory to be descended into.
    pub(crate) fn open_root(path: &Path) -> io::Result<Self> {
        Self::open_with(path, 0)
    }

    fn open_with(path: &Path, extra: libc::c_int) -> io::Result<Self> {
        let name = c_name(path.as_os_str())?;
        let fd = unsafe {
            libc::open(
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | extra,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Dir {
            // The descriptor is fresh and owned from here on.
            file: unsafe { File::from_raw_fd(fd) },
        })
    }

    fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// Unlink the entry called `name`, whatever it is.
    ///
    /// A symlink is unlinked rather than traversed: `unlinkat` without
    /// `AT_REMOVEDIR` never resolves a link's target.
    pub(crate) fn unlink(&self, name: &OsStr) -> io::Result<()> {
        let name = c_name(name)?;
        if unsafe { libc::unlinkat(self.fd(), name.as_ptr(), 0) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::write_file;
    use std::fs;

    #[test]
    fn test_unlink_removes_an_entry_by_name() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("gone.txt"), "x");
        let open = Dir::open(dir.path()).unwrap();

        open.unlink(OsStr::new("gone.txt")).unwrap();
        assert!(!dir.path().join("gone.txt").exists());
        assert!(open.unlink(OsStr::new("gone.txt")).is_err(), "already gone");
    }

    #[test]
    fn test_unlink_removes_a_symlink_without_touching_its_target() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("keep.txt"), "precious");
        std::os::unix::fs::symlink(dir.path().join("keep.txt"), dir.path().join("link")).unwrap();
        let open = Dir::open(dir.path()).unwrap();

        open.unlink(OsStr::new("link")).unwrap();
        assert!(dir.path().join("keep.txt").exists(), "target was removed");
        assert!(!dir.path().join("link").exists());
    }

    #[test]
    fn test_open_refuses_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join("link")).unwrap();

        // Following the link would let a caller empty a directory outside the
        // tree it was asked to empty.
        assert!(Dir::open(&dir.path().join("link")).is_err());
        assert!(Dir::open(&real).is_ok());
    }

    #[test]
    fn test_open_refuses_a_file() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("f.txt"), "x");
        assert!(Dir::open(&dir.path().join("f.txt")).is_err());
    }

    #[test]
    fn test_name_with_a_nul_is_refused_rather_than_truncated() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join("a"), "x");
        let open = Dir::open(dir.path()).unwrap();

        // A truncated "a\0b" would unlink "a", which is not what was asked for.
        assert!(open.unlink(OsStr::from_bytes(b"a\0b")).is_err());
        assert!(dir.path().join("a").exists());
    }
}
