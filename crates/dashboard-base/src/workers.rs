//! Work a query splits over spare cores: each worker reads the query's one publication and runs
//! under its priority, CPU accounting and memory labels.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// `work` of each item, in order, over at most half the cores, each worker taking the next item
/// as it finishes one.
pub(crate) fn map<T: Sync, R: Send>(
    items: &[T],
    label: &'static str,
    work: impl Fn(&T) -> io::Result<R> + Sync,
) -> io::Result<Vec<R>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |cores| (cores.get() / 2).max(1))
        .min(items.len());
    if workers <= 1 {
        return items.iter().map(&work).collect();
    }
    let scopes = store::Scopes::current();
    let context = replay_frame::cpu::Context::capture();
    let memory = engine_allocation::labels::current();
    let room = engine_allocation::spill::room();
    let (next, failed) = (AtomicUsize::new(0), AtomicBool::new(false));
    let (work, next, failed) = (&work, &next, &failed);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let (scopes, context, room) = (scopes.clone(), context.clone(), room.clone());
                job_priority::spawn_in(scope, move || {
                    let _scopes = scopes.enter();
                    let _memory = engine_allocation::labels::resume(memory);
                    let _room = engine_allocation::spill::enter(room);
                    let _cpu = context.enter(label);
                    let mut done = Vec::new();
                    while !failed.load(Ordering::Relaxed) {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else {
                            break;
                        };
                        match work(item) {
                            Ok(result) => done.push((index, result)),
                            Err(error) => {
                                failed.store(true, Ordering::Relaxed);
                                return Err(error);
                            }
                        }
                    }
                    Ok(done)
                })
            })
            .collect();
        let mut out: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
        let mut error = None;
        for handle in handles {
            match handle.join() {
                Ok(Ok(done)) => {
                    for (index, result) in done {
                        out[index] = Some(result);
                    }
                }
                Ok(Err(why)) => {
                    error.get_or_insert(why);
                }
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(out
            .into_iter()
            .map(|result| result.expect("every item worked"))
            .collect())
    })
}
