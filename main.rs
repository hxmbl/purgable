use rand::rng;
use rand::RngExt;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

pub const TARGET: &str = "PURGABLE";
const SHRED_BUF_SIZE: usize = 64 * 1024;

pub struct Stats {
    pub found: u32,
    pub deleted: u32,
    pub shredded: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Delete,
    Shred,
    Skip,
    Exit,
}

pub struct ActionAll {
    pub action: Action,
    pub all: bool,
}

pub fn validate_root(root: &str) -> io::Result<fs::Metadata> {
    let info = fs::metadata(root)?;
    if !info.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{:?} is not a directory", root),
        ));
    }
    Ok(info)
}

pub fn find(root: &str, warn: &mut impl io::Write) -> io::Result<Vec<PathBuf>> {
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

fn is_regular_file(entry: &DirEntry) -> bool {
    entry.file_type().is_file()
}

pub fn purge(
    root: &str,
    in_reader: &mut impl io::BufRead,
    out: &mut impl io::Write,
    warn: &mut impl io::Write,
) -> io::Result<Stats> {
    validate_root(root)?;

    let matches = find(root, warn)?;
    let mut stats = Stats {
        found: matches.len() as u32,
        deleted: 0,
        shredded: 0,
        skipped: 0,
    };

    if matches.is_empty() {
        writeln!(out, "No PURGABLE directories found.")?;
        return Ok(stats);
    }

    let mut default_action: Option<ActionAll> = None;

    for dir in &matches {
        let dir_str = dir.to_string_lossy().to_string();
        let mut all = false;
        let action = if let Some(da) = &default_action {
            da.action
        } else {
            write!(out, "Action for {} [d/s/k/e]? ", dir_str)?;
            out.flush()?;
            let mut line = String::new();
            let n = in_reader.read_line(&mut line)?;
            if n == 0 {
                Action::Skip
            } else {
                let ans = line.trim().to_lowercase();
                if ans.is_empty() {
                    Action::Skip
                } else {
                    let (act, al, ok) = parse_action(&ans);
                    if !ok {
                        writeln!(out, "  invalid action {:?}, skipping", ans)?;
                        Action::Skip
                    } else {
                        all = al;
                        act
                    }
                }
            }
        };

        if all {
            default_action = Some(ActionAll { action, all: true });
        }

        match action {
            Action::Delete => match remove_dir(&dir_str) {
                Ok(()) => stats.deleted += 1,
                Err(e) => {
                    writeln!(out, "  failed to delete {}: {}", dir_str, e)?;
                    stats.skipped += 1;
                }
            },
            Action::Shred => match shred_dir(&dir_str) {
                Ok(()) => stats.shredded += 1,
                Err(e) => {
                    writeln!(out, "  failed to shred {}: {}", dir_str, e)?;
                    stats.skipped += 1;
                }
            },
            Action::Skip => {
                stats.skipped += 1;
            }
            Action::Exit => {
                return Ok(stats);
            }
        }
    }

    Ok(stats)
}

pub fn parse_action(s: &str) -> (Action, bool, bool) {
    let mut all = false;
    let s = if let Some(stripped) = s.strip_suffix("-all") {
        all = true;
        stripped
    } else {
        s
    };
    match s {
        "d" => (Action::Delete, all, true),
        "s" => (Action::Shred, all, true),
        "k" => (Action::Skip, all, true),
        "e" => (Action::Exit, all, true),
        _ => (Action::Skip, false, false),
    }
}

fn remove_dir(path: &str) -> io::Result<()> {
    fs::remove_dir_all(path)
}

fn shred_dir(path: &str) -> io::Result<()> {
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
            shred_file(p)?;
        }
    }
    Ok(())
}

