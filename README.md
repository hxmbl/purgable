# purgable

Mark disposable directories automatically, then decide what to do with them.

`purgable` finds directories that are safe to throw away — Cargo build output,
`node_modules`, virtualenvs — drops a `PURGABLE` marker in each, and then asks
you what to do with them. You keep the decision; the tool does the finding.

It only finds what it can prove is disposable. Every policy needs a build-system
file next to the directory, or a file only that tool writes inside it, before it
will match — which is how it tells Cargo's `target` apart from
`linux/kernel/drivers/target`, both real directories with the same name.
[`will_delete.txt`](will_delete.txt) is the full audit of what that rules out and
what it allows.

## Install

```sh
brew install Hxmbl/tap/purgable
```

Or build from source:

```sh
cargo build --release
```

Prebuilt binaries for Linux and macOS (amd64/arm64) are attached to each
[release](https://github.com/Hxmbl/purgable/releases).

## Quick start

```sh
purgable init                      # write ~/.config/purgable.toml
purgable mark ~/Projects           # find candidates, drop markers
purgable review ~/Projects         # decide, largest first
```

`mark` only writes markers. Nothing is deleted until you answer a prompt in
`review`.

## Usage

```
purgable mark <directory> [--policy NAME]... [--min-size SIZE] [--dry-run]
purgable review <directory> [--dry-run]
purgable list <directory>
purgable unmark <directory> [--policy NAME] [--all] [--dry-run]
purgable init [--force]
purgable --help | -h
purgable --version | -v
```

### What it will delete

The starter config ships with 40 policies, and
[`will_delete.txt`](will_delete.txt) is the full audit: every directory name a
1,237-line list of candidates proposed, with a verdict for each one and the guard
that justifies it. Of 1,100 unique names, **86 can be marked and 930 cannot**:

| verdict | unique | meaning |
|---------|-------:|---------|
| POLICY  | 86  | a policy matches it, when its guard passes |
| PATH    | 84  | a nested path like `target/debug/deps`, covered by its parent |
| EXCLUDED | 293 | deliberately never matched, reason given |
| CANNOT-MATCH | 45 | a file, a glob, or a symlink the traversal cannot mark |
| NO POLICY | 592 | no tool in the default set creates this name |

So the short answer: purgable deletes build output, dependency trees, and caches,
and it will not touch your downloads, your logs, your vendored source, or any
directory it cannot first prove is disposable.

### mark

Walks the tree and applies every enabled policy from `~/.config/purgable.toml`,
writing a `PURGABLE` marker into each directory that matches and is large enough.

```
$ purgable mark ~/Projects
  3 matched  10.6G total
  ─────────────────────────────────────────────────────
              size  action          policy            path
  ─────────────────────────────────────────────────────
  [ 1/3 ]     10.0G  would mark      cargo-target      .../native/src-tauri/target
  [ 2/3 ]      561M  would mark      cargo-target      ~/Projects/purgable/target
  [ 3/3 ]      112M  already marked  node-modules      ~/Projects/web/node_modules

  Done.  2 marked  1 already marked
  run `purgable review ~/Projects` to decide what to do
```

Directories that already carry a marker are left alone, so `mark` is safe to
run repeatedly. Use `--dry-run` to preview, `--policy NAME` to apply one policy,
and `--min-size SIZE` to override the configured threshold.

### review

Prompts for each marked directory, **largest first**. The prompt is a bordered
block with the path on its own line, the deciding facts beneath it, and every
action spelled out:

```
  3 to review  1.3M total

  ┌──────────────────────────────────────────────────────────────┐
  │ ~/Projects/winnow-alpha/native/src-tauri/target              │
  │ 10.0G · cargo-target, 2h ago                                 │
  └──────────────────────────────────────────────────────────────┘
    d  delete    directory and contents
    c  clear     contents only, keep directory
    s  shred     overwrite files, keep directory
    x  shred all overwrite files, delete directory
    k  skip      keep everything, continue
    e  exit      stop now

  [1/3] Enter choice: d
  [ 1/3 ]  10.0G  deleted  ~/Projects/winnow-alpha/native/src-tauri/target

  Done.  10.0G freed  1 deleted  0 cleared  0 shredded, 2 skipped
```

Details worth knowing:

- Each decision gets a result line, so you always know what just happened.
- Once something has been freed, later prompts show a running `freed so far`.
- After `-ALL`, remaining directories are listed with their pending action
  instead of re-prompting.
- Pressing Enter says so explicitly rather than silently skipping.

### Display

- Paths are shortened to `~` and truncated to your terminal width, keeping the
  filename visible since that is the identifying part.
- Colour marks outcomes: cyan for pending, green for done, yellow for skipped,
  red for failures, magenta for the policy that matched.
- Colour is disabled when output is not a terminal, and honours `NO_COLOR` and
  `TERM=dumb`. Piped and redirected output is always plain.

### Threads

Scanning, measuring, and emptying directories all use every core, because the
work is almost entirely waiting on the filesystem rather than computing
anything. Set `PURGABLE_JOBS` to a number between 1 and 64 to cap it, which is
worth doing when a scan is competing with a build for the same disk.

### Actions

Method and scope are independent: pick how to destroy the data, and whether the
directory itself survives.

| Input | Method   | Scope                       |
|-------|----------|-----------------------------|
| `d`   | delete   | directory and contents      |
| `c`   | delete   | contents only, keep the dir |
| `s`   | shred    | contents only, keep the dir |
| `x`   | shred    | directory and contents      |
| `k`   | —        | skip this directory         |
| `e`   | —        | exit immediately            |

Append `-ALL` to apply an action to this and every later directory without
prompting again: `d-ALL`, `c-ALL`, `s-ALL`, `x-ALL`, `k-ALL`, `e-ALL`.

Pressing Enter means `k`. Unrecognised input is skipped with a notice. If an
action fails, that directory counts as skipped and the run continues.

`c` and `s` leave the `PURGABLE` marker in place, so the directory stays marked.
Use `unmark` to clear those markers.

### Shredding

Shredding overwrites each regular file with random data before removing it.
This cannot guarantee physical destruction on SSDs, flash storage, or
filesystems with copy-on-write or snapshots, and it recovers no extra space
beyond the delete. **Use `d` to reclaim space; use `s` or `x` for secrets.**

## Policies

Policies live in `~/.config/purgable.toml` (override the path with
`$PURGABLE_CONFIG`). Run `purgable init` to write a starter file, which ships
with 40 policies covering the common ecosystems.

```toml
[defaults]
min_size = "500M"
enabled = true

[[policy]]
name = "cargo-target"
dir_name = "target"
require_sibling = ["Cargo.toml"]
require_child_any = [".rustc_info.json", "debug", "release", "CACHE", "incremental"]
```

| Key                  | Meaning                                                |
|----------------------|--------------------------------------------------------|
| `name`               | Policy name, recorded in the marker                     |
| `dir_name`           | Exact directory name to match                           |
| `dir_name_any`       | Match any one of these names                           |
| `require_sibling`    | All of these must exist in the **parent**               |
| `require_sibling_any`| At least one must exist in the parent                   |
| `require_child_any`  | At least one must exist **inside** the directory        |
| `min_size`           | Overrides `defaults.min_size` for this policy           |
| `enabled`            | Set `false` to disable without deleting                 |

`require_sibling` and `require_sibling_any` are both applied, so a policy can
demand a `package.json` *and* a bundler config. `require_child_any` is an
`or`, so it cannot say "both of these" — which is why build systems that share
a directory name rely on the sibling check instead.

Policies are tried in file order and the first match wins, so put specific rules
above general ones. A per-policy `min_size` also beats `--min-size`, so leave it
off policies you want `--min-size` to keep controlling. Use `--min-size 50M` to
sweep the smaller caches; `purgable unmark --all` when a policy overreaches.

### Why the guards matter

Directory names like `build`, `target`, `dist` and `out` are shared by dozens of
tools, and they are also the names of real source directories. `require_sibling`
is what keeps a source tree that merely happens to be called `target` out of the
results:

- In a Linux kernel checkout, `drivers/target`, `include/target` and
  `Documentation/target` are directories named `target` containing actual code.
  Only a `Cargo.toml` sibling makes `target` Cargo build output.
- `linux/kernel/tools/build` contains a `Build` subdirectory, which is why the
  Xcode policy requires `Index.noindex` or `ModuleCache.noindex` instead of
  settling for `Build`.
- `dist` is where bundlers put output, and also where some projects park release
  archives and checksums they care about. The `js-bundle` policy demands a
  `package.json`, a bundler or TypeScript config, and an entry point inside.
- `bin` is never matched at all: virtualenvs, committed scripts and half the
  command line tools on a system have one.

### What is deliberately not matched

The shipped config only deletes what it can prove is disposable. Left alone:

- **User data** — `downloads`, `images`, `thumbnails`, `previews`, `videos`,
  `renders`, `samples`, `data`. A `Downloads` directory is exactly the thing
  never to automate.
- **Checked-in source** — `packages`, `deps`, `dependencies`, `third_party`,
  `external`, `sources`, `src`, `lib`, `scripts`.
- **Records** — `logs`, `reports`, `snapshots`, `fixtures`, `archives`. These
  can be the only account of what happened.
- **Tool home directories** — `~/.cargo`, `~/.rustup`, `~/.gradle`, `~/.m2`,
  `~/.ivy2`, `~/.sbt`, `~/.nuget`, `~/.gem`, `~/.bundle`, `~/.mix`, `~/.hex`,
  `~/.poetry`, `~/.pyenv`, `~/.uv`, `~/.terraform.d`, `~/.pulumi`,
  `~/.serverless`, `~/.aws-sam`, `~/.docker`, `~/.vercel`. These mix caches with
  credentials, installed tools, or deployment state — `~/.cargo` holds your
  `cargo install`ed binaries, `~/.gradle` can hold repository passwords, and
  `~/.pulumi` holds infrastructure state. Clear the cache subdirectory with the
  tool that owns it instead: `cargo cache`, `uv cache clean`, `go clean
  -modcache`, `docker builder prune`, `bazel clean --expunge`.
- **Tracked files** — `Package.resolved`, `Podfile.lock`, `.dockerignore`. These
  are files rather than directories, so they can never be marked, and deleting
  them changes your build.

A per-project Gradle `.gradle` and Terraform `.terraform` *are* matched, since
the build-system sibling tells them apart from the tool home directories of the
same name.

## Markers

Any regular file named exactly `PURGABLE` (case-sensitive, no extension) marks
its containing directory. Markers written by `mark` record their provenance:

```
purgable:v1
policy=cargo-target
marked_at=1759600751
size=10737418240
```

An empty marker is treated as hand-placed. That distinction is why `unmark`
removes only tool-written markers by default — pass `--all` to include manual
ones. Markers outside the requested scope are listed as `kept` rather than
silently omitted, so `0 removed` is never ambiguous.

The marker itself is never independently deleted, modified, or shredded. It is
removed only as a consequence of its directory being removed, or by `unmark`.

## Exit codes

- `0` completed (regardless of actions taken)
- `1` the root directory or config could not be read
- `2` invalid usage

## License

MIT