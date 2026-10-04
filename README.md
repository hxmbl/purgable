# purgable

Mark disposable directories automatically, then decide what to do with them.

`purgable` finds directories that are safe to throw away — Cargo build output,
`node_modules`, virtualenvs — drops a `PURGABLE` marker in each, and then asks
you what to do with them. You keep the decision; the tool does the finding.

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

### mark

Walks the tree and applies every enabled policy from `~/.config/purgable.toml`,
writing a `PURGABLE` marker into each directory that matches and is large enough.

```
$ purgable mark ~/Projects
  marked        10.0G  /Users/me/Projects/winnow-alpha/native/src-tauri/target (cargo-target)
  would mark     561M  /Users/me/Projects/purgable/target (cargo-target)

Matched 3. 1 new, 2 already marked. Run `purgable review` to decide what to do.
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
`$PURGABLE_CONFIG`). Run `purgable init` to write a starter file with
`cargo-target`, `node-modules`, and `python-venv` policies.

```toml
[defaults]
min_size = "500M"
enabled = true

[[policy]]
name = "cargo-target"
dir_name = "target"
require_sibling = ["Cargo.toml"]
require_child_any = [".rustc_info.json", "debug", "release", "CACHE"]
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

Policies are tried in file order and the first match wins, so put specific rules
above general ones.

### Why `require_sibling` matters

A directory called `target` is only Cargo build output if its parent has a
`Cargo.toml`. Without that check, real source trees get swept up: in a Linux
kernel checkout, `drivers/target`, `include/target`, and `fs/target` are all
directories named `target` containing actual code, and Xcode's
`XCBuildData/target` collides too. The starter config requires both a
`Cargo.toml` sibling and a build-artifact child.

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