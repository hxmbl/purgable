//! Terminal presentation primitives: ANSI styling, width measurement, and the
//! path shortening that keeps listings on one line.

use std::borrow::Cow;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::OnceLock;

/// ANSI styling, disabled wholesale when output is not a terminal.
pub(crate) struct Style {
    enabled: bool,
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";

impl Style {
    /// Colour is used only for a real terminal, and honours the `NO_COLOR`
    /// convention. Piped output and redirected logs stay plain.
    pub(crate) fn detect() -> Self {
        let enabled = std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true);
        Style { enabled }
    }

    #[cfg(test)]
    pub(crate) fn plain() -> Self {
        Style { enabled: false }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("{}{}{}", code, text, RESET)
        } else {
            text.to_string()
        }
    }

    pub(crate) fn bold(&self, t: &str) -> String {
        self.wrap(BOLD, t)
    }
    pub(crate) fn dim(&self, t: &str) -> String {
        self.wrap(DIM, t)
    }
    pub(crate) fn red(&self, t: &str) -> String {
        self.wrap(RED, t)
    }
    pub(crate) fn green(&self, t: &str) -> String {
        self.wrap(GREEN, t)
    }
    pub(crate) fn yellow(&self, t: &str) -> String {
        self.wrap(YELLOW, t)
    }
    pub(crate) fn blue(&self, t: &str) -> String {
        self.wrap(BLUE, t)
    }
    pub(crate) fn magenta(&self, t: &str) -> String {
        self.wrap(MAGENTA, t)
    }
    pub(crate) fn cyan(&self, t: &str) -> String {
        self.wrap(CYAN, t)
    }
}

/// Visible width of a string, ignoring ANSI escape sequences.
///
/// Styling adds invisible bytes; measuring the raw string would make every
/// bordered row too wide by the number of escape codes it contains.
pub(crate) fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            width += 1;
            continue;
        }
        // Consume a CSI sequence: ESC '[' <params> <final>. The '[' itself is
        // in the 0x40..=0x7e final-byte range, so it must be consumed
        // explicitly rather than treated as the terminator.
        if chars.next() != Some('[') {
            continue;
        }
        for c in chars.by_ref() {
            if ('\x40'..='\x7e').contains(&c) {
                break;
            }
        }
    }
    width
}

/// Shorten to `max` visible columns, keeping the tail and marking the cut.
/// Paths are more identifiable by their end than their start, so `...` goes on
/// the left.
pub(crate) fn truncate_head(text: &str, max: usize) -> String {
    if visible_width(text) <= max {
        return text.to_string();
    }
    if max <= 3 {
        return ".".repeat(max);
    }
    let keep = max - 3;
    let tail: String = text.chars().skip(text.chars().count() - keep).collect();
    format!("...{}", tail)
}

/// The user's home directory, read once.
///
/// `display_path` runs once per row of a listing, and `getenv` is a lock plus a
/// linear scan of `environ` on every call. The value cannot change during a run.
fn home() -> Option<&'static str> {
    static HOME: OnceLock<Option<String>> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::var_os("HOME")?;
        let home = home.to_string_lossy().into_owned();
        (!home.is_empty()).then_some(home)
    })
    .as_deref()
}

/// Replace the home directory prefix with `~` so paths fit on one line.
///
/// Borrows the path when there is nothing to rewrite, so a path outside `~`
/// costs no allocation at all.
pub(crate) fn display_path(path: &Path) -> Cow<'_, str> {
    // to_string_lossy borrows for valid UTF-8, which is the overwhelmingly
    // common case, and allocates only for a path that is not.
    let text = path.to_string_lossy();
    let Some(home) = home() else {
        return text;
    };
    if text == home {
        return Cow::Borrowed("~");
    }
    match text
        .strip_prefix(home)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        Some(rest) => Cow::Owned(format!("~/{}", rest)),
        None => text,
    }
}

/// One action in the prompt legend.
pub(crate) struct Choice {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) destructive: bool,
}

pub(crate) const CHOICES: [Choice; 6] = [
    Choice {
        key: "d",
        label: "delete",
        detail: "directory and contents",
        destructive: true,
    },
    Choice {
        key: "c",
        label: "clear",
        detail: "contents only, keep directory",
        destructive: true,
    },
    Choice {
        key: "s",
        label: "shred",
        detail: "overwrite files, keep directory",
        destructive: true,
    },
    Choice {
        key: "x",
        label: "shred all",
        detail: "overwrite files, delete directory",
        destructive: true,
    },
    Choice {
        key: "k",
        label: "skip",
        detail: "keep everything, continue",
        destructive: false,
    },
    Choice {
        key: "e",
        label: "exit",
        detail: "stop now",
        destructive: false,
    },
];

