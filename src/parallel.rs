//! Parallel filesystem traversal.
//!
//! Walking a tree, measuring it, and emptying it are all dominated by
//! syscalls that leave the CPU idle, so the only way to go faster is to
//! overlap them. These helpers hand work to every core while producing exactly
//! the result a single-threaded walk would: nothing is reported until the walk
//! finishes, and each command sorts what it collected before printing.
//!
//! Semantics deliberately match the previous `walkdir` behaviour, because that
//! behaviour is the safety model: symlinks are never descended into, and a
//! directory entry's type comes from the directory listing itself, so a
//! symlink named like a build output is still rejected.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread;

/// Override for the worker count, so tests and benchmarks can pin the pool
/// size without touching the process environment. `0` means "use the default".
static THREAD_POOL_OVERRIDE: AtomicUsize = AtomicUsize::new(0);

/// How many threads to use for filesystem work.
///
/// Override with `PURGABLE_JOBS` when benchmarking or when a scan is
/// competing with a build for the disk; tests and benchmarks can set
/// [`THREAD_POOL_OVERRIDE`] directly instead of going through the environment.
pub(crate) fn thread_count() -> usize {
    let overridden = THREAD_POOL_OVERRIDE.load(Ordering::Relaxed);
    if overridden != 0 {
        return overridden.clamp(1, 64);
    }
    if let Some(raw) = std::env::var_os("PURGABLE_JOBS") {
        if let Some(n) = raw.to_str().and_then(|s| s.trim().parse::<usize>().ok()) {
            return n.clamp(1, 64);
        }
    }
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 32)
}

/// One directory queued for visiting, tagged with the index of the root it
/// belongs to so a caller can attribute results back to its own input.
struct Item {
    task: usize,
    path: PathBuf,
}

/// Handle passed to a visit callback: which root a directory belongs to, and
/// where to send its children.
pub(crate) struct Cursor<'a, 'b> {
    task: usize,
    queue: &'a Queue,
    local: &'b mut Vec<Item>,
}

impl Cursor<'_, '_> {
    /// Index of the root this directory was reached from.
    pub(crate) fn task(&self) -> usize {
        self.task
    }

    /// Queue a subdirectory to be visited.
    ///
    /// The caller is responsible for having established that `child` is a real
    /// directory: the entry type comes from the parent listing, and symlinks
    /// are filtered out before this point.
    pub(crate) fn descend(&mut self, child: PathBuf) {
        let task = self.task;
        self.queue.push(self.local, Item { task, path: child });
    }
}

/// The shared queue: how many directories are outstanding and where the next
/// one comes from.
struct Queue {
    pending: Mutex<VecDeque<Item>>,
    outstanding: AtomicUsize,
    wake: Condvar,
    cap: usize,
}

impl Queue {
    fn push(&self, local: &mut Vec<Item>, item: Item) {
        self.outstanding.fetch_add(1, Ordering::Relaxed);
        let mut pending = self.lock();
        if pending.len() < self.cap {
            pending.push_back(item);
            drop(pending);
            self.wake.notify_one();
        } else {
            // The shared queue is full, which happens on very wide trees.
            // Holding it in this thread keeps the queue bounded without
            // stalling the others.
            local.push(item);
        }
    }

    fn take(&self, local: &mut Vec<Item>) -> Option<Item> {
        if let Some(item) = local.pop() {
            return Some(item);
        }
        let mut pending = self.lock();
        loop {
            if let Some(item) = pending.pop_back() {
                return Some(item);
            }
            if self.outstanding.load(Ordering::Acquire) == 0 {
                return None;
            }
            // Everything outstanding is in another thread's local stack, so
            // there is genuinely nothing to do until that thread finishes.
            pending = self.wake.wait(pending).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Marks one directory finished. Runs even if the visit panicked, so a
    /// panic in one thread cannot strand the others.
    fn done(&self) {
        if self.outstanding.fetch_sub(1, Ordering::Release) == 1 {
            drop(self.lock());
            self.wake.notify_all();
        }
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<Item>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Calls `finish` when dropped, so an unwinding visit still retires its slot.
struct Retire<'a> {
    queue: &'a Queue,
}

impl Drop for Retire<'_> {
    fn drop(&mut self) {
        self.queue.done();
    }
}

/// Visit every directory below `roots`, in parallel.
///
/// `visit` receives the directory, its open entries, and a cursor for queueing
/// its children, so it can both inspect the listing and decide what to descend
/// into. `on_error` reports directories that could not be opened.
pub(crate) fn walk<F, E>(roots: &[PathBuf], visit: F, on_error: E)
where
    F: Fn(&mut Cursor, &Path, fs::ReadDir) + Sync,
    E: Fn(&Path, &io::Error) + Sync,
{
    let threads = thread_count();
    if roots.is_empty() {
        return;
    }
    let seed: VecDeque<Item> = roots
        .iter()
        .enumerate()
        .map(|(task, path)| Item {
            task,
            path: path.clone(),
        })
        .collect();
    let queue = Queue {
        pending: Mutex::new(seed),
        outstanding: AtomicUsize::new(roots.len()),
        wake: Condvar::new(),
        cap: (threads * 32).clamp(256, 4096),
    };

    let drive = |queue: &Queue| {
        let mut local = Vec::new();
        while let Some(item) = queue.take(&mut local) {
            let _retire = Retire { queue };
            match fs::read_dir(&item.path) {
                Ok(entries) => visit(
                    &mut Cursor {
                        task: item.task,
                        queue,
                        local: &mut local,
                    },
                    &item.path,
                    entries,
                ),
                Err(e) => on_error(&item.path, &e),
            }
        }
    };

    if threads == 1 {
        drive(&queue);
        return;
    }
    thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| drive(&queue));
        }
    });
}

