use super::*;
use crate::columnar_frames::{
    Budget, Limits,
    fixtures::{every, recording},
};
use replay_columnar::envelope::UNIT_ORDINALS;
use std::sync::Arc;

/// Each hour a segment's operator saw: its branch, hour, run, ordinal and whether it
/// started the segment.
struct Seen(Vec<(u64, i32, u32, u32, bool)>);

impl Operator for Seen {
    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        self.0
            .push((hour.branch, hour.hour, hour.run, hour.ordinal, hour.starts));
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(std::mem::take(&mut self.0)))
    }
}

/// The dictionary size of the state each segment starts from.
struct Started(Option<usize>);

impl Operator for Started {
    fn start(&mut self, start: Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.0 = Some(match start {
            Start::Restart(state) => state.dictionary().len(),
            Start::Run { .. } => 0,
        });
        Ok(())
    }

    fn hour(&mut self, _: &Hour<'_>) -> io::Result<()> {
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(std::mem::take(&mut self.0)))
    }
}

/// Each hour it saw and whether it started the walk; it cannot start from a restart whose
/// dictionary holds `refused` strings.
struct Picky {
    refused: usize,
    seen: Vec<(i32, bool)>,
}

impl Operator for Picky {
    fn starts_at(&self, start: Start<'_>) -> bool {
        match start {
            Start::Restart(state) => state.dictionary().len() != self.refused,
            Start::Run { .. } => true,
        }
    }

    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        self.seen.push((hour.hour, hour.starts));
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(std::mem::take(&mut self.seen)))
    }
}

/// Each hour it saw, whether it started the walk and opened a segment, and each run it was
/// carried into; it cannot start a run of a branch an earlier run is of.
#[derive(Default)]
struct Continuing {
    seen: Vec<(i32, bool, bool)>,
    crossed: Vec<i32>,
}

impl Operator for Continuing {
    fn starts_at(&self, start: Start<'_>) -> bool {
        !matches!(start, Start::Run { continues: true })
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        self.crossed.push(first);
        Ok(())
    }

    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        assert!(hour.state.dictionary().len() <= (hour.ordinal + 1) as usize);
        self.seen.push((hour.hour, hour.starts, hour.opens));
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new((
            std::mem::take(&mut self.seen),
            std::mem::take(&mut self.crossed),
        )))
    }
}

/// Fails at `hour`.
struct Failing(i32);

impl Operator for Failing {
    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        if hour.hour == self.0 {
            return Err(io::Error::other("operator failed"));
        }
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(()))
    }
}

/// Panics at `hour`.
struct Panicking(i32);

impl Operator for Panicking {
    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        assert_ne!(hour.hour, self.0, "operator panicked");
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(()))
    }
}

/// Set once, and waited for.
#[derive(Default)]
struct Flag(Mutex<bool>, Condvar);

impl Flag {
    fn set(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }

    fn wait(&self) {
        let mut set = self.0.lock().unwrap();
        while !*set {
            set = self.1.wait(set).unwrap();
        }
    }
}

/// Holds its walker's share of the budget at its walk's first hour until `failed` is set and
/// no walker waits for the budget.
struct Holding {
    budget: Arc<Budget>,
    failed: Arc<Flag>,
}

impl Operator for Holding {
    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        if hour.starts {
            self.failed.wait();
            self.budget.wait_for_no_waiters();
        }
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(()))
    }
}

/// Fails as its walk starts, or panics, once a walker waits for the budget; sets `failed`.
struct Refusing {
    budget: Arc<Budget>,
    failed: Arc<Flag>,
    panics: bool,
}

impl Operator for Refusing {
    fn start(&mut self, _: Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.budget.wait_for_waiters(1);
        self.failed.set();
        assert!(!self.panics, "operator panicked");
        Err(io::Error::other("operator failed"))
    }

    fn hour(&mut self, _: &Hour<'_>) -> io::Result<()> {
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(()))
    }
}

