# purgable

Find directories marked with a `PURGABLE` file and ask before deleting or shredding them.

## How it works

`purgable <directory>` recursively scans for regular files named exactly
`PURGABLE` (case-sensitive, no extension). Each such file marks its containing
directory as purgable. For every marked directory, you are asked what to do.

The `PURGABLE` marker itself is NEVER independently deleted, modified, or
shredded. It is only removed as a consequence of its containing directory being
deleted or shredded.

Symlinks are never followed, and filesystem errors encountered while walking are
reported as warnings without aborting the scan.

## Install

```sh
brew install Hxmbl/tap/purgable
```

Or build from source (dep-free Go module):

```sh
go build -o purgable .
```

Prebuilt binaries for Linux and macOS (amd64/arm64) are attached to each
[release](https://github.com/Hxmbl/purgable/releases).

## Usage

```
purgable <directory>
purgable --help | -h
purgable --version | -v
```

Example:

```sh
$ purgable ~/Downloads
Action for /Users/me/Downloads/old-stuff [d/s/k/e]? d
Action for /Users/me/Downloads/temp [d/s/k/e]? s

Done. Found 2, deleted 1, shredded 1, skipped 0.
```

### Actions

| Input  | Action                                                        |
|--------|---------------------------------------------------------------|
| `d`    | Delete the containing directory and everything in it.         |
| `s`    | Shred (secure-delete) the contents, then remove everything.   |
| `k`    | Skip this directory and continue scanning.                    |
| `e`    | Exit immediately.                                             |
| `d-ALL`| Delete this and all subsequent PURGABLE directories.          |
| `s-ALL`| Shred this and all subsequent PURGABLE directories.           |
| `k-ALL`| Skip this and all subsequent PURGABLE directories.            |
| `e-ALL`| Exit immediately (equivalent to `e`).                         |

Pressing Enter with no input is treated as `k` (skip). Unrecognised input is
treated as `k` with a notice. If a delete or shred fails, that directory is
counted as skipped and the run continues.

### Shredding

`shredDir` overwrites each regular file with random data before removing it.
This cannot guarantee physical destruction on SSDs, flash storage, or
filesystems with copy-on-write/snapshots.

## Exit codes

- `0` completed (regardless of actions taken)
- `1` the root directory does not exist or cannot be accessed
- `2` invalid usage

## License

MIT