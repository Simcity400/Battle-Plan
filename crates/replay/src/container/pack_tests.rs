//! The pack pool: submission order, the sequential writer's first error and
//! the cores helpers may take.

use super::*;
use std::time::{Duration, Instant};

/// A job: its value, how long it takes and how it ends.
#[derive(Clone, Copy)]
enum End {
    Packed,
    Fails(&'static str),
    Panics(&'static str),
}

fn pack((value, millis, end): (usize, u64, End), _: &Cores) -> io::Result<usize> {
    thread::sleep(Duration::from_millis(millis));
    match end {
        End::Packed => Ok(value * 10),
        End::Fails(reason) => Err(io::Error::other(reason)),
        End::Panics(reason) => panic!("{reason}"),
    }
}

/// `jobs` submitted in order, then `written`, packed on `workers` threads.
fn run(
    workers: usize,
    jobs: &[(usize, u64, End)],
    written: io::Result<&'static str>,
) -> io::Result<(&'static str, Vec<usize>)> {
    blocks(workers, pack, |packer| {
        for job in jobs {
            packer.submit(*job)?;
        }
        written
    })
}

const COUNTS: [usize; 4] = [1, 2, 3, 8];

#[test]
fn packed_jobs_keep_submission_order_for_any_worker_count() {
    // Later jobs finish first.
    let jobs: Vec<_> = (0..24)
        .map(|value| (value, (24 - value as u64) % 5, End::Packed))
        .collect();
    for workers in COUNTS {
        let (written, packed) = run(workers, &jobs, Ok("written")).unwrap();
        assert_eq!(written, "written");
        assert_eq!(packed, (0..24).map(|value| value * 10).collect::<Vec<_>>());
    }
}

#[test]
fn the_first_failed_job_in_order_wins_over_later_failures_and_the_writer() {
    let mut jobs: Vec<_> = (0..12).map(|value| (value, 1, End::Packed)).collect();
    // The earlier failure is the slower one.
    jobs[3] = (3, 40, End::Fails("three"));
    jobs[7] = (7, 0, End::Fails("seven"));
    for workers in COUNTS {
        let error = run(workers, &jobs, Err(io::Error::other("writer"))).unwrap_err();
        assert_eq!(error.to_string(), "three", "{workers} workers");
    }
}

#[test]
fn the_writers_error_follows_every_job_submitted_before_it() {
    let jobs: Vec<_> = (0..6).map(|value| (value, 5, End::Packed)).collect();
    for workers in COUNTS {
        let error = run(workers, &jobs, Err(io::Error::other("writer"))).unwrap_err();
        assert_eq!(error.to_string(), "writer");
    }
    // A failure the writer never submitted cannot happen.
    for workers in COUNTS {
        let error = blocks(workers, pack, |packer| {
            packer.submit((0, 5, End::Packed))?;
            Err::<(), _>(io::Error::other("writer"))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "writer");
    }
}

#[test]
fn a_panicking_job_resumes_in_order_after_earlier_errors() {
    let reason = |jobs: &[(usize, u64, End)], workers| match panic::catch_unwind(|| {
        run(workers, jobs, Ok("written"))
    }) {
        Ok(result) => result.unwrap_err().to_string(),
        Err(panic) => format!("panic {}", panic.downcast_ref::<String>().unwrap()),
    };
    let mut jobs: Vec<_> = (0..8).map(|value| (value, 2, End::Packed)).collect();
    jobs[2] = (2, 30, End::Fails("two"));
    jobs[4] = (4, 0, End::Panics("four"));
    let mut first = jobs.clone();
    first[1] = (1, 30, End::Panics("one"));
    for workers in COUNTS {
        assert_eq!(reason(&jobs, workers), "two");
        assert_eq!(reason(&first, workers), "panic one");
    }
}

#[test]
fn a_job_failing_before_the_writer_panics_still_fails_first() {
    for workers in COUNTS {
        let error = panic::catch_unwind(|| {
            blocks(workers, pack, |packer| {
                packer.submit((0, 30, End::Fails("zero")))?;
                packer.submit((1, 0, End::Packed))?;
                panic!("writer")
            })
            .map(|_: ((), Vec<usize>)| ())
        })
        .expect("the job's error, not the writer's panic")
        .unwrap_err();
        assert_eq!(error.to_string(), "zero", "{workers} workers");
    }
}

#[test]
fn each_returns_results_in_item_order_and_the_first_error_in_it() {
    for limit in COUNTS {
        let cores = Cores::new(limit);
        let items: Vec<u64> = (0..16).collect();
        let doubled = each(&cores, items.clone(), |item| {
            thread::sleep(Duration::from_millis((16 - item) % 3));
            Ok(item * 2)
        })
        .unwrap();
        assert_eq!(
            doubled,
            items.iter().map(|item| item * 2).collect::<Vec<_>>()
        );
        let error = each(&cores, items, |item| {
            match item {
                5 => thread::sleep(Duration::from_millis(30)),
                9 => {}
                _ => return Ok(item),
            }
            Err(io::Error::other(format!("item {item}")))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "item 5");
        // Every helper returned its core.
        assert_eq!(cores.take(usize::MAX), limit - 1);
    }
}

#[test]
fn helpers_take_only_free_cores_and_workers_wait_for_one() {
    let cores = Cores::new(3);
    assert_eq!(cores.take(5), 2);
    assert_eq!(cores.take(1), 0);
    cores.give(1);
    assert_eq!(cores.take(4), 1);
    // A worker starting a job waits until a core is free.
    thread::scope(|scope| {
        let started = scope.spawn(|| {
            cores.acquire();
            Instant::now()
        });
        thread::sleep(Duration::from_millis(30));
        let freed = Instant::now();
        cores.give(1);
        assert!(started.join().unwrap() >= freed);
    });
    assert_eq!(cores.take(1), 0);
    cores.give(2);
    assert_eq!(cores.take(9), 2);
    assert_eq!(Cores::new(0).take(1), 0);
}

#[test]
fn default_workers_are_half_the_logical_cpus() {
    let cpus = thread::available_parallelism().map_or(1, usize::from);
    WORKERS.set(None);
    assert_eq!(workers(), (cpus / 2).max(1));
    WORKERS.set(Some(3));
    assert_eq!(workers(), 3);
    WORKERS.set(None);
}
