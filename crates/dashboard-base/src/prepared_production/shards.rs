//! Production's emit shares its per-row and per-holder work with the run's idle workers
//! ([`replay::segments::share`]).

use std::{
    io,
    sync::{Mutex, PoisonError},
};

/// Runs `step` on contiguous chunks of `items`, each with the index of its first item, one
/// for each way the run's idle workers let the work go, and gives back what each gave, in
/// order: one chunk, run here, where none is idle.
pub(crate) fn chunks<T, R, F>(items: &mut [T], step: F) -> io::Result<Vec<R>>
where
    T: Send,
    R: Send,
    F: Fn(usize, &mut [T]) -> io::Result<R> + Sync,
{
    let ways = replay::segments::ways().min(items.len());
    if ways <= 1 {
        return Ok(vec![step(0, items)?]);
    }
    let size = items.len().div_ceil(ways);
    let parts: Vec<Mutex<(usize, &mut [T], Option<io::Result<R>>)>> = items
        .chunks_mut(size)
        .enumerate()
        .map(|(at, chunk)| Mutex::new((at * size, chunk, None)))
        .collect();
    replay::segments::share(parts.len(), &|index| {
        let mut part = parts[index].lock().unwrap_or_else(PoisonError::into_inner);
        let (first, chunk, given) = &mut *part;
        *given = Some(step(*first, chunk));
    });
    parts
        .into_iter()
        .map(|part| {
            let (_, _, given) = part.into_inner().unwrap_or_else(PoisonError::into_inner);
            given.unwrap_or_else(|| Err(io::Error::other("A production shard gave nothing")))
        })
        .collect()
}
