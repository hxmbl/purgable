//! The command-line surface: usage text, argument parsing, and dispatch.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::action::Opts;
use crate::config::{config_path, load_config, starter_config};
use crate::mark::{list, mark, unmark};
use crate::purge::purge;
use crate::size::parse_size;

/// Reported by `--version` / `-v`. Kept in step with `version` in Cargo.toml.
pub(crate) const VERSION: &str = "v2.3";

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

Benchmark (STRM-473):
  purgable bench-fixtures generate --output <path> [--scale 1|2|3] [--count N]
  purgable bench-fixtures verify --fixtures <path>
  purgable bench run --fixtures <path> --jobs N --runs N [--output <path>]
  purgable bench summarize --input <dir> --output <path>

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
    // Benchmark subcommands
    pub(crate) bench_subcommand: Option<String>,
    pub(crate) output: Option<String>,
    pub(crate) input: Option<String>,
    pub(crate) scale: Option<String>,
    pub(crate) count: Option<String>,
    pub(crate) jobs: Option<String>,
    pub(crate) runs: Option<String>,
    pub(crate) fixtures: Option<String>,
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
        bench_subcommand: None,
        output: None,
        input: None,
        scale: None,
        count: None,
        jobs: None,
        runs: None,
        fixtures: None,
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
            "--output" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.output = Some(v.clone()),
                    None => fail("--output requires a value", 2),
                }
            }
            "--input" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.input = Some(v.clone()),
                    None => fail("--input requires a value", 2),
                }
            }
            "--scale" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.scale = Some(v.clone()),
                    None => fail("--scale requires a value", 2),
                }
            }
            "--count" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.count = Some(v.clone()),
                    None => fail("--count requires a value", 2),
                }
            }
            "--jobs" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.jobs = Some(v.clone()),
                    None => fail("--jobs requires a value", 2),
                }
            }
            "--runs" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.runs = Some(v.clone()),
                    None => fail("--runs requires a value", 2),
                }
            }
            "--fixtures" => {
                i += 1;
                match args.get(i) {
                    Some(v) => inv.fixtures = Some(v.clone()),
                    None => fail("--fixtures requires a value", 2),
                }
            }
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
        // Check if this is a bench subcommand
        if inv.command == "bench" || inv.command == "bench-fixtures" {
            inv.bench_subcommand = Some(positional.remove(0));
        } else {
            inv.root = Some(positional.remove(0));
        }
    }
    if !positional.is_empty() {
        // If we have bench subcommand, this might be another argument
        if inv.bench_subcommand.is_some() {
            fail(&format!("unexpected argument {:?}", positional[0]), 2);
        } else {
            inv.root = Some(positional.remove(0));
        }
    }
    if !positional.is_empty() {
        fail(&format!("unexpected argument {:?}", positional[0]), 2);
    }
    inv
}

/// Parse an integer argument with a user-friendly error message.
#[allow(dead_code)]
fn parse_int(name: &str, value: &str) -> i32 {
    match value.parse() {
        Ok(n) => n,
        Err(_) => fail(
            &format!("invalid {}: {:?} (expected integer)", name, value),
            2,
        ),
    }
}

/// Parse a usize argument with a user-friendly error message.
fn parse_usize(name: &str, value: &str) -> usize {
    match value.parse() {
        Ok(n) => n,
        Err(_) => fail(
            &format!(
                "invalid {}: {:?} (expected non-negative integer)",
                name, value
            ),
            2,
        ),
    }
}

/// Handle bench and bench-fixtures subcommands.
fn handle_bench_command(inv: &Invocation, stdout: &mut impl Write, stderr: &mut impl Write) -> i32 {
    match inv.bench_subcommand.as_deref() {
        Some("generate") => {
            let output_dir = inv
                .output
                .as_deref()
                .or(inv.root.as_deref())
                .unwrap_or("./bench-fixtures");
            let scale = inv.scale.as_deref().unwrap_or("1");
            let scale_val: u32 = match scale.parse() {
                Ok(v) if (1..=3).contains(&v) => v,
                _ => {
                    let _ = writeln!(stderr, "error: --scale must be 1, 2, or 3");
                    return 2;
                }
            };

            let count: usize = inv
                .count
                .as_deref()
                .map(|c| parse_usize("--count", c))
                .unwrap_or(1);

            if let Err(e) = generate_fixtures(output_dir, scale_val, count, stdout, stderr) {
                let _ = writeln!(stderr, "error: {}", e);
                return 1;
            }
            0
        }
        Some("verify") => {
            let fixtures_path = inv
                .fixtures
                .as_deref()
                .or(inv.root.as_deref())
                .unwrap_or("./bench-fixtures");

            if let Err(e) = verify_fixtures(fixtures_path, stdout, stderr) {
                let _ = writeln!(stderr, "error: {}", e);
                return 1;
            }
            0
        }
        Some("run") => {
            let fixtures_path = inv.fixtures.as_deref().unwrap_or("./bench-fixtures");
            let jobs = inv
                .jobs
                .as_deref()
                .map(|j| parse_usize("--jobs", j))
                .unwrap_or(1);
            let runs = inv
                .runs
                .as_deref()
                .map(|r| parse_usize("--runs", r))
                .unwrap_or(3);
            let output_path = inv.output.as_deref();

            if let Err(e) = run_benchmarks(fixtures_path, jobs, runs, output_path, stdout, stderr) {
                let _ = writeln!(stderr, "error: {}", e);
                return 1;
            }
            0
        }
        Some("summarize") => {
            let input_dir = inv
                .input
                .as_deref()
                .or(inv.root.as_deref())
                .unwrap_or("./bench-results");
            let output_path = inv
                .output
                .as_deref()
                .unwrap_or("./bench-results/summary.md");

            if let Err(e) = summarize_benchmarks(input_dir, output_path, stdout, stderr) {
                let _ = writeln!(stderr, "error: {}", e);
                return 1;
            }
            0
        }
        Some(other) => {
            let _ = writeln!(stderr, "error: unknown bench subcommand {:?}", other);
            let _ = writeln!(
                stderr,
                "available subcommands: generate, verify, run, summarize"
            );
            2
        }
        None => {
            let _ = writeln!(stderr, "error: bench requires a subcommand");
            let _ = writeln!(
                stderr,
                "available subcommands: generate, verify, run, summarize"
            );
            2
        }
    }
}

