use super::*;
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, source};
use replay::format::FormatVersion;
use std::thread::ThreadId;

/// A family whose facts are the hours it observed and the thread it ran on.
struct Hours(Vec<i32>);

impl Extract for Hours {
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        // A block alone starts with every table changed.
        assert_eq!(
            self.0.is_empty(),
            changes.rows_changed[TableId::Countries.index()]
        );
        self.0.push(frame.hour);
        Ok(())
    }
    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new((std::thread::current().id(), self.0)))
    }
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(Hours(Vec::new()))
}

static HOURS: Family = Family {
    name: "hours",
    extract: "hours.extract",
    sequential: "hours.sequential",
    fallback: "hours.fallback",
    tables: &[TableId::Countries],
    fields: None,
    start,
};

/// Branch 1 in `blocks` blocks of three consecutive hours each.
fn recording(blocks: i32) -> Arc<Replay> {
    let hours =
        |block: i32| (100 + 3 * block..103 + 3 * block).map(|hour| source(1, hour, Some(1)));
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, hours(0).collect())]).unwrap();
    for block in 1..blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, hours(block).collect())])
            .unwrap();
    }
    assert_eq!(replay.index.len(), blocks as usize);
    Arc::new(replay)
}

fn facts(outcome: Outcome) -> (ThreadId, Vec<i32>) {
    match outcome {
        Outcome::Facts(facts) => *facts.downcast::<(ThreadId, Vec<i32>)>().unwrap(),
        Outcome::Sequential => panic!("block observed hourly"),
    }
}

#[test]
fn prepass_hands_every_block_in_order_extracted_off_the_coordinator_within_its_bound() {
    let replay = recording(9);
    let cancelled = AtomicBool::new(false);
    for workers in [1, 3] {
        let prepass = PrePass::start(
            Arc::clone(&replay),
            2,
            vec![&HOURS],
            workers,
            Knobs::default(),
        );
        for position in 2..replay.index.len() {
            // A slow coordinator: the workers stop at their bound meanwhile.
            std::thread::sleep(Duration::from_millis(20));
            let mut outcomes = prepass.take(position, &cancelled).unwrap();
            assert_eq!(outcomes.len(), 1);
            let (thread, hours) = facts(outcomes.remove(0).0);
            assert_ne!(thread, std::thread::current().id());
            let first = replay.index[position].first_hour;
            assert_eq!(hours, (first..first + 3).collect::<Vec<_>>());
        }
        assert!(
            prepass.peak() <= workers + 1,
            "{} blocks ahead with {workers} workers",
            prepass.peak()
        );
        assert!(
            prepass.take(0, &cancelled).is_err(),
            "blocks are taken in order"
        );
    }
    // A byte budget below one block's raw size claims only the block the
    // coordinator takes next, however many workers there are.
    let prepass = PrePass::start(
        Arc::clone(&replay),
        0,
        vec![&HOURS],
        4,
        Knobs {
            budget: Some(1),
            ..Knobs::default()
        },
    );
    for position in 0..replay.index.len() {
        std::thread::sleep(Duration::from_millis(10));
        facts(prepass.take(position, &cancelled).unwrap().remove(0).0);
    }
    assert_eq!(prepass.peak(), 1);
}

#[test]
fn prepass_hands_failed_and_refused_extracts_to_hourly_observation() {
    let replay = recording(4);
    let cancelled = AtomicBool::new(false);
    let hook: Hook = Arc::new(|_, frame| match frame.hour {
        104 => Fault::Error,
        107 => Fault::Panic,
        _ => Fault::Continue,
    });
    let knobs = Knobs {
        hook: Some(hook),
        refusals: vec![("hours", 110)],
        ..Knobs::default()
    };
    let prepass = PrePass::start(Arc::clone(&replay), 0, vec![&HOURS], 2, knobs);
    let extracted: Vec<bool> = (0..4)
        .map(|position| {
            matches!(
                prepass.take(position, &cancelled).unwrap()[0].0,
                Outcome::Facts(_)
            )
        })
        .collect();
    assert_eq!(extracted, [true, false, false, false]);
}

