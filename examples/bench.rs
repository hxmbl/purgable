//! STRM-473 benchmark runner. Not part of the shipped binary.
//!
//! Drives a built `purgable` binary as a black box over deterministic
//! synthetic trees, so nothing here touches the hot path.
//!
//!   cargo build --release
//!   cargo run --release --example bench -- run --out bench-results
//!   cargo run --release --example bench -- gen --workload broad --dir /tmp/fx
//!   cargo run --release --example bench -- list-workloads
//!
//! See BENCHMARKS.md for what is measured, the validation rules, and the
//! warm-cache assumptions.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_json::{json, Value};

const MARKER: &str = "PURGABLE";
/// Directory name the bench policy matches, so `mark --dry-run` has work to do.
const CAND: &str = "build_out";
const SIZES: [usize; 5] = [512, 1024, 2048, 4096, 8192];

const WORKLOADS: &[(&str, &str)] = &[
    (
        "historical",
        "32,503 files / 4,099 dirs, broad and shallow (the AGENTS.md baseline shape)",
    ),
    ("broad", "balanced broad tree, fan-out 8"),
    (
        "high_fanout",
        "one directory with more than 4,096 subdirectories (above the walk queue cap)",
    ),
    (
        "deep_chain",
        "a 1,000-level directory chain, candidate halfway down",
    ),
    ("file_heavy", "few directories, many files"),
    ("many_small_roots", "thousands of small candidate roots"),
    (
        "few_huge_roots",
        "four large candidate roots with internal trees",
    ),
    (
        "nested_markers",
        "candidates containing further nested markers",
    ),
    (
        "wide_dir",
        "one candidate directory holding every file flat",
    ),
];

// ---------------------------------------------------------------- fixtures

#[derive(Default, Clone)]
struct Shape {
    dirs: u64,
    files: u64,
    bytes: u64,
    cand_roots: u64,
    markers: u64,
    cand_files: u64,
    cand_dirs: u64,
    out_files: u64,
    out_dirs: u64,
}

impl Shape {
    fn to_json(&self, workload: &str, scale: f64) -> Value {
        json!({
            "workload": workload, "scale": scale,
            "directories": self.dirs, "files": self.files, "bytes": self.bytes,
            "candidate_roots": self.cand_roots,
            "marker_files": self.markers,
            "files_in_candidates": self.cand_files,
            "dirs_in_candidates": self.cand_dirs,
            "files_outside_candidates": self.out_files,
            "dirs_outside_candidates": self.out_dirs,
        })
    }
}

struct Builder(Shape);

impl Builder {
    fn mkdir(&mut self, p: &Path, in_cand: bool) -> io::Result<()> {
        fs::create_dir(p)?;
        self.0.dirs += 1;
        if in_cand {
            self.0.cand_dirs += 1;
        } else {
            self.0.out_dirs += 1;
        }
        Ok(())
    }
    fn file(&mut self, p: &Path, size: usize, in_cand: bool) -> io::Result<()> {
        if p.file_name().is_some_and(|n| n == MARKER) {
            self.0.markers += 1;
        }
        let byte = (size % 251) as u8;
        fs::write(p, vec![byte; size])?;
        self.0.files += 1;
        self.0.bytes += size as u64;
        if in_cand {
            self.0.cand_files += 1;
        } else {
            self.0.out_files += 1;
        }
        Ok(())
    }
    /// A marked directory that is the outermost candidate of its subtree.
    fn cand_root(&mut self, p: &Path) -> io::Result<()> {
        self.mkdir(p, true)?;
        self.file(&p.join(MARKER), 0, true)?;
        self.0.cand_roots += 1;
        Ok(())
    }
}

fn n(base: usize, scale: f64) -> usize {
    ((base as f64 * scale).round() as usize).max(1)
}

