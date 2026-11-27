
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
    let strict = crate::content::is_strict();
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
                        let _strict = crate::content::enter_strict(strict);
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
    let strict = crate::content::is_strict();
    crate::content::generation::carry(cpu::carry(move || {
        let _memory = engine_allocation::labels::resume(memory);
        let _strict = crate::content::enter_strict(strict);
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
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn results_keep_their_items_places() {
        let items: Vec<usize> = (0..1000).collect();
        assert_eq!(
            super::map(&items, |item| item * 2),
            items.iter().map(|item| item * 2).collect::<Vec<_>>()
        );
        assert!(super::map(&[] as &[u8], |_| 0).is_empty());
    }

    #[test]
    fn strict_setting_reaches_workers_and_restores_on_return_and_panic() {
        let items = [0usize, 1, 2, 3];

        let non_strict_map = super::map(&items, |_| crate::content::is_strict());
        assert!(non_strict_map.iter().all(|&strict| !strict));

        let strict_map =
            crate::content::strict(|| super::map(&items, |_| crate::content::is_strict()));
        assert!(strict_map.iter().all(|&strict| strict));
        assert!(!crate::content::is_strict());

        let (spawn_non_strict, spawn_strict) = std::thread::scope(|scope| {
            let ordinary = super::join(super::spawn(scope, crate::content::is_strict));
            let strict = crate::content::strict(|| {
                super::join(super::spawn(scope, crate::content::is_strict))
            });
            (ordinary, strict)
        });
        assert!(!spawn_non_strict);
        assert!(spawn_strict);

        let dag_strict = crate::content::strict(|| {
            let mut dag = super::dag::Dag::default();
            let first = dag.add(&[], || Ok(crate::content::is_strict()));
            let second = dag.add(&[], || Ok(crate::content::is_strict()));
            dag.run();
            (
                first.take().expect("first dag task succeeded"),
                second.take().expect("second dag task succeeded"),
            )
        });
        assert!(dag_strict.0 && dag_strict.1);

        let carried_false = super::carried(crate::content::is_strict);
        crate::content::strict(|| {
            assert!(!carried_false());
            assert!(crate::content::is_strict());
        });
        assert!(!crate::content::is_strict());

        let carried_true = crate::content::strict(|| super::carried(crate::content::is_strict));
        assert!(carried_true());
        assert!(!crate::content::is_strict());

        let panic_false = super::carried(|| {
            assert!(!crate::content::is_strict());
            panic!("worker panic under non-strict");
        });
        crate::content::strict(|| {
            assert!(catch_unwind(AssertUnwindSafe(panic_false)).is_err());
            assert!(crate::content::is_strict());
        });
        assert!(!crate::content::is_strict());

        let panic_true = crate::content::strict(|| {
            super::carried(|| {
                assert!(crate::content::is_strict());
                panic!("worker panic under strict");
            })
        });
        assert!(catch_unwind(AssertUnwindSafe(panic_true)).is_err());
        assert!(!crate::content::is_strict());
    }
}
