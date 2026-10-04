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
    r#"# purgable policies - ~/.config/purgable.toml
#
# `purgable mark <root>` walks the tree and drops a PURGABLE marker in every
# directory a policy matches. `purgable review <root>` then asks what to do.
#
# Policies are tried in the order below and the first match wins, so put
# specific rules above general ones.
#
# Two guard kinds keep a policy from eating something it should not:
#   require_sibling      every one of these must exist in the PARENT
#   require_sibling_any  at least one of these must exist in the PARENT
#   require_child_any    at least one of these must exist INSIDE the directory
# `require_sibling` and `require_sibling_any` are combined, so a policy can
# demand both a package.json and a bundler config. There is no "must not
# exist" guard, so generic names are only ever matched alongside a marker file
# that is unique to one build system.

[defaults]
# Skip anything smaller than this unless a policy overrides it. Per-policy
# min_size beats this AND --min-size, so leave it off policies you want
# --min-size to still control. Use `--min-size 50M` to sweep small caches.
min_size = "500M"
enabled = true

# ---------------------------------------------------------------------------
# Build output
#
# These hold the most bytes and cost only a rebuild to restore. A build
# directory is never descended into once matched, so one match covers
# everything nested inside it.
# ---------------------------------------------------------------------------

# Cargo build output. require_sibling is the important part: a directory named
# `target` is only build output if its parent has a Cargo.toml. Without this,
# real source trees such as linux/kernel/drivers/target get matched too.
[[policy]]
name = "cargo-target"
dir_name = "target"
require_sibling = ["Cargo.toml"]
require_child_any = [".rustc_info.json", "debug", "release", "CACHE", "incremental"]

# Maven output. Also called `target`, so the pom.xml sibling is what separates
# it from cargo-target above.
[[policy]]
name = "maven-target"
dir_name = "target"
require_sibling = ["pom.xml"]
require_child_any = ["classes", "maven-status", "generated-sources", "maven-archiver", "test-classes"]

# sbt and Clojure CLI output, also `target`.
[[policy]]
name = "sbt-target"
dir_name = "target"
require_sibling_any = ["build.sbt", "project.clj", "deps.edn"]
require_child_any = ["classes", "test-classes", "streams", "resolution-cache"]

# CMake out-of-source builds. CMakeCache.txt is written by nothing else, so
# this guard is decisive on its own. That is what keeps a source directory
# named `build` (linux/kernel/tools/build) out of the results.
[[policy]]
name = "cmake-build"
dir_name_any = ["build", "cmake-build", "cmake-build-debug", "cmake-build-release", "cmake-build-relwithdebinfo", "cmake-build-minsizerel", "out", "_build"]
require_sibling_any = ["CMakeLists.txt"]
require_child_any = ["CMakeCache.txt"]

# Ninja build trees. build.ninja is likewise unique to ninja.
[[policy]]
name = "ninja-build"
dir_name_any = ["build", "out"]
require_child_any = ["build.ninja"]

# Meson build trees, which may be named anything the user chose.
[[policy]]
name = "meson-build"
dir_name_any = ["build", "builddir", "_build", "out"]
require_child_any = ["meson-private", "meson-info", "meson-logs"]

# Xcode build output. Xcode allows a custom build location, so this matches a
# directory called `build` as well as DerivedData. The guard deliberately
# requires Index.noindex or ModuleCache.noindex rather than Build: a bare Build
# subdirectory also exists inside the kernel's source tree.
[[policy]]
name = "xcode-derived-data"
dir_name_any = ["DerivedData", "build"]
require_child_any = ["Index.noindex", "ModuleCache.noindex", "SDKStatCaches.noindex"]

# Gradle, Android and Flutter output. A build.gradle sibling plus Gradle's own
# output directories; Flutter's `build` has the same shape.
[[policy]]
name = "gradle-build"
dir_name = "build"
require_sibling_any = ["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"]
require_child_any = ["intermediates", "tmp", "outputs", "generated", "reports", "kotlin", "libs", "classes", "js"]