#[test]
fn prepass_cancellation_ends_the_wait_and_dropping_it_joins_the_workers() {
    let replay = recording(6);
    let cancelled = AtomicBool::new(false);
    let prepass = PrePass::start(Arc::clone(&replay), 0, vec![&HOURS], 2, Knobs::default());
    prepass.take(0, &cancelled).unwrap();
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        prepass.take(1, &cancelled).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    drop(prepass);
}

/// The pool's workers serve the family with the least extract work left:
/// once a costly family's blocks are measured, a cheap family's next block
/// is claimed first even when the cheap family is further ahead, and a
/// family not yet measured is claimed first to learn its cost.
#[test]
fn pool_claims_go_to_the_family_with_the_least_work_left() {
    let lane = |next: usize, handed: usize, spent_ms: u64, measured: u32| Lane {
        family: &HOURS,
        next,
        handed,
        end: 100,
        done: BTreeMap::new(),
        retired: false,
        spent: Duration::from_millis(spent_ms),
        measured,
        facts: 0,
        reserved: BTreeMap::new(),
    };
    let shared = |lanes: Vec<Lane>| PoolShared {
        state: Mutex::new(PoolState {
            lanes,
            ahead: 8,
            sizes: vec![1; 100],
            in_flight: 0,
            budget: u64::MAX,
            alive: 1,
        }),
        changed: Condvar::new(),
        stop: AtomicBool::new(false),
    };
    // Costly lane 0 at its next block; cheap lane 1 three blocks ahead.
    let pool = shared(vec![lane(10, 10, 1_000, 10), lane(23, 20, 10, 23)]);
    assert_eq!(
        pool.claim().map(|(lane, block, _)| (lane, block)),
        Some((1, 23))
    );
    // A family with nothing measured is served before one with work left.
    let pool = shared(vec![lane(10, 10, 10, 10), lane(0, 0, 0, 0)]);
    assert_eq!(
        pool.claim().map(|(lane, block, _)| (lane, block)),
        Some((1, 0))
    );
    // Measured costs come from finished extracts.
    let pool = shared(vec![lane(1, 0, 0, 0)]);
    pool.finish(0, 0, (Outcome::Sequential, Duration::from_millis(7)), 0);
    let state = pool.lock();
    assert_eq!(
        (state.lanes[0].spent, state.lanes[0].measured),
        (Duration::from_millis(7), 1)
    );
}

