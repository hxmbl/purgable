//! The `mark`, `unmark`, and `list` commands: creating markers from policy
//! matches, removing them, and reporting what is currently marked.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::action::Opts;
use crate::config::{config_path, Config, Policy};
use crate::discovery::{find, scan_for_policies, validate_root};
use crate::marker::{marker_path, read_marker, write_marker};
use crate::prompt::{write_row, Row, RowState};
use crate::size::{dir_size, human_size, now_secs, parse_size};
use crate::style::{display_path, term_width, truncate_head, Style};

/// Tally of what a `mark` run did.
pub(crate) struct MarkStats {
    /// Number of policy matches. Reported through the summary table, and
    /// asserted directly by the test suite.
    #[allow(dead_code)]
    pub(crate) matched: u32,
    pub(crate) marked: u32,
    pub(crate) already: u32,
}

/// Tally of what an `unmark` run did.
pub(crate) struct UnmarkStats {
    pub(crate) found: u32,
    pub(crate) removed: u32,
    /// Markers left alone because they fell outside the requested scope.
    pub(crate) skipped: u32,
}

/// Scan `root` for policy matches and drop a marker in each one.
///
/// Directories that already carry a marker are left alone and counted
/// separately. With `dry_run` nothing is written.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mark(
    root: &str,
    config: &Config,
    only: &[String],
    min_override: Option<u64>,
    opts: Opts,
    out: &mut impl io::Write,
    warn: &mut impl io::Write,
) -> io::Result<MarkStats> {
    validate_root(root)?;

    let default_min =
        min_override.or_else(|| config.defaults.min_size.as_deref().and_then(parse_size));
    let selected: Vec<&Policy> = config
        .policies
        .iter()
        .filter(|p| p.is_enabled())
        .filter(|p| only.is_empty() || only.iter().any(|name| name == &p.name))
        .collect();

    let style = Style::detect();
    let width = term_width();

    if selected.is_empty() {
        let _ = writeln!(
            out,
            "\n  {} {}",
            style.yellow("no enabled policies."),
            style.dim("check the `enabled` keys in your config")
        );
        return Ok(MarkStats {
            matched: 0,
            marked: 0,
            already: 0,
        });
    }

    let mut found = Vec::new();
    scan_for_policies(Path::new(root), &selected, default_min, &mut found, warn);
    found.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));

    let mut stats = MarkStats {
        matched: found.len() as u32,
        marked: 0,
        already: 0,
    };

    if found.is_empty() {
        let _ = writeln!(
            out,
            "\n  {} {}",
            style.dim("nothing matched."),
            style.dim(&format!(
                "lower --min-size, or add a policy to {}",
                display_path(&config_path())
            ))
        );
        return Ok(stats);
    }

    let now = now_secs();
    let total: u64 = found
        .iter()
        .map(|c| c.size)
        .fold(0u64, |a, b| a.saturating_add(b));
    writeln!(
        out,
        "\n  {}  {}",
        style.bold(&format!("{} matched", found.len())),
        style.dim(&format!("{} total", human_size(total)))
    )?;
    let rule = style.dim(&"─".repeat(width.saturating_sub(4)));
    writeln!(out, "  {}", rule)?;
    writeln!(
        out,
        "  {:>6}  {:>8}  {:<14}  {:<16}  {}",
        style.dim(""),
        style.dim("size"),
        style.dim("action"),
        style.dim("policy"),
        style.dim("path")
    )?;
    writeln!(out, "  {}", rule)?;
    for (index, candidate) in found.iter().enumerate() {
        let counter = style.dim(&format!("[{:>2}/{:<2}]", index + 1, found.len()));
        let size_text = style.cyan(&human_size(candidate.size));
        let path = truncate_head(&display_path(&candidate.path), width.saturating_sub(46));
        let policy = style.magenta(&truncate_head(&candidate.policy, 16));

        let (action_text, path_text) = if marker_path(&candidate.path).exists() {
            stats.already += 1;
            (style.dim("already marked"), style.dim(&path))
        } else if opts.dry_run {
            (style.yellow("would mark"), style.dim(&path))
        } else {
            write_marker(&candidate.path, &candidate.policy, candidate.size, now)?;
            stats.marked += 1;
            (style.green("marked"), style.dim(&path))
        };

        writeln!(
            out,
            "  {}  {:>8}  {:<14}  {:<16}  {}",
            counter, size_text, action_text, policy, path_text
        )?;
    }

    writeln!(out)?;
    writeln!(
        out,
        "  {}  {}  {}",
        style.bold("Done."),
        style.green(&format!("{} marked", stats.marked)),
        style.dim(&format!(
            "{} already marked{}",
            stats.already,
            if opts.dry_run {
                ", dry run: nothing written"
            } else {
                ""
            }
        ))
    )?;
    if stats.marked > 0 && !opts.dry_run {
        writeln!(
            out,
            "  {}",
            style.dim(&format!(
                "run `purgable review {}` to decide what to do",
                display_path(Path::new(root))
            ))
        )?;
    }
    writeln!(out)?;
    Ok(stats)
}