fn opened(runs: &[(u64, i32, u32)]) -> Recording {
    let (bytes, _) = recording(runs, every(runs), 1, |_| {});
    Recording::open(bytes, Limits::default()).unwrap()
}

/// Segments walk on several workers, at most a bounded number ahead, and reach the caller
/// in segment order, each with every hour of its own once, its first marked: of two runs,
/// and of six runs of twenty segments, three runs of one branch.
#[test]
fn segments_reach_the_caller_in_order_with_their_own_hours() {
    let few = [(4, 10, 3 * UNIT_ORDINALS + 5), (9, 50_000, 7)];
    let many = [
        (4, 10, 3 * UNIT_ORDINALS + 5),
        (9, 50_000, 7),
        (4, 90_000, 5 * UNIT_ORDINALS),
        (7, 200_000, 2 * UNIT_ORDINALS + 1),
        (4, 300_000, 6 * UNIT_ORDINALS),
        (8, 400_000, 1),
    ];
    for runs in [&few[..], &many[..]] {
        let file = opened(runs);
        let make = |_: &Segment| -> io::Result<Operators> { Ok(vec![Box::new(Seen(Vec::new()))]) };
        for (threads, ahead) in [(1, 1), (3, 1), (8, 2)] {
            let mut order = Vec::new();
            let mut hours = Vec::new();
            run(&file, threads, ahead, &make, &mut |segment, results| {
                order.push(*segment);
                let [seen] = <[_; 1]>::try_from(results).unwrap();
                let seen = *seen.downcast::<Vec<(u64, i32, u32, u32, bool)>>().unwrap();
                assert!(
                    seen.iter()
                        .enumerate()
                        .all(|(at, hour)| hour.4 == (at == 0))
                );
                hours.extend(seen);
                Ok(())
            })
            .unwrap();
            assert_eq!(order, file.segments());
            let expected: Vec<_> = (0..)
                .zip(runs)
                .flat_map(|(run, &(branch, first, ordinals))| {
                    (0..ordinals).map(move |ordinal| (branch, first + ordinal as i32, run, ordinal))
                })
                .collect();
            let walked: Vec<_> = hours
                .iter()
                .map(|&(branch, hour, run, ordinal, _)| (branch, hour, run, ordinal))
                .collect();
            assert_eq!(walked, expected);
        }
    }
    let file = opened(&many);
    assert_eq!((file.run_count(), file.segments().len()), (6, 20));
}

/// Each segment's operators see the state its first hour applies to: empty at a run's start,
/// else what its restart holds (here, one dictionary string per hour before it).
#[test]
fn operators_start_from_the_state_before_the_first_hour() {
    let runs = [(1, 0, 3 * UNIT_ORDINALS)];
    let file = opened(&runs);
    let make = |_: &Segment| -> io::Result<Operators> { Ok(vec![Box::new(Started(None))]) };
    let mut sizes = Vec::new();
    run(&file, 2, 2, &make, &mut |_, results| {
        let [started] = <[_; 1]>::try_from(results).unwrap();
        sizes.push(*started.downcast::<Option<usize>>().unwrap());
        Ok(())
    })
    .unwrap();
    let ordinals = UNIT_ORDINALS as usize;
    assert_eq!(sizes, [Some(0), Some(ordinals), Some(2 * ordinals)]);
}

/// A segment its operators cannot start is walked by the walk of the segment before, on from
/// the hour before it, within its run only, and its results are its own: every hour is
/// walked once, in order, and only a walk's first hour starts it.
#[test]
fn a_segment_its_operators_cannot_start_is_walked_by_the_one_before() {
    let runs = [(1, 0, 4 * UNIT_ORDINALS), (2, 9_000, 2 * UNIT_ORDINALS)];
    let file = opened(&runs);
    // Each run's second restart holds one string per hour before it.
    let refused = UNIT_ORDINALS as usize;
    let make = |_: &Segment| -> io::Result<Operators> {
        Ok(vec![Box::new(Picky {
            refused,
            seen: Vec::new(),
        })])
    };
    for (threads, ahead) in [(1, 1), (1, 2), (4, 1), (4, 2)] {
        let mut walks = Vec::new();
        run(&file, threads, ahead, &make, &mut |_, results| {
            walks.push(
                results
                    .into_iter()
                    .next()
                    .map(|seen| *seen.downcast::<Vec<(i32, bool)>>().unwrap()),
            );
            Ok(())
        })
        .unwrap();
        // Each segment's first hour; the second restart of each run is walked on through.
        let starts: Vec<Option<bool>> = walks
            .iter()
            .map(|seen| {
                seen.as_ref()
                    .and_then(|seen| seen.first())
                    .map(|&(_, starts)| starts)
            })
            .collect();
        assert_eq!(starts, [true, false, true, true, true, false].map(Some));
        for seen in walks.iter().flatten() {
            assert!(seen.iter().skip(1).all(|&(_, starts)| !starts));
        }
        let hours: Vec<i32> = walks
            .iter()
            .flatten()
            .flatten()
            .map(|&(hour, _)| hour)
            .collect();
        let expected: Vec<i32> = runs
            .iter()
            .flat_map(|&(_, first, ordinals)| (0..ordinals as i32).map(move |at| first + at))
            .collect();
        assert_eq!(hours, expected);
    }
}

/// A run of a branch an earlier run is of, which its operators cannot start, is walked by the
/// walk of that earlier run's last segment, from empty tables, past the runs between, and its
/// results are its own; the rest of the run walks from its restarts.
#[test]
fn a_branch_s_later_run_its_operators_cannot_start_is_walked_by_its_earlier_run() {
    let units = UNIT_ORDINALS as i32;
    let runs = [
        (1, 0, 2 * UNIT_ORDINALS),
        (2, 50_000, UNIT_ORDINALS),
        (1, 90_000, UNIT_ORDINALS + 5),
    ];
    let file = opened(&runs);
    let make = |_: &Segment| -> io::Result<Operators> { Ok(vec![Box::new(Continuing::default())]) };
    let hours = |first: i32, count: i32| (first..first + count).collect::<Vec<_>>();
    for threads in [1, 4] {
        let mut walks = Vec::new();
        run(&file, threads, 2, &make, &mut |_, results| {
            walks.push(results.into_iter().next().map(|walked| {
                *walked
                    .downcast::<(Vec<(i32, bool, bool)>, Vec<i32>)>()
                    .unwrap()
            }));
            Ok(())
        })
        .unwrap();
        let walked: Vec<Option<(Vec<i32>, Vec<i32>)>> = walks
            .iter()
            .map(|walk| {
                walk.as_ref().map(|(seen, crossed)| {
                    (
                        seen.iter().map(|&(hour, ..)| hour).collect(),
                        crossed.clone(),
                    )
                })
            })
            .collect();
        assert_eq!(
            walked,
            [
                Some((hours(0, units), vec![])),
                Some((hours(units, units), vec![])),
                Some((hours(50_000, units), vec![])),
                Some((hours(90_000, units), vec![90_000])),
                Some((hours(90_000 + units, 5), vec![])),
            ]
        );
        for (seen, _) in walks.iter().flatten() {
            for &(hour, starts, opens) in seen {
                assert_eq!(opens, hour == seen[0].0, "hour {hour}");
                // Every segment here starts a walk, or a crossing into its run.
                assert_eq!(starts, opens, "hour {hour}");
            }
        }
    }
}

/// No more segments are walked or waiting than the look-ahead allows, whatever the
/// number of workers.
#[test]
fn the_look_ahead_bounds_the_segments_held() {
    let runs = [(1, 0, 6 * UNIT_ORDINALS)];
    let file = opened(&runs);
    let (started, most) = (
        std::sync::atomic::AtomicUsize::new(0),
        std::sync::atomic::AtomicUsize::new(0),
    );
    let emitted = std::sync::atomic::AtomicUsize::new(0);
    let make = |_: &Segment| -> io::Result<Operators> {
        let held = started.fetch_add(1, Ordering::SeqCst) + 1 - emitted.load(Ordering::SeqCst);
        most.fetch_max(held, Ordering::SeqCst);
        Ok(vec![Box::new(Failing(-1))])
    };
    for ahead in [1, 2, usize::MAX] {
        started.store(0, Ordering::SeqCst);
        emitted.store(0, Ordering::SeqCst);
        most.store(0, Ordering::SeqCst);
        run(&file, 8, ahead, &make, &mut |_, _| {
            emitted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert!(most.load(Ordering::SeqCst) <= ahead);
        assert_eq!(emitted.load(Ordering::SeqCst), 6);
    }
}

/// A panic in an operator or in the caller's handling stops the run instead of leaving it
/// waiting, and reaches the caller.
#[test]
fn a_panic_stops_the_run() {
    let runs = [(1, 0, 4 * UNIT_ORDINALS)];
    let file = opened(&runs);
    let panicking = |_: &Segment| -> io::Result<Operators> { Ok(vec![Box::new(Panicking(800))]) };
    let walked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&file, 2, 1, &panicking, &mut |_, _| Ok(()))
    }));
    assert!(walked.is_err());
    let quiet = |_: &Segment| -> io::Result<Operators> { Ok(vec![Box::new(Failing(-1))]) };
    let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&file, 4, 1, &quiet, &mut |_, _| panic!("caller panicked"))
    }));
    assert!(handled.is_err());
}

/// An operator's failure, or the caller's, stops the run and is what it returns.
#[test]
fn the_first_failure_stops_the_run() {
    let runs = [(1, 0, 4 * UNIT_ORDINALS)];
    let file = opened(&runs);
    let failing = |_: &Segment| -> io::Result<Operators> { Ok(vec![Box::new(Failing(800))]) };
    let mut emitted = 0;
    let error = run(&file, 2, 1, &failing, &mut |_, _| {
        emitted += 1;
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "operator failed");
    assert!(emitted <= 1);
    let quiet = |_: &Segment| -> io::Result<Operators> { Ok(vec![Box::new(Failing(-1))]) };
    let error = run(&file, 2, 1, &quiet, &mut |_, _| {
        Err(io::Error::other("caller failed"))
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "caller failed");
}

/// Three runs on three workers under a budget admitting one walker at a time: one of the
/// first two runs' walkers holds it until the run stops and the other waits for it, while
/// the last run's operator fails, or panics, as its walk starts. What the run returned.
fn stopped_while_waiting(panics: bool) -> std::thread::Result<io::Result<()>> {
    let runs = [(1, 0, 10), (2, 100, 10), (3, 200, 10)];
    let (bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let budget = Arc::new(Budget::new(1));
    let limits = Limits {
        budget: budget.clone(),
        ..Limits::default()
    };
    let file = Recording::open(bytes, limits).unwrap();
    let failed = Arc::new(Flag::default());
    let make = |segment: &Segment| -> io::Result<Operators> {
        let (budget, failed) = (budget.clone(), failed.clone());
        let operator: Box<dyn Operator> = if segment.run == 2 {
            Box::new(Refusing {
                budget,
                failed,
                panics,
            })
        } else {
            Box::new(Holding { budget, failed })
        };
        Ok(vec![operator])
    };
    let walked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&file, 3, 3, &make, &mut |_, _| Ok(()))
    }));
    assert_eq!(budget.walkers(), 0);
    walked
}

/// An operator's failure while a walker waits for the budget stops the run: the walker
/// waiting leaves, the one holding the budget ends its walk, and the failure is returned.
#[test]
fn a_failure_while_a_walker_waits_for_the_budget_stops_the_run() {
    let error = stopped_while_waiting(false).unwrap().unwrap_err();
    assert_eq!(error.to_string(), "operator failed");
}

/// An operator's panic while a walker waits for the budget stops the run as a failure does,
/// and reaches the caller.
#[test]
fn a_panic_while_a_walker_waits_for_the_budget_stops_the_run() {
    assert!(stopped_while_waiting(true).is_err());
}
