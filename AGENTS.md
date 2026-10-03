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

## Dependencies

- `rand` 0.10 - Random number generation for shredding
- `walkdir` 2 - Recursive directory traversal
- `tempfile` 3 (dev) - Temporary directory creation for tests

## Version Management

- Update version in `Cargo.toml`
- Update `VERSION` constant in `main.rs`
- Tag the commit with `vX.Y.Z` to trigger release workflow
