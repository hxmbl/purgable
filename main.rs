use rand::rng;
use rand::RngExt;
use serde::Deserialize;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::{DirEntry, WalkDir};

pub const TARGET: &str = "PURGABLE";
const MARKER_MAGIC: &str = "purgable:v1";
const SHRED_BUF_SIZE: usize = 64 * 1024;

pub struct Stats {
    pub found: u32,
    /// Directories removed entirely (actions `d` and `x`).
    pub deleted: u32,
    /// Directories emptied but kept on disk (actions `c` and `s`).
    pub cleared: u32,
    /// Directories whose files were overwritten before removal (`s` and `x`).
    pub shredded: u32,
    pub skipped: u32,
    /// Approximate bytes reclaimed.
    pub freed: u64,
}

pub struct MarkStats {
    pub matched: u32,
    pub marked: u32,
    pub already: u32,
}

pub struct UnmarkStats {
    pub found: u32,
    pub removed: u32,
    /// Markers left alone because they fell outside the requested scope.
    pub skipped: u32,
}

// ---------------------------------------------------------------------------
// Terminal presentation
// ---------------------------------------------------------------------------

/// ANSI styling, disabled wholesale when output is not a terminal.
pub struct Style {
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
    pub fn detect() -> Self {
        let enabled = std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true);
        Style { enabled }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Style { enabled: false }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("{}{}{}", code, text, RESET)
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, t: &str) -> String {
        self.wrap(BOLD, t)
    }
    pub fn dim(&self, t: &str) -> String {
        self.wrap(DIM, t)
    }
    pub fn red(&self, t: &str) -> String {
        self.wrap(RED, t)
    }
    pub fn green(&self, t: &str) -> String {
        self.wrap(GREEN, t)
    }
    pub fn yellow(&self, t: &str) -> String {
        self.wrap(YELLOW, t)
    }
    pub fn blue(&self, t: &str) -> String {
        self.wrap(BLUE, t)
    }
    pub fn magenta(&self, t: &str) -> String {
        self.wrap(MAGENTA, t)
    }
    pub fn cyan(&self, t: &str) -> String {
        self.wrap(CYAN, t)
    }
}

/// Visible width of a string, ignoring ANSI escape sequences.
///
/// Styling adds invisible bytes; measuring the raw string would make every
/// bordered row too wide by the number of escape codes it contains.
fn visible_width(s: &str) -> usize {
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
fn truncate_head(text: &str, max: usize) -> String {
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

/// Replace the home directory prefix with `~` so paths fit on one line.
pub fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().to_string();
        if !home.is_empty() {
            if text == home {
                return "~".to_string();
            }
            if let Some(rest) = text.strip_prefix(&format!("{}/", home)) {
                return format!("~/{}", rest);
            }
        }
    }
    text
}

/// One action in the prompt legend.
pub struct Choice {
    pub key: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
    pub destructive: bool,
}

pub const CHOICES: [Choice; 6] = [
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
pub fn term_width() -> usize {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `d` - delete the directory and everything in it.
    DeleteAll,
    /// `c` - delete everything inside, keep the directory.
    ClearContents,
    /// `s` - overwrite files, then empty the directory, keeping the directory.
    ShredContents,
    /// `x` - overwrite files, then delete the directory and everything in it.
    ShredAll,
    Skip,
    Exit,
}

impl Action {
    /// Whether this action empties the directory without removing it.
    fn keeps_dir(self) -> bool {
        matches!(self, Action::ClearContents | Action::ShredContents)
    }

    fn shreds(self) -> bool {
        matches!(self, Action::ShredContents | Action::ShredAll)
    }
}

pub struct ActionAll {
    pub action: Action,
    pub all: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Opts {
    pub dry_run: bool,
}

// ---------------------------------------------------------------------------
// Sizes
// ---------------------------------------------------------------------------

const SIZE_UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];

/// Recursively sum the size of regular files under `path`.
///
/// Symlinks are never followed and never counted, so a link pointing at a large
/// tree cannot inflate the figure. Hardlinks are counted once per directory
/// entry, which can overstate a tree containing many links to one file.
pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    for entry in WalkDir::new(path).follow_links(false).into_iter().flatten() {
        if entry.path() == path {
            continue;
        }
        if is_regular_file(&entry) {
            total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    total
}

pub fn human_size(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < SIZE_UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{}{}", bytes, SIZE_UNITS[0])
    } else {
        format!("{:.1}{}", value, SIZE_UNITS[unit])
    }
}

/// Parse a human size such as `500M`, `1.5G`, `2TiB`. A bare number is bytes.
pub fn parse_size(input: &str) -> Option<u64> {
    let text = input.trim().to_ascii_uppercase();
    if text.is_empty() {
        return None;
    }
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    // Accept "M", "MB", "MiB" and friends: drop a trailing "B", then "I".
    let suffix = unit.trim().trim_end_matches('B').trim_end_matches('I');
    let multiplier: u64 = match suffix {
        "" => 1,
        "K" => 1024,
        "M" => 1024 * 1024,
        "G" => 1024 * 1024 * 1024,
        "T" => 1024u64 * 1024 * 1024 * 1024,
        _ => return None,
    };
    let value: f64 = number.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * multiplier as f64) as u64)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn human_age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{}s ago", s),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