/// Index-addressed k-ary directory tree plus round-robin files.
/// Directory `i` has parent `(i-1)/fanout`; directories at `cand_depth` whose
/// index is a multiple of `cand_mod` become candidate roots. `total_files`
/// includes marker files, so shapes can be exact.
#[allow(clippy::too_many_arguments, clippy::manual_is_multiple_of)]
fn tree(
    b: &mut Builder,
    base: &Path,
    base_in_cand: bool,
    dirs: usize,
    total_files: usize,
    fanout: usize,
    cand: Option<(usize, usize)>,
) -> io::Result<()> {
    let mut paths: Vec<PathBuf> = vec![base.to_path_buf()];
    let mut depth: Vec<usize> = vec![0];
    let mut in_cand: Vec<bool> = vec![base_in_cand];
    let mut roots = 0usize;
    let mut made = 0usize;
    let mut i = 0usize;
    while made < dirs {
        i += 1;
        let parent = (i - 1) / fanout;
        let d = depth[parent] + 1;
        let parent_in = in_cand[parent];
        // `dirs` is the exact directory total, wrappers included
        let is_root =
            !parent_in && made + 2 <= dirs && cand.is_some_and(|(cd, cm)| d == cd && i % cm == 0);
        let name = if is_root {
            CAND.to_string()
        } else {
            format!("d{i}")
        };
        let mut p = paths[parent].join(name);
        if is_root {
            // siblings cannot share the candidate name, so each gets a wrapper
            let wrap = paths[parent].join(format!("w{i}"));
            b.mkdir(&wrap, false)?;
            p = wrap.join(CAND);
            b.cand_root(&p)?;
            roots += 1;
            made += 2;
        } else {
            b.mkdir(&p, parent_in)?;
            made += 1;
        }
        paths.push(p);
        depth.push(d);
        in_cand.push(parent_in || is_root);
    }
    let plain = total_files.saturating_sub(roots);
    for j in 0..plain {
        let idx = j % paths.len();
        b.file(
            &paths[idx].join(format!("f{j}.dat")),
            SIZES[j % SIZES.len()],
            in_cand[idx],
        )?;
    }
    Ok(())
}

fn build(workload: &str, scale: f64, root: &Path) -> io::Result<Shape> {
    let mut b = Builder(Shape::default());
    match workload {
        "historical" => {
            // Fixed counts: 4,099 directories and 32,503 files including markers.
            let s = if scale == 1.0 { 1.0 } else { scale };
            tree(
                &mut b,
                root,
                false,
                n(4099, s),
                n(32503, s),
                8,
                Some((3, 4)),
            )?
        }
        "broad" => tree(
            &mut b,
            root,
            false,
            n(2000, scale),
            n(16000, scale),
            8,
            Some((3, 4)),
        )?,
        "high_fanout" => tree(
            &mut b,
            root,
            false,
            n(5200, scale).max(4200),
            n(20000, scale),
            n(5200, scale).max(4200),
            Some((1, 25)),
        )?,
        "deep_chain" => {
            let levels = 1000usize;
            let mut p = root.to_path_buf();
            for i in 0..levels {
                let in_cand = i >= levels / 2;
                if i == levels / 2 {
                    p = p.join(CAND);
                    b.cand_root(&p)?;
                } else {
                    p = p.join("a");
                    b.mkdir(&p, in_cand)?;
                }
                for k in 0..n(2, scale) {
                    b.file(&p.join(format!("f{k}.dat")), SIZES[(i + k) % 5], in_cand)?;
                }
            }
        }
        "file_heavy" => tree(&mut b, root, false, 200, n(60000, scale), 4, Some((2, 3)))?,
        "many_small_roots" => {
            for i in 0..n(3000, scale) {
                let c = root.join(format!("p{i}")).join(CAND);
                b.mkdir(c.parent().unwrap(), false)?;
                b.cand_root(&c)?;
                for (k, size) in SIZES.iter().enumerate() {
                    b.file(&c.join(format!("f{k}.dat")), *size, true)?;
                }
            }
        }
        "few_huge_roots" => {
            for i in 0..4 {
                let wrap = root.join(format!("big{i}"));
                b.mkdir(&wrap, false)?;
                let c = wrap.join(CAND);
                b.cand_root(&c)?;
                tree(&mut b, &c, true, n(1000, scale), n(8000, scale), 6, None)?;
            }
        }
        "nested_markers" => {
            for i in 0..n(200, scale) {
                let wrap = root.join(format!("o{i}"));
                b.mkdir(&wrap, false)?;
                let mut p = wrap.join(CAND);
                b.cand_root(&p)?;
                for lvl in 0..3 {
                    p = p.join(format!("inner{lvl}"));
                    b.mkdir(&p, true)?;
                    b.file(&p.join(MARKER), 0, true)?;
                    for k in 0..20 {
                        b.file(&p.join(format!("f{k}.dat")), SIZES[k % 5], true)?;
                    }
                }
            }
        }
        "wide_dir" => {
            let c = root.join(CAND);
            b.cand_root(&c)?;
            for j in 0..n(30000, scale) {
                b.file(&c.join(format!("f{j}.dat")), SIZES[j % 5], true)?;
            }
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown workload {other}"),
            ))
        }
    }
    Ok(b.0)
}