/// Run `f` over `items` in parallel, in fixed-size chunks so that one
/// oversized item cannot leave a thread idle at the end.
pub(crate) fn for_each_chunk<T, F>(items: &[T], chunk: usize, f: F)
where
    T: Sync,
    F: Fn(&[T]) + Sync,
{
    if items.is_empty() {
        return;
    }
    let chunk = chunk.max(1);
    let workers = chunk_worker_count(items.len(), chunk, thread_count());
    if workers == 1 {
        for slice in items.chunks(chunk) {
            f(slice);
        }
        return;
    }
    let next = AtomicUsize::new(0);
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let start = next.fetch_add(chunk, Ordering::Relaxed);
                if start >= items.len() {
                    return;
                }
                let end = (start + chunk).min(items.len());
                f(&items[start..end]);
            });
        }
    });
}

fn chunk_worker_count(item_count: usize, chunk_size: usize, max_workers: usize) -> usize {
    item_count.div_ceil(chunk_size.max(1)).min(max_workers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_dir_with_content, write_file};
    use std::sync::Mutex;
    use std::time::Instant;

    fn tree(root: &Path) {
        // A deliberately lopsided tree: one wide directory and one deep one,
        // so a walker that only handles the easy shape would still pass.
        write_dir_with_content(root, &[("a/1/2/3/4/deep.txt", "x")]);
        let wide = root.join("wide");
        fs::create_dir_all(&wide).unwrap();
        for i in 0..64 {
            write_dir_with_content(&wide.join(format!("d{i}/inner")), &[("f", "x")]);
        }
    }

    #[test]
    fn test_walk_visits_every_directory_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path());

        let seen: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
        walk(
            &[dir.path().to_path_buf()],
            |cursor, dir, entries| {
                seen.lock().unwrap().push(dir.to_path_buf());
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        cursor.descend(entry.path());
                    }
                }
            },
            |_, _| {},
        );

        let seen = seen.into_inner().unwrap();
        // root + deep chain (a, a/1 .. a/1/2/3/4) + wide + 64 dirs + 64 inners.
        assert_eq!(seen.len(), 1 + 5 + 1 + 64 + 64, "{:?}", seen.len());
        let mut sorted = seen.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "a directory was visited twice");
    }

    #[test]
    fn test_walk_keeps_tasks_separate() {
        let dir = tempfile::tempdir().unwrap();
        let one = dir.path().join("one");
        let two = dir.path().join("two");
        write_dir_with_content(&one.join("inner"), &[("f", "x")]);
        write_dir_with_content(&two.join("inner"), &[("f", "x")]);

        let per_task: Mutex<Vec<Vec<PathBuf>>> = Mutex::new(vec![Vec::new(), Vec::new()]);
        walk(
            &[one.clone(), two.clone()],
            |cursor, dir, entries| {
                per_task.lock().unwrap()[cursor.task()].push(dir.to_path_buf());
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        cursor.descend(entry.path());
                    }
                }
            },
            |_, _| {},
        );

        let per_task = per_task.into_inner().unwrap();
        assert_eq!(per_task[0].len(), 2, "{:?}", per_task[0]);
        assert_eq!(per_task[1].len(), 2, "{:?}", per_task[1]);
        // A subtree must never contribute to another root's tally.
        assert!(
            per_task[0].iter().all(|p| p.starts_with(&one)),
            "{:?}",
            per_task[0]
        );
        assert!(
            per_task[1].iter().all(|p| p.starts_with(&two)),
            "{:?}",
            per_task[1]
        );
    }

    #[test]
    fn test_walk_reports_unreadable_directories_and_keeps_going() {
        let dir = tempfile::tempdir().unwrap();
        write_dir_with_content(&dir.path().join("locked/inner"), &[("f", "x")]);
        write_dir_with_content(&dir.path().join("open"), &[("f", "x")]);
        let locked = dir.path().join("locked");
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o000);
        fs::set_permissions(&locked, perms).unwrap();

        let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let visited: Mutex<usize> = Mutex::new(0);
        walk(
            &[dir.path().to_path_buf()],
            |cursor, _, entries| {
                *visited.lock().unwrap() += 1;
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        cursor.descend(entry.path());
                    }
                }
            },
            |path, e| {
                errors
                    .lock()
                    .unwrap()
                    .push(format!("{}: {e}", path.display()))
            },
        );

        // Restore before asserting so the temp directory can be cleaned up.
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&locked, perms).unwrap();

        if unsafe { libc::geteuid() } == 0 {
            // Root can read anything, so there is nothing to report.
            return;
        }
        assert_eq!(
            *visited.lock().unwrap(),
            2,
            "the open branch must still be visited"
        );
        assert_eq!(errors.lock().unwrap().len(), 1, "{:?}", errors);
    }

    #[test]
    fn test_walk_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        write_dir_with_content(&outside, &[("secret", "x")]);
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

        let seen: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
        walk(
            std::slice::from_ref(&root),
            |cursor, dir, entries| {
                seen.lock().unwrap().push(dir.to_path_buf());
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        cursor.descend(entry.path());
                    }
                }
            },
            |_, _| {},
        );

        let seen = seen.into_inner().unwrap();
        assert_eq!(seen, vec![root], "a symlink must not be descended into");
    }

    #[test]
    fn test_walk_survives_a_directory_wider_than_the_shared_queue() {
        // The shared queue is bounded, so a directory with more children than
        // the bound spills into the visiting thread's own stack. Overflowing it
        // has to keep every child, not just the ones that fitted.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        let children = 5_000;
        for i in 0..children {
            write_dir_with_content(&root.join(format!("d{i}")), &[("f", "x")]);
        }

        let seen: Mutex<usize> = Mutex::new(0);
        walk(
            std::slice::from_ref(&root),
            |cursor, _, entries| {
                *seen.lock().unwrap() += 1;
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        cursor.descend(entry.path());
                    }
                }
            },
            |_, _| {},
        );

        assert_eq!(*seen.lock().unwrap(), children + 1);
    }

    #[test]
    fn test_for_each_chunk_covers_requested_sizes_once() {
        for item_count in [1_usize, 2, 8, 16, 64, 1024] {
            let items: Vec<usize> = (0..item_count).collect();
            for chunk_size in [0_usize, 1, 2, 8, 16, 64, 1024] {
                let seen: Mutex<Vec<usize>> = Mutex::new(Vec::new());
                for_each_chunk(&items, chunk_size, |slice| {
                    seen.lock().unwrap().extend_from_slice(slice);
                });
                let mut seen = seen.into_inner().unwrap();
                seen.sort_unstable();
                assert_eq!(seen, items, "items={item_count}, chunk={chunk_size}");
            }
        }
    }

    #[test]
    fn test_chunk_worker_count_never_exceeds_available_chunks() {
        for item_count in [0_usize, 1, 2, 8, 16, 64, 1024] {
            for chunk_size in [0_usize, 1, 2, 8, 16, 64, 1024] {
                for max_workers in [1, 2, 8, 16, 64] {
                    let chunks = item_count.div_ceil(chunk_size.max(1));
                    assert_eq!(
                        chunk_worker_count(item_count, chunk_size, max_workers),
                        chunks.min(max_workers),
                        "items={item_count}, chunk={chunk_size}, workers={max_workers}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_for_each_chunk_on_empty_slice() {
        let items: Vec<usize> = Vec::new();
        for_each_chunk(&items, 8, |_| panic!("must not be called"));
    }

    // ------------------------------------------------------------------
    // STRM-477: walk load-balance benchmark harness.
    //
    // Runs synthetic trees through `walk` and reports how the work spread
    // over the worker threads: elapsed time, entries processed per thread,
    // and the per-thread distribution. Ignored by default; run with
    // `cargo test -- --ignored`.
    // ------------------------------------------------------------------

    /// Instrumentation for one `walk` run: how many entries each visiting
    /// thread processed, and the grand total.
    #[derive(Default)]
    struct WalkRunStats {
        per_thread: Mutex<Vec<(thread::ThreadId, usize)>>,
        total: AtomicUsize,
    }

    impl WalkRunStats {
        fn new() -> Self {
            Self::default()
        }

        fn record(&self, entries: usize) {
            self.total.fetch_add(entries, Ordering::Relaxed);
            let mut per_thread = self.per_thread.lock().unwrap_or_else(|e| e.into_inner());
            let id = thread::current().id();
            if let Some(slot) = per_thread.iter_mut().find(|(tid, _)| *tid == id) {
                slot.1 += entries;
            } else {
                per_thread.push((id, entries));
            }
        }

        fn per_thread(&self) -> Vec<(thread::ThreadId, usize)> {
            self.per_thread
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }

        fn total(&self) -> usize {
            self.total.load(Ordering::Relaxed)
        }
    }

    /// Run one `walk` over `root` while counting entries per visiting thread
    /// and timing the run.
    fn benchmark_walk_once(root: &Path) -> (std::time::Duration, WalkRunStats) {
        let stats = WalkRunStats::new();
        let start = Instant::now();
        let root_buf = root.to_path_buf();
        walk(
            std::slice::from_ref(&root_buf),
            |cursor, _, entries| {
                let children: Vec<_> = entries.flatten().collect();
                stats.record(children.len());
                for entry in children {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        cursor.descend(entry.path());
                    }
                }
            },
            |_, _| {},
        );
        (start.elapsed(), stats)
    }

    fn write_wide_tree(root: &Path) {
        // Broad and shallow: many top-level directories, each with a few
        // subdirectories of files. The queue fans out wide.
        fs::create_dir_all(root).unwrap();
        for i in 0..64 {
            let top = root.join(format!("d{i}"));
            fs::create_dir_all(&top).unwrap();
            for j in 0..8 {
                let sub = top.join(format!("s{j}"));
                fs::create_dir_all(&sub).unwrap();
                for f in 0..4 {
                    write_file(&sub.join(format!("f{f}")), "x");
                }
            }
        }
    }

    fn write_deep_tree(root: &Path) {
        // Narrow and deep: a single chain of nested directories.
        let mut dir = root.to_path_buf();
        fs::create_dir_all(&dir).unwrap();
        for _ in 0..12 {
            dir = dir.join("level");
            fs::create_dir_all(&dir).unwrap();
        }
        write_file(&dir.join("deep.txt"), "x");
    }

    fn write_mixed_tree(root: &Path) {
        // A dozen projects at two levels of depth, each holding nested crates.
        fs::create_dir_all(root).unwrap();
        for p in 0..12 {
            let project = root.join(format!("p{p}"));
            fs::create_dir_all(&project).unwrap();
            for c in 0..3 {
                let crate_dir = project.join(format!("crate{c}"));
                fs::create_dir_all(&crate_dir).unwrap();
                for f in 0..2 {
                    write_file(&crate_dir.join(format!("f{f}")), "x");
                }
            }
        }
    }

    #[test]
    #[ignore = "STRM-477: synthetic walk load-balance benchmark; run with `cargo test -- --ignored`"]
    fn benchmark_walk_load_balance_wide_tree() {
        let dir = tempfile::tempdir().unwrap();
        write_wide_tree(dir.path());
        for jobs in [1_usize, 2, 4, 8] {
            THREAD_POOL_OVERRIDE.store(jobs, Ordering::Relaxed);
            let (elapsed, stats) = benchmark_walk_once(dir.path());
            let per_thread = stats.per_thread();
            eprintln!(
                "benchmark wide: jobs={jobs} elapsed={elapsed:?} entries={} per_thread={per_thread:?}",
                stats.total()
            );
        }
    }

    #[test]
    #[ignore = "STRM-477: synthetic walk load-balance benchmark; run with `cargo test -- --ignored`"]
    fn benchmark_walk_load_balance_deep_tree() {
        let dir = tempfile::tempdir().unwrap();
        write_deep_tree(dir.path());
        for jobs in [1_usize, 2, 4, 8] {
            THREAD_POOL_OVERRIDE.store(jobs, Ordering::Relaxed);
            let (elapsed, stats) = benchmark_walk_once(dir.path());
            eprintln!(
                "benchmark deep: jobs={jobs} elapsed={elapsed:?} entries={} per_thread={:?}",
                stats.total(),
                stats.per_thread()
            );
        }
    }

    #[test]
    #[ignore = "STRM-477: synthetic walk load-balance benchmark; run with `cargo test -- --ignored`"]
    fn benchmark_walk_load_balance_mixed_tree() {
        let dir = tempfile::tempdir().unwrap();
        write_mixed_tree(dir.path());
        for jobs in [1_usize, 2, 4, 8] {
            THREAD_POOL_OVERRIDE.store(jobs, Ordering::Relaxed);
            let (elapsed, stats) = benchmark_walk_once(dir.path());
            eprintln!(
                "benchmark mixed: jobs={jobs} elapsed={elapsed:?} entries={} per_thread={:?}",
                stats.total(),
                stats.per_thread()
            );
        }
    }
}
