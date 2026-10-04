//! The interactive prompt widget: the bordered question box, the action legend,
//! and the one-line result rows that follow each decision.

use std::io::{self, Write};
use std::path::Path;

use crate::action::{parse_action, Action};
use crate::size::human_size;
use crate::style::{display_path, pad_to, term_width, truncate_head, Style, CHOICES};

/// Outcome of one row, which decides the colour and wording of its summary.
pub(crate) enum RowState {
    Preview,
    Done(Action),
    Skipped,
}

/// One line in a listing: counter, size, outcome, then the shortened path.
pub(crate) struct Row<'a> {
    pub(crate) index: usize,
    pub(crate) count: usize,
    pub(crate) size: u64,
    pub(crate) dir: &'a Path,
    pub(crate) provenance: &'a str,
    pub(crate) state: RowState,
}

pub(crate) fn write_row(out: &mut impl Write, style: &Style, row: Row<'_>) -> io::Result<()> {
    let Row {
        index,
        count,
        size,
        dir,
        provenance,
        state,
    } = row;
    let counter = style.dim(&format!("[{:>2}/{:<2}]", index, count));
    let size_text = match state {
        RowState::Preview => style.cyan(&human_size(size)),
        RowState::Done(_) => style.bold(&human_size(size)),
        RowState::Skipped => style.dim(&human_size(size)),
    };
    let path = truncate_head(&display_path(dir), term_width().saturating_sub(30));
    let note = match state {
        RowState::Preview => style.dim(provenance),
        RowState::Done(action) => {
            let text = match action {
                Action::DeleteAll => "deleted",
                Action::ClearContents => "cleared, directory kept",
                Action::ShredContents => "shredded, directory kept",
                Action::ShredAll => "shredded and deleted",
                Action::Skip | Action::Exit => "",
            };
            style.green(text)
        }
        RowState::Skipped => style.yellow("skipped"),
    };
    writeln!(
        out,
        "  {}  {:>8}  {:<6}  {}",
        counter,
        size_text,
        note,
        style.dim(&path)
    )
}

/// Draw the full prompt block and read one answer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prompt(
    out: &mut impl Write,
    style: &Style,
    index: usize,
    count: usize,
    size: u64,
    dir: &Path,
    provenance: &str,
    freed_so_far: u64,
    in_reader: &mut impl io::BufRead,
) -> io::Result<(Action, bool)> {
    // Box geometry. The rules are "  " + corner + rule + corner, while content
    // rows are "  " + border + space + text + space + border. Matching them
    // means the text budget is two narrower than the rule.
    let rule_width = term_width().saturating_sub(4).max(34);
    let inner = rule_width - 2;
    let rule = "─".repeat(rule_width);

    writeln!(out)?;
    writeln!(
        out,
        "  {}{}{}",
        style.dim("┌"),
        style.dim(&rule),
        style.dim("┐")
    )?;

    // Path on its own line, shortened from the left so the filename survives.
    // Shorten $HOME to ~ first: on a real tree that removes far more than the
    // width budget ever will.
    let path_text = style.bold(&truncate_head(&display_path(dir), inner));
    writeln!(
        out,
        "  {} {} {}",
        style.dim("│"),
        pad_to(&path_text, inner),
        style.dim("│")
    )?;

    // Second line: the facts that drive the decision.
    let mut facts = vec![style.cyan(&human_size(size))];
    if !provenance.is_empty() {
        facts.push(style.dim(provenance));
    }
    if freed_so_far > 0 {
        let freed = format!("{} freed so far", human_size(freed_so_far));
        facts.push(style.dim(&freed));
    }
    let facts_text = facts.join(style.dim(" · ").as_str());
    writeln!(
        out,
        "  {} {} {}",
        style.dim("│"),
        pad_to(&facts_text, inner),
        style.dim("│")
    )?;

    writeln!(
        out,
        "  {}{}{}",
        style.dim("└"),
        style.dim(&rule),
        style.dim("┘")
    )?;

    for choice in CHOICES.iter() {
        let key = if choice.destructive {
            style.yellow(choice.key)
        } else {
            style.blue(choice.key)
        };
        writeln!(
            out,
            "    {}  {:<11}  {}",
            key,
            style.bold(choice.label),
            style.dim(choice.detail)
        )?;
    }

    write!(
        out,
        "\n  {} ",
        style.bold(&format!(
            "[{}/{}] Enter choice:",
            style.dim(&index.to_string()),
            style.dim(&count.to_string())
        ))
    )?;
    out.flush()?;

    let mut line = String::new();
    let n = in_reader.read_line(&mut line)?;
    if n == 0 {
        // EOF: piped input ran out. Skip quietly rather than looping.
        writeln!(out)?;
        return Ok((Action::Skip, false));
    }
    // The prompt itself has no trailing newline, so the terminal echoes the
    // answer on the same line. Close it before anything else is written.
    writeln!(out)?;
    let ans = line.trim().to_lowercase();
    if ans.is_empty() {
        writeln!(
            out,
            "  {}",
            style.dim("no input, skipping (press d to delete)")
        )?;
        return Ok((Action::Skip, false));
    }
    let (action, all, ok) = parse_action(&ans);
    if !ok {
        writeln!(
            out,
            "  {} {}",
            style.red("invalid action"),
            style.dim(&format!("{:?}; skipping", ans))
        )?;
        return Ok((Action::Skip, false));
    }
    if all {
        writeln!(
            out,
            "  {} {}",
            style.yellow("applying to all remaining"),
            style.dim(&format!("({} left)", count.saturating_sub(index)))
        )?;
    }
    Ok((action, all))
}