/// Fresh fixture directory with a shape the caller can check against.
fn fresh(work: &Path, workload: &str, scale: f64) -> io::Result<(PathBuf, Shape)> {
    let dir = work.join(workload);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    let shape = build(workload, scale, &dir)?;
    if shape.cand_roots == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{workload} at scale {scale} has no candidates; raise --scale"),
        ));
    }
    Ok((dir, shape))
}

/// Independent recount of the tree on disk, never following symlinks.
fn census(root: &Path) -> io::Result<(u64, u64)> {
    let (mut dirs, mut files) = (0u64, 0u64);
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in fs::read_dir(&p)? {
            let e = e?;
            let t = e.file_type()?;
            if t.is_dir() {
                dirs += 1;
                stack.push(e.path());
            } else {
                files += 1;
            }
        }
    }
    Ok((dirs, files))
}

// ------------------------------------------------------------------ running

#[derive(Clone, Copy, PartialEq)]
enum Op {
    Mark,
    List,
    Review,
    Delete,
    Shred,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Mark => "mark_dry_run",
            Op::List => "list",
            Op::Review => "review_dry_run",
            Op::Delete => "delete",
            Op::Shred => "shred",
        }
    }
    fn destructive(self) -> bool {
        matches!(self, Op::Delete | Op::Shred)
    }
}

struct Sample {
    ms: f64,
    valid: bool,
    note: String,
}

/// The number written immediately before `word`, e.g. "2 marked" -> 2.
fn count_before(text: &str, word: &str) -> Option<u64> {
    let idx = text.find(word)?;
    text[..idx].split_whitespace().next_back()?.parse().ok()
}

