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
- `src/discovery.rs` - `validate_root`, `find`, policy matching
- `src/mark.rs` - `mark`, `unmark`, `list`
- `src/purge.rs` - The interactive review loop
- `src/prompt.rs` - Prompt box, action legend, result rows
- `src/style.rs` - ANSI styling, width measurement, path shortening
- `src/size.rs` - Size accounting and human-readable formatting
- `src/shred.rs` - Destructive filesystem operations
- `src/test_support.rs` - Test fixtures shared across modules

Unit tests live in a `#[cfg(test)] mod tests` inside the module they cover.

Destructive operations are confined to `src/shred.rs`. The `PURGABLE` marker is
never shredded or cleared on its own: it is metadata about a directory, so
`clear_dir` and `shred_dir` deliberately leave it in place.

## Dependencies

- `rand` 0.10 - Random number generation for shredding
- `walkdir` 2 - Recursive directory traversal
- `libc` 0.2 - Terminal width via `ioctl`
- `serde` + `toml` - Policy config file parsing
- `tempfile` 3 (dev) - Temporary directory creation for tests

## Version Management

- Update version in `Cargo.toml`
- Update `VERSION` constant in `src/cli.rs`
- Tag the commit with `vX.Y.Z` to trigger release workflow