/// The pool counts what an extracted block's facts hold against its budget
/// until the family takes the block: a block claimed reserves its family's
/// mean facts so far, replaced by its own once extracted, so workers stop
/// reading ahead while a family's facts fill the budget, and taking or
/// retiring releases them.
#[test]
fn pool_counts_extracted_facts_against_its_budget_until_taken() {
    let lane = Lane {
        family: &HOURS,
        next: 2,
        handed: 0,
        end: 10,
        done: BTreeMap::new(),
        retired: false,
        spent: Duration::ZERO,
        measured: 0,
        facts: 0,
        reserved: BTreeMap::new(),
    };
    let pool = Pool {
        shared: Arc::new(PoolShared {
            state: Mutex::new(PoolState {
                lanes: vec![lane],
                ahead: 8,
                sizes: vec![1; 10],
                // Blocks 0 and 1 are claimed.
                in_flight: 2,
                budget: 100,
                alive: 1,
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        }),
        workers: Vec::new(),
    };
    let facts = || (Outcome::Facts(Box::new(())), Duration::ZERO);
    // Block 0's facts hold 60 bytes: a third block's raw byte and expected
    // facts would pass the budget.
    pool.shared.finish(0, 0, facts(), 60);
    {
        let state = pool.shared.lock();
        assert_eq!(state.in_flight, 62);
        assert_eq!(state.lanes[0].expected(), 60);
        assert_eq!(state.pick(), None, "its facts hold the next claim back");
    }
    // Taken, block 0 releases its raw byte and its facts.
    let flag = AtomicBool::new(false);
    pool.take(0, 0, &flag).unwrap();
    assert_eq!(pool.shared.lock().in_flight, 1);
    // Block 2 is claimed reserving the 60 bytes expected, replaced by the 30
    // its facts hold once extracted.
    assert_eq!(
        pool.shared.claim().map(|(lane, block, _)| (lane, block)),
        Some((0, 2))
    );
    assert_eq!(pool.shared.lock().in_flight, 62);
    pool.shared.finish(0, 2, facts(), 30);
    assert_eq!(pool.shared.lock().in_flight, 32);
    // Retired, the family's extracted and unclaimed blocks release all.
    pool.shared.finish(0, 1, facts(), 20);
    pool.retire(0);
    assert_eq!(pool.shared.lock().in_flight, 0);
}

/// A block's facts count each table view once however many kept frames
/// share it, at the capacity of its rows: a kept frame sharing every view
/// with the one before adds only itself, and one whose countries were read
/// again adds that table's rows.
#[test]
fn held_bytes_count_each_shared_view_once_at_its_rows_capacity() {
    use crate::test_fixtures::scale_frame;
    use dashboard_base::prepared::prepass::HeldBytes;
    let first = scale_frame(1, 100, 500);
    let mut same = first.clone();
    same.hour = 101;
    let mut reread = same.clone();
    reread.hour = 102;
    let countries = match &*first.tables[TableId::Countries.index()] {
        TableView::Complete(rows) => rows.clone(),
        _ => unreachable!(),
    };
    *reread.tables[TableId::Countries.index()] = TableView::Complete(countries.clone());
    let held = |frames: &[&DecodedFrame]| {
        let mut held = HeldBytes::default();
        for frame in frames {
            held.frame(frame);
        }
        held.total()
    };
    let one = held(&[&first]);
    assert!(one >= countries.heap_bytes() as u64);
    assert_eq!(
        held(&[&first, &same]) - one,
        size_of::<DecodedFrame>() as u64,
        "a frame sharing every view adds only itself"
    );
    assert!(
        held(&[&first, &same, &reread]) - held(&[&first, &same])
            >= (size_of::<DecodedFrame>() + countries.heap_bytes()) as u64,
        "a table read again adds its rows"
    );
}

/// Extracts report what their facts hold, growing with the recording: a
/// recorded-call family (wars: its first frame and recorded calls) and a
/// kept-frame family (the identity history: its kept frames).
#[test]
fn extracts_report_what_their_facts_hold() {
    let measure = |family: &'static Family, frames: Vec<DecodedFrame>| -> u64 {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let mut outcomes = extract_block_measured(
            &replay,
            &replay.index[0],
            &[family],
            &AtomicBool::new(false),
            &Knobs::default(),
        );
        let (outcome, _, bytes) = outcomes.pop().unwrap();
        assert!(
            matches!(outcome, Outcome::Facts(_)),
            "{} extracted its block",
            family.name
        );
        bytes
    };
    let wars = |count| {
        measure(
            &crate::products::wars::block::FAMILY,
            (100..=105)
                .map(|hour| crate::products::wars::test_fixtures::scaled_recorded(hour, count))
                .collect(),
        )
    };
    let identities = |count| {
        measure(
            &crate::products::country_history::prepared::FAMILY,
            (100..=105)
                .map(|hour| crate::test_fixtures::scale_frame(1, hour, count))
                .collect(),
        )
    };
    for (family, small, large) in [
        ("wars", wars(3), wars(300)),
        ("identities", identities(3), identities(300)),
    ] {
        assert!(small > 0, "{family} reports its facts");
        assert!(
            large > small * 10,
            "{family}'s facts grow with the countries: {small} for 3, {large} for 300"
        );
    }
}