/// Remove markers under `root`.
///
/// The default scope is deliberately narrow: only markers this tool wrote.
/// Hand-placed markers survive unless `all` is set, and are still listed so a
/// zero result reads as "these are yours, not mine" rather than "nothing found".
pub(crate) fn unmark(
    root: &str,
    policy: Option<&str>,
    all: bool,
    dry_run: bool,
    out: &mut impl io::Write,
    warn: &mut impl io::Write,
) -> io::Result<UnmarkStats> {
    validate_root(root)?;
    let style = Style::detect();
    let width = term_width();
    let marked = find(root, warn)?;
    let mut stats = UnmarkStats {
        found: marked.len() as u32,
        removed: 0,
        skipped: 0,
    };

    if marked.is_empty() {
        writeln!(out, "\n  {}", style.dim("no marked directories."))?;
        return Ok(stats);
    }

    let scope = match (policy, all) {
        (Some(name), _) => format!("policy {}", name),
        (None, true) => "all markers".to_string(),
        (None, false) => "markers written by mark".to_string(),
    };

    writeln!(
        out,
        "\n  {}  {}",
        style.bold(&format!("{} marked", stats.found)),
        style.dim(&format!("removing {}", scope))
    )?;
    writeln!(out)?;

    let rule = style.dim(&"─".repeat(width.saturating_sub(4)));
    writeln!(out, "  {}", rule)?;
    writeln!(
        out,
        "  {:>6}  {:<14}  {:<16}  {}",
        style.dim(""),
        style.dim("action"),
        style.dim("marked by"),
        style.dim("path")
    )?;
    writeln!(out, "  {}", rule)?;

    for dir in &marked {
        let marker = read_marker(dir);
        // Default is deliberately narrow: only markers this tool wrote. Hand
        // placed markers survive unless --all is given.
        let selected = all
            || match policy {
                Some(name) => marker.policy.as_deref() == Some(name),
                None => marker.is_ours(),
            };
        let label = marker.describe(now_secs());
        let path = truncate_head(&display_path(dir), width.saturating_sub(40));

        // Out-of-scope markers are still listed, dimmed. Silently omitting them
        // makes "0 removed" look broken when the real answer is "these two are
        // yours, not mine".
        let (action_text, path_text) = if !selected {
            stats.skipped += 1;
            (style.dim("kept"), style.dim(&path))
        } else if dry_run {
            (style.yellow("would unmark"), style.dim(&path))
        } else {
            fs::remove_file(marker_path(dir))?;
            stats.removed += 1;
            (style.green("unmarked"), style.dim(&path))
        };
        writeln!(
            out,
            "  {:>6}  {:<14}  {:<16}  {}",
            style.dim(""),
            action_text,
            style.magenta(&truncate_head(&label, 16)),
            path_text
        )?;
    }

    writeln!(out)?;
    writeln!(
        out,
        "  {}  {}",
        style.bold("Done."),
        if dry_run {
            style.yellow(&format!("{} would be removed", stats.removed))
        } else {
            style.green(&format!("{} removed", stats.removed))
        }
    )?;
    if stats.skipped > 0 {
        writeln!(
            out,
            "  {}",
            style.dim(&format!(
                "{} kept: outside `{}`; use --all to include them",
                stats.skipped, scope
            ))
        )?;
    }
    writeln!(out)?;
    Ok(stats)
}