// ---------------------------------------------------------------------------
// Markers
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Marker {
    pub policy: Option<String>,
    pub marked_at: Option<u64>,
    pub size: Option<u64>,
}

impl Marker {
    /// True when this marker was written by `purgable mark`, as opposed to a
    /// marker a human created by hand (which is just an empty file).
    pub fn is_ours(&self) -> bool {
        self.policy.is_some()
    }

    /// Human-readable provenance for the review prompt.
    pub fn describe(&self, now: u64) -> String {
        match (&self.policy, self.marked_at) {
            (Some(policy), Some(at)) => {
                let age = now.saturating_sub(at);
                format!("{}, {}", policy, human_age(age))
            }
            (Some(policy), None) => policy.clone(),
            _ => "manual".to_string(),
        }
    }
}

pub fn marker_path(dir: &Path) -> PathBuf {
    dir.join(TARGET)
}

/// Read the marker in `dir`.
///
/// A missing or empty file yields a default `Marker`, which is how a
/// hand-placed marker is represented. Only files carrying the magic first line
/// are treated as tool-written.
pub fn read_marker(dir: &Path) -> Marker {
    let mut marker = Marker::default();
    let Ok(text) = fs::read_to_string(marker_path(dir)) else {
        return marker;
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MARKER_MAGIC) {
        return marker;
    }
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "policy" => marker.policy = Some(value.to_string()),
            "marked_at" => marker.marked_at = value.parse().ok(),
            "size" => marker.size = value.parse().ok(),
            _ => {}
        }
    }
    marker
}

pub fn write_marker(dir: &Path, policy: &str, size: u64, at: u64) -> io::Result<()> {
    let body = format!(
        "{}\npolicy={}\nmarked_at={}\nsize={}\n",
        MARKER_MAGIC, policy, at, size
    );
    fs::write(marker_path(dir), body)
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    /// Policies in file order; the first match wins, so put specific rules first.
    #[serde(default, rename = "policy")]
    pub policies: Vec<Policy>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Defaults {
    #[serde(default)]
    pub min_size: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct Policy {
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Exact directory name this policy applies to.
    #[serde(default)]
    pub dir_name: Option<String>,
    /// Any one of these directory names.
    #[serde(default)]
    pub dir_name_any: Vec<String>,
    /// All of these must exist in the parent directory. This is the guard that
    /// stops a source tree that merely happens to be called `target` from being
    /// treated as build output.
    #[serde(default)]
    pub require_sibling: Vec<String>,
    /// At least one of these must exist in the parent directory.
    #[serde(default)]
    pub require_sibling_any: Vec<String>,
    /// At least one of these must exist inside the directory.
    #[serde(default)]
    pub require_child_any: Vec<String>,
    /// Overrides `defaults.min_size` for this policy.
    #[serde(default)]
    pub min_size: Option<String>,
}

impl Policy {
    fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    fn name_matches(&self, name: &str) -> bool {
        if let Some(exact) = &self.dir_name {
            if exact == name {
                return true;
            }
        }
        self.dir_name_any.iter().any(|candidate| candidate == name)
    }

    fn parent_ok(&self, parent: &Path) -> bool {
        for required in &self.require_sibling {
            if !parent.join(required).exists() {
                return false;
            }
        }
        if !self.require_sibling_any.is_empty() {
            let any = self
                .require_sibling_any
                .iter()
                .any(|candidate| parent.join(candidate).exists());
            if !any {
                return false;
            }
        }
        true
    }

    fn child_ok(&self, dir: &Path) -> bool {
        if self.require_child_any.is_empty() {
            return true;
        }
        self.require_child_any
            .iter()
            .any(|candidate| dir.join(candidate).exists())
    }

    /// Resolve this policy's minimum size, preferring the policy override.
    fn effective_min(&self, default_min: Option<u64>) -> Option<u64> {
        self.min_size
            .as_deref()
            .map(parse_size)
            .unwrap_or(default_min)
    }
}

pub fn parse_config(text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e| format!("invalid config: {}", e))
}

pub fn config_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os("PURGABLE_CONFIG") {
        return PathBuf::from(explicit);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config").join("purgable.toml")
}

pub fn load_config(path: &Path) -> Result<Config, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    parse_config(&text)
}

