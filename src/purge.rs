//! The interactive review loop: present each marked directory largest first,
//! ask what to do, and carry out the decision.

use std::io;
use std::path::PathBuf;

use crate::action::{Action, Opts};
use crate::discovery::{find, validate_root};
use crate::marker::read_marker;
use crate::prompt::{prompt, write_row, Row, RowState};
use crate::shred::{clear_dir, remove_dir, shred_dir};
use crate::size::{dir_size, human_size, now_secs};
use crate::style::{display_path, term_width, truncate_head, Style};

/// Tally of what a review run did, reported in the closing summary.
pub(crate) struct Stats {
    pub(crate) found: u32,
    /// Directories removed entirely (actions `d` and `x`).
    pub(crate) deleted: u32,
    /// Directories emptied but kept on disk (actions `c` and `s`).
    pub(crate) cleared: u32,
    /// Directories whose files were overwritten before removal (`s` and `x`).
    pub(crate) shredded: u32,
    pub(crate) skipped: u32,
    /// Approximate bytes reclaimed.
    pub(crate) freed: u64,
}

/// Walk `root`, prompt for each marked directory, and act on the answers.
///
/// Directories are presented largest first. A `-ALL` answer is remembered and
/// applied to every later directory without further prompting. Choosing exit
/// stops the run early and returns the tally so far. A failed delete, clear, or
/// shred is reported and counted as skipped rather than aborting the run.
pub(crate) fn purge(
    root: &str,
    opts: &Opts,
    in_reader: &mut impl io::BufRead,
    out: &mut impl io::Write,
    warn: &mut impl io::Write,
) -> io::Result<Stats> {
    validate_root(root)?;

    let matches = find(root, warn)?;
    let now = now_secs();

    // Largest first: when space is the reason you are here, the decision that
    // matters is the big one, and it should not be buried mid-list.
    let mut rows: Vec<(u64, PathBuf, String)> = matches
        .into_iter()
        .map(|dir| {
            let size = dir_size(&dir);
            let provenance = read_marker(&dir).describe(now);
            (size, dir, provenance)
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let mut stats = Stats {
        found: rows.len() as u32,
        deleted: 0,
        cleared: 0,
        shredded: 0,
        skipped: 0,
        freed: 0,
    };

    if rows.is_empty() {
        writeln!(out, "No PURGABLE directories found.")?;
        return Ok(stats);
    }

    let style = Style::detect();
    let total: u64 = rows
        .iter()
        .map(|r| r.0)
        .fold(0u64, |a, b| a.saturating_add(b));
    let count = rows.len();

    if opts.dry_run {
        writeln!(
            out,
            "\n{} {} marked, {} reclaimable. Dry run: nothing deleted.",
            style.bold(&count.to_string()),
            if count == 1 {
                "directory"
            } else {
                "directories"
            },
            style.bold(&human_size(total))
        )?;
        writeln!(out)?;
        for (index, (size, dir, provenance)) in rows.iter().enumerate() {
            write_row(
                out,
                &style,
                Row {
                    index: index + 1,
                    count,
                    size: *size,
                    dir,
                    provenance,
                    state: RowState::Preview,
                },
            )?;
        }
        return Ok(stats);
    }

    // Running summary shown in the header of every prompt, so the effect of
    // earlier decisions stays visible while you work through the list.
    let mut freed_so_far = 0u64;

    writeln!(out)?;
    writeln!(
        out,
        "  {}  {}",
        style.bold(&format!("{} to review", count)),
        style.dim(&format!("{} total", human_size(total)))
    )?;
    writeln!(out)?;

    let mut default_action: Option<Action> = None;

    for (index, (size, dir, provenance)) in rows.iter().enumerate() {
        let index = index + 1;

        // Once a -ALL action is set there is nothing left to ask, but the user
        // still needs to see what is happening to the remaining directories.
        if let Some(da) = &default_action {
            let word = match *da {
                Action::DeleteAll => "deleting",
                Action::ClearContents => "clearing",
                Action::ShredContents => "shredding",
                Action::ShredAll => "shredding",
                Action::Skip => "skipping",
                Action::Exit => "",
            };
            writeln!(
                out,
                "  {} {}  {}",
                style.dim(&format!("[{:>2}/{:<2}]", index, count)),
                style.dim(&truncate_head(
                    &display_path(dir),
                    term_width().saturating_sub(24)
                )),
                if word.is_empty() {
                    String::new()
                } else {
                    style.dim(&format!("{}...", word))
                }
            )?;
        }

        let mut all = false;
        let action = if let Some(da) = &default_action {
            *da
        } else {
            let choice = prompt(
                out,
                &style,
                index,
                count,
                *size,
                dir,
                provenance,
                freed_so_far,
                in_reader,
            )?;
            // prompt reports the -ALL intent out of band, since Action itself
            // does not carry it.
            all = choice.1;
            choice.0
        };

        if all {
            default_action = Some(action);
        }

        let result = match action {
            Action::DeleteAll => remove_dir(dir),
            Action::ClearContents => clear_dir(dir),
            Action::ShredContents => shred_dir(dir),
            Action::ShredAll => shred_dir(dir).and_then(|()| remove_dir(dir)),
            Action::Skip => {
                stats.skipped += 1;
                write_row(
                    out,
                    &style,
                    Row {
                        index,
                        count,
                        size: *size,
                        dir,
                        provenance,
                        state: RowState::Skipped,
                    },
                )?;
                continue;
            }
            Action::Exit => {
                writeln!(
                    out,
                    "\n  {} {}",
                    style.yellow("stopped."),
                    style.dim("remaining directories were left untouched")
                )?;
                return Ok(stats);
            }
        };

        match result {
            Ok(()) => {
                if action.shreds() {
                    stats.shredded += 1;
                }
                if action.keeps_dir() {
                    stats.cleared += 1;
                } else {
                    stats.deleted += 1;
                }
                stats.freed = stats.freed.saturating_add(*size);
                freed_so_far = stats.freed;
                write_row(
                    out,
                    &style,
                    Row {
                        index,
                        count,
                        size: *size,
                        dir,
                        provenance,
                        state: RowState::Done(action),
                    },
                )?;
            }
            Err(e) => {
                writeln!(
                    out,
                    "  {} {:>7}  {}  {}",
                    style.dim(&format!("[{:>2}/{:<2}]", index, count)),
                    style.yellow("failed"),
                    style.dim(&truncate_head(
                        &display_path(dir),
                        term_width().saturating_sub(24)
                    )),
                    style.red(&e.to_string())
                )?;
                stats.skipped += 1;
            }
        }
    }

    writeln!(out)?;
    writeln!(
        out,
        "  {}  {}  {}  {}  {}",
        style.bold("Done."),
        style.dim(&format!("{} freed", human_size(stats.freed))),
        style.green(&format!("{} deleted", stats.deleted)),
        style.green(&format!("{} cleared", stats.cleared)),
        style.dim(&format!(
            "{} shredded, {} skipped",
            stats.shredded, stats.skipped
        ))
    )?;
    if stats.freed == 0 && stats.skipped == stats.found && stats.found > 0 {
        writeln!(
            out,
            "  {}",
            style.dim("nothing was removed; run `purgable mark` to refresh candidates")
        )?;
    }
    writeln!(out)?;

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Opts;
    use crate::marker::TARGET;
    use crate::size::human_size;
    use crate::test_support::{opts, write_dir_with_content};
    use std::path::Path;
    use std::path::PathBuf;

    fn marked_dir(root: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = root.join(name);
        let mut all: Vec<(&str, &str)> = vec![(TARGET, "")];
        all.extend_from_slice(files);
        write_dir_with_content(&dir, &all);
        dir
    }

    #[test]
    fn test_d_removes_directory_and_contents() {
        let dir = tempfile::tempdir().unwrap();
        let sub = marked_dir(dir.path(), "sub", &[("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "d\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 1);
        assert_eq!(stats.deleted, 1);
        assert_eq!(stats.cleared, 0);
        assert_eq!(stats.skipped, 0);
        assert!(!sub.exists());
    }

    #[test]
    fn test_c_keeps_directory_and_removes_contents() {
        let dir = tempfile::tempdir().unwrap();
        let sub = marked_dir(
            dir.path(),
            "sub",
            &[("data.txt", "secret"), ("nested/deep.txt", "more")],
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "c\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.cleared, 1);
        assert!(sub.exists(), "directory must survive action c");
        assert!(!sub.join("data.txt").exists());
        assert!(!sub.join("nested").exists());
        assert!(sub.join(TARGET).exists(), "marker is preserved");
    }

    #[test]
    fn test_s_keeps_directory_and_removes_nested_dirs() {
        // Regression: the previous implementation shredded files but left the
        // empty directory skeleton behind.
        let dir = tempfile::tempdir().unwrap();
        let sub = marked_dir(
            dir.path(),
            "sub",
            &[("data.txt", "secret"), ("nested/deep.txt", "more")],
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "s\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.shredded, 1);
        assert_eq!(stats.cleared, 1);
        assert!(sub.exists());
        assert!(!sub.join("data.txt").exists());
        assert!(
            !sub.join("nested").exists(),
            "empty subdirectories must be removed"
        );
        assert!(sub.join(TARGET).exists());
    }

    #[test]
    fn test_x_shreds_and_removes_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = marked_dir(dir.path(), "sub", &[("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "x\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.shredded, 1);
        assert_eq!(stats.deleted, 1);
        assert_eq!(stats.cleared, 0);
        assert!(!sub.exists());
    }

    #[test]
    fn test_skip_leaves_everything_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let sub = marked_dir(dir.path(), "sub", &[("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "k\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.skipped, 1);
        assert!(sub.join("data.txt").exists());
    }

    #[test]
    fn test_empty_input_skips() {
        let dir = tempfile::tempdir().unwrap();
        marked_dir(dir.path(), "sub", &[("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.skipped, 1);
    }

    #[test]
    fn test_eof_skips() {
        let dir = tempfile::tempdir().unwrap();
        let sub = marked_dir(dir.path(), "sub", &[("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.skipped, 1);
        assert!(sub.join("data.txt").exists());
    }

    #[test]
    fn test_invalid_input_skips_with_notice() {
        let dir = tempfile::tempdir().unwrap();
        marked_dir(dir.path(), "sub", &[]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "wat\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let text = String::from_utf8(stdout).unwrap();
        assert!(text.contains("invalid action"));
    }

    #[test]
    fn test_delete_all_applies_to_subsequent() {
        let dir = tempfile::tempdir().unwrap();
        let first = marked_dir(dir.path(), "aaa", &[("a.txt", "x")]);
        let second = marked_dir(dir.path(), "bbb", &[("b.txt", "x")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "d-ALL\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.deleted, 2);
        assert!(!first.exists());
        assert!(!second.exists());
    }

    #[test]
    fn test_prompt_stops_after_exit() {
        let dir = tempfile::tempdir().unwrap();
        marked_dir(dir.path(), "aaa", &[("a.txt", "x")]);
        marked_dir(dir.path(), "bbb", &[("b.txt", "x")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "e\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 2);
        assert_eq!(stats.deleted, 0);
        assert!(dir.path().join("aaa").exists());
        assert!(dir.path().join("bbb").exists());
    }

    #[test]
    fn test_largest_reviewed_first() {
        let dir = tempfile::tempdir().unwrap();
        // Alphabetical order is deliberately the reverse of size order, so an
        // alphabetical implementation would fail this test.
        marked_dir(
            dir.path(),
            "aaa_small_name",
            &[("big.bin", &"x".repeat(8192))],
        );
        marked_dir(dir.path(), "zzz_large_name", &[("s.bin", "x")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "k\nk\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let text = String::from_utf8(stdout).unwrap();
        // Display paths may be shortened, and only one is under 10K, so assert
        // on the position of each directory's own size in the prompt stream.
        let big_size = human_size(8192);
        let small_size = human_size(1);
        let big_at = text
            .find(&big_size)
            .unwrap_or_else(|| panic!("bigger directory not prompted:\n{text}"));
        let small_at = text
            .find(&small_size)
            .unwrap_or_else(|| panic!("smaller directory not prompted:\n{text}"));
        assert!(
            big_at < small_at,
            "bigger directory should be prompted first"
        );
    }

    #[test]
    fn test_dry_run_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let sub = marked_dir(dir.path(), "sub", &[("data.txt", "secret")]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &Opts { dry_run: true },
            &mut "".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 1);
        assert_eq!(stats.deleted, 0);
        assert!(sub.join("data.txt").exists());
        let text = String::from_utf8(stdout).unwrap();
        assert!(text.contains("Dry run"));
    }

    #[test]
    fn test_freed_bytes_accounted() {
        let dir = tempfile::tempdir().unwrap();
        marked_dir(dir.path(), "sub", &[("blob", &"x".repeat(4096))]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "d\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert!(stats.freed >= 4096);
    }

    #[test]
    fn test_no_matches_reports_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let stats = purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stats.found, 0);
        assert!(String::from_utf8(stdout).unwrap().contains("No PURGABLE"));
    }

    #[test]
    fn test_freed_so_far_accumulates_across_prompts() {
        let dir = tempfile::tempdir().unwrap();
        marked_dir(dir.path(), "aaa", &[("a.bin", &"x".repeat(8192))]);
        marked_dir(dir.path(), "bbb", &[("b.bin", &"x".repeat(4096))]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "d\nk\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let text = String::from_utf8(stdout).unwrap();
        assert!(
            text.contains("freed so far"),
            "second prompt should show progress:\n{text}"
        );
    }
}
