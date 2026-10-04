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

/// How many threads to use for filesystem work.
///
/// Override with `PURGABLE_JOBS` when benchmarking or when a scan is
/// competing with a build for the disk.
pub(crate) fn thread_count() -> usize {
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
    let threads = thread_count();
    let chunk = chunk.max(1);
    if threads == 1 {
        for slice in items.chunks(chunk) {
            f(slice);
        }
        return;
    }
    let next = AtomicUsize::new(0);
    thread::scope(|scope| {
        for _ in 0..threads {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::write_dir_with_content;
    use std::sync::Mutex;

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
    fn test_for_each_chunk_covers_everything_once() {
        let items: Vec<usize> = (0..1000).collect();
        let seen: Mutex<Vec<usize>> = Mutex::new(Vec::new());
        for_each_chunk(&items, 7, |slice| {
            let mut guard = seen.lock().unwrap();
            guard.extend_from_slice(slice);
        });
        let mut seen = seen.into_inner().unwrap();
        seen.sort_unstable();
        assert_eq!(seen, items);
    }

    #[test]
    fn test_for_each_chunk_on_empty_slice() {
        let items: Vec<usize> = Vec::new();
        for_each_chunk(&items, 8, |_| panic!("must not be called"));
    }
}
