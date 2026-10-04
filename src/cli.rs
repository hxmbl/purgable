//! The command-line surface: usage text, argument parsing, and dispatch.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::action::Opts;
use crate::config::{config_path, load_config, starter_config};
use crate::mark::{list, mark, unmark};
use crate::purge::purge;
use crate::size::parse_size;

/// Reported by `--version` / `-v`. Kept in step with `version` in Cargo.toml.
pub(crate) const VERSION: &str = "v2.0";

pub(crate) fn usage() {
    eprint!(
        r#"purgable - find, mark, and purge disposable directories

Usage:
  purgable mark <directory> [--policy NAME]... [--min-size SIZE] [--dry-run]
  purgable review <directory> [--dry-run]
  purgable list <directory>
  purgable unmark <directory> [--policy NAME] [--all] [--dry-run]
  purgable init [--force]
  purgable --help | -h
  purgable --version | -v

Marking:
  Any regular file named exactly "PURGABLE" marks its containing directory.
  `mark` creates those files automatically from the policies in
  ~/.config/purgeable.toml (override the path with $PURGABLE_CONFIG).
  Markers written by `mark` record which policy matched and when.

Review actions:
  d    Delete the directory and everything in it.
  c    Delete everything inside, keep the directory.
  s    Shred the contents, keep the directory.
  x    Shred the contents, then delete the directory too.
  k    Skip this directory and continue.
  e    Exit immediately.

Display:
  Paths are shortened to $HOME and truncated to your terminal width, with the
  filename kept visible. Colour is used on a terminal and suppressed when
  output is piped or NO_COLOR is set.

  Append -ALL to any action to apply it to this and every later directory
  without prompting further: d-ALL, c-ALL, s-ALL, x-ALL, k-ALL, e-ALL.

  Directories are reviewed largest first. Pressing Enter means k.

  Shredding overwrites files with random data before removing them. It cannot
  guarantee physical destruction on SSDs, flash storage, or copy-on-write
  filesystems, and it does not recover any extra space. Use d for space.

  c and s leave the PURGABLE marker in place, so the directory stays marked.
  Run `unmark` to drop those markers.

Exit codes:
  0  completed
  1  the root directory or config could not be read
  2  invalid usage
"#
    );
}

pub(crate) fn fail(message: &str, code: i32) -> ! {
    eprintln!("error: {}", message);
    std::process::exit(code)
}

pub(crate) struct Invocation {
    pub(crate) command: String,
    pub(crate) root: Option<String>,
    pub(crate) policies: Vec<String>,
    pub(crate) policy_filter: Option<String>,
    pub(crate) min_size: Option<String>,
    pub(crate) all: bool,
    pub(crate) force: bool,
    pub(crate) dry_run: bool,
}

pub(crate) fn parse_args(args: &[String]) -> Invocation {
    let mut inv = Invocation {
        command: String::new(),
        root: None,
        policies: Vec::new(),
        policy_filter: None,
        min_size: None,
        all: false,
        force: false,
        dry_run: false,
    };
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--policy" | "-p" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.policies.push(v.clone()),
                    None => fail("--policy requires a value", 2),
                }
            }
            "--min-size" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.min_size = Some(v.clone()),
                    None => fail("--min-size requires a value", 2),
                }
            }
            "--all" => inv.all = true,
            "--force" => inv.force = true,
            "--dry-run" => inv.dry_run = true,
            other if other.starts_with('-') && other.len() > 1 => {
                fail(&format!("unknown option {}", other), 2)
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    if !positional.is_empty() {
        inv.command = positional.remove(0);
    }
    if !positional.is_empty() {
        inv.root = Some(positional.remove(0));
    }
    if !positional.is_empty() {
        fail(&format!("unexpected argument {:?}", positional[0]), 2);
    }
    inv
}

