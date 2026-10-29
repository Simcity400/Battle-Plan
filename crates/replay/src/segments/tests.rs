use super::*;
use crate::columnar_frames::fixtures::{every, recording};
use replay_columnar::envelope::UNIT_ORDINALS;

/// Each hour a segment's operator saw: its branch, hour, run, ordinal and whether it
/// started the segment.
struct Seen(Vec<(u64, i32, u32, u32, bool)>);

impl Operator for Seen {
    fn hour(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        self.0
            .push((hour.branch, hour.hour, hour.run, hour.ordinal, hour.starts));
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(self.0))
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

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
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

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(()))
    }
}

fn opened(runs: &[(u64, i32, u32)]) -> Recording {
    let (bytes, _) = recording(runs, every(runs), 1, |_| {});
    Recording::open(bytes).unwrap()
}

/// Segments walk on several workers, at most a bounded number ahead, and reach the caller
/// in segment order, each with every hour of its own once, its first marked.
#[test]
fn segments_reach_the_caller_in_order_with_their_own_hours() {
    let runs = [(4, 10, 3 * UNIT_ORDINALS + 5), (9, 50_000, 7)];
    let file = opened(&runs);
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
            .flat_map(|(run, (branch, first, ordinals))| {
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
