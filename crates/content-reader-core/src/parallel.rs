
use std::sync::atomic::{AtomicUsize, Ordering};

pub mod cpu;
pub mod dag;

pub fn map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let caller = job_priority::Caller::current();
    map_entering(items, || caller.clone().enter(), work)
}

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
    let diagnostics = crate::content::diagnostics::Context::capture();
    let memory = engine_allocation::labels::current();
    let mut results: Vec<Option<R>> = Vec::new();
    results.resize_with(items.len(), || None);
    std::thread::scope(|scope| {
        let (next, enter, work) = (&next, &enter, &work);
        let tasks: Vec<_> = (0..workers)
            .map(|_| {
                let cpu = job_priority::accounting::Context::capture();
                let diagnostics = &diagnostics;
                job_priority::spawn_in(
                    scope,
                    crate::content::generation::carry(cpu::carry(move || {
                        let _memory = engine_allocation::labels::resume(memory);
                        diagnostics.enter(|| {
                            let _cpu = cpu.enter("content parallel");
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
                    })),
                )
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

pub fn spawn<'scope, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    work: impl FnOnce() -> T + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, T> {
    job_priority::spawn_in(scope, carried(work))
}

fn carried<T>(work: impl FnOnce() -> T + Send) -> impl FnOnce() -> T + Send {
    let caller = job_priority::Caller::current();
    let cpu = job_priority::accounting::Context::capture();
    let diagnostics = crate::content::diagnostics::Context::capture();
    let memory = engine_allocation::labels::current();
    crate::content::generation::carry(cpu::carry(move || {
        let _memory = engine_allocation::labels::resume(memory);
        diagnostics.enter(|| {
            let _cpu = cpu.enter("content helper");
            caller.enter();
            work()
        })
    }))
}

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