fn run_once(
    bin: &Path,
    cfg: &Path,
    op: Op,
    dir: &Path,
    jobs: usize,
    shape: &Shape,
) -> io::Result<Sample> {
    let mut cmd = Command::new(bin);
    cmd.env("PURGABLE_JOBS", jobs.to_string())
        .env("PURGABLE_CONFIG", cfg)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut stdin_text = "";
    match op {
        Op::Mark => {
            cmd.args(["mark"]).arg(dir).arg("--dry-run");
        }
        Op::List => {
            cmd.args(["list"]).arg(dir);
        }
        Op::Review => {
            cmd.args(["review"]).arg(dir).arg("--dry-run");
        }
        Op::Delete => {
            cmd.args(["review"]).arg(dir);
            stdin_text = "d-ALL\n";
        }
        Op::Shred => {
            cmd.args(["review"]).arg(dir);
            stdin_text = "s-ALL\n";
        }
    }
    cmd.stdin(Stdio::piped());
    let start = Instant::now();
    let mut child = cmd.spawn()?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_text.as_bytes())?;
    let out = child.wait_with_output()?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let text = String::from_utf8_lossy(&out.stdout).to_string();

    let mut problems: Vec<String> = Vec::new();
    if !out.status.success() {
        problems.push(format!("exit status {:?}", out.status.code()));
    }
    let roots = shape.cand_roots;
    match op {
        Op::Mark => {
            if count_before(&text, "matched") != Some(roots) {
                problems.push(format!("expected {roots} matched"));
            }
        }
        Op::Review => {
            let got = count_before(&text, "directories marked")
                .or_else(|| count_before(&text, "directory marked"));
            if got != Some(shape.markers) {
                problems.push(format!("expected {} directories marked", shape.markers));
            }
        }
        Op::List => {
            if count_before(&text, "marked") != Some(shape.markers) {
                problems.push(format!("expected {} marked", shape.markers));
            }
        }
        Op::Delete | Op::Shred => {
            let word = if op == Op::Delete {
                "deleted"
            } else {
                "shredded"
            };
            let done = text.lines().find(|l| l.contains("Done.")).unwrap_or("");
            // With nested markers purgable reviews inner markers as separate
            // rows; once the outer directory is gone those rows are skipped.
            // The filesystem recount below is the authority in that case.
            if shape.markers == roots {
                if count_before(done, word) != Some(roots) {
                    problems.push(format!("expected {roots} {word} in summary"));
                }
                if count_before(done, "skipped") != Some(0) {
                    problems.push("summary reports skipped directories".into());
                }
            }
            // Independent recount of the filesystem: the trap AGENTS.md warns
            // about is a run that reports success having removed nothing.
            let (dirs, files) = census(dir)?;
            let (want_dirs, want_files) = if op == Op::Delete {
                (shape.out_dirs, shape.out_files)
            } else {
                // shredding keeps every marked directory and its marker, nested
                // ones included (the marker is never shredded)
                (
                    shape.out_dirs + shape.markers,
                    shape.out_files + shape.markers,
                )
            };
            if dirs != want_dirs || files != want_files {
                problems.push(format!(
                    "final state {dirs} dirs/{files} files, expected {want_dirs}/{want_files}"
                ));
            }
        }
    }
    Ok(Sample {
        ms,
        valid: problems.is_empty(),
        note: problems.join("; "),
    })
}

fn stats(v: &[f64]) -> Value {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let len = s.len();
    let median = if len == 0 {
        f64::NAN
    } else if len % 2 == 1 {
        s[len / 2]
    } else {
        (s[len / 2 - 1] + s[len / 2]) / 2.0
    };
    let mean = s.iter().sum::<f64>() / len.max(1) as f64;
    let sd = (s.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / len.max(1) as f64).sqrt();
    json!({
        "n": len, "median_ms": median, "mean_ms": mean, "stddev_ms": sd,
        "min_ms": s.first(), "max_ms": s.last(),
        "spread_ms": s.last().zip(s.first()).map(|(a, b)| a - b),
    })
}

// ------------------------------------------------------------- environment

fn sh(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn cpu_model() -> String {
    if let Ok(t) = fs::read_to_string("/proc/cpuinfo") {
        if let Some(l) = t.lines().find(|l| l.starts_with("model name")) {
            return l.split(':').nth(1).unwrap_or("").trim().to_string();
        }
    }
    let m = sh("sysctl", &["-n", "machdep.cpu.brand_string"]);
    if m.is_empty() {
        "unknown".into()
    } else {
        m
    }
}

fn filesystem(path: &Path) -> String {
    let p = path.to_string_lossy().to_string();
    let linux = sh("stat", &["-f", "-c", "%T", &p]);
    if !linux.is_empty() {
        return linux;
    }
    // macOS: the mount line for the device holding the path
    let dev = sh("df", &["-P", &p])
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().next().map(String::from));
    if let Some(dev) = dev {
        for line in sh("mount", &[]).lines() {
            if line.starts_with(&dev) {
                if let (Some(a), Some(b)) = (line.find('('), line.find(')')) {
                    return line[a + 1..b].split(',').next().unwrap_or("").to_string();
                }
            }
        }
    }
    "unknown".into()
}