pub fn starter_config() -> String {
    r#"# purgable policies - ~/.config/purgeable.toml
#
# `purgable mark <root>` walks the tree and drops a PURGABLE marker in every
# directory a policy matches. `purgable review <root>` then asks what to do.
#
# Policies are tried in the order below and the first match wins, so put
# specific rules above general ones.

[defaults]
# Skip anything smaller than this unless a policy overrides it.
min_size = "500M"
enabled = true

# Cargo build output. require_sibling is the important part: a directory named
# `target` is only build output if its parent has a Cargo.toml. Without this,
# real source trees such as linux/kernel/drivers/target get matched too.
[[policy]]
name = "cargo-target"
dir_name = "target"
require_sibling = ["Cargo.toml"]
require_child_any = [".rustc_info.json", "debug", "release", "CACHE"]

# Installed npm packages, regenerable with `npm install`.
[[policy]]
name = "node-modules"
dir_name = "node_modules"
require_sibling = ["package.json"]

# Python virtualenvs, regenerable with `python -m venv`.
[[policy]]
name = "python-venv"
dir_name_any = [".venv", "venv"]
require_sibling_any = ["pyproject.toml", "requirements.txt", "setup.py", "Pipfile"]
"#
    .to_string()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

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

pub struct Candidate {
    pub path: PathBuf,
    pub policy: String,
    pub size: u64,
}

/// Test one directory against the policies, returning the first match.
fn match_policy(dir: &Path, policies: &[&Policy], default_min: Option<u64>) -> Option<Candidate> {
    let name = dir.file_name()?.to_str()?;
    for policy in policies {
        if !policy.is_enabled() || !policy.name_matches(name) {
            continue;
        }
        let parent = dir.parent()?;
        if !policy.parent_ok(parent) || !policy.child_ok(dir) {
            continue;
        }
        let size = dir_size(dir);
        if let Some(min) = policy.effective_min(default_min) {
            if size < min {
                continue;
            }
        }
        return Some(Candidate {
            path: dir.to_path_buf(),
            policy: policy.name.clone(),
            size,
        });
    }
    None
}

/// Walk `dir` collecting policy matches. Matched directories are not descended
/// into, so a build tree never yields nested candidates.
fn scan_for_policies(
    dir: &Path,
    policies: &[&Policy],
    default_min: Option<u64>,
    found: &mut Vec<Candidate>,
    warn: &mut impl io::Write,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            let _ = writeln!(warn, "warning: cannot read {}: {}", dir.display(), e);
            return;
        }
    };
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
        if let Some(candidate) = match_policy(&path, policies, default_min) {
            found.push(candidate);
            continue;
        }
        scan_for_policies(&path, policies, default_min, found, warn);
    }
}