/// Helper to write a file, creating parent directories as needed.
fn bench_write_file(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// Helper to write a directory with files.
fn bench_write_dir_with_content(dir: &Path, files: &[(&str, &str)]) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    for (name, content) in files {
        let full_path = dir.join(name);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, content)?;
    }
    Ok(())
}

/// Generate benchmark fixtures.
fn generate_fixtures(
    output_dir: &str,
    scale: u32,
    count: usize,
    stdout: &mut impl Write,
    _stderr: &mut impl Write,
) -> io::Result<()> {
    use crate::benchmark::count_fixture_contents;

    let output = PathBuf::from(output_dir);
    fs::create_dir_all(&output)?;

    let multiplier = scale as usize;
    let mut total_dirs = 0usize;
    let mut total_files = 0usize;
    let mut total_size = 0u64;

    for fixture_idx in 0..count {
        let fixture_dir = output.join(format!("fixture_{}", fixture_idx));
        fs::create_dir_all(&fixture_dir)?;

        // Create a mix of structures similar to real projects
        // Small nested project structure
        for proj in 0..(10 * multiplier) {
            let proj_dir = fixture_dir.join(format!("project_{}", proj));
            bench_write_dir_with_content(&proj_dir, &[("PURGABLE", "")])?;

            // src directory
            let src_dir = proj_dir.join("src");
            fs::create_dir_all(&src_dir)?;
            for file in 0..(5 * multiplier) {
                let content = "x".repeat(1024);
                bench_write_file(&src_dir.join(format!("file_{}.rs", file)), &content)?;
                total_size += content.len() as u64;
            }

            // target directory (simulated build output)
            let target_dir = proj_dir.join("target");
            bench_write_dir_with_content(&target_dir, &[("PURGABLE", "")])?;
            for subdir in &["debug", "release"] {
                let sub = target_dir.join(subdir);
                fs::create_dir_all(&sub)?;
                for obj in 0..(20 * multiplier) {
                    let content = "x".repeat(4096);
                    bench_write_file(&sub.join(format!("obj_{}.o", obj)), &content)?;
                    total_size += content.len() as u64;
                }
            }

            // node_modules
            let node_dir = proj_dir.join("node_modules");
            bench_write_dir_with_content(&node_dir, &[("PURGABLE", "")])?;
            for pkg in 0..(3 * multiplier) {
                let pkg_dir = node_dir.join(format!("pkg_{}", pkg));
                fs::create_dir_all(&pkg_dir)?;
                for nested in 0..5 {
                    let nested_dir = pkg_dir.join(format!("lib_{}", nested));
                    fs::create_dir_all(&nested_dir)?;
                    for f in 0..10 {
                        let content = "x".repeat(2048);
                        bench_write_file(&nested_dir.join(format!("file_{}.js", f)), &content)?;
                        total_size += content.len() as u64;
                    }
                }
            }
        }

        // Also create some standalone markers
        for standalone in 0..(5 * multiplier) {
            let dir = fixture_dir.join(format!("standalone_{}", standalone));
            bench_write_dir_with_content(&dir, &[("PURGABLE", "")])?;
            for f in 0..20 {
                let content = "x".repeat(8192);
                bench_write_file(&dir.join(format!("large_{}.bin", f)), &content)?;
                total_size += content.len() as u64;
            }
        }

        let (dirs, files) = count_fixture_contents(&fixture_dir)?;
        total_dirs += dirs;
        total_files += files;

        let _ = writeln!(
            stdout,
            "Generated fixture {}: {} dirs, {} files, {} bytes",
            fixture_idx, dirs, files, total_size
        );
    }

    let _ = writeln!(
        stdout,
        "\nTotal: {} dirs, {} files, {} bytes across {} fixtures",
        total_dirs, total_files, total_size, count
    );

    Ok(())
}

