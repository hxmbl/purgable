# Benchmarks

Reproducible filesystem benchmarks for purgable (STRM-473). The runner lives in
`examples/bench.rs`, not in the shipped binary: it drives a built `purgable` as
a black box, so there is no instrumentation in the hot path.

```sh
cargo build --release
cargo run --release --example bench -- list-workloads
cargo run --release --example bench -- run --out bench-results
cargo run --release --example bench -- gen --workload historical --dir /tmp/fx   # inspect one fixture
```

`run` flags: `--bin` (default `target/release/purgable`), `--work` (fixture dir,
default `bench-fixtures`; put it on the filesystem you want to measure),
`--workloads a,b`, `--jobs 1,2,4,8,12,16,32`, `--reps 5`, `--destructive-reps 3`,
`--scale 1`, `--shred-scale` (default scale/10), `--all-jobs true`.

## What is measured

Workloads (all deterministic: no randomness, index-addressed trees):
`historical` (exactly 32,503 files / 4,099 dirs, the AGENTS.md baseline shape),
`broad`, `high_fanout` (one directory above the 4,096 queue cap), `deep_chain`,
`file_heavy`, `many_small_roots`, `few_huge_roots`, `nested_markers`, `wide_dir`.

Operations, each measured separately across a `PURGABLE_JOBS` sweep:
`mark --dry-run`, `list`, `review --dry-run`, ordinary delete (`d-ALL`) and shred
(`s-ALL`). Jobs above the machine's logical core count are skipped unless
`--all-jobs true`.

Each result group records the exact fixture shape (dirs, files, bytes,
candidate roots, marker files), raw samples, median, mean, stddev, min, max and
spread. Environment captured: commit (and whether the tree was dirty), binary
version, OS and kernel, arch, CPU model, logical cores, filesystem type, rustc.
Outputs: `results.json`, `results.csv` (one row per sample), `summary.md`.

## Validity rules

- Read-only ops: one discarded warm-up, then at least five samples (`--reps`),
  reporting median and spread, never only the best run. Each sample checks the
  reported candidate count.
- Destructive ops: the fixture is recreated for every sample, recounted from
  disk before the run, and after the run the filesystem is recounted again.
  A sample is rejected if the summary line disagrees or the final directory and
  file counts differ from the expected state. Rejected samples are excluded from
  statistics, counted in the summary, and make the runner exit non-zero. This
  covers the "unlink reports success but deleted nothing" trap in AGENTS.md.
- Shred is reported in its own table, never mixed with ordinary delete, and runs
  on a smaller tree by default because it syncs every file.

Notes on behaviour the validation encodes: shredding keeps every marked
directory and its marker, including nested ones. With nested markers, review
lists inner markers as separate rows, so the delete/shred summary can show
skipped rows once the outer directory is gone; for that workload the on-disk
recount is the authority.

## Safety boundaries

Destructive runs only ever act on fixtures this runner generated, under `--work`.
The runner refuses a `--work` directory that is non-empty and was not created by
it (it leaves a `.purgable-bench-workdir` sentinel), uses its own policy file
(`PURGABLE_CONFIG`, so your `~/.config/purgable.toml` is never read), passes
every review answer on stdin (no prompts), and only points purgable at the
generated fixture directory. Use a fresh path such as `/tmp/purgable-bench`,
on the volume you want to measure. `gen --dir` removes `<dir>/<workload>`
before regenerating, so give it a scratch directory too.

## Running on macOS

```sh
cargo build --release --locked
cargo build --release --locked --example bench
target/release/examples/bench run \
  --work /tmp/purgable-bench --out bench-results-macos \
  --jobs 1,2,4,8,12 --reps 5 --destructive-reps 3
```

Add `--workloads historical` for a quick run on the baseline tree shape only.
Jobs above the logical core count are skipped (`--all-jobs true` forces them).
Send back `bench-results-macos/summary.md` and `results.json` (the environment
block records chip, cores and filesystem as reported by the OS). Compare trends
against `benchmarks/baseline/` only as ratios (jobs scaling, op ordering), not
absolute times: that baseline is Linux ext4 on 2 cores.

## Cache and platform assumptions

Measurements are warm-cache: fixtures are created immediately before use and the
page cache is never dropped, so numbers describe syscall and scheduling cost, not
cold disk reads. Results are comparable only on the same machine, filesystem and
core count. The historical baseline in AGENTS.md was macOS APFS with 12 logical
cores; this harness reproduces its tree shape, not its hardware. Compare trends
between commits on one machine.

## Baseline

`benchmarks/baseline/` holds a run at commit-parent of this change (Linux ext4
VM, 2 logical cores, jobs 1 and 2, scale 1). It is a trend reference for that
machine only. Sweep the full job list on a many-core machine for scheduler work.
The on-demand `Benchmark` GitHub workflow runs the same harness and uploads the
results as an artifact.
