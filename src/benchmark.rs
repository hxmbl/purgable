//! Benchmark harness for STRM-473: deterministic filesystem workloads,
//! repeatable measurements, validated destructive runs, machine readable output.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::{Serialize, Deserialize};

/// Environment information captured at benchmark time.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub commit: String,
    pub version: String,
    pub os: String,
    pub cpu: String,
    pub filesystem: String,
    pub kernel: String,
    pub rustc: String,
}

/// Benchmark configuration.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkConfig {
    pub fixture_path: PathBuf,
    pub jobs: usize,
    pub runs: usize,
}

/// Timing for a single benchmark run.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timing {
    pub wall_clock_ms: f64,
    pub cpu_time_ms: Option<f64>,
}

/// Results from a single benchmark run.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub command: String,
    pub target: String,
    pub timing: Timing,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Results from all runs of a benchmark configuration.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub config: BenchmarkConfig,
    pub environment: EnvironmentInfo,
    pub runs: Vec<RunResult>,
    /// Statistics computed from runs.
    pub stats: Option<RunStatistics>,
}

/// Statistics computed from multiple runs.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStatistics {
    pub mean_wall_clock_ms: f64,
    pub std_dev_wall_clock_ms: f64,
    pub min_wall_clock_ms: f64,
    pub max_wall_clock_ms: f64,
    pub median_wall_clock_ms: f64,
}

impl RunStatistics {
    #[allow(dead_code)]
    fn compute(runs: &[RunResult]) -> Option<Self> {
        if runs.is_empty() {
            return None;
        }
        let times: Vec<f64> = runs.iter().map(|r| r.timing.wall_clock_ms).collect();
        
        let n = times.len() as f64;
        let sum: f64 = times.iter().sum();
        let mean = sum / n;
        
        let variance = times.iter()
            .map(|t| (t - mean).powi(2))
            .sum::<f64>() / n;
        let std_dev = variance.sqrt();
        
        let min = times.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let max = times.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        
        let mut sorted = times.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = if sorted.len() % 2 == 1 {
            sorted[sorted.len() / 2]
        } else {
            (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
        };
        
        Some(Self {
            mean_wall_clock_ms: mean,
            std_dev_wall_clock_ms: std_dev,
            min_wall_clock_ms: min,
            max_wall_clock_ms: max,
            median_wall_clock_ms: median,
        })
    }
}

/// Fixture shape description.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureShape {
    pub description: String,
    pub directory_count: usize,
    pub file_count: usize,
    pub marker_count: usize,
    pub total_size_bytes: u64,
}

/// Complete benchmark report.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub timestamp: String,
    pub environment: EnvironmentInfo,
    pub fixture_shape: FixtureShape,
    pub results: Vec<BenchmarkResult>,
}

impl BenchmarkReport {
    #[allow(dead_code)]
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str("# Benchmark Report\n\n");
        md.push_str(&format!("**Timestamp:** {}\n\n", self.timestamp));
        
        md.push_str("## Environment\n\n");
        md.push_str(&format!("- Commit: {}\n", self.environment.commit));
        md.push_str(&format!("- Version: {}\n", self.environment.version));
        md.push_str(&format!("- OS: {}\n", self.environment.os));
        md.push_str(&format!("- CPU: {}\n", self.environment.cpu));
        md.push_str(&format!("- Filesystem: {}\n", self.environment.filesystem));
        md.push_str(&format!("- Kernel: {}\n", self.environment.kernel));
        md.push_str(&format!("- Rust: {}\n\n", self.environment.rustc));
        
        md.push_str("## Fixture Shape\n\n");
        md.push_str(&format!("- Description: {}\n", self.fixture_shape.description));
        md.push_str(&format!("- Directories: {}\n", self.fixture_shape.directory_count));
        md.push_str(&format!("- Files: {}\n", self.fixture_shape.file_count));
        md.push_str(&format!("- Markers: {}\n", self.fixture_shape.marker_count));
        md.push_str(&format!("- Total Size: {} bytes\n\n", self.fixture_shape.total_size_bytes));
        
        md.push_str("## Results\n\n");
        md.push_str("| Command | Jobs | Runs | Mean (ms) | Std Dev (ms) | Min (ms) | Median (ms) | Max (ms) |\n");
        md.push_str("|---|---|---|---|---|---|---|---|\n");
        
