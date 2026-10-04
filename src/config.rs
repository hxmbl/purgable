//! The policy configuration file: parsing, defaults, and policy matching
//! rules.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::size::parse_size;

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    #[serde(default)]
    pub(crate) defaults: Defaults,
    /// Policies in file order; the first match wins, so put specific rules first.
    #[serde(default, rename = "policy")]
    pub(crate) policies: Vec<Policy>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Defaults {
    #[serde(default)]
    pub(crate) min_size: Option<String>,
    /// Parsed and asserted by the test suite, but not yet consulted when
    /// marking: a policy is gated by its own `enabled`, not this one. Kept so
    /// the `[defaults]` block of the shipped starter config stays modelled.
    #[serde(default = "default_true")]
    #[allow(dead_code)]
    pub(crate) enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub(crate) struct Policy {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) enabled: Option<bool>,
    /// Exact directory name this policy applies to.
    #[serde(default)]
    pub(crate) dir_name: Option<String>,
    /// Any one of these directory names.
    #[serde(default)]
    pub(crate) dir_name_any: Vec<String>,
    /// All of these must exist in the parent directory. This is the guard that
    /// stops a source tree that merely happens to be called `target` from being
    /// treated as build output.
    #[serde(default)]
    pub(crate) require_sibling: Vec<String>,
    /// At least one of these must exist in the parent directory.
    #[serde(default)]
    pub(crate) require_sibling_any: Vec<String>,
    /// At least one of these must exist inside the directory.
    #[serde(default)]
    pub(crate) require_child_any: Vec<String>,
    /// Overrides `defaults.min_size` for this policy.
    #[serde(default)]
    pub(crate) min_size: Option<String>,
}

impl Policy {
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub(crate) fn name_matches(&self, name: &str) -> bool {
        if let Some(exact) = &self.dir_name {
            if exact == name {
                return true;
            }
        }
        self.dir_name_any.iter().any(|candidate| candidate == name)
    }

    pub(crate) fn parent_ok(&self, parent: &Path) -> bool {
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

    pub(crate) fn child_ok(&self, dir: &Path) -> bool {
        if self.require_child_any.is_empty() {
            return true;
        }
        self.require_child_any
            .iter()
            .any(|candidate| dir.join(candidate).exists())
    }

    /// Resolve this policy's minimum size, preferring the policy override.
    pub(crate) fn effective_min(&self, default_min: Option<u64>) -> Option<u64> {
        self.min_size
            .as_deref()
            .map(parse_size)
            .unwrap_or(default_min)
    }
}

pub(crate) fn parse_config(text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e| format!("invalid config: {}", e))
}

pub(crate) fn config_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os("PURGABLE_CONFIG") {
        return PathBuf::from(explicit);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config").join("purgable.toml")
}

pub(crate) fn load_config(path: &Path) -> Result<Config, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    parse_config(&text)
}

pub(crate) fn starter_config() -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