fn environment(bin: &Path, work: &Path) -> Value {
    json!({
        "commit": sh("git", &["rev-parse", "HEAD"]),
        "dirty": !sh("git", &["status", "--porcelain"]).is_empty(),
        "binary_version": sh(&bin.to_string_lossy(), &["--version"]),
        "os": format!("{} {}", sh("uname", &["-s"]), sh("uname", &["-r"])),
        "arch": sh("uname", &["-m"]),
        "cpu": cpu_model(),
        "logical_cores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "filesystem": filesystem(work),
        "rustc": sh("rustc", &["--version"]),
        "cache": "warm: fixtures are created immediately before each measurement and the page cache is never dropped",
    })
}

// -------------------------------------------------------------------- main

struct Args(BTreeMap<String, String>);

impl Args {
    fn get(&self, k: &str, d: &str) -> String {
        self.0.get(k).cloned().unwrap_or_else(|| d.to_string())
    }
    fn num<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.0.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
}

fn parse(args: &[String]) -> Args {
    let mut m = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        if let Some(k) = args[i].strip_prefix("--") {
            let v = args.get(i + 1).cloned().unwrap_or_default();
            m.insert(k.to_string(), v);
            i += 2;
        } else {
            i += 1;
        }
    }
    Args(m)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match argv.first().map(String::as_str) {
        Some("list-workloads") => {
            for (n, d) in WORKLOADS {
                println!("{n:18} {d}");
            }
            0
        }
        Some("gen") => cmd_gen(&parse(&argv[1..])),
        Some("run") => cmd_run(&parse(&argv[1..])),
        Some("merge") => cmd_merge(&argv[1..]),
        _ => {
            eprintln!("usage: bench <gen|run|merge|list-workloads> [--flag value]...\n  gen --workload NAME --dir PATH [--scale F]\n  run [--bin PATH] [--work PATH] [--out DIR] [--workloads a,b] [--jobs 1,2,4] [--reps 5] [--destructive-reps 3] [--scale F] [--shred-scale F] [--all-jobs true]");
            2
        }
    };
    std::process::exit(code);
}

/// Combine results.json files from partial runs (same machine) into one report.
/// usage: bench merge OUT_DIR IN_DIR...
fn cmd_merge(a: &[String]) -> i32 {
    let (Some(out), ins) = (a.first(), &a[a.len().min(1)..]) else {
        return 2;
    };
    let mut merged: Option<Value> = None;
    for d in ins {
        let Ok(t) = fs::read_to_string(Path::new(d).join("results.json")) else {
            eprintln!("cannot read {d}/results.json");
            return 1;
        };
        let v: Value = serde_json::from_str(&t).unwrap();
        match &mut merged {
            None => merged = Some(v),
            Some(m) => m["results"]
                .as_array_mut()
                .unwrap()
                .extend(v["results"].as_array().unwrap().clone()),
        }
    }
    let Some(m) = merged else { return 2 };
    let _ = fs::create_dir_all(out);
    let _ = fs::write(
        Path::new(out).join("results.json"),
        serde_json::to_string_pretty(&m).unwrap(),
    );
    let _ = fs::write(Path::new(out).join("summary.md"), summary(&m));
    0
}

