//! Independent work on the machine's threads, each result in its item's place.

use std::sync::atomic::{AtomicUsize, Ordering};

/// `work` over every item on up to the machine's threads, results in item order. Each worker
/// thread runs as its caller does ([`job_priority::Caller`]): it works for the caller, which
/// waits for it.
pub fn map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let caller = job_priority::Caller::current();
    map_entering(items, || caller.clone().enter(), work)
}

/// [`map`] with each worker thread first running `enter`, which sets the job and priority the
/// thread works at ([`job_priority`]).
pub fn map_entering<T: Sync, R: Send>(
    items: &[T],
    enter: impl Fn() + Sync,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(items.len());
    if workers <= 1 {
        return items.iter().map(work).collect();
    }
    let next = AtomicUsize::new(0);
    let mut results: Vec<Option<R>> = Vec::new();
    results.resize_with(items.len(), || None);
    std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    enter();
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else {
                            return done;
                        };
                        done.push((index, work(item)));
                    }
                })
            })
            .collect();
        for task in tasks {
            let done = join(task);
            for (index, result) in done {
                results[index] = Some(result);
            }
        }
    });
    results
        .into_iter()
        .map(|result| result.expect("every item is worked"))
        .collect()
}

/// Starts `work` on a scoped thread that runs as the calling thread does
/// ([`job_priority::Caller`]): the caller waits for it.
pub fn spawn<'scope, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    work: impl FnOnce() -> T + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, T> {
    let caller = job_priority::Caller::current();
    scope.spawn(move || {
        caller.enter();
        work()
    })
}

/// A scoped thread's result, its panic resumed on this thread.
pub fn join<T>(task: std::thread::ScopedJoinHandle<'_, T>) -> T {
    task.join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

#[cfg(test)]
mod tests {
    #[test]
    fn results_keep_their_items_places() {
        let items: Vec<usize> = (0..1000).collect();
        assert_eq!(
            super::map(&items, |item| item * 2),
            items.iter().map(|item| item * 2).collect::<Vec<_>>()
        );
        assert!(super::map(&[] as &[u8], |_| 0).is_empty());
    }
}
