//! purgable: find, mark, and purge disposable directories.
//!
//! Each concern lives in its own module; this file only wires them together
//! and hands off to the CLI.

mod action;
mod cli;
mod config;
mod dirfd;
mod discovery;
mod mark;
mod marker;
mod parallel;
mod prompt;
mod purge;
mod shred;
mod size;
mod style;

#[cfg(test)]
mod test_support;

fn main() {
    std::process::exit(cli::run());
}