fn cmd_gen(a: &Args) -> i32 {
    let w = a.get("workload", "historical");
    let dir = PathBuf::from(a.get("dir", "bench-fixtures"));
    let scale: f64 = a.num("scale", 1.0);
    match fresh(&dir, &w, scale).and_then(|(d, s)| census(&d).map(|c| (d, s, c))) {
        Ok((d, s, (cd, cf))) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&s.to_json(&w, scale)).unwrap()
            );
            if (cd, cf) != (s.dirs, s.files) {
                eprintln!("shape mismatch: disk has {cd} dirs/{cf} files");
                return 1;
            }
            println!("fixture at {}", d.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_run(a: &Args) -> i32 {
    match run(a) {
        Ok(invalid) => i32::from(invalid > 0),
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

fn run(a: &Args) -> io::Result<usize> {
    let bin = PathBuf::from(a.get("bin", "target/release/purgable")).canonicalize()?;
    let out = PathBuf::from(a.get("out", "bench-results"));
    let work = PathBuf::from(a.get("work", "bench-fixtures"));
    let reps: usize = a.num("reps", 5);
    let dreps: usize = a.num("destructive-reps", 3);
    let scale: f64 = a.num("scale", 1.0);
    // Shredding syncs every file, so it runs on a smaller tree by default.
    let sscale: f64 = a.num("shred-scale", scale / 10.0);
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let all_jobs = a.get("all-jobs", "false") == "true";
    let jobs: Vec<usize> = a
        .get("jobs", "1,2,4,8,12,16,32")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .filter(|j| all_jobs || *j <= cores)
        .collect();
    let want: Vec<String> = a
        .get("workloads", "all")
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let workloads: Vec<&str> = WORKLOADS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| want.iter().any(|w| w == "all" || w == n))
        .collect();
    let ops = [Op::Mark, Op::List, Op::Review, Op::Delete, Op::Shred];

    fs::create_dir_all(&out)?;
    // Destructive runs delete and recreate directories under --work, so refuse
    // any directory that this runner did not create (or that is not empty).
    let sentinel = work.join(".purgable-bench-workdir");
    if work.exists() && !sentinel.exists() && fs::read_dir(&work)?.next().is_some() {
        return Err(io::Error::other(format!(
            "{} is not empty and was not created by this runner; pass a fresh --work directory",
            work.display()
        )));
    }
    fs::create_dir_all(&work)?;
    fs::write(&sentinel, "created by examples/bench.rs; safe to delete\n")?;
    let work = work.canonicalize()?;
    let cfg = work.join("bench-policy.toml");
    fs::write(
        &cfg,
        format!("[defaults]\nmin_size = \"0\"\nenabled = true\n\n[[policy]]\nname = \"bench\"\ndir_name = \"{CAND}\"\n"),
    )?;

    let env = environment(&bin, &work);
    eprintln!("jobs sweep: {jobs:?} (machine has {cores} logical cores)");
    let mut groups: Vec<Value> = Vec::new();
    let mut csv = String::from("workload,op,jobs,rep,wall_ms,valid,note\n");
    let mut invalid_total = 0usize;

    for w in &workloads {
        for op in ops {
            let s = if op == Op::Shred { sscale } else { scale };
            let n_reps = if op.destructive() { dreps } else { reps };
            // Read-only ops share one fixture; destructive ops get a new one per sample.
            let mut shared: Option<(PathBuf, Shape)> = None;
            if !op.destructive() {
                shared = Some(fresh(&work, w, s)?);
            }
            for &j in &jobs {
                let mut samples: Vec<Sample> = Vec::new();
                if let Some((d, sh)) = &shared {
                    // one discarded warm-up so the first sample is not a cold outlier
                    let _ = run_once(&bin, &cfg, op, d, j, sh)?;
                }
                for _ in 0..n_reps {
                    let sample = if op.destructive() {
                        let (d, sh) = fresh(&work, w, s)?;
                        let (cd, cf) = census(&d)?;
                        if (cd, cf) != (sh.dirs, sh.files) {
                            samples.push(Sample {
                                ms: 0.0,
                                valid: false,
                                note: format!("fixture shape mismatch {cd}/{cf}"),
                            });
                            continue;
                        }
                        let r = run_once(&bin, &cfg, op, &d, j, &sh)?;
                        let _ = fs::remove_dir_all(&d);
                        r
                    } else {
                        let (d, sh) = shared.as_ref().unwrap();
                        run_once(&bin, &cfg, op, d, j, sh)?
                    };
                    samples.push(sample);
                }
                let shape = match &shared {
                    Some((_, sh)) => sh.clone(),
                    None => build_shape_only(&work, w, s)?,
                };
                let good: Vec<f64> = samples.iter().filter(|x| x.valid).map(|x| x.ms).collect();
                for (i, x) in samples.iter().enumerate() {
                    let _ = writeln!(
                        csv,
                        "{w},{},{j},{i},{:.3},{},{}",
                        op.name(),
                        x.ms,
                        x.valid,
                        x.note.replace(',', ";")
                    );
                }
                let bad = samples.len() - good.len();
                invalid_total += bad;
                let mut g = json!({
                    "workload": w, "op": op.name(), "jobs": j, "scale": s,
                    "shape": shape.to_json(w, s),
                    "raw_samples_ms": samples.iter().map(|x| x.ms).collect::<Vec<_>>(),
                    "valid_samples": good.len(), "rejected_samples": bad,
                    "rejection_notes": samples.iter().filter(|x| !x.valid).map(|x| x.note.clone()).collect::<Vec<_>>(),
                    "stats": stats(&good),
                });
                if op.destructive() && !good.is_empty() {
                    let med = g["stats"]["median_ms"].as_f64().unwrap_or(f64::NAN) / 1000.0;
                    g["files_per_s"] = json!(shape.cand_files as f64 / med);
                    g["mib_per_s"] = json!(shape.bytes as f64 / 1048576.0 / med);
                }
                eprintln!(
                    "{w:18} {:15} jobs={j:<3} median={:>10.2}ms valid={}/{}",
                    op.name(),
                    g["stats"]["median_ms"].as_f64().unwrap_or(f64::NAN),
                    good.len(),
                    samples.len()
                );
                groups.push(g);
            }
            if let Some((d, _)) = shared {
                let _ = fs::remove_dir_all(d);
            }
        }
    }

    let report = json!({
        "unix_time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "environment": env,
        "settings": {"reps": reps, "destructive_reps": dreps, "scale": scale, "shred_scale": sscale, "jobs": jobs},
        "results": groups,
    });
    fs::write(
        out.join("results.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )?;
    fs::write(out.join("results.csv"), csv)?;
    fs::write(out.join("summary.md"), summary(&report))?;
    eprintln!(
        "wrote {}/results.json, results.csv, summary.md",
        out.display()
    );
    if invalid_total > 0 {
        eprintln!("{invalid_total} sample(s) were rejected as invalid; see rejection_notes");
    }
    Ok(invalid_total)
}

/// The shape a destructive group measured, rebuilt in a scratch dir (the
/// per-sample fixtures are gone by then). Deterministic, so identical.
fn build_shape_only(work: &Path, w: &str, s: f64) -> io::Result<Shape> {
    let (d, sh) = fresh(work, w, s)?;
    let _ = fs::remove_dir_all(d);
    Ok(sh)
}

fn summary(r: &Value) -> String {
    let e = &r["environment"];
    let mut m = String::from("# Purgable benchmark summary\n\n");
    for k in [
        "commit",
        "binary_version",
        "os",
        "arch",
        "cpu",
        "logical_cores",
        "filesystem",
        "rustc",
        "cache",
    ] {
        let _ = writeln!(
            m,
            "- {k}: {}",
            e[k].as_str()
                .map(String::from)
                .unwrap_or_else(|| e[k].to_string())
        );
    }
    let _ = writeln!(m, "- tree has uncommitted changes: {}\n", e["dirty"]);
    let _ = writeln!(m, "Times are medians of valid samples; spread is max-min. Rejected samples are excluded and counted.\n");
    for (title, destructive, shred) in [
        ("Read-only", false, false),
        ("Ordinary delete", true, false),
        (
            "Shred (reported separately; fdatasync per file)",
            true,
            true,
        ),
    ] {
        let _ = writeln!(m, "## {title}\n");
        let extra = if destructive {
            " files/s | MiB/s |"
        } else {
            ""
        };
        let _ = writeln!(m, "| workload | op | jobs | files | dirs | candidates | n | median ms | min | max | spread |{extra} rejected |");
        let cols = 12 + if destructive { 2 } else { 0 };
        let _ = writeln!(m, "|{}", "---|".repeat(cols));
        for g in r["results"].as_array().unwrap() {
            let op = g["op"].as_str().unwrap();
            let is_d = op == "delete" || op == "shred";
            if is_d != destructive || (op == "shred") != shred {
                continue;
            }
            let st = &g["stats"];
            let f = |v: &Value| v.as_f64().map(|x| format!("{x:.1}")).unwrap_or("-".into());
            let tp = if destructive {
                format!(" {} | {} |", f(&g["files_per_s"]), f(&g["mib_per_s"]))
            } else {
                String::new()
            };
            let _ = writeln!(
                m,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |{tp} {} |",
                g["workload"].as_str().unwrap(),
                op,
                g["jobs"],
                g["shape"]["files"],
                g["shape"]["directories"],
                g["shape"]["candidate_roots"],
                st["n"],
                f(&st["median_ms"]),
                f(&st["min_ms"]),
                f(&st["max_ms"]),
                f(&st["spread_ms"]),
                g["rejected_samples"],
            );
        }
        m.push('\n');
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_before_reads_the_number_before_the_word() {
        assert_eq!(
            count_before("  7 matched  151.0K total", "matched"),
            Some(7)
        );
        assert_eq!(
            count_before("Done.  1K freed  3 deleted  0 cleared", "deleted"),
            Some(3)
        );
        assert_eq!(count_before("nothing here", "matched"), None);
    }

    #[test]
    fn stats_use_median_not_best() {
        let s = stats(&[10.0, 30.0, 20.0, 100.0, 40.0]);
        assert_eq!(s["median_ms"], 30.0);
        assert_eq!(s["min_ms"], 10.0);
        assert_eq!(s["spread_ms"], 90.0);
    }

    #[test]
    fn historical_shape_is_exact_and_deterministic() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let sa = build("historical", 1.0, a.path()).unwrap();
        let sb = build("historical", 1.0, b.path()).unwrap();
        assert_eq!((sa.dirs, sa.files), (4099, 32503));
        assert_eq!((sa.dirs, sa.files, sa.bytes), (sb.dirs, sb.files, sb.bytes));
        // the recount from disk agrees with what the generator claims
        assert_eq!(census(a.path()).unwrap(), (sa.dirs, sa.files));
    }

    #[test]
    fn every_workload_matches_its_reported_shape() {
        for (name, _) in WORKLOADS {
            let d = tempfile::tempdir().unwrap();
            let s = build(name, 0.05, d.path()).unwrap();
            assert_eq!(census(d.path()).unwrap(), (s.dirs, s.files), "{name}");
            assert_eq!(s.dirs, s.cand_dirs + s.out_dirs, "{name}");
            assert_eq!(s.files, s.cand_files + s.out_files, "{name}");
            assert!(s.cand_roots > 0 && s.markers >= s.cand_roots, "{name}");
        }
    }

    #[test]
    fn high_fanout_exceeds_the_queue_cap() {
        let d = tempfile::tempdir().unwrap();
        build("high_fanout", 1.0, d.path()).unwrap();
        assert!(fs::read_dir(d.path()).unwrap().count() > 4096);
    }
}
