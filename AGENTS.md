# Development Guide for purgable

## Build Commands

### Debug build
```sh
cargo build
```

### Release build
```sh
cargo build --release
```

The release binary will be at `target/release/purgable`.

## Test Commands

### Run all tests
```sh
cargo test
```

### Run tests with output
```sh
cargo test -- --nocapture
```

### Run specific test
```sh
cargo test test_name
```

## Code Quality

### Format code
```sh
cargo fmt
```

### Check formatting without modifying
```sh
cargo fmt --check
```

### Run linter
```sh
cargo clippy -- -D warnings
```

### Run all quality checks
```sh
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

## CI/CD

The project has two GitHub Actions workflows:

- **CI** (`.github/workflows/ci.yml`): Runs on every push and PR to main
  - Checks code formatting
  - Runs clippy with strict warnings
  - Runs tests on multiple platforms (Linux, macOS x86_64/arm64)

- **Release** (`.github/workflows/release.yml`): Runs on version tags
  - Builds release binaries for all platforms
  - Creates GitHub release with assets
  - Generates and pushes Homebrew formula

## Project Layout

Single binary crate using the standard Cargo `src/` layout.

- `src/main.rs` - Entry point only: calls `cli::run()`
- `src/cli.rs` - `VERSION`, usage text, `parse_args`, command dispatch
- `src/config.rs` - Policy config file: `Config`, `Policy`, `load_config`
- `src/marker.rs` - The `PURGABLE` marker file and its provenance format
- `src/parallel.rs` - The parallel tree walker every scan, measure, and purge goes through
- `src/discovery.rs` - `validate_root`, `find`, `Matcher`, policy matching
- `src/mark.rs` - `mark`, `unmark`, `list`
- `src/purge.rs` - The interactive review loop
- `src/prompt.rs` - Prompt box, action legend, result rows
- `src/style.rs` - ANSI styling, width measurement, path shortening
- `src/size.rs` - Size accounting and human-readable formatting
- `src/dirfd.rs` - Naming entries by their parent directory's descriptor
- `src/shred.rs` - Destructive filesystem operations
- `src/test_support.rs` - Test fixtures shared across modules

Unit tests live in a `#[cfg(test)] mod tests` inside the module they cover.

Destructive operations are confined to `src/shred.rs`. The `PURGABLE` marker is
never shredded or cleared on its own: it is metadata about a directory, so
`clear_dir` and `shred_dir` deliberately leave it in place. Actions `d` and `x`
do remove it, because removing the directory is the point of those actions.

## Concurrency

Every traversal goes through `parallel::walk`, and every bulk file operation
through `parallel::for_each_chunk`. The reason is that all of this work is
syscall-bound, so a single thread leaves the machine idle. Set `PURGABLE_JOBS`
to override the thread count (clamped to 1..=64) when a scan is competing with a
build for the disk.

`walk` takes a callback per directory plus its already-open entries. A callback
queues subdirectories through the `Cursor` it is handed, and must only descend
into entries it classified as real directories from the parent listing. That
constraint is the safety model, not an optimisation: entry types come from the
listing, never from a stat of the name, so a symlink named like a build output
is still rejected.

Because the walk is concurrent, anything a command prints must be collected
first and sorted before it is written. Every command does this.

`size::seq_dir_size` is the single-threaded size walk, used for policy matching
from inside `scan_for_policies` so that a scan does not spawn a nested pool.

## Safety model

Three independent rules keep a run inside the tree it was pointed at:

1. Directory entry types come from the parent listing, and a symlink is never
   descended into.
2. Destruction is named relative to a descriptor for the directory that was just
   inspected (`dirfd::Dir`), not by spelling out a full path. `unlinkat` and
   `openat` without following a link mean a symlink swapped in after the listing
   is unlinked as a link rather than resolved.
3. `dirfd::Dir::open` uses `O_NOFOLLOW`, so a directory replaced by a symlink
   between the listing and the open is refused. The refusal is reported and the
   directory is left alone; there is deliberately no fallback to full paths,
   because by that point a full path would resolve whatever the name points at
   now.

The one exception is the root of a run, which the user named on the command
line: `Dir::open_root` follows it, because resolving it is what they asked for.

## Dependencies

- `rand` 0.10 - Random number generation for shredding
- `libc` 0.2 - Terminal width via `ioctl`, and the `*at()` calls in `dirfd`
- `serde` + `toml` - Policy config file parsing
- `tempfile` 3 (dev) - Temporary directory creation for tests

`walkdir` was removed. `parallel::walk` covers the same ground, keeps the
symlink semantics the safety model depends on, and allocates no `PathBuf` per
entry before deciding whether that entry is needed.

## Performance notes

Measured on macOS APFS (12 logical cores), release build, against the previous
single-threaded `walkdir` implementation. Read-only figures are the best of
five on a 32,503-file / 4,099-directory tree; shredding figures are single runs,
because one `fdatasync` per file makes each of them take minutes.

| Workload | before | after | |
|---|---|---|---|
| `mark --dry-run` (32.5k files) | 0.386s | 0.102s | 3.8x |
| `list` (32.5k files) | 0.329s | 0.093s | 3.5x |
| shred, wide tree (120 dirs x 50) | 114.1s | 92.6s | 1.2x |
| shred, narrow tree (6 dirs x 1000) | 110.6s | 79.8s | 1.4x |
| delete (20000 files in one dir) | 1.66s | 1.72s | parity |

Two things worth knowing before changing any of this:

- **Traversal and shredding want different shapes of parallelism.** `mark`,
  `list` and `review --dry-run` parallelise inside one `walk` over the whole
  tree, so they scale with cores. A destructive review instead empties one
  directory per prompt, so it only reaches every core if the expensive work is
  deferred and batched: shredding inside the walk left all but one core idle and
  measured no faster than single-threaded. If you move shredding back into the
  walk, re-check that case.
- **`unlink` on APFS is journal work, not path resolution.** Naming entries
  through a directory descriptor is not measurably faster than a full path, and
  `unlink` is no faster at 12 threads than at 4. `dirfd` earns its place on the
  safety property above, not on speed; shredding's win comes from threading
  `fdatasync`, which does scale (20ms/file at one thread, ~13ms at twelve).

One trap when microbenchmarking unlink: count the files actually deleted. A
harness whose `unlinkat` silently fails with `ENOENT` reports orders of
magnitude faster than it is, because it deleted nothing.

## Version Management

- Update version in `Cargo.toml`
- Update `VERSION` constant in `src/cli.rs`
- Tag the commit with `vX.Y.Z` to trigger release workflow