# Per-project Gradle cache. Safe here and unsafe at ~/.gradle, which holds
# credentials; the build.gradle sibling is what tells them apart.
[[policy]]
name = "gradle-project-cache"
dir_name = ".gradle"
require_sibling_any = ["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "gradlew"]
require_child_any = ["caches", "buildOutputCleanup", "daemon", "native", "wrapper"]

# Swift Package Manager checkout and build products, regenerable with
# `swift build`.
[[policy]]
name = "swift-build"
dir_name = ".build"
require_sibling_any = ["Package.swift"]
require_child_any = ["checkouts", "artifacts", "debug", "release", "manifest.db"]

# Elixir/Mix compiled output, regenerable with `mix deps.compile`.
[[policy]]
name = "elixir-build"
dir_name = "_build"
require_sibling = ["mix.exs"]
require_child_any = ["lib", "consolidated", "rebar3"]

# Haskell stack work dir.
[[policy]]
name = "stack-work"
dir_name = ".stack-work"
require_sibling_any = ["stack.yaml", "stack.yaml.lock"]
require_child_any = ["dist", "build", "stack.sqlite"]

# Cabal build products.
[[policy]]
name = "cabal-dist"
dir_name = "dist-newstyle"
require_sibling_any = ["cabal.project", "cabal.project.local", "stack.yaml", "package.yaml"]
require_child_any = ["build", "packagedb", "cache"]

# Zig build and cache directories. build.zig is present in every Zig project.
[[policy]]
name = "zig-build"
dir_name_any = ["zig-out", "zig-cache", ".zig-cache"]
require_sibling_any = ["build.zig", "build.zig.zon"]

# OCaml dune build trees and metals/bloop caches. `_dune` holds only a
# `default` subdirectory, which is what makes it safe to match without a
# sibling check: `_dune` appears in every directory of a dune project, not
# just the root.
[[policy]]
name = "ocaml-build"
dir_name_any = [".dune", "_dune"]
require_child_any = ["default"]

# OCaml editor and formatter caches.
[[policy]]
name = "ocaml-cache"
dir_name_any = [".bloop", ".dune-cache", ".merlin", ".merlin-cache", ".ocamlformat-cache", ".metals"]
require_sibling_any = ["dune-project", "dune-workspace"]
require_child_any = [".json", "db", "merlin", "cache"]

# Android NDK native build output, regenerable with ./gradlew. These two
# directories are siblings of each other in the app directory, which has the
# build.gradle that the guard requires.
[[policy]]
name = "android-native-build"
dir_name_any = [".cxx", ".externalNativeBuild"]
require_sibling_any = ["build.gradle", "build.gradle.kts", "CMakeLists.txt", ".cxx", ".externalNativeBuild"]
require_child_any = ["debug", "release", "json"]

# .NET intermediate output. obj/ is matched but bin/ is deliberately not: bin
# is far too generic a name to delete (virtualenv bin/, committed scripts, and
# anything else that happens to be called bin).
[[policy]]
name = "dotnet-obj"
dir_name = "obj"
require_child_any = ["project.assets.json", "project.nuget.cache"]

# Terraform plugin and child-module downloads, regenerable with
# `terraform init`. The home directory ~/.terraform.d is NOT covered because it
# can hold registry credentials.
[[policy]]
name = "terraform-modules"
dir_name = ".terraform"
require_sibling_any = ["main.tf", "versions.tf", "terraform.tf", ".terraform.lock.hcl"]
require_child_any = ["modules", "providers"]

# JavaScript bundler output. Demands both a package.json and a real bundler or
# TypeScript config, plus an entry point or asset folder inside, because `dist`
# is also where people park release archives they care about.
[[policy]]
name = "js-bundle"
dir_name_any = ["dist", "build", "out"]
require_sibling = ["package.json"]
require_sibling_any = ["tsconfig.json", "vite.config.js", "vite.config.ts", "vite.config.mjs", "next.config.js", "next.config.mjs", "svelte.config.js", "rollup.config.js", "webpack.config.js", "astro.config.mjs", "nuxt.config.ts", "angular.json"]
require_child_any = ["index.html", "index.js", "main.js", "assets", ".vite"]

# ---------------------------------------------------------------------------
# Dependencies
# ---------------------------------------------------------------------------

# Installed npm packages, regenerable with `npm install`.
[[policy]]
name = "node-modules"
dir_name = "node_modules"
require_sibling = ["package.json"]

# Python virtualenvs, regenerable with `python -m venv`. A directory called
# `env` is left alone on purpose: it is too common a name to mean venv.
[[policy]]
name = "python-venv"
dir_name_any = [".venv", "venv"]
require_sibling_any = ["pyproject.toml", "requirements.txt", "setup.py", "Pipfile"]

# Bundler vendor directory, which holds both vendored gems and vendor/bundle.
[[policy]]
name = "ruby-vendor"
dir_name = "vendor"
require_sibling_any = ["Gemfile", "Gemfile.lock", ".ruby-version", "Rakefile", "config.ru"]
require_child_any = ["bundle", "cache", "gems"]

# Rails tmp. `tmp` alone is far too generic a name, so both a Rails-shaped
# parent and a Rails-shaped child are required.
[[policy]]
name = "rails-tmp"
dir_name = "tmp"
require_sibling_any = ["Gemfile", "Gemfile.lock", "config.ru", "Rakefile", "config"]
require_child_any = ["pids", "sockets", "cache", "storage"]

# CocoaPods checkout, regenerable with `pod install`.
[[policy]]
name = "cocoapods"
dir_name = "Pods"
require_sibling_any = ["Podfile", "Podfile.lock"]

# ---------------------------------------------------------------------------
# Framework caches
# ---------------------------------------------------------------------------

# Next.js build output.
[[policy]]
name = "next-build"
dir_name = ".next"
require_sibling_any = ["next.config.js", "next.config.mjs", "next.config.ts", "package.json"]
require_child_any = ["build-manifest.json", "cache", "server", "static"]

# Nuxt output and its Nitro build directory.
[[policy]]
name = "nuxt-build"
dir_name_any = [".nuxt", ".output"]
require_sibling_any = ["nuxt.config.js", "nuxt.config.ts", "nuxt.config.mjs"]
require_child_any = ["dist", "nitro.json", "tsconfig.json", "server"]

# SvelteKit output.
[[policy]]
name = "sveltekit-build"
dir_name = ".svelte-kit"
require_sibling_any = ["svelte.config.js", "svelte.config.ts", "svelte.config.mjs"]
require_child_any = ["output", "generated"]

# Turborepo remote cache. turbo.json is an exact, unambiguous sibling.
[[policy]]
name = "turbo-cache"
dir_name = ".turbo"
require_sibling = ["turbo.json"]
require_child_any = ["cache"]

# Angular CLI cache.
[[policy]]
name = "angular-cache"
dir_name = ".angular"
require_sibling = ["angular.json"]
require_child_any = ["cache"]

# Built Storybook.
[[policy]]
name = "storybook-static"
dir_name_any = [".storybook-static", "storybook-static"]
require_sibling = [".storybook"]

# Playwright reports.
[[policy]]
name = "playwright-report"
dir_name_any = ["playwright-report", "blob-report"]
require_sibling_any = ["playwright.config.ts", "playwright.config.js", "playwright.config.mjs"]
require_child_any = ["index.html", "report.json", "data"]

# Jekyll's generated site. `_site` is Jekyll-specific, unlike `site`, which is
# just as likely to be a source directory.
[[policy]]
name = "jekyll-site"
dir_name = "_site"
require_sibling_any = ["_config.yml", "_config.yaml", "_config.toml", "Gemfile"]
require_child_any = ["index.html", "assets", "feed.xml"]

# Compiler caches. These reach tens of gigabytes and hold nothing but object
# files keyed by hash, so deleting them costs only rebuild time.
[[policy]]
name = "compiler-cache"
dir_name_any = ["ccache", "sccache", "distcc"]
require_child_any = ["caches", "stats", "cache", "lock", "state", "ccache.conf", "sccache.conf", "2", "3", "4"]

# Misc JS tool caches. All dot-prefixed names that nothing else uses.
[[policy]]
name = "js-tool-cache"
dir_name_any = [".parcel-cache", ".vite", ".vite-cache", ".webpack-cache", ".rspack-cache", ".swc", ".babel-cache", ".cache-loader", ".astro", ".yarn-cache", ".pnpm-store", ".pnpm", ".bun", ".bun-cache", ".gatsby-cache", ".jekyll-cache", ".sass-cache"]

# ---------------------------------------------------------------------------
# Generic caches
#
# These names carry no build-system signal, so each one is either an
# unambiguous dot-directory or a cache suffix nothing else claims.
# ---------------------------------------------------------------------------

# XDG cache directory, plus the per-project .cache convention.
[[policy]]
name = "cache-dir"
dir_name = ".cache"

# macOS per-user caches, which is usually the single largest win on a Mac.
# The sibling list is a check that the parent looks like ~/Library. Note that
# a required sibling equal to the directory's own name would be a vacuous
# guard, since the candidate always exists next to itself.
[[policy]]
name = "macos-library-cache"
dir_name = "Caches"
require_sibling_any = ["Application Support", "Preferences", "Logs", "Containers", "Fonts", "Developer"]

# npm download cache and logs.
[[policy]]
name = "npm-cache"
dir_name = ".npm"
require_child_any = ["_cacache", "_logs", "_npx"]

# Python bytecode and tool caches. pytest_cache, mypy_cache and tox are all
# regenerable. .hypothesis is not listed: its example database is what
# reproduces a failing test.
[[policy]]
name = "python-tool-cache"
dir_name_any = ["__pycache__", ".pytest_cache", ".mypy_cache", ".pyre_cache", ".pyre", ".pytype", ".pylint.d", ".ruff_cache", ".tox", ".nox", ".nyc_output"]

# Coverage and test reports. These are small and rarely reach the size
# threshold, but the guard keeps the names from matching a directory that
# merely calls itself `htmlcov`.
[[policy]]
name = "coverage-report"
dir_name_any = ["htmlcov", "coverage-html", "coverage-xml", "coverage-json", "lcov-report", ".nyc_output", ".jest-cache"]
require_sibling_any = ["package.json", "pyproject.toml", "setup.cfg", "setup.py", "tox.ini", ".coveragerc", "Makefile", "Cargo.toml"]

# Deliberately NOT matched, for the record:
#
#   bin, obj, packages, deps, dependencies, third_party, external, sources
#     Checked-in source, vendored code, or monorepo package roots.
#   dist, build, out, output, site, public
#     Too generic on their own; only matched via js-bundle above.
#   downloads, images, thumbnails, previews, renders, videos, recordings
#     User data. A Downloads directory is exactly the thing to never automate.
#   logs, reports, snapshots, fixtures
#     Can be the only record of what happened.
#   env, ENV, virtualenv
#     Usually a venv, but too often something else to risk.
#   index, workspace, work, models, samples, data
#     As likely to be source as output.
#   ~/.cargo, ~/.rustup, ~/.gradle, ~/.m2, ~/.ivy2, ~/.sbt, ~/.nuget
#   ~/.gem, ~/.bundle, ~/.mix, ~/.hex, ~/.poetry, ~/.pyenv, ~/.uv, ~/.bun
#   ~/.terraform.d, ~/.pulumi, ~/.serverless, ~/.aws-sam, ~/.docker, ~/.vercel
#     Tool state directories that also hold credentials, installed tools, or
#     deployment state. Delete the cache subdirectory by hand instead, e.g.
#     `cargo cache`, `uv cache clean`, `docker builder prune`.
#   Package.resolved, Podfile.lock, *.xcuserstate, .dockerignore, Makefile,
#   config.h, CMakeCache.txt (as a bare name), install_manifest.txt,
#   compile_commands.json, .hugo_build.lock, .eslintcache
#     Files, not directories, so they can never be marked. Several are tracked
#     in git and deleting them changes your build.
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
        assert_eq!(
            config.policies[0].name, "cargo-target",
            "cargo-target must stay first: it is the documented example"
        );
    }

    #[test]
    fn test_starter_config_policy_names_are_unique() {
        let config = parse_config(&starter_config()).unwrap();
        let mut names: Vec<&str> = config.policies.iter().map(|p| p.name.as_str()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            count,
            "duplicate policy name in starter config"
        );
    }

    /// Every shipped policy must prove the directory is build output rather
    /// than just claiming a generic name. A policy with no guard at all can
    /// match any directory whose name happens to be common, so an unguarded
    /// policy is only allowed when every name it matches is one that no other
    /// tool would create.
    #[test]
    fn test_starter_config_policies_are_guarded() {
        let config = parse_config(&starter_config()).unwrap();
        for policy in &config.policies {
            let names = dir_names(policy);
            assert!(
                !names.is_empty(),
                "{} matches no directory name",
                policy.name
            );
            let has_guard = !policy.require_sibling.is_empty()
                || !policy.require_sibling_any.is_empty()
                || !policy.require_child_any.is_empty();
            if has_guard {
                continue;
            }
            for name in names {
                assert!(
                    is_tool_specific(name),
                    "{} matches {:?} with no guard, and that name is not unique to one tool",
                    policy.name,
                    name
                );
            }
        }
    }

    /// Every directory name a policy matches.
    fn dir_names(policy: &Policy) -> Vec<&str> {
        policy
            .dir_name
            .iter()
            .map(|s| s.as_str())
            .chain(policy.dir_name_any.iter().map(|s| s.as_str()))
            .collect()
    }

    /// True for names no other tool uses, so matching them needs no guard: a
    /// leading dot or underscore is the hidden-directory convention, and the
    /// rest are created by exactly one build system.
    fn is_tool_specific(name: &str) -> bool {
        const TOOL_OWNED: &[&str] = &[
            "__pycache__",
            "_build",
            "_site",
            "DerivedData",
            "Pods",
            "zig-out",
        ];
        name.starts_with('.') || name.starts_with('_') || TOOL_OWNED.contains(&name)
    }

    /// Names that appear in many ecosystems collide with real source
    /// directories. These may only be matched when a build-system marker is
    /// also required.
    #[test]
    fn test_generic_names_require_a_guard() {
        let generic = [
            "build", "dist", "out", "output", "target", "bin", "obj", "site", "public", "logs",
            "cache", "tmp", "temp", "vendor", "env",
        ];
        let config = parse_config(&starter_config()).unwrap();
        for name in generic {
            for policy in &config.policies {
                if !dir_names(policy).contains(&name) {
                    continue;
                }
                // A bare name match with no sibling and no child requirement is
                // how `downloads` would have ended up in this file.
                assert!(
                    !policy.require_child_any.is_empty()
                        || !policy.require_sibling.is_empty()
                        || !policy.require_sibling_any.is_empty(),
                    "{} matches the generic name {:?} with no guard",
                    policy.name,
                    name
                );
            }
        }
    }

    /// User data and checked-in source must stay unreachable from the shipped
    /// config, whatever they happen to be called.
    #[test]
    fn test_starter_config_never_matches_user_data() {
        let config = parse_config(&starter_config()).unwrap();
        let forbidden = [
            // User data.
            "downloads",
            "Downloads",
            "documents",
            "Documents",
            "desktop",
            "pictures",
            "music",
            "movies",
            "images",
            "thumbnails",
            "previews",
            "videos",
            "recordings",
            "renders",
            "samples",
            "data",
            "models",
            "archives",
            // Checked-in source.
            "packages",
            "deps",
            "dependencies",
            "third_party",
            "third-party",
            "external",
            "externals",
            "sources",
            "source",
            "src",
            "lib",
            "scripts",
            "migrations",
            // Logs and reports can be the only record of what happened.
            "logs",
            "log",
            "reports",
            "report",
            "snapshots",
            "fixtures",
            // Too generic to delete blind.
            "bin",
            "env",
            "ENV",
            "index",
            "workspace",
            "work",
            "public",
            "site",
            "wwwroot",
        ];
        for policy in &config.policies {
            for name in dir_names(policy) {
                assert!(
                    !forbidden.contains(&name),
                    "{} would match {:?}, which is user data or source",
                    policy.name,
                    name
                );
            }
        }
    }

    /// The documented false positives must stay rejected by the shipped
    /// config. Each case is a real directory name paired with the guard that
    /// should turn it away.
    #[test]
    fn test_starter_config_rejects_known_source_directories() {
        use std::fs;
        use std::path::Path;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // A kernel-style source tree: real directories named `target` and
        // `build` that hold code, with no build-system marker nearby.
        let kernel = root.join("kernel/drivers/target");
        let kernel_build = root.join("kernel/tools/build");
        fs::create_dir_all(&kernel).unwrap();
        fs::create_dir_all(&kernel_build).unwrap();
        // kernel/tools/build really does contain a Build subdirectory, which
        // is why the xcode policy must not settle for that alone.
        fs::create_dir_all(kernel_build.join("Build")).unwrap();
        fs::write(kernel.join("module.c"), "").unwrap();
        fs::write(kernel_build.join("Makefile"), "").unwrap();

        // A project whose `public` and `downloads` directories hold assets.
        let app = root.join("app");
        fs::create_dir_all(app.join("public")).unwrap();
        fs::create_dir_all(app.join("downloads")).unwrap();
        fs::write(app.join("public/logo.svg"), "").unwrap();
        fs::write(app.join("downloads/invoice.pdf"), "").unwrap();
        fs::write(app.join("package.json"), "{}").unwrap();

        // A venv `bin` and `lib`, which must never be treated as build output.
        let venv = root.join("venv");
        fs::create_dir_all(venv.join("bin")).unwrap();
        fs::create_dir_all(venv.join("lib/python3.12/site-packages/build")).unwrap();
        fs::write(venv.join("bin/activate"), "").unwrap();

        // A checked-in `packages` directory (monorepo package roots).
        let mono = root.join("mono/packages");
        fs::create_dir_all(&mono).unwrap();
        fs::write(mono.join("index.ts"), "").unwrap();
        fs::write(root.join("mono/package.json"), "{}").unwrap();

        // A `dist` holding curated release archives in a non-JS project.
        let go = root.join("gomod/dist");
        fs::create_dir_all(&go).unwrap();
        fs::write(go.join("app_Darwin_arm64.tar.gz"), "").unwrap();
        fs::write(root.join("gomod/go.mod"), "module x\n").unwrap();

        let policies = config_policies();
        let selected: Vec<&Policy> = policies.iter().collect();
        let mut found = Vec::new();
        crate::discovery::scan_for_policies(
            Path::new(root),
            &crate::discovery::Matcher::new(&selected),
            None,
            &mut found,
            &mut Vec::new(),
        );
        let paths: Vec<String> = found.iter().map(|c| c.path.display().to_string()).collect();

        for rejected in [
            "kernel/drivers/target",
            "kernel/tools/build",
            "app/public",
            "app/downloads",
            "venv/bin",
            "venv/lib/python3.12/site-packages/build",
            "mono/packages",
            "gomod/dist",
        ] {
            let full = root.join(rejected).display().to_string();
            assert!(
                !paths.contains(&full),
                "{} must not be marked, but the default config matched it",
                rejected
            );
        }
    }

    /// The cases above are only half the story: a config that matched nothing
    /// would pass them too. These directories really are disposable, so the
    /// default config is expected to find each one.
    #[test]
    fn test_starter_config_matches_real_build_output() {
        use std::fs;
        use std::path::Path;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // (relative directory, marker child, marker files in the parent)
        let fixtures: &[(&str, &str, &[&str])] = &[
            ("rust/target", ".rustc_info.json", &["Cargo.toml"]),
            ("maven/target", "maven-status", &["pom.xml"]),
            ("cmake/build", "CMakeCache.txt", &["CMakeLists.txt"]),
            ("ninja/out", "build.ninja", &["CMakeLists.txt"]),
            ("meson/builddir", "meson-private", &["meson.build"]),
            ("xcode/DerivedData", "Index.noindex", &[]),
            ("gradle/build", "intermediates", &["build.gradle"]),
            ("android/.gradle", "caches", &["build.gradle"]),
            ("swift/.build", "checkouts", &["Package.swift"]),
            ("elixir/_build", "consolidated", &["mix.exs"]),
            ("haskell/.stack-work", "stack.sqlite", &["stack.yaml"]),
            ("cabal/dist-newstyle", "packagedb", &["cabal.project"]),
            ("zig/zig-out", "", &["build.zig"]),
            ("dotnet/obj", "project.assets.json", &["app.csproj"]),
            ("terraform/.terraform", "providers", &["main.tf"]),
            ("web/dist", "index.html", &["package.json", "tsconfig.json"]),
            ("web/node_modules", "react", &["package.json"]),
            ("py/.venv", "pyvenv.cfg", &["pyproject.toml"]),
            ("ruby/vendor", "bundle", &["Gemfile"]),
            ("ios/Pods", "Manifest.lock", &["Podfile"]),
            ("next/.next", "build-manifest.json", &["package.json"]),
            ("nuxt/.nuxt", "tsconfig.json", &["nuxt.config.ts"]),
            ("svelte/.svelte-kit", "generated", &["svelte.config.js"]),
            ("turbo/.turbo", "cache", &["turbo.json"]),
            ("angular/.angular", "cache", &["angular.json"]),
            ("tooling/js/.parcel-cache", "", &[]),
            ("npm/.npm", "_cacache", &[]),
            ("home/.cache", "", &[]),
            ("python/pkg/__pycache__", "", &[]),
            ("jekyll/_site", "index.html", &["_config.yml"]),
            ("tools/ccache", "stats", &[]),
            ("rails/tmp", "pids", &["Gemfile"]),
            (
                "Library/Caches",
                "",
                &["Application Support", "Preferences", "Logs", "Containers"],
            ),
        ];

        for (rel, child, markers) in fixtures {
            let path = root.join(rel);
            fs::create_dir_all(&path).unwrap();
            if !child.is_empty() {
                fs::create_dir_all(path.join(child)).unwrap();
            }
            let project = path.parent().unwrap();
            for marker in *markers {
                fs::write(project.join(marker), "{}").unwrap();
            }
        }

        let policies = config_policies();
        let selected: Vec<&Policy> = policies.iter().collect();
        let mut found = Vec::new();
        crate::discovery::scan_for_policies(
            Path::new(root),
            &crate::discovery::Matcher::new(&selected),
            None,
            &mut found,
            &mut Vec::new(),
        );
        let matched: Vec<String> = found
            .iter()
            .map(|c| {
                format!(
                    "{}={}",
                    c.path.strip_prefix(Path::new(root)).unwrap().display(),
                    c.policy
                )
            })
            .collect();

        for (rel, _, _) in fixtures {
            assert!(
                matched.iter().any(|m| m.starts_with(&format!("{}=", rel))),
                "expected {} to be matched by the default config, got {:?}",
                rel,
                matched
            );
        }
    }

    fn config_policies() -> Vec<Policy> {
        parse_config(&starter_config()).unwrap().policies
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