// ---------------------------------------------------------------------------
// mark
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn mark(
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

// ---------------------------------------------------------------------------
// unmark
// ---------------------------------------------------------------------------

pub fn unmark(
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

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

pub fn list(root: &str, out: &mut impl io::Write, warn: &mut impl io::Write) -> io::Result<()> {
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

// ---------------------------------------------------------------------------
// review
// ---------------------------------------------------------------------------

pub fn parse_action(s: &str) -> (Action, bool, bool) {
    let mut all = false;
    let s = if let Some(stripped) = s.strip_suffix("-all") {
        all = true;
        stripped
    } else {
        s
    };
    match s {
        "d" => (Action::DeleteAll, all, true),
        "c" => (Action::ClearContents, all, true),
        "s" => (Action::ShredContents, all, true),
        "x" => (Action::ShredAll, all, true),
        "k" => (Action::Skip, all, true),
        "e" => (Action::Exit, all, true),
        _ => (Action::Skip, false, false),
    }
}

pub fn purge(
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

    let mut default_action: Option<ActionAll> = None;

    for (index, (size, dir, provenance)) in rows.iter().enumerate() {
        let index = index + 1;

        // Once a -ALL action is set there is nothing left to ask, but the user
        // still needs to see what is happening to the remaining directories.
        if let Some(da) = &default_action {
            let word = match da.action {
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
            da.action
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
            default_action = Some(ActionAll { action, all: true });
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

/// Outcome of one row, which decides the colour and wording of its summary.
enum RowState {
    Preview,
    Done(Action),
    Skipped,
}

/// One line in a listing: counter, size, outcome, then the shortened path.
struct Row<'a> {
    index: usize,
    count: usize,
    size: u64,
    dir: &'a Path,
    provenance: &'a str,
    state: RowState,
}

fn write_row(out: &mut impl Write, style: &Style, row: Row<'_>) -> io::Result<()> {
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
fn prompt(
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

/// Pad a styled string to `width` visible columns, ignoring escape codes.
fn pad_to(text: &str, width: usize) -> String {
    let visible = visible_width(text);
    if visible >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width - visible))
}

// ---------------------------------------------------------------------------
// Filesystem actions
// ---------------------------------------------------------------------------

fn remove_dir(path: &Path) -> io::Result<()> {
    fs::remove_dir_all(path)
}

/// Delete everything inside `path`, keeping `path` itself and its marker.
fn clear_dir(path: &Path) -> io::Result<()> {
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
fn remove_any(path: &Path) -> io::Result<()> {
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
fn shred_dir(path: &Path) -> io::Result<()> {
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

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

const VERSION: &str = "v2.0";

fn usage() {
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

fn fail(message: &str, code: i32) -> ! {
    eprintln!("error: {}", message);
    std::process::exit(code)
}

struct Invocation {
    command: String,
    root: Option<String>,
    policies: Vec<String>,
    policy_filter: Option<String>,
    min_size: Option<String>,
    all: bool,
    force: bool,
    dry_run: bool,
}

fn parse_args(args: &[String]) -> Invocation {
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

fn run() -> i32 {
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

fn main() {
    std::process::exit(run());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn write_dir_with_content(dir: &Path, files: &[(&str, &str)]) {
        fs::create_dir_all(dir).unwrap();
        for (name, content) in files {
            // Names may contain a path, e.g. "nested/deep.txt".
            write_file(&dir.join(name), content);
        }
    }

    fn opts() -> Opts {
        Opts::default()
    }

    // -- discovery ----------------------------------------------------------

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

    // -- sizes --------------------------------------------------------------

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("500M"), Some(500 * 1024 * 1024));
        assert_eq!(parse_size("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size("1.5G"), Some(1536 * 1024 * 1024));
        assert_eq!(parse_size("2TiB"), Some(2 * 1024 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("12Q"), None);
    }

    #[test]
    fn test_human_size() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(999), "999B");
        assert_eq!(human_size(1024), "1.0K");
        assert_eq!(human_size(500 * 1024 * 1024), "500.0M");
        assert_eq!(human_size(7 * 1024 * 1024 * 1024), "7.0G");
    }

    #[test]
    fn test_dir_size_counts_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("a/big.bin"), &"x".repeat(2048));
        write_file(&root.join("a/b/small.bin"), &"y".repeat(1024));
        assert_eq!(dir_size(root), 3072);
    }

    #[test]
    fn test_dir_size_ignores_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(&root.join("real.bin"), &"x".repeat(4096));
        symlink(root.join("real.bin"), root.join("link.bin")).unwrap();
        assert_eq!(dir_size(root), 4096);
    }

    // -- markers ------------------------------------------------------------

    #[test]
    fn test_write_and_read_marker_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_marker(root, "cargo-target", 4096, 1_700_000_000).unwrap();

        let marker = read_marker(root);
        assert!(marker.is_ours());
        assert_eq!(marker.policy.as_deref(), Some("cargo-target"));
        assert_eq!(marker.size, Some(4096));
        assert_eq!(marker.marked_at, Some(1_700_000_000));
    }

    #[test]
    fn test_hand_written_marker_is_not_ours() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(TARGET), "").unwrap();
        let marker = read_marker(root);
        assert!(!marker.is_ours());
        assert_eq!(marker.describe(0), "manual");
    }

    #[test]
    fn test_marker_missing_reads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_marker(dir.path()), Marker::default());
    }

    // -- config -------------------------------------------------------------

    #[test]
    fn test_parse_starter_config() {
        let config = parse_config(&starter_config()).unwrap();
        assert!(config.defaults.enabled);
        assert_eq!(config.defaults.min_size.as_deref(), Some("500M"));
        assert_eq!(config.policies.len(), 3);
        assert_eq!(config.policies[0].name, "cargo-target");
    }

    #[test]
    fn test_parse_config_min_size_override() {
        let config = parse_config(
            r#"
            [defaults]
            min_size = "100M"

            [[policy]]
            name = "big-only"
            dir_name = "target"
            require_sibling = ["Cargo.toml"]
            min_size = "2G"
            "#,
        )
        .unwrap();
        let policies = [&config.policies[0]];
        assert_eq!(
            policies[0].effective_min(Some(123)),
            Some(2 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn test_parse_config_rejects_garbage() {
        assert!(parse_config("this is not toml = = =").is_err());
    }

    // -- mark ---------------------------------------------------------------

    /// Build a fake cargo project whose target dir is `bytes` large.
    fn fake_cargo_project(root: &Path, name: &str, bytes: usize) -> PathBuf {
        let project = root.join(name);
        write_file(&project.join("Cargo.toml"), "[package]\nname=\"x\"\n");
        write_file(&project.join("target/debug/blob"), &"x".repeat(bytes));
        project
    }

    const CARGO_CONFIG: &str = r#"
        [defaults]
        min_size = "1M"

        [[policy]]
        name = "cargo-target"
        dir_name = "target"
        require_sibling = ["Cargo.toml"]
        require_child_any = [".rustc_info.json", "debug", "release", "CACHE"]
    "#;

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

    /// The regression that motivated require_sibling: a source directory that
    /// merely shares the name `target` must never be marked.
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

    // -- unmark -------------------------------------------------------------

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

    // -- action parsing -----------------------------------------------------

    #[test]
    fn test_parse_action() {
        assert_eq!(parse_action("d"), (Action::DeleteAll, false, true));
        assert_eq!(parse_action("c"), (Action::ClearContents, false, true));
        assert_eq!(parse_action("s"), (Action::ShredContents, false, true));
        assert_eq!(parse_action("x"), (Action::ShredAll, false, true));
        assert_eq!(parse_action("k"), (Action::Skip, false, true));
        assert_eq!(parse_action("e"), (Action::Exit, false, true));
    }

    #[test]
    fn test_parse_action_all_suffix() {
        assert_eq!(parse_action("d-all"), (Action::DeleteAll, true, true));
        assert_eq!(parse_action("c-all"), (Action::ClearContents, true, true));
        assert_eq!(parse_action("x-all"), (Action::ShredAll, true, true));
    }

    #[test]
    fn test_parse_action_rejects_unknown() {
        let (action, all, ok) = parse_action("q");
        assert_eq!(action, Action::Skip);
        assert!(!all);
        assert!(!ok);
    }

    // -- review actions -----------------------------------------------------

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

    // -- shred primitives ---------------------------------------------------

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

    #[test]
    fn test_validate_root_rejects_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("file.txt");
        fs::write(&f, "x").unwrap();
        assert!(validate_root(f.to_str().unwrap()).is_err());
    }

    // -- presentation -------------------------------------------------------

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

    // -- argument parsing ---------------------------------------------------

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
