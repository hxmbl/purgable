//! Fixture helpers shared by the unit tests in each module.

use std::fs;
use std::path::Path;

use crate::action::Opts;

pub(crate) fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

pub(crate) fn write_dir_with_content(dir: &Path, files: &[(&str, &str)]) {
    fs::create_dir_all(dir).unwrap();
    for (name, content) in files {
        // Names may contain a path, e.g. "nested/deep.txt".
        write_file(&dir.join(name), content);
    }
}

pub(crate) fn opts() -> Opts {
    Opts::default()
}