/// Report every marked directory under `root`, largest first.
pub(crate) fn list(
    root: &str,
    out: &mut impl io::Write,
    warn: &mut impl io::Write,
) -> io::Result<()> {
    validate_root(root)?;
    let style = Style::detect();
    let marked = find(root, warn)?;
    if marked.is_empty() {
        writeln!(
            out,
            "\n  {} {}",
            style.dim("no marked directories."),
            style.dim(&format!(
                "try `purgable mark {}`",
                display_path(Path::new(root))
            ))
        )?;
        return Ok(());
    }

    let now = now_secs();
    let mut rows: Vec<(u64, PathBuf, String)> = Vec::new();
    let mut total = 0u64;
    for dir in &marked {
        let size = dir_size(dir);
        total = total.saturating_add(size);
        rows.push((size, dir.clone(), read_marker(dir).describe(now)));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    writeln!(
        out,
        "\n  {}  {}",
        style.bold(&format!("{} marked", rows.len())),
        style.dim(&format!("{} total", human_size(total)))
    )?;
    writeln!(out)?;
    for (index, (size, dir, provenance)) in rows.iter().enumerate() {
        write_row(
            out,
            &style,
            Row {
                index: index + 1,
                count: rows.len(),
                size: *size,
                dir,
                provenance,
                state: RowState::Preview,
            },
        )?;
    }
    writeln!(
        out,
        "\n  {}",
        style.dim("run `purgable review` to decide what to do with them")
    )?;
    writeln!(out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Opts;
    use crate::config::parse_config;
    use crate::marker::{read_marker, write_marker, TARGET};
    use crate::size::{dir_size, human_size, now_secs};
    use crate::test_support::{opts, write_dir_with_content, write_file};
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::path::PathBuf;

    const CARGO_CONFIG: &str = r#"
    [defaults]
    min_size = "1M"

    [[policy]]
    name = "cargo-target"
    dir_name = "target"
    require_sibling = ["Cargo.toml"]
    require_child_any = [".rustc_info.json", "debug", "release", "CACHE"]
"#;

    fn fake_cargo_project(root: &Path, name: &str, bytes: usize) -> PathBuf {
        let project = root.join(name);
        write_file(&project.join("Cargo.toml"), "[package]\nname=\"x\"\n");
        write_file(&project.join("target/debug/blob"), &"x".repeat(bytes));
        project
    }

    #[test]
    fn test_mark_finds_cargo_target() {
        let dir = tempfile::tempdir().unwrap();
        fake_cargo_project(dir.path(), "app", 2 * 1024 * 1024);
        let config = parse_config(CARGO_CONFIG).unwrap();

        let mut out = Vec::new();
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &[],
            None,
            opts(),
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();

        assert_eq!(stats.matched, 1);
        assert_eq!(stats.marked, 1);
        assert!(dir.path().join("app/target").join(TARGET).exists());
    }

    #[test]
    fn test_mark_ignores_non_cargo_target_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = dir.path().join("linux/kernel/include/target");
        write_file(&kernel.join("keepme.txt"), "critical source");
        let config = parse_config(CARGO_CONFIG).unwrap();

        let mut out = Vec::new();
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &[],
            None,
            opts(),
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();

        assert_eq!(stats.matched, 0);
        assert!(!kernel.join(TARGET).exists());
        assert!(kernel.join("keepme.txt").exists());
    }

    #[test]
    fn test_mark_respects_min_size() {
        let dir = tempfile::tempdir().unwrap();
        fake_cargo_project(dir.path(), "small", 512);
        let config = parse_config(CARGO_CONFIG).unwrap();

        let mut out = Vec::new();
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &[],
            None,
            opts(),
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.matched, 0);
    }

    #[test]
    fn test_mark_min_size_override_argument() {
        let dir = tempfile::tempdir().unwrap();
        fake_cargo_project(dir.path(), "app", 2048);
        let config = parse_config(CARGO_CONFIG).unwrap();

        let mut out = Vec::new();
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &[],
            Some(1), // override the 1M default down to 1 byte
            opts(),
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.marked, 1);
    }

    #[test]
    fn test_mark_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        fake_cargo_project(dir.path(), "app", 2 * 1024 * 1024);
        let config = parse_config(CARGO_CONFIG).unwrap();

        let mut out = Vec::new();
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &[],
            None,
            Opts { dry_run: true },
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.matched, 1);
        assert_eq!(stats.marked, 0);
        assert!(!dir.path().join("app/target").join(TARGET).exists());
    }

    #[test]
    fn test_mark_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        fake_cargo_project(dir.path(), "app", 2 * 1024 * 1024);
        let config = parse_config(CARGO_CONFIG).unwrap();

        let mut out = Vec::new();
        for _ in 0..2 {
            mark(
                dir.path().to_str().unwrap(),
                &config,
                &[],
                None,
                opts(),
                &mut out,
                &mut io::stderr(),
            )
            .unwrap();
        }
        let marker = read_marker(&dir.path().join("app/target"));
        assert_eq!(marker.policy.as_deref(), Some("cargo-target"));
    }

    #[test]
    fn test_mark_does_not_descend_into_matches() {
        let dir = tempfile::tempdir().unwrap();
        let project = fake_cargo_project(dir.path(), "app", 2 * 1024 * 1024);
        // A vendored crate inside the build tree should not be reported.
        write_file(&project.join("target/debug/vendor/inner/Cargo.toml"), "x");
        write_file(
            &project.join("target/debug/vendor/inner/target/blob"),
            &"y".repeat(9),
        );
        let config = parse_config(CARGO_CONFIG).unwrap();

        let mut out = Vec::new();
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &[],
            None,
            opts(),
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.matched, 1, "should not report nested matches");
    }

    #[test]
    fn test_mark_skips_git_directories() {
        let dir = tempfile::tempdir().unwrap();
        let project = fake_cargo_project(dir.path(), "app", 2 * 1024 * 1024);
        fs::create_dir_all(project.join(".git")).unwrap();
        let config = parse_config(CARGO_CONFIG).unwrap();

        let mut out = Vec::new();
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &[],
            None,
            opts(),
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.matched, 1);
    }

    #[test]
    fn test_mark_policy_filter() {
        let dir = tempfile::tempdir().unwrap();
        fake_cargo_project(dir.path(), "app", 2 * 1024 * 1024);
        let config = parse_config(
            r#"
        [defaults]
        min_size = "1M"

        [[policy]]
        name = "cargo-target"
        dir_name = "target"
        require_sibling = ["Cargo.toml"]

        [[policy]]
        name = "node-modules"
        dir_name = "node_modules"
        "#,
        )
        .unwrap();

        let mut out = Vec::new();
        // Select only node-modules: nothing in this tree matches it.
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &["node-modules".to_string()],
            None,
            opts(),
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.matched, 0);
    }

    #[test]
    fn test_mark_dir_name_any_and_sibling_any() {
        let dir = tempfile::tempdir().unwrap();
        let venv = dir.path().join("proj/.venv");
        write_file(&dir.path().join("proj/pyproject.toml"), "x");
        write_file(&venv.join("lib/thing.py"), &"x".repeat(2 * 1024 * 1024));
        let config = parse_config(
            r#"
        [defaults]
        min_size = "1M"

        [[policy]]
        name = "python-venv"
        dir_name_any = [".venv", "venv"]
        require_sibling_any = ["pyproject.toml", "requirements.txt"]
        "#,
        )
        .unwrap();

        let mut out = Vec::new();
        let stats = mark(
            dir.path().to_str().unwrap(),
            &config,
            &[],
            None,
            opts(),
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.marked, 1);
        assert_eq!(read_marker(&venv).policy.as_deref(), Some("python-venv"));
    }

    #[test]
    fn test_unmark_leaves_manual_markers_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let manual = dir.path().join("manual");
        write_dir_with_content(&manual, &[(TARGET, "")]);

        let mut out = Vec::new();
        let stats = unmark(
            dir.path().to_str().unwrap(),
            None,
            false,
            false,
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.removed, 0);
        assert!(manual.join(TARGET).exists());
    }

    #[test]
    fn test_unmark_all_removes_manual_markers() {
        let dir = tempfile::tempdir().unwrap();
        let manual = dir.path().join("manual");
        write_dir_with_content(&manual, &[(TARGET, "")]);

        let mut out = Vec::new();
        let stats = unmark(
            dir.path().to_str().unwrap(),
            None,
            true,
            false,
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.removed, 1);
        assert!(!manual.join(TARGET).exists());
        assert!(manual.exists());
    }

    #[test]
    fn test_unmark_by_policy() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        write_dir_with_content(&a, &[]);
        write_dir_with_content(&b, &[]);
        write_marker(&a, "cargo-target", 1, 1).unwrap();
        write_marker(&b, "node-modules", 1, 1).unwrap();

        let mut out = Vec::new();
        let stats = unmark(
            dir.path().to_str().unwrap(),
            Some("cargo-target"),
            false,
            false,
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.removed, 1);
        assert!(!a.join(TARGET).exists());
        assert!(b.join(TARGET).exists());
    }

    #[test]
    fn test_unmark_reports_out_of_scope_markers() {
        // A manual marker outside the default scope must be listed as kept, not
        // silently dropped: "0 removed" is otherwise indistinguishable from a
        // broken run.
        let dir = tempfile::tempdir().unwrap();
        let manual = dir.path().join("manual");
        write_dir_with_content(&manual, &[(TARGET, "")]);

        let mut out = Vec::new();
        let stats = unmark(
            dir.path().to_str().unwrap(),
            None,
            false,
            true,
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert_eq!(stats.removed, 0);
        assert_eq!(stats.skipped, 1);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("kept"), "{text}");
        assert!(text.contains("--all"), "{text}");
    }

    #[test]
    fn test_unmark_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        write_dir_with_content(&a, &[]);
        write_marker(&a, "cargo-target", 1, 1).unwrap();

        let mut out = Vec::new();
        unmark(
            dir.path().to_str().unwrap(),
            None,
            true,
            true,
            &mut out,
            &mut io::stderr(),
        )
        .unwrap();
        assert!(a.join(TARGET).exists());
    }

    #[test]
    fn test_list_reports_marked_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        write_dir_with_content(&sub, &[(TARGET, ""), ("blob", &"x".repeat(2048))]);
        write_marker(&sub, "cargo-target", 2048, now_secs()).unwrap();
        let expected = human_size(dir_size(&sub));

        let mut out = Vec::new();
        list(dir.path().to_str().unwrap(), &mut out, &mut io::stderr()).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("cargo-target"));
        assert!(text.contains(&expected), "size {expected} missing: {text}");
    }
}