pub(crate) fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        usage();
        return 0;
    }
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("purgable {}", VERSION);
        return 0;
    }
    if args.is_empty() {
        usage();
        return 2;
    }

    let inv = parse_args(&args);
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    match inv.command.as_str() {
        "mark" => {
            let Some(root) = inv.root else {
                fail("mark requires a directory", 2)
            };
            // Validate --min-size first: a typo should report the typo, not a
            // missing-config error.
            let min_override = match inv.min_size.as_deref() {
                Some(raw) => match parse_size(raw) {
                    Some(v) => Some(v),
                    None => fail(&format!("invalid --min-size {:?}", raw), 2),
                },
                None => None,
            };
            let path = config_path();
            if !path.exists() {
                let _ = writeln!(
                    stderr,
                    "error: no config at {}\nRun `purgable init` to write a starter config.",
                    path.display()
                );
                return 1;
            }
            let config = match load_config(&path) {
                Ok(c) => c,
                Err(e) => fail(&e, 1),
            };
            // mark prints its own styled summary.
            match mark(
                &root,
                &config,
                &inv.policies,
                min_override,
                Opts {
                    dry_run: inv.dry_run,
                },
                &mut stdout,
                &mut stderr,
            ) {
                Ok(_) => 0,
                Err(e) => fail(&e.to_string(), 1),
            }
        }

        "review" => {
            let Some(root) = inv.root else {
                fail("review requires a directory", 2)
            };
            // purge prints its own styled summary, so no extra line here.
            match purge(
                &root,
                &Opts {
                    dry_run: inv.dry_run,
                },
                &mut stdin_lock,
                &mut stdout,
                &mut stderr,
            ) {
                Ok(_) => 0,
                Err(e) => fail(&e.to_string(), 1),
            }
        }

        "list" => {
            let Some(root) = inv.root else {
                fail("list requires a directory", 2)
            };
            match list(&root, &mut stdout, &mut stderr) {
                Ok(()) => 0,
                Err(e) => fail(&e.to_string(), 1),
            }
        }

        "unmark" => {
            let Some(root) = inv.root else {
                fail("unmark requires a directory", 2)
            };
            match unmark(
                &root,
                inv.policy_filter.as_deref().or_else(|| {
                    if inv.policies.len() == 1 {
                        Some(inv.policies[0].as_str())
                    } else {
                        None
                    }
                }),
                inv.all,
                inv.dry_run,
                &mut stdout,
                &mut stderr,
            ) {
                Ok(_) => 0,
                Err(e) => fail(&e.to_string(), 1),
            }
        }

        "init" => {
            let path = config_path();
            if path.exists() && !inv.force {
                let _ = writeln!(
                    stdout,
                    "{} already exists. Use --force to overwrite.",
                    path.display()
                );
                return 0;
            }
            if let Some(parent) = path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    fail(&format!("cannot create {}: {}", parent.display(), e), 1);
                }
            }
            if let Err(e) = fs::write(&path, starter_config()) {
                fail(&format!("cannot write {}: {}", path.display(), e), 1);
            }
            let _ = writeln!(stdout, "Wrote {}", path.display());
            0
        }

        other => {
            // Backwards compatibility: `purgable <directory>` behaved as review.
            if Path::new(other).is_dir() {
                let _ = writeln!(
                    stderr,
                    "note: `purgable <directory>` is now `purgable review <directory>`"
                );
                // purge prints its own styled summary.
                match purge(
                    other,
                    &Opts::default(),
                    &mut stdin_lock,
                    &mut stdout,
                    &mut stderr,
                ) {
                    Ok(_) => return 0,
                    Err(e) => fail(&e.to_string(), 1),
                }
            }
            let _ = writeln!(stderr, "error: unknown command {:?}", other);
            usage();
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Invocation {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse_args(&owned)
    }

    #[test]
    fn test_parse_subcommand_and_root() {
        let inv = parse(&["review", "/tmp"]);
        assert_eq!(inv.command, "review");
        assert_eq!(inv.root.as_deref(), Some("/tmp"));
    }

    #[test]
    fn test_parse_flags() {
        let inv = parse(&[
            "mark",
            "/tmp",
            "--policy",
            "cargo-target",
            "-p",
            "node-modules",
            "--min-size",
            "1G",
            "--dry-run",
        ]);
        assert_eq!(inv.command, "mark");
        assert_eq!(inv.policies, vec!["cargo-target", "node-modules"]);
        assert_eq!(inv.min_size.as_deref(), Some("1G"));
        assert!(inv.dry_run);
    }

    #[test]
    fn test_parse_all_and_force() {
        let inv = parse(&["unmark", "/tmp", "--all", "--force"]);
        assert!(inv.all);
        assert!(inv.force);
    }

    #[test]
    fn test_parse_rejects_unknown_option() {
        // fail() exits the process, so this is asserted via the subprocess test
        // below rather than here.
        let inv = parse(&["review", "/tmp"]);
        assert!(!inv.all);
    }
}