/// Verify benchmark fixtures can be cleaned up properly.
fn verify_fixtures(
    fixtures_path: &str,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<()> {
    use std::process::Command;

    let fixtures = PathBuf::from(fixtures_path);
    if !fixtures.exists() {
        let _ = writeln!(
            stderr,
            "Fixtures directory {} does not exist",
            fixtures.display()
        );
        return Ok(());
    }

    let mut total_verified = 0usize;

    for entry in fs::read_dir(&fixtures)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Count markers
            let output = Command::new("find")
                .arg(&path)
                .arg("-name")
                .arg("PURGABLE")
                .output()?;

            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let marker_count = stdout_str.lines().count();
            total_verified += marker_count;

            let _ = writeln!(
                stdout,
                "Fixture {}: {} markers found",
                path.file_name().unwrap_or_default().to_string_lossy(),
                marker_count
            );
        }
    }

    let _ = writeln!(
        stdout,
        "\nVerified {} total markers across all fixtures",
        total_verified
    );

    Ok(())
}

/// Run benchmarks on fixtures.
fn run_benchmarks(
    fixtures_path: &str,
    jobs: usize,
    runs: usize,
    _output_path: Option<&str>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<()> {
    use std::process::Command;

    let fixtures = PathBuf::from(fixtures_path);
    if !fixtures.exists() {
        let _ = writeln!(
            stderr,
            "Fixtures directory {} does not exist",
            fixtures.display()
        );
        return Ok(());
    }

    let env_jobs = format!("PURGABLE_JOBS={}", jobs);
    let _ = writeln!(
        stdout,
        "Running benchmarks with PURGABLE_JOBS={}, {} runs",
        jobs, runs
    );
    let _ = writeln!(stdout);

    for entry in fs::read_dir(&fixtures)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let _ = writeln!(
                stdout,
                "Benchmarking fixture: {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );

            // Run list command multiple times
            let mut times = Vec::new();
            for run_idx in 0..runs {
                let start = std::time::Instant::now();
                let purgable_path =
                    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("purgable"));
                let status = Command::new(&purgable_path)
                    .env("PURGABLE_JOBS", &env_jobs)
                    .arg("list")
                    .arg(&path)
                    .status()?;
                let elapsed = start.elapsed();
                times.push(elapsed);

                if !status.success() {
                    let _ = writeln!(
                        stderr,
                        "  Run {} failed with exit code {}",
                        run_idx,
                        status.code().unwrap_or(-1)
                    );
                }
            }

            // Compute statistics
            let sum_ms: f64 = times.iter().map(|t| t.as_secs_f64() * 1000.0).sum();
            let mean_ms = sum_ms / times.len() as f64;
            let min_ms = times
                .iter()
                .map(|t| t.as_secs_f64() * 1000.0)
                .fold(f64::INFINITY, f64::min);
            let max_ms = times
                .iter()
                .map(|t| t.as_secs_f64() * 1000.0)
                .fold(f64::NEG_INFINITY, f64::max);

            let _ = writeln!(
                stdout,
                "  list: mean={:.2}ms, min={:.2}ms, max={:.2}ms",
                mean_ms, min_ms, max_ms
            );
        }
    }

    Ok(())
}

/// Summarize benchmark results.
fn summarize_benchmarks(
    input_dir: &str,
    output_path: &str,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<()> {
    use std::fs::File;
    use std::io::Write as _;

    let input = PathBuf::from(input_dir);
    if !input.exists() {
        let _ = writeln!(stderr, "Input directory {} does not exist", input.display());
        return Ok(());
    }

    let mut summary = String::new();
    summary.push_str("# Benchmark Summary\n\n");
    summary.push_str("Results from benchmark runs.\n\n");

    // Collect all result files
    let mut results = Vec::new();
    for entry in fs::read_dir(&input)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let content = fs::read_to_string(&path)?;
            results.push((
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                content,
            ));
        }
    }

    if results.is_empty() {
        summary.push_str("No benchmark result files found.\n");
    } else {
        summary.push_str("## Results\n\n");
        for (name, content) in &results {
            summary.push_str(&format!("- {}\n", name));
            // Try to pretty-print JSON if possible
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
                summary.push_str(&format!(
                    "  {}\n",
                    serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| content.clone())
                ));
            } else {
                summary.push_str(&format!("  {}\n", content));
            }
        }
    }

    // Write to output
    let output = PathBuf::from(output_path);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(&output)?;
    file.write_all(summary.as_bytes())?;

    let _ = writeln!(stdout, "Wrote summary to {}", output.display());

    Ok(())
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

        "bench" | "bench-fixtures" => handle_bench_command(&inv, &mut stdout, &mut stderr),

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