#[cfg(test)]
mod tests {
    use crate::marker::{write_marker, TARGET};
    use crate::purge::purge;
    use crate::size::{dir_size, human_size, now_secs};
    use crate::style::{visible_width, CHOICES};
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
    fn test_prompt_shows_size_and_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        // The marker itself carries bytes, so measure rather than assume.
        write_dir_with_content(&sub, &[(TARGET, ""), ("blob", &"x".repeat(4096))]);
        write_marker(&sub, "cargo-target", 4096, now_secs()).unwrap();
        let expected = human_size(dir_size(&sub));

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "k\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let text = String::from_utf8(stdout).unwrap();
        assert!(
            text.contains(&expected),
            "size {expected} missing from prompt: {text}"
        );
        assert!(text.contains("cargo-target"), "provenance missing: {text}");
    }

    #[test]
    fn test_no_escape_codes_when_piped() {
        // Colour must never reach a pipe or a log file.
        let dir = tempfile::tempdir().unwrap();
        let sub = marked_dir(dir.path(), "sub", &[("blob", &"x".repeat(2048))]);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "k\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let text = String::from_utf8(stdout).unwrap();
        assert!(
            !text.contains('\x1b'),
            "escape codes leaked into piped output"
        );
        assert!(
            !text.contains('\u{1b}'),
            "escape codes leaked into piped output"
        );
        assert!(sub.exists());
    }

    #[test]
    fn test_box_borders_are_aligned() {
        let dir = tempfile::tempdir().unwrap();
        marked_dir(dir.path(), "sub", &[("blob", &"x".repeat(2048))]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "k\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let text = String::from_utf8(stdout).unwrap();

        // Every border row must be the same visible width, or the box is ragged.
        let borders: Vec<usize> = text
            .lines()
            .filter(|l| {
                let t = l.trim();
                t.starts_with('┌') || t.starts_with('│') || t.starts_with('└')
            })
            .map(|l| visible_width(l.trim()))
            .collect();
        assert!(borders.len() >= 4, "expected a full box, got {borders:?}");
        let first = borders[0];
        for width in &borders {
            assert_eq!(*width, first, "box borders misaligned: {borders:?}");
        }
    }

    #[test]
    fn test_prompt_shows_action_legend() {
        let dir = tempfile::tempdir().unwrap();
        marked_dir(dir.path(), "sub", &[("blob", &"x".repeat(1024))]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "k\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let text = String::from_utf8(stdout).unwrap();

        // Every advertised choice and its description must be on screen.
        for choice in CHOICES.iter() {
            assert!(text.contains(choice.key), "missing key {}", choice.key);
            assert!(
                text.contains(choice.detail),
                "missing detail for {}: {}",
                choice.key,
                choice.detail
            );
        }
        assert!(text.contains("Enter choice:"));
        assert!(text.contains("[1/1]"));
    }

    #[test]
    fn test_result_row_reports_action_taken() {
        let dir = tempfile::tempdir().unwrap();
        // c keeps the directory, so it produces a "cleared" row.
        marked_dir(dir.path(), "sub", &[("blob", &"x".repeat(1024))]);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        purge(
            dir.path().to_str().unwrap(),
            &opts(),
            &mut "c\n".as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let text = String::from_utf8(stdout).unwrap();
        assert!(
            text.contains("cleared, directory kept"),
            "missing outcome row:\n{text}"
        );
    }
}