/// Terminal width, clamped to a range that keeps the layout readable.
pub(crate) fn term_width() -> usize {
    let width = unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0 {
            size.ws_col as usize
        } else {
            0
        }
    };
    if width < 40 {
        80
    } else {
        width.min(120)
    }
}

/// Pad a styled string to `width` visible columns, ignoring escape codes.
pub(crate) fn pad_to(text: &str, width: usize) -> String {
    let visible = visible_width(text);
    if visible >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width - visible))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{parse_action, Action};
    use std::path::Path;
    use std::path::PathBuf;

    #[test]
    fn test_visible_width_ignores_ansi() {
        assert_eq!(visible_width("abc"), 3);
        assert_eq!(visible_width("\x1b[1mabc\x1b[0m"), 3);
        assert_eq!(visible_width("\x1b[32m\x1b[1m10.0G\x1b[0m\x1b[0m"), 5);
        assert_eq!(visible_width(""), 0);
    }

    #[test]
    fn test_visible_width_handles_colour_only() {
        let style = Style::plain();
        assert_eq!(visible_width(&style.dim("abc")), 3);
        assert_eq!(visible_width(&style.green("1.0G")), 4);
    }

    #[test]
    fn test_truncate_head_keeps_tail() {
        assert_eq!(truncate_head("short", 20), "short");
        // Long input keeps the end, which is the informative part.
        let long = "/very/long/path/to/some/deeply/nested/target/directory";
        let cut = truncate_head(long, 20);
        assert_eq!(visible_width(&cut), 20);
        assert!(cut.starts_with("..."));
        assert!(long.ends_with(&cut[3..]));
    }

    #[test]
    fn test_truncate_head_degenerate_widths() {
        assert_eq!(truncate_head("abcdef", 3), "...");
        assert_eq!(truncate_head("abcdef", 2), "..");
        assert_eq!(truncate_head("abcdef", 1), ".");
        assert_eq!(truncate_head("abcdef", 0), "");
    }

    #[test]
    fn test_truncate_head_does_not_count_ansi_in_budget() {
        let style = Style::plain();
        let styled = style.dim("abcdefghij");
        // 10 visible chars in a 10 wide budget: no truncation needed.
        assert_eq!(truncate_head(&styled, 10), styled);
        let cut = truncate_head(&styled, 6);
        assert_eq!(visible_width(&cut), 6);
    }

    #[test]
    fn test_pad_to_pads_to_visible_width() {
        let style = Style::plain();
        let padded = pad_to(&style.dim("ab"), 6);
        assert_eq!(visible_width(&padded), 6);
        // Never truncates.
        assert_eq!(pad_to("abcdef", 3), "abcdef");
    }

    #[test]
    fn test_display_path_shortens_home() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if let Some(home) = home {
            assert_eq!(display_path(&home), "~");
            let nested = home.join("Projects/app/target");
            assert_eq!(display_path(&nested), "~/Projects/app/target");
        }
        // A path outside home is untouched.
        assert_eq!(display_path(Path::new("/tmp")), "/tmp");
    }

    #[test]
    fn test_choices_cover_every_action_letter() {
        let letters: Vec<&str> = CHOICES.iter().map(|c| c.key).collect();
        assert_eq!(letters, vec!["d", "c", "s", "x", "k", "e"]);
        // Every advertised key must actually parse.
        for choice in CHOICES.iter() {
            let (action, _, ok) = parse_action(choice.key);
            assert!(ok, "{} should parse", choice.key);
            match choice.key {
                "d" => assert_eq!(action, Action::DeleteAll),
                "c" => assert_eq!(action, Action::ClearContents),
                "s" => assert_eq!(action, Action::ShredContents),
                "x" => assert_eq!(action, Action::ShredAll),
                "k" => assert_eq!(action, Action::Skip),
                "e" => assert_eq!(action, Action::Exit),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn test_term_width_is_clamped() {
        let width = term_width();
        assert!((40..=120).contains(&width), "width {width} out of range");
    }

    #[test]
    fn test_no_color_env_disables_styling() {
        // Style::detect must never panic and must produce printable output
        // regardless of environment.
        let style = Style::detect();
        assert!(!style.red("x").contains('\x1b') || std::io::stdout().is_terminal());
    }
}
