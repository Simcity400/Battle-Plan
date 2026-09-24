//! Order-preserving parallel map over a bounded number of worker threads.
//!
//! Content loads read and decode one file per item, and a modded content set
//! can hold tens of thousands of flag, portrait or country files. Spawning a
//! thread per file creates that many OS threads at once, and a spawn the OS
//! refuses panics the load. These helpers keep the worker count at the
//! machine's parallelism and return the results in input order, so callers
//! merge them exactly as a serial load would.

use std::{num::NonZeroUsize, thread};

/// Maps `items` through `work` on at most `available_parallelism` threads,
/// returning the results in input order.
///
/// Items are split into contiguous runs, one per worker. A worker the OS
/// refuses to start leaves its run to the calling thread instead of failing.
pub fn map<T, R, F>(items: &[T], work: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let workers = thread::available_parallelism()
        .map_or(4, NonZeroUsize::get)
        .min(items.len());
    if workers <= 1 {
        return items.iter().map(&work).collect();
    }
    let run = items.len().div_ceil(workers);
    let work = &work;
    thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(run)
            .map(|chunk| {
                let spawned = thread::Builder::new()
                    .spawn_scoped(scope, move || chunk.iter().map(work).collect::<Vec<R>>());
                (chunk, spawned.ok())
            })
            .collect();
        let mut out = Vec::with_capacity(items.len());
        for (chunk, handle) in handles {
            match handle {
                Some(handle) => match handle.join() {
                    Ok(results) => out.extend(results),
                    Err(panic) => std::panic::resume_unwind(panic),
                },
                None => out.extend(chunk.iter().map(work)),
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn results_keep_the_input_order_for_many_items() {
        let items: Vec<u32> = (0..75_000).collect();
        let out = super::map(&items, |value| u64::from(*value) * 3);
        assert_eq!(out.len(), items.len());
        assert!(out.iter().enumerate().all(|(i, v)| *v == i as u64 * 3));
        assert!(super::map(&[] as &[u32], |value| *value).is_empty());
        assert_eq!(super::map(&[7u8], |value| *value + 1), [8]);
    }
}