        for result in &self.results {
            if let Some(stats) = &result.stats {
                md.push_str(&format!(
                    "| {} | {} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |\n",
                    "list",
                    result.config.jobs,
                    result.config.runs,
                    stats.mean_wall_clock_ms,
                    stats.std_dev_wall_clock_ms,
                    stats.min_wall_clock_ms,
                    stats.median_wall_clock_ms,
                    stats.max_wall_clock_ms,
                ));
            }
        }
        
        md.push_str("\n## Notes\n\n");
        md.push_str(
            "Benchmark results are representative of the GitHub Actions runner environment.\n\n",
        );
        md.push_str("For reproducible local comparisons, run benchmarks on comparable hardware and filesystem.\n");
        
        md
    }
}

/// Run a single benchmark command and return timing.
#[allow(dead_code)]
fn run_benchmark_command(
    command: &str,
    args: &[&str],
    working_dir: &Path,
) -> io::Result<RunResult> {
    let full_command = command.to_string();
    let target = if !args.is_empty() {
        format!("{} {}", command, args.join(" "))
    } else {
        command.to_string()
    };
    
    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd.current_dir(working_dir);
    
    let start = Instant::now();
    let output = cmd.output()?;
    let elapsed = start.elapsed();
    
    Ok(RunResult {
        command: full_command,
        target,
        timing: Timing {
            wall_clock_ms: elapsed.as_secs_f64() * 1000.0,
            cpu_time_ms: None, // Would need platform-specific code
        },
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Run multiple iterations of a benchmark.
#[allow(dead_code)]
pub fn run_benchmark_iterations(
    command: &str,
    args: &[&str],
    working_dir: &Path,
    iterations: usize,
) -> io::Result<Vec<RunResult>> {
    let mut results = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        results.push(run_benchmark_command(command, args, working_dir)?);
    }
    Ok(results)
}

/// Generate a benchmark report from results.
#[allow(dead_code)]
pub fn create_benchmark_report(
    environment: EnvironmentInfo,
    fixture_shape: FixtureShape,
    config: BenchmarkConfig,
    runs: Vec<RunResult>,
) -> BenchmarkReport {
    let stats = RunStatistics::compute(&runs);
    let environment_clone = environment.clone();
    BenchmarkReport {
        timestamp: chrono::Utc::now().to_rfc3339(),
        environment: environment_clone,
        fixture_shape,
        results: vec![BenchmarkResult {
            config,
            environment,
            runs,
            stats,
        }],
    }
}

/// Count directories and files recursively.
pub fn count_fixture_contents(root: &Path) -> io::Result<(usize, usize)> {
    let mut dirs = 0usize;
    let mut files = 0usize;
    
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                dirs += 1;
                stack.push(entry.path());
            } else if metadata.is_file() {
                files += 1;
            }
        }
    }
    
    Ok((dirs, files))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_run_statistics_compute() {
        let runs = vec![
            RunResult {
                command: "test".into(),
                target: "test".into(),
                timing: Timing { wall_clock_ms: 100.0, cpu_time_ms: None },
                exit_code: 0,
                stdout: "".into(),
                stderr: "".into(),
            },
            RunResult {
                command: "test".into(),
                target: "test".into(),
                timing: Timing { wall_clock_ms: 200.0, cpu_time_ms: None },
                exit_code: 0,
                stdout: "".into(),
                stderr: "".into(),
            },
            RunResult {
                command: "test".into(),
                target: "test".into(),
                timing: Timing { wall_clock_ms: 300.0, cpu_time_ms: None },
                exit_code: 0,
                stdout: "".into(),
                stderr: "".into(),
            },
        ];
        
        let stats = RunStatistics::compute(&runs).unwrap();
        assert!((stats.mean_wall_clock_ms - 200.0).abs() < 0.01);
        assert!((stats.min_wall_clock_ms - 100.0).abs() < 0.01);
        assert!((stats.max_wall_clock_ms - 300.0).abs() < 0.01);
        assert!((stats.median_wall_clock_ms - 200.0).abs() < 0.01);
    }
    
    #[test]
    fn test_run_statistics_empty() {
        let runs: Vec<RunResult> = vec![];
        let stats = RunStatistics::compute(&runs);
        assert!(stats.is_none());
    }
}