fn shred_file(path: &Path) -> io::Result<()> {
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
    use std::os::unix::fs::symlink;
    use std::os::unix::fs::PermissionsExt;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn write_dir_with_content(dir: &Path, files: &[(&str, &str)]) {
        fs::create_dir_all(dir).unwrap();
        for (name, content) in files {
            fs::write(dir.join(name), content).unwrap();
        }
    }

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
        for m in &matches {
            assert!(m.join("PURGABLE").exists());
        }
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
    fn test_delete_action_removes_containing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(
            &sub,
            &[("PURGABLE", ""), ("data.txt", "secret"), ("other", "file")],
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "d\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 1);
        assert_eq!(stats.deleted, 1);
        assert_eq!(stats.shredded, 0);
        assert_eq!(stats.skipped, 0);
        assert!(!sub.exists());
        assert!(!sub.join("PURGABLE").exists());
    }

    #[test]
    fn test_skip_action_leaves_everything_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[("PURGABLE", ""), ("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "k\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 1);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.shredded, 0);
        assert_eq!(stats.skipped, 1);
        assert!(sub.exists());
        assert!(sub.join("PURGABLE").exists());
        assert!(sub.join("data.txt").exists());
    }

    #[test]
    fn test_shred_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(
            &sub,
            &[("PURGABLE", ""), ("data.txt", "secret content to shred")],
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "s\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 1);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.shredded, 1);
        assert_eq!(stats.skipped, 0);
        assert!(sub.exists());
        assert!(sub.join("PURGABLE").exists());
        assert!(!sub.join("data.txt").exists());
    }

    #[test]
    fn test_delete_all() {
        let dir = tempfile::tempdir().unwrap();
        let sub1 = dir.path().join("sub1");
        let sub2 = dir.path().join("sub2");
        write_dir_with_content(&sub1, &[("PURGABLE", ""), ("a", "1")]);
        write_dir_with_content(&sub2, &[("PURGABLE", ""), ("b", "2")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "d-ALL\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 2);
        assert_eq!(stats.deleted, 2);
        assert_eq!(stats.shredded, 0);
        assert_eq!(stats.skipped, 0);
        assert!(!sub1.exists());
        assert!(!sub2.exists());
    }

    #[test]
    fn test_shred_all() {
        let dir = tempfile::tempdir().unwrap();
        let sub1 = dir.path().join("sub1");
        let sub2 = dir.path().join("sub2");
        write_dir_with_content(&sub1, &[("PURGABLE", ""), ("a", "1")]);
        write_dir_with_content(&sub2, &[("PURGABLE", ""), ("b", "2")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "s-ALL\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 2);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.shredded, 2);
        assert_eq!(stats.skipped, 0);
        assert!(sub1.exists());
        assert!(sub2.exists());
        assert!(sub1.join("PURGABLE").exists());
        assert!(sub2.join("PURGABLE").exists());
        assert!(!sub1.join("a").exists());
        assert!(!sub2.join("b").exists());
    }

    #[test]
    fn test_skip_all() {
        let dir = tempfile::tempdir().unwrap();
        let sub1 = dir.path().join("sub1");
        let sub2 = dir.path().join("sub2");
        write_dir_with_content(&sub1, &[("PURGABLE", ""), ("a", "1")]);
        write_dir_with_content(&sub2, &[("PURGABLE", ""), ("b", "2")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "k-ALL\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 2);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.shredded, 0);
        assert_eq!(stats.skipped, 2);
        assert!(sub1.exists());
        assert!(sub2.exists());
    }

    #[test]
    fn test_exit_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let sub1 = dir.path().join("sub1");
        let sub2 = dir.path().join("sub2");
        write_dir_with_content(&sub1, &[("PURGABLE", ""), ("a", "1")]);
        write_dir_with_content(&sub2, &[("PURGABLE", ""), ("b", "2")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "e\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 2);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.shredded, 0);
        assert_eq!(stats.skipped, 0);
        assert!(sub1.exists());
        assert!(sub2.exists());
    }

    #[test]
    #[cfg(unix)]
    fn test_symlinks_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("PURGABLE"), "x");
        let sub = root.join("sub");
        write_dir_with_content(&sub, &[("PURGABLE", "")]);
        symlink(&sub, root.join("link")).unwrap();

        let matches = find(root.to_str().unwrap(), &mut io::stderr()).unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_filesystem_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("PURGABLE"), "x");
        let no_read = root.join("noread");
        fs::create_dir(&no_read).unwrap();
        write_file(&no_read.join("PURGABLE"), "x");
        fs::set_permissions(&no_read, fs::Permissions::from_mode(0o000)).unwrap();

        let matches = find(root.to_str().unwrap(), &mut io::stderr()).unwrap();
        let found = matches.iter().any(|m| m == root);
        assert!(found);

        fs::set_permissions(&no_read, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn test_marker_never_independently_targeted() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[("PURGABLE", ""), ("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &mut "d\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert!(!sub.exists());

        let sub2 = dir.path().join("sub2");
        write_dir_with_content(&sub2, &[("PURGABLE", ""), ("data.txt", "secret")]);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &mut "s\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert!(sub2.join("PURGABLE").exists());
        assert!(!sub2.join("data.txt").exists());
    }

    #[test]
    fn test_default_skip_on_empty_input() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[("PURGABLE", ""), ("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.shredded, 0);
        assert_eq!(stats.skipped, 1);
        assert!(sub.exists());
    }

    #[test]
    fn test_multiple_choices() {
        let dir = tempfile::tempdir().unwrap();
        let sub1 = dir.path().join("sub1");
        let sub2 = dir.path().join("sub2");
        let sub3 = dir.path().join("sub3");
        write_dir_with_content(&sub1, &[("PURGABLE", ""), ("a", "1")]);
        write_dir_with_content(&sub2, &[("PURGABLE", ""), ("b", "2")]);
        write_dir_with_content(&sub3, &[("PURGABLE", ""), ("c", "3")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &mut "d\ns\nk\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 3);
        assert_eq!(stats.deleted, 1);
        assert_eq!(stats.shredded, 1);
        assert_eq!(stats.skipped, 1);
        assert!(!sub1.exists());
        assert!(sub2.exists());
        assert!(!sub2.join("b").exists());
        assert!(sub3.exists());
    }

    #[test]
    fn test_missing_root_directory() {
        let dir = tempfile::tempdir().unwrap();
        let result = purge(
            &dir.path().join("does-not-exist").to_str().unwrap(),
            &mut "".as_bytes(),
            &mut Vec::new(),
            &mut io::stderr(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_action() {
        let tests = [
            ("d", Action::Delete, false, true),
            ("s", Action::Shred, false, true),
            ("k", Action::Skip, false, true),
            ("e", Action::Exit, false, true),
            ("d-all", Action::Delete, true, true),
            ("s-all", Action::Shred, true, true),
            ("k-all", Action::Skip, true, true),
            ("e-all", Action::Exit, true, true),
            ("x", Action::Skip, false, false),
            ("", Action::Skip, false, false),
        ];
        for (input, want_action, want_all, want_ok) in &tests {
            let (got_action, got_all, got_ok) = parse_action(input);
            assert_eq!(got_action, *want_action);
            assert_eq!(got_all, *want_all);
            assert_eq!(got_ok, *want_ok);
        }
    }

    #[test]
    fn test_validate_root() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(validate_root(missing.to_str().unwrap()).is_err());

        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(validate_root(file.to_str().unwrap()).is_err());

        let info = validate_root(dir.path().to_str().unwrap()).unwrap();
        assert!(info.is_dir());
    }

    #[test]
    fn test_shred_file_disappeared() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("vanish.txt");
        fs::write(&f, "secret").unwrap();
        fs::remove_file(&f).unwrap();
        assert!(shred_file(&f).is_ok());
    }
}

const VERSION: &str = "v1.5";

fn usage() {
    eprint!(
        r#"purgable - find directories marked with PURGABLE and act on them

Usage:
  purgable <directory>
  purgable --help | -h
  purgable --version | -v

Recursively scans <directory> for regular files named exactly "PURGABLE"
(case-sensitive, no extension). Each such file marks its containing directory
as purgable. For each marked directory, the tool presents an action prompt.

The PURGABLE marker itself is NEVER independently deleted, modified, or
shredded. It is removed only as a consequence of its containing directory
being deleted or shredded.

Actions:
  d        Delete the containing directory and everything in it.
  s        Shred (secure-delete) the contents of the containing directory.
            Note: shredding cannot guarantee physical destruction on SSDs,
            flash storage, or filesystems with copy-on-write/snapshots.
  k        Skip this directory and continue scanning.
  e        Exit immediately.

  d-ALL    Delete this and all subsequent PURGABLE directories without prompting.
  s-ALL    Shred this and all subsequent PURGABLE directories without prompting.
  k-ALL    Skip this and all subsequent PURGABLE directories without prompting.
  e-ALL    Exit immediately (equivalent to e).

Options:
  -h, --help      Show this help and exit.
  -v, --version   Show version and exit.

Exit codes:
  0  completed (regardless of actions taken)
  1  the root directory does not exist or cannot be accessed
  2  invalid usage
"#
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for a in &args {
        match a.as_str() {
            "--help" | "-h" => {
                usage();
                return;
            }
            "--version" | "-v" => {
                println!("purgable {}", VERSION);
                return;
            }
            _ => {}
        }
    }

    if args.len() != 1 {
        usage();
        std::process::exit(2);
    }

    let root = &args[0];
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    let stats = match purge(root, &mut stdin_lock, &mut stdout, &mut stderr) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(stderr, "error: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = writeln!(
        stdout,
        "\nDone. Found {}, deleted {}, shredded {}, skipped {}.",
        stats.found, stats.deleted, stats.shredded, stats.skipped
    ) {
        let _ = writeln!(stderr, "error: failed to write summary: {}", e);
        std::process::exit(1);
    }
}
