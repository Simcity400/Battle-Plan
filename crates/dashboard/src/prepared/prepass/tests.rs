use super::*;
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, source};
use replay_data::format::FormatVersion;
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
        Outcome::Declined(error) | Outcome::Failed(error) => panic!("extract failed: {error}"),
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
fn prepass_and_pool_extracts_keep_the_callers_job() {
    let replay = recording(1);
    let cancelled = AtomicBool::new(false);
    let background = Arc::new(AtomicBool::new(true));
    JobPriority::run(Arc::clone(&background), || {
        for expected in [true, false] {
            background.store(expected, Ordering::Release);
            let called = Arc::new(AtomicBool::new(false));
            let seen = Arc::clone(&called);
            let knobs = Knobs {
                hook: Some(Arc::new(move |_, _| {
                    assert_eq!(JobPriority::current().background(), expected);
                    seen.store(true, Ordering::Release);
                    Fault::Continue
                })),
                ..Knobs::default()
            };
            let prepass = PrePass::start(Arc::clone(&replay), 0, vec![&HOURS], 1, knobs.clone());
            facts(prepass.take(0, &cancelled).unwrap().remove(0).0);
            assert!(called.swap(false, Ordering::AcqRel));
            drop(prepass);
            let pool = Pool::start(
                Arc::clone(&replay),
                vec![(&HOURS, 0, false)],
                1,
                knobs,
                None,
            );
            facts(pool.take(0, 0, &cancelled).unwrap().0);
            assert!(called.load(Ordering::Acquire));
        }
    });
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

/// A pool lane as the scheduling tests build it.
fn lane(next: usize, handed: usize, spent_ms: u64, measured: u32) -> Lane {
    Lane {
        section_consumer: None,
        family: &HOURS,
        bound: None,
        binding: false,
        next,
        handed,
        end: 100,
        done: BTreeMap::new(),
        retired: false,
        spent: Duration::from_millis(spent_ms),
        measured,
        folded: Duration::ZERO,
        folds: 0,
        facts: 0,
        reserved: BTreeMap::new(),
    }
}

/// A pool's shared state over `lanes` with `alive` workers, `in_flight`
/// bytes claimed (each of `held`'s blocks claimed once) and a byte budget.
fn pool_state(
    lanes: Vec<Lane>,
    alive: usize,
    in_flight: u64,
    held: &[usize],
    budget: u64,
) -> PoolShared {
    PoolShared {
        state: Mutex::new(PoolState {
            walk: None,
            pace: None,
            paced_lanes: false,
            lanes,
            ahead: 8,
            sizes: vec![1; 100],
            in_flight,
            budget,
            alive,
            holders: held.iter().map(|&block| (block, 1)).collect(),
            busy: 0,
            read: Duration::ZERO,
            reads: 0,
        }),
        changed: Condvar::new(),
        stop: AtomicBool::new(false),
        depot: None,
    }
}

/// A task's block and its members' lanes.
fn members(task: Option<Task>) -> Option<(usize, Vec<usize>)> {
    task.map(|task| {
        (
            task.block,
            task.members.iter().map(|(lane, ..)| *lane).collect(),
        )
    })
}

/// A finished task of one lane: its outcome holding `bytes`, after `took`.
fn finished(
    block: usize,
    lane: usize,
    outcome: Outcome,
    took: Duration,
    bytes: u64,
) -> (Task, Measured) {
    (
        Task {
            block,
            members: vec![(lane, &HOURS, None)],
        },
        Measured {
            outcomes: vec![(outcome, took, bytes)],
            read: Duration::from_millis(3),
            reads: 1,
        },
    )
}

/// A pool worker claims the longest chain's next block for every lane
/// whose next block it is and within reach, costliest first, counting the
/// block's raw bytes once; a lane waiting for its reads claims nothing. With
/// fewer blocks coming than workers waiting, a block is split between them,
/// each part at least one lane.
#[test]
fn pool_claims_the_longest_chain_and_shares_its_next_block() {
    let mut waiting = lane(10, 10, 0, 0);
    waiting.binding = true;
    let pool = pool_state(
        vec![
            lane(10, 10, 10, 10),
            lane(12, 12, 0, 0),
            lane(10, 8, 1_000, 10),
            waiting,
        ],
        1,
        0,
        &[],
        u64::MAX,
    );
    assert_eq!(members(pool.claim()), Some((10, vec![2, 0])));
    assert_eq!(pool.lock().in_flight, 1, "the block's raw bytes count once");
    assert_eq!(members(pool.claim()), Some((11, vec![2, 0])));
    assert_eq!(members(pool.claim()), Some((12, vec![2, 1, 0])));
    // One block coming, four workers waiting.
    let lanes = (0..4)
        .map(|_| Lane {
            end: 6,
            ..lane(5, 5, 10, 1)
        })
        .collect();
    let pool = pool_state(lanes, 4, 0, &[], u64::MAX);
    assert_eq!(members(pool.claim()), Some((5, vec![0])));
    assert_eq!(members(pool.claim()), Some((5, vec![1])));
    assert_eq!(
        pool.lock().in_flight,
        1,
        "a split block's raw bytes count once"
    );
    // Measured costs and read time come from finished tasks.
    let pool = pool_state(vec![lane(1, 0, 0, 0)], 1, 0, &[0], u64::MAX);
    let (task, measured) = finished(0, 0, Outcome::Sequential, Duration::from_millis(7), 0);
    pool.finish(task, measured);
    let state = pool.lock();
    assert_eq!(
        (state.lanes[0].spent, state.lanes[0].measured),
        (Duration::from_millis(7), 1)
    );
    assert_eq!((state.read, state.reads), (Duration::from_millis(3), 1));
}

/// The pool counts what an extracted block's facts hold against its budget
/// until the family takes the block: a block claimed reserves its family's
/// mean facts so far, replaced by its own once extracted, so workers stop
/// reading ahead while a family's facts fill the budget, and taking or
/// retiring releases them.
#[test]
fn pool_counts_extracted_facts_against_its_budget_until_taken() {
    let pool = Pool {
        // Blocks 0 and 1 are claimed.
        shared: Arc::new(pool_state(
            vec![Lane {
                end: 10,
                ..lane(2, 0, 0, 0)
            }],
            1,
            2,
            &[0, 1],
            100,
        )),
        workers: Vec::new(),
    };
    let facts = || Outcome::Facts(Box::new(()));
    // Block 0's facts hold 60 bytes: a third block's raw byte and expected
    // facts would pass the budget.
    let (task, measured) = finished(0, 0, facts(), Duration::ZERO, 60);
    pool.shared.finish(task, measured);
    {
        let state = pool.shared.lock();
        assert_eq!(state.in_flight, 62);
        assert_eq!(state.lanes[0].expected(), 60);
        assert!(state.pick().is_none(), "its facts hold the next claim back");
    }
    // Taken, block 0 releases its raw byte and its facts.
    let flag = AtomicBool::new(false);
    pool.take(0, 0, &flag).unwrap();
    assert_eq!(pool.shared.lock().in_flight, 1);
    // Block 2 is claimed reserving the 60 bytes expected, replaced by the 30
    // its facts hold once extracted.
    assert_eq!(members(pool.shared.claim()), Some((2, vec![0])));
    assert_eq!(pool.shared.lock().in_flight, 62);
    let (task, measured) = finished(2, 0, facts(), Duration::ZERO, 30);
    pool.shared.finish(task, measured);
    assert_eq!(pool.shared.lock().in_flight, 32);
    // Retired, the family's extracted and unclaimed blocks release all.
    let (task, measured) = finished(1, 0, facts(), Duration::ZERO, 20);
    pool.shared.finish(task, measured);
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
        )
        .outcomes;
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
                .map(|hour| {
                    let mut frame = crate::test_fixtures::scale_frame(1, hour, count);
                    // Identity history carries balance-only changes. Exercise
                    // the identity payload this growth test intends to measure.
                    let mut countries = frame
                        .complete::<replay_data::rows::tables::Countries>()
                        .unwrap()
                        .to_vec();
                    for country in &mut countries {
                        country.tag = format!("C{}H{hour}", country.id).into_bytes();
                    }
                    *frame.tables[TableId::Countries.index()] =
                        TableView::Complete(replay_data::rows::TableRows::Countries(countries));
                    frame
                })
                .collect(),
        )
    };
    // Each family also holds facts no country adds (the recording's other
    // tables, its recorded calls); measured from a recording without
    // countries, the countries' share must grow with them.
    for (family, none, small, large) in [
        ("wars", wars(0), wars(3), wars(300)),
        ("identities", identities(0), identities(3), identities(300)),
    ] {
        assert!(small > 0, "{family} reports its facts");
        assert!(
            large - none > (small - none) * 10,
            "{family}'s facts grow with the countries: {none} for none, {small} for 3, {large} for 300"
        );
    }
}

/// Each hour a family's extract was given: its hour, the block reader's
/// changed tables where it was given them, and whether its frame held the
/// country links.
type Given = Vec<(i32, Option<Vec<TableId>>, bool)>;

struct Reported(Given);

impl Reported {
    fn push(&mut self, frame: &DecodedFrame, reported: Option<&[TableId]>) {
        let links = matches!(
            *frame.tables[TableId::CountryLinks.index()],
            TableView::Complete(_)
        );
        self.0
            .push((frame.hour, reported.map(<[TableId]>::to_vec), links));
    }
}

impl Extract for Reported {
    fn observe(&mut self, _: &Replay, frame: &DecodedFrame, _: &FrameChanges) -> io::Result<()> {
        self.push(frame, None);
        Ok(())
    }
    fn observe_reported(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        _: &FrameChanges,
        reported: &[TableId],
    ) -> io::Result<()> {
        self.push(frame, Some(reported));
        Ok(())
    }
    fn quiet(&mut self, _: &Replay, _: &QuietHours, _: std::ops::Range<usize>) -> io::Result<()> {
        Ok(())
    }
    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(self.0))
    }
}

/// The countries' political power, which the bound read watches.
fn power() -> TableFields {
    TableFields::named(TableId::Countries, &["political_power_raw"]).unwrap()
}

fn reported_fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![power(), TableFields::all(TableId::CountryLinks)])
}

/// A family that declares the countries and their links.
static REPORTED: Family = Family {
    name: "reported",
    extract: "reported.extract",
    sequential: "reported.sequential",
    fallback: "reported.fallback",
    tables: &[TableId::Campaign, TableId::Countries, TableId::CountryLinks],
    fields: Some(reported_fields),
    start: |_| -> Box<dyn Extract> { Box::new(Reported(Vec::new())) },
};

/// A family that watches only the countries' keys.
static KEYS: Family = Family {
    name: "keys",
    extract: "keys.extract",
    sequential: "keys.sequential",
    fallback: "keys.fallback",
    tables: &[TableId::Countries],
    fields: Some(|| Ok(vec![TableFields::keys(TableId::Countries)])),
    start: |_| -> Box<dyn Extract> { Box::new(Reported(Vec::new())) },
};

/// Hours 100 to 106 in one block, the countries' political power moving at
/// 102 and 104 and their keys never.
fn power_moves() -> Replay {
    let frames: Vec<DecodedFrame> = (100..=106)
        .map(|hour| {
            source(
                1,
                hour,
                Some(if hour < 102 {
                    1
                } else if hour < 104 {
                    2
                } else {
                    3
                }),
            )
        })
        .collect();
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

/// Families read together are each handed what their own read alone hands
/// them: the hours their own fields change, with the tables changed for
/// those fields alone, although another family watching more of the same
/// table shares the read.
#[test]
fn families_read_together_are_each_handed_what_their_own_read_hands_them() {
    let replay = power_moves();
    let given = |families: &[&'static Family]| -> Vec<Given> {
        extract_block_measured(
            &replay,
            &replay.index[0],
            families,
            &AtomicBool::new(false),
            &Knobs::default(),
        )
        .outcomes
        .into_iter()
        .map(|(outcome, ..)| match outcome {
            Outcome::Facts(facts) => *facts.downcast::<Given>().unwrap(),
            Outcome::Sequential => panic!("the block was observed hourly"),
            Outcome::Declined(error) | Outcome::Failed(error) => panic!("extract failed: {error}"),
        })
        .collect()
    };
    let keys = given(&[&KEYS]).remove(0);
    let reported = given(&[&REPORTED]).remove(0);
    assert_eq!(
        keys.iter().map(|(hour, ..)| *hour).collect::<Vec<_>>(),
        [100]
    );
    assert_eq!(
        reported.iter().map(|(hour, ..)| *hour).collect::<Vec<_>>(),
        [100, 102, 104]
    );
    assert_eq!(given(&[&KEYS, &REPORTED, &HOURS_POWER]), {
        let hours = given(&[&HOURS_POWER]).remove(0);
        [keys, reported, hours]
    });
}

#[derive(Debug, PartialEq)]
enum Captured {
    Frame {
        bytes: Vec<u8>,
        sampled: Vec<Option<i32>>,
        changed: Vec<bool>,
        reported: Option<Vec<TableId>>,
        shared: Vec<Vec<bool>>,
    },
    Quiet {
        hours: Vec<i32>,
        metrics: Vec<replay_frame::frame::Metrics>,
        sampled: Vec<Option<i32>>,
        rebuilt: Vec<bool>,
    },
}

#[derive(Default)]
struct Capture {
    events: Vec<Captured>,
    identities: Vec<Vec<replay_frame::frame::ViewIdentity>>,
    last: Option<i32>,
}

impl Capture {
    fn frame(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        reported: Option<&[TableId]>,
    ) {
        let mut bytes = Vec::new();
        frame.encode_checkpoint(&mut bytes).unwrap();
        self.events.push(Captured::Frame {
            bytes,
            sampled: frame.tables.iter().map(SharedView::sampled_at).collect(),
            changed: changes.rows_changed.to_vec(),
            reported: reported.map(<[TableId]>::to_vec),
            shared: self
                .identities
                .iter()
                .map(|before| {
                    before
                        .iter()
                        .zip(&frame.tables)
                        .map(|(before, view)| before.is(view))
                        .collect()
                })
                .collect(),
        });
        self.identities
            .push(frame.tables.iter().map(SharedView::identity).collect());
        self.last = Some(frame.hour);
    }
}

impl Extract for Capture {
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.frame(frame, changes, None);
        Ok(())
    }

    fn observe_reported(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        reported: &[TableId],
    ) -> io::Result<()> {
        self.frame(frame, changes, Some(reported));
        Ok(())
    }

    fn quiet(
        &mut self,
        _: &Replay,
        hours: &QuietHours,
        span: std::ops::Range<usize>,
    ) -> io::Result<()> {
        self.events.push(Captured::Quiet {
            hours: hours.hours()[span.clone()].to_vec(),
            metrics: hours.metrics()[span.clone()]
                .iter()
                .map(|metrics| replay_frame::frame::Metrics::clone(metrics))
                .collect(),
            sampled: span
                .clone()
                .flat_map(|position| TableId::ALL.map(|table| hours.sampled_at(position, table)))
                .collect(),
            rebuilt: span
                .flat_map(|position| TableId::ALL.map(|table| hours.rebuilt(position, table)))
                .collect(),
        });
        Ok(())
    }

    fn deadline(&self) -> Option<i32> {
        self.last.and_then(|hour| hour.checked_add(2))
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(self.events))
    }
}

static CAPTURE_POWER: Family = Family {
    name: "capture power",
    extract: "capture-power.extract",
    sequential: "capture-power.sequential",
    fallback: "capture-power.fallback",
    tables: &[TableId::Countries, TableId::States],
    fields: Some(|| Ok(vec![power(), TableFields::keys(TableId::States)])),
    start: |_| Box::<Capture>::default(),
};

static CAPTURE_ALL: Family = Family {
    name: "capture all",
    extract: "capture-all.extract",
    sequential: "capture-all.sequential",
    fallback: "capture-all.fallback",
    tables: &[TableId::Countries, TableId::States],
    fields: Some(|| {
        Ok(vec![
            TableFields::all(TableId::Countries),
            TableFields::all(TableId::States),
        ])
    }),
    start: |_| Box::<Capture>::default(),
};

fn capture(
    replay: &Replay,
    families: &[&'static Family],
    releasing: bool,
) -> (
    Vec<Vec<Captured>>,
    [[u64; 2]; replay_data::rows::TABLE_COUNT],
    [[u64; 2]; replay_data::rows::TABLE_COUNT],
) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            RELEASING.with(|value| value.set(self.0));
        }
    }
    let _restore = Restore(RELEASING.with(|value| value.replace(releasing)));
    let (copies, reuses) = (
        replay_frame::cpu::delta_copies(),
        replay_frame::cpu::delta_reuses(),
    );
    let events = extract_block_measured(
        replay,
        &replay.index[0],
        families,
        &AtomicBool::new(false),
        &Knobs::default(),
    )
    .outcomes
    .into_iter()
    .map(|(outcome, ..)| match outcome {
        Outcome::Facts(facts) => *facts.downcast::<Vec<Captured>>().unwrap(),
        Outcome::Sequential => panic!("the block was observed hourly"),
        Outcome::Declined(error) | Outcome::Failed(error) => panic!("extract failed: {error}"),
    })
    .collect();
    let since = |before: [[u64; 2]; replay_data::rows::TABLE_COUNT],
                 after: [[u64; 2]; replay_data::rows::TABLE_COUNT]| {
        std::array::from_fn(|table| {
            std::array::from_fn(|field| after[table][field] - before[table][field])
        })
    };
    (
        events,
        since(copies, replay_frame::cpu::delta_copies()),
        since(reuses, replay_frame::cpu::delta_reuses()),
    )
}

fn moving_tables(count: u32, quiet: bool) -> Replay {
    let frames = (100..=110)
        .filter(|&hour| !quiet || hour != 105)
        .map(|hour| {
            let mut frame = crate::test_fixtures::scale_frame(1, hour, count);
            if let TableView::Complete(replay_data::rows::TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].political_power_raw = Some(i64::from(if quiet { hour / 5 } else { hour }));
                rows[0].command_power_raw = i64::from(hour);
                if quiet && hour >= 109 {
                    rows.pop();
                }
            }
            if let TableView::Complete(replay_data::rows::TableRows::States(rows)) =
                &mut *frame.tables[TableId::States.index()]
            {
                rows[0].controller = hour as u32;
            }
            if quiet && matches!(hour, 103 | 104) {
                *frame.tables[TableId::States.index()] = TableView::Truncated {
                    rows: replay_data::rows::TableRows::States(Vec::new()),
                    missing: 1,
                };
            }
            frame.metrics.table_ns[TableId::Countries.index()] = u64::from(hour % 2 == 0);
            frame
        })
        .collect();
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

/// Every frame, quiet span, change fact, sample hour and pairwise allocation
/// relation stays equal when the framework releases replaced views.
#[test]
fn releasing_follow_views_preserves_partial_watches_deadlines_gaps_and_partial_tables() {
    let replay = moving_tables(32, true);
    for families in [
        &[&CAPTURE_POWER][..],
        &[&CAPTURE_ALL][..],
        &[&CAPTURE_POWER, &CAPTURE_ALL][..],
    ] {
        assert_eq!(
            capture(&replay, families, false).0,
            capture(&replay, families, true).0
        );
    }
}

#[test]
fn runtime_preserves_main_projection_quiet_deadline_carry_and_gap_delivery() {
    static PROJECTED: Family = Family {
        name: "capture projected",
        extract: "capture-projected.extract",
        sequential: "capture-projected.sequential",
        fallback: "capture-projected.fallback",
        tables: &[TableId::Countries, TableId::States, TableId::CountryLinks],
        fields: Some(|| Ok(vec![power(), TableFields::keys(TableId::States)])),
        start: |_| Box::<Capture>::default(),
    };
    for quiet in [false, true] {
        let replay = moving_tables(32, quiet);
        for families in [&[&PROJECTED][..], &[&PROJECTED, &CAPTURE_ALL][..]] {
            let events = |knobs: &Knobs| -> Vec<Vec<Captured>> {
                extract_block_measured(
                    &replay,
                    &replay.index[0],
                    families,
                    &AtomicBool::new(false),
                    knobs,
                )
                .outcomes
                .into_iter()
                .map(|(outcome, ..)| match outcome {
                    Outcome::Facts(facts) => *facts.downcast::<Vec<Captured>>().unwrap(),
                    other => panic!("expected extracted facts: {other:?}"),
                })
                .collect()
            };
            let main = events(&Knobs::default());
            let session = runtime::Session::start().unwrap();
            let borrowed = events(&Knobs::current());
            assert_eq!(main, borrowed);
            assert_eq!(
                session.report().unwrap()["delivered_family_count"],
                families.len()
            );
        }
    }
}

/// A family's panic in hour 101 stops the shared read before hour 102: the
/// read's later interruption must not replace the panic as that family's failure.
#[test]
fn runtime_keeps_an_earlier_panic_over_a_later_shared_read_failure() {
    for telemetry in [false, true] {
        let replay = recording(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let hook: Hook = Arc::new(move |family, frame| {
            if family == HOURS.name && frame.hour == 101 {
                stopping.store(true, Ordering::Release);
                return Fault::Panic;
            }
            Fault::Continue
        });
        let _session = telemetry.then(runtime::Session::start).transpose().unwrap();
        let knobs = Knobs {
            runtime: runtime::current(),
            hook: Some(hook),
            ..Knobs::default()
        };
        let errors: Vec<io::Error> = extract_block_measured(
            &replay,
            &replay.index[0],
            &[&HOURS, &HOURS_POWER],
            &stop,
            &knobs,
        )
        .outcomes
        .into_iter()
        .map(|(outcome, ..)| match outcome {
            Outcome::Failed(error) => error,
            other => panic!("expected a runtime failure: {other:?}"),
        })
        .collect();
        assert!(runtime::panicking(&errors[0]), "{}", errors[0]);
        assert!(!runtime::panicking(&errors[1]), "{}", errors[1]);
        assert_eq!(
            errors[1].kind(),
            io::ErrorKind::Interrupted,
            "{}",
            errors[1]
        );
    }
}

#[test]
fn a_setup_panic_names_only_its_family_and_preserves_healthy_sibling_facts() {
    struct BadChanges;
    impl Extract for BadChanges {
        fn read_by_changes(&mut self) {
            panic!("injected read_by_changes panic")
        }
        fn observe(&mut self, _: &Replay, _: &DecodedFrame, _: &FrameChanges) -> io::Result<()> {
            panic!("a failed setup must never observe")
        }
        fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
            panic!("a failed setup must never finish")
        }
    }
    static BAD_FIELDS: Family = Family {
        name: "broken field selection",
        fields: Some(|| panic!("injected fields panic")),
        start: |_| panic!("field selection failed before start"),
        ..CAPTURE_POWER
    };
    static BAD_START: Family = Family {
        name: "broken start",
        start: |_| panic!("injected start panic"),
        ..CAPTURE_POWER
    };
    static BAD_CHANGES: Family = Family {
        name: "broken read_by_changes",
        start: |_| Box::new(BadChanges),
        ..CAPTURE_POWER
    };
    let replay = moving_tables(32, true);
    let expected = capture(&replay, &[&CAPTURE_POWER], true).0.remove(0);
    for telemetry in [false, true] {
        for broken in [&BAD_FIELDS, &BAD_START, &BAD_CHANGES] {
            for reverse in [false, true] {
                let session = telemetry.then(runtime::Session::start).transpose().unwrap();
                let families = if reverse {
                    [broken, &CAPTURE_POWER]
                } else {
                    [&CAPTURE_POWER, broken]
                };
                let measured = extract_block_measured(
                    &replay,
                    &replay.index[0],
                    &families,
                    &AtomicBool::new(false),
                    &Knobs::current(),
                );
                assert_eq!(measured.reads, 1);
                assert_eq!(measured.outcomes.len(), 2);
                for (family, (outcome, ..)) in families.into_iter().zip(measured.outcomes) {
                    if family.name == broken.name {
                        let Outcome::Failed(error) = outcome else {
                            panic!("expected setup failure: {outcome:?}")
                        };
                        assert!(runtime::panicking(&error), "{error}");
                        assert!(error.to_string().contains(broken.name), "{error}");
                        assert!(!error.to_string().contains(CAPTURE_POWER.name), "{error}");
                        assert!(error.to_string().contains("physical block 0"), "{error}");
                    } else {
                        let Outcome::Facts(facts) = outcome else {
                            panic!("healthy sibling failed: {outcome:?}")
                        };
                        assert_eq!(*facts.downcast::<Vec<Captured>>().unwrap(), expected);
                    }
                }
                if let Some(session) = session {
                    let report = session.report().unwrap();
                    assert_eq!(report["delivered_family_count"], 2);
                    for family in report["families"].as_array().unwrap() {
                        assert_eq!(
                            family["failures"],
                            usize::from(family["family"] == broken.name)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn follow_releases_replaced_views_before_the_next_delta() {
    let count = 32;
    let replay = moving_tables(count, false);
    let (before, copies_before, reuses_before) =
        capture(&replay, &[&CAPTURE_POWER, &CAPTURE_ALL], false);
    let (after, copies_after, reuses_after) =
        capture(&replay, &[&CAPTURE_POWER, &CAPTURE_ALL], true);
    assert_eq!(before, after);
    for table in [TableId::Countries, TableId::States] {
        assert_eq!(
            copies_before[table.index()],
            [1, u64::from(count)],
            "{table:?}: the first changed hour copies the baseline held"
        );
        assert_eq!(
            reuses_before[table.index()],
            [9, 9 * u64::from(count)],
            "{table:?}: later ones catch up the view the hour before replaced"
        );
        assert_eq!(
            copies_after[table.index()],
            [0, 0],
            "{table:?}: no framework-held baseline copies"
        );
        assert_eq!(reuses_after[table.index()], [0, 0]);
    }
}

/// A family reading every hour of the countries, its facts as given.
static HOURS_POWER: Family = Family {
    name: "hours power",
    extract: "hours-power.extract",
    sequential: "hours-power.sequential",
    fallback: "hours-power.fallback",
    tables: &[TableId::Countries],
    fields: None,
    start: |_| -> Box<dyn Extract> { Box::new(Reported(Vec::new())) },
};

/// A pool reads each block once for every lane whose next block it is and
/// within reach: three lanes from the first block, two reading every hour
/// and one by its changes, take two blocks read twice, not six times, and
/// each lane's facts are those of its own read.
#[test]
fn a_pool_reads_each_block_once_for_every_lane_it_reaches() {
    let replay = recording(2);
    let pool = Pool::start(
        Arc::clone(&replay),
        vec![
            (&HOURS, 0, false),
            (&HOURS, 0, false),
            (&REPORTED, 0, false),
        ],
        1,
        Knobs::default(),
        None,
    );
    let cancelled = AtomicBool::new(false);
    for block in 0..2 {
        let first = replay.index[block].first_hour;
        for lane in 0..2 {
            let (_, hours) = facts(pool.take(lane, block, &cancelled).unwrap().0);
            assert_eq!(hours, (first..first + 3).collect::<Vec<_>>());
        }
        let (outcome, _) = pool.take(2, block, &cancelled).unwrap();
        let Outcome::Facts(given) = outcome else {
            panic!("the block was observed hourly");
        };
        let alone = extract_block_measured(
            &replay,
            &replay.index[block],
            &[&REPORTED],
            &cancelled,
            &Knobs::default(),
        )
        .outcomes
        .remove(0)
        .0;
        let Outcome::Facts(alone) = alone else {
            panic!("the block was observed hourly");
        };
        assert_eq!(
            *given.downcast::<Given>().unwrap(),
            *alone.downcast::<Given>().unwrap()
        );
    }
    assert_eq!(
        pool.read_time().1,
        2,
        "each block read once for all three lanes"
    );
}

/// A pool lane whose family chooses its run's reads waits for them: no block
/// is claimed until it binds them. Each block is then read with the bound
/// tables alone (no country links, which the family declares) and by the
/// bound fields, and its extract is given at each hour the block reader
/// changed for those fields exactly the tables the reader reports there.
#[test]
fn a_pool_lane_reads_by_its_runs_bound_reads_and_hands_the_readers_changed_tables() {
    // Hours 100 to 106 in one block, the power moving at 102 and 104.
    let frames: Vec<DecodedFrame> = (100..=106)
        .map(|hour| {
            source(
                1,
                hour,
                Some(if hour < 102 {
                    1
                } else if hour < 104 {
                    2
                } else {
                    3
                }),
            )
        })
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let pool = Pool::start(
        Arc::clone(&replay),
        vec![(&REPORTED, 0, true)],
        2,
        Knobs::default(),
        None,
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        pool.shared.lock().lanes[0].next,
        0,
        "no block is claimed before its reads are bound"
    );
    pool.bind(
        0,
        Some(Bound {
            tables: vec![TableId::Campaign, TableId::Countries],
            fields: Some(vec![power()]),
            start: Box::new(|_| -> Box<dyn Extract> { Box::new(Reported(Vec::new())) }),
        }),
    );
    let (outcome, _) = pool.take(0, 0, &AtomicBool::new(false)).unwrap();
    let Outcome::Facts(facts) = outcome else {
        panic!("the block was observed hourly");
    };
    let given = *facts.downcast::<Given>().unwrap();
    let mut expected: Given = Vec::new();
    replay
        .read_block_changes(
            &replay.index[0],
            &[power()],
            |change| {
                if let BlockChange::Changed { frame, tables, .. } = change {
                    expected.push((frame.hour, Some(tables.to_vec()), false));
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        expected.iter().map(|(hour, ..)| *hour).collect::<Vec<_>>(),
        [100, 102, 104]
    );
    assert_eq!(given, expected);
}

#[test]
fn leaders_fold_shared_variable_and_tag_visibility_without_decoding_on_the_family_thread() {
    use crate::products::unit_leaders::{self, Content, prepared::Prepared};
    use dashboard_base::prepared::series::tests::Fixture;
    use dashboard_text::triggers::{Eval, Evaluator, inputs};
    use replay::container::test_work::table_decodes;
    use replay_data::{
        rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow,
        rows::LexicalTokenRow, rows::Recorded, rows::ScriptVariableEntry, rows::ScriptVariablesRow,
        rows::TableRows,
    };

    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/test.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={}",
        ),
        (
            "common/characters/test.txt",
            "characters={mod_commander={corps_commander={visible={NOT={has_war_with=AAA has_war_with=BBB} check_variable={leader_gate=1}}}}}",
        ),
    ]);
    let templates =
        installed_content_core::characters::leader_templates(&catalogue.content.layers).unwrap();
    let template = &templates[0];
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = unit_leaders::prepared::test_fixtures::frame(hour, 1);
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].canonical_slot = 1;
                rows[0].tag = b"AAA".to_vec();
                rows.push(CountryRow {
                    id: 100_001,
                    canonical_slot: 2,
                    tag: b"BBB".to_vec(),
                    ..Default::default()
                });
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows[0].canonical_slot = 1;
                rows.push(CountryLinkRow {
                    id: 100_001,
                    canonical_slot: 2,
                });
            }
            *frame.tables[TableId::CountryCounterInputs.index()] =
                TableView::Complete(TableRows::CountryCounterInputs(
                    [100_000, 100_001]
                        .into_iter()
                        .map(|country| CountryCounterInputRow {
                            country,
                            owned_state_count: 1,
                            pride_of_the_fleet: hour as u64,
                            ..Default::default()
                        })
                        .collect(),
                ));
            *frame.tables[TableId::WarRelations.index()] =
                TableView::Complete(TableRows::WarRelations(Vec::new()));
            // Tag existence reads the owned-state counts above, not province rows.
            *frame.tables[TableId::Provinces.index()] =
                TableView::Complete(TableRows::Provinces(Vec::new()));
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *frame.tables[TableId::UnitLeaders.index()]
            {
                rows[0].visibility_choice = 1;
            }
            if let TableView::Complete(TableRows::Characters(rows)) =
                &mut *frame.tables[TableId::Characters.index()]
            {
                rows[0].template_present = 1;
                rows[0].template_tag = 901;
                rows[0].template_file = template.path.as_bytes().to_vec();
                rows[0].template_line = template.line.unwrap() as i32;
            }
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 901,
                    literal: template.tag.as_bytes().to_vec(),
                }]));
            *frame.tables[TableId::ScriptVariables.index()] =
                TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
                    scope_kind: 1,
                    scope_id: 100_000,
                    seed: Recorded::Value(Some(7)),
                    entries: vec![ScriptVariableEntry {
                        name: b"leader_gate".to_vec(),
                        value_raw: if hour == 101 { 0 } else { 100_000 },
                    }],
                }]));
            frame
        })
        .collect();
    let (source, context) = crate::test_fixtures::source_with_content(frames.clone(), &folder.0);
    let catalogue = crate::catalogue::catalogue(&source, context.content().unwrap()).unwrap();
    let content = Content::for_catalogue(&source, &context, Some(catalogue.clone()))
        .unwrap()
        .unwrap();
    let scripted = catalogue.scripted_content().unwrap();
    for (frame, expected) in frames[..2].iter().zip([Eval::True, Eval::False]) {
        let world = scripted.recorded_world(frame).unwrap();
        let row = &frame
            .complete::<replay_data::rows::tables::UnitLeaders>()
            .unwrap()[0];
        let visibility = world.decoded_leader_visibility(row);
        let (value, reads) = inputs::capture(frame, || {
            Evaluator::new(&world, None).decide_leader_visibility(row, visibility, true)
        });
        assert_eq!(value, expected);
        // Each explicit tag records these two scope reads. Before their
        // declarations, two tags reject a kept hour with four unknown inputs.
        for tag in ["AAA", "BBB"] {
            assert!(reads.keys().any(|key| *key
                == inputs::Key::Scopes(dashboard_text::scope::Query::AliasEntry(tag.into()))));
            assert!(reads.keys().any(|key| *key
                == inputs::Key::Scopes(dashboard_text::scope::Query::ResolveTag(tag.into()))));
        }
        assert!(reads.keys().all(|key| key.dependence().is_some()));
        assert!(
            reads
                .keys()
                .any(|key| key.dependence().is_some_and(|reads| {
                    reads
                        .iter()
                        .any(|read| read.table == TableId::ScriptVariables)
                }))
        );
    }

    let mut missing_provinces = frames[0].clone();
    *missing_provinces.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "missing test provinces".into(),
    };
    let world = scripted.recorded_world(&missing_provinces).unwrap();
    let row = &missing_provinces
        .complete::<replay_data::rows::tables::UnitLeaders>()
        .unwrap()[0];
    let decided = Evaluator::new(&world, None).decide_leader_visibility(
        row,
        world.decoded_leader_visibility(row),
        true,
    );
    assert_eq!(decided, Eval::True);

    let replay = source.replay_ref();
    let bound = unit_leaders::block::bound(true);
    assert!(bound.tables.contains(&TableId::Provinces));
    let readings = [
        Reading {
            family: &unit_leaders::block::FAMILY,
            bound: Some(&bound),
        },
        Reading::of(&HOURS),
    ];
    // The shared reader also wants the trigger kit's recorded updates.
    // A kept-only table is decoded but not reconstructed for an extract.
    let mut deposited: Vec<Arc<replay_frame::frame::RecordedFrame>> = Vec::new();
    let mut deposit =
        |frames: &[Arc<replay_frame::frame::RecordedFrame>]| deposited = frames.to_vec();
    let (mut measured, decodes) = table_decodes(|| {
        extract_bound(
            replay,
            &replay.index[0],
            &readings,
            &AtomicBool::new(false),
            &Knobs::default(),
            Some(Keep {
                tables: dashboard_text::triggers::RECORDED_TABLES,
                frames: &mut deposit,
            }),
        )
    });
    assert_eq!(measured.reads, 1);
    for shared in [TableId::ScriptVariables, TableId::CountryCounterInputs] {
        assert_eq!(
            decodes
                .iter()
                .filter(|(_, _, table, _)| *table == shared)
                .count(),
            1,
            "{shared:?} decoded once by the shared reader"
        );
    }
    let mut supplied = 0;
    assert!(
        replay
            .reconstruct_alone(
                &deposited,
                &[TableId::ScriptVariables],
                |_, frame| {
                    assert!(
                        frame
                            .complete::<replay_data::rows::tables::ScriptVariables>()
                            .is_ok()
                    );
                    supplied += 1;
                    Ok(())
                },
                || Ok(())
            )
            .unwrap()
    );
    assert_eq!(supplied, frames.len());
    let (Outcome::Facts(facts), ..) = measured.outcomes.remove(0) else {
        panic!("leaders' shared extract refused");
    };

    let mut folded = Prepared::new(replay, 1);
    let ((fold, reached), decodes) = table_decodes(|| {
        inputs::reusing(|| {
            unit_leaders::block::fold(
                &mut folded,
                facts,
                replay,
                &replay.index[0],
                Some(&content),
                &mut |_, result| result.map(|()| true),
                &mut || Ok(()),
            )
            .unwrap()
        })
    });
    assert_eq!(fold, Folded::Fast);
    assert_eq!(reached, Some(104));
    assert!(decodes.is_empty(), "family-thread decodes: {decodes:?}");
    assert!(bound.tables.contains(&TableId::ScriptVariables));

    let mut hourly = Prepared::new(replay, 1);
    let mut tracker = FrameChangeTracker::new();
    replay
        .read_block_alone(
            &replay.index[0],
            unit_leaders::block::tables(),
            |raw, frame| {
                let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                hourly.observe(frame, replay, Some(&content), &changes)
            },
            || Ok(()),
        )
        .unwrap();
    let publish = |prepared: &mut Prepared| {
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        prepared
            .flush(&mut writer, 1, Some(&content), &[], &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        fixture.payloads()
    };
    assert_eq!(publish(&mut folded), publish(&mut hourly));
}

/// The serial fold can dominate a cheap extract. Live fold and publication
/// costs must change both task selection and the order within a shared read.
#[test]
fn longest_first_uses_live_fold_and_publication_costs_across_block_frontiers() {
    let pool = Pool {
        shared: Arc::new(pool_state(
            vec![lane(10, 10, 100, 10), lane(12, 12, 10, 10)],
            1,
            0,
            &[],
            u64::MAX,
        )),
        workers: Vec::new(),
    };
    assert_eq!(pool.shared.lock().pick(), Some((10, vec![0])));
    pool.folded(1, Duration::from_millis(20));
    pool.published(1, Duration::from_millis(10));
    assert_eq!(pool.shared.lock().pick(), Some((12, vec![1])));
    {
        let mut state = pool.shared.lock();
        state.lanes[0].next = 12;
        state.lanes[0].handed = 12;
        assert_eq!(state.pick(), Some((12, vec![1, 0])));
        // A newly measured expensive fold changes the order again.
    }
    pool.folded(0, Duration::from_millis(50));
    assert_eq!(pool.shared.lock().pick(), Some((12, vec![0, 1])));
    let mut state = pool.shared.lock();
    // Remaining work matters, as well as mean work per block.
    state.lanes[0].end = 13;
    assert_eq!(state.pick(), Some((12, vec![1, 0])));
}

#[test]
fn longest_chain_does_not_claim_past_binding_read_ahead_or_byte_limits() {
    let mut expensive = lane(10, 10, 1000, 1);
    let state = pool_state(vec![lane(8, 8, 1, 1), expensive], 1, 0, &[], u64::MAX);
    assert_eq!(state.lock().pick(), Some((10, vec![1])));
    state.lock().lanes[1].handed = 2;
    assert_eq!(state.lock().pick(), Some((8, vec![0])));
    expensive = lane(10, 10, 1000, 1);
    expensive.binding = true;
    state.lock().lanes[1] = expensive;
    assert_eq!(state.lock().pick(), Some((8, vec![0])));
    {
        let mut locked = state.lock();
        locked.lanes[1].binding = false;
        locked.lanes[1].handed = 9;
        locked.budget = 0;
    }
    assert_eq!(state.lock().pick(), Some((8, vec![0])));
}

/// The two walk extracts feed one carried decoder/map/land chain. Its measured
/// fold and batch serialization must move both prerequisites ahead of shorter
/// family chains, while a more expensive family can still lead it.
#[test]
fn walk_chain_competes_with_families_by_its_live_combined_cost() {
    let pool = Pool {
        shared: Arc::new(pool_state(
            vec![
                lane(8, 8, 100, 10),
                lane(10, 10, 10, 10),
                lane(10, 10, 20, 10),
            ],
            1,
            0,
            &[],
            u64::MAX,
        )),
        workers: Vec::new(),
    };
    pool.shared.lock().walk = Some(1..3);
    assert_eq!(pool.shared.lock().pick(), Some((8, vec![0])));
    pool.walk_folded(Duration::from_millis(8));
    pool.walk_published(Duration::from_millis(4));
    assert_eq!(pool.shared.lock().pick(), Some((10, vec![1, 2])));
    {
        let mut state = pool.shared.lock();
        state.lanes[0].next = 10;
        state.lanes[0].handed = 10;
        assert_eq!(state.pick(), Some((10, vec![1, 2, 0])));
        // Only measured remaining work decides: no privileged fixed family list.
        state.lanes[0].folded = Duration::from_millis(30);
        state.lanes[0].folds = 1;
        assert_eq!(state.pick(), Some((10, vec![0, 1, 2])));
        for lane in &mut state.lanes[1..3] {
            lane.binding = true;
        }
        assert_eq!(state.pick(), Some((10, vec![0])));
    }
}

#[test]
fn walk_prepass_uses_the_registered_pool_and_keeps_extract_outputs_exact() {
    let replay = recording(3);
    let flag = AtomicBool::new(false);
    let pool = Pool::start_with_walk(
        Arc::clone(&replay),
        vec![(&HOURS, 0, false)],
        2,
        Knobs::default(),
        None,
        Some(0),
    );
    {
        let state = pool.shared.lock();
        assert!(state.lanes[1..].iter().all(|lane| lane.binding));
    }
    let walk = pool.walk().unwrap();
    assert!(Arc::ptr_eq(&walk.pooled.as_ref().unwrap().0, &pool.shared));
    let standalone = PrePass::start(Arc::clone(&replay), 0, families(), 1, Knobs::default());
    for block in 0..replay.index.len() {
        facts(pool.take(0, block, &flag).unwrap().0);
        // These synthetic frames do not supply army/resource facts: both paths
        // must make the same fallback decision for each independently read block.
        let pooled = walk.take(block, &flag).unwrap();
        let own = standalone.take(block, &flag).unwrap();
        for ((left, _), (right, _)) in pooled.iter().zip(&own) {
            assert_eq!(
                matches!(left, Outcome::Sequential),
                matches!(right, Outcome::Sequential)
            );
        }
        pool.walk_folded(Duration::from_millis(1));
    }
    drop(walk);
    assert!(
        pool.shared.lock().lanes[1..]
            .iter()
            .all(|lane| lane.retired)
    );
}

#[test]
fn map_page_extracts_report_their_lanes_states_and_steps() {
    use dashboard_base::prepared::prepass::HeldBytes;
    use dashboard_base::prepared_map::pages::{self, BlockPages};
    let measure = |hours: std::ops::RangeInclusive<i32>| -> (u64, BlockPages) {
        let frames = hours
            .map(|hour| crate::test_fixtures::scale_frame(1, hour, 3))
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let (outcome, _, bytes) = extract_block_measured(
            &replay,
            &replay.index[0],
            &[&*pages::FAMILY],
            &AtomicBool::new(false),
            &Knobs::default(),
        )
        .outcomes
        .pop()
        .unwrap();
        let Outcome::Facts(facts) = outcome else {
            panic!("map pages extracted the block");
        };
        (bytes, *facts.downcast::<BlockPages>().unwrap())
    };
    let (none, empty) = measure(1..=19);
    let (one, single) = measure(15..=30);
    let (many, full) = measure(1..=100);
    for (bytes, facts) in [(none, &empty), (one, &single), (many, &full)] {
        assert_eq!(bytes, HeldBytes::default().value(facts).total());
    }
    assert!(
        none >= (size_of::<BlockPages>() + 4 * size_of::<Option<DecodedFrame>>()) as u64,
        "lanes without a chain point hold their inline states: {none}"
    );
    let state = single.head(20).unwrap();
    assert_eq!(single.last(20).map(|last| last.hour), Some(20));
    assert_eq!(
        one - none,
        HeldBytes::default().heap(state).total(),
        "a block's first and last state, one frame, counted once"
    );
    assert!(many > one, "{many} for many chain points, {one} for one");
    let pool = Pool {
        shared: Arc::new(pool_state(
            vec![Lane {
                end: 10,
                ..lane(1, 0, 0, 0)
            }],
            1,
            1,
            &[0],
            1 + 2 * many,
        )),
        workers: Vec::new(),
    };
    let (task, measured) = finished(0, 0, Outcome::Facts(Box::new(full)), Duration::ZERO, many);
    pool.shared.finish(task, measured);
    {
        let state = pool.shared.lock();
        assert_eq!(state.in_flight, 1 + many);
        assert!(state.pick().is_none(), "its facts hold the next claim back");
    }
    pool.take(0, 0, &AtomicBool::new(false)).unwrap();
    assert_eq!(pool.shared.lock().in_flight, 0);
}

#[test]
fn a_pending_walk_reports_its_first_block_while_its_released_lanes_claim_ahead() {
    use crate::prepared::pace::Lifecycle;
    let replay = recording(5);
    let blocks = replay.index.len();
    let flag = AtomicBool::new(false);
    // A family's lane from block 0, and the walk resumed at block 1.
    let pool = Pool::start_with_walk(
        Arc::clone(&replay),
        vec![(&HOURS, 0, false)],
        2,
        Knobs::default(),
        None,
        Some(1),
    );
    let (pace, ended) = WalkPace::start(1, blocks);
    pool.pace_walk(Arc::clone(&pace));
    let until = |what: &str, done: &dyn Fn(&PoolState) -> bool| {
        let mut state = pool.shared.lock();
        let failure = loop {
            if done(&state) {
                break None;
            }
            if state.alive == 0 {
                break Some("every worker left");
            }
            state = pool
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        };
        drop(state);
        if let Some(failure) = failure {
            panic!("waiting for {what}: {failure}");
        }
    };
    until("the family's claims", &|state| state.lanes[0].next >= 3);
    assert_eq!(
        pace.snapshot().claimed,
        None,
        "a family's claims are not the walk's"
    );
    let walk = pool.walk().unwrap();
    // Under the pool's lock, then the obligation's, as claims take them.
    until("the walk's claims", &|_| {
        pace.snapshot().claimed.is_some_and(|claimed| claimed > 1)
    });
    let pending = pace.snapshot();
    assert_eq!(pending.lifecycle, Lifecycle::Pending);
    assert_eq!(pending.window(), Some((1, 1)), "only its first block");
    pace.at(1, blocks);
    let running = pace.snapshot();
    assert_eq!(running.window(), Some((1, running.claimed.unwrap())));
    for block in 0..blocks {
        facts(pool.take(0, block, &flag).unwrap().0);
    }
    for block in 1..blocks {
        pace.at(block, blocks);
        walk.take(block, &flag).unwrap();
    }
    assert!(
        pace.snapshot()
            .claimed
            .is_some_and(|claimed| (1..blocks).contains(&claimed)),
        "claims are absolute blocks of the recording"
    );
    pace.at(blocks, blocks);
    assert_eq!(
        pace.snapshot().window(),
        None,
        "past its last block it reads none"
    );
    drop(walk);
    drop(ended);
    assert_eq!(
        pace.history(),
        [Lifecycle::Pending, Lifecycle::Running, Lifecycle::Terminal]
    );
}

#[test]
fn a_walk_with_no_pool_worker_observes_hourly_within_its_cursor_alone() {
    use crate::prepared::pace::Lifecycle;
    let flag = AtomicBool::new(false);
    let blocks = 3;
    let mut shared = pool_state(
        vec![Lane {
            end: blocks,
            ..lane(0, 0, 0, 0)
        }],
        0,
        0,
        &[],
        u64::MAX,
    );
    shared
        .state
        .get_mut()
        .unwrap_or_else(|error| error.into_inner())
        .walk = Some(0..1);
    let pool = Pool {
        shared: Arc::new(shared),
        workers: Vec::new(),
    };
    let (pace, ended) = WalkPace::start(0, blocks);
    pool.pace_walk(Arc::clone(&pace));
    let walk = pool.walk().unwrap();
    assert_eq!(pace.snapshot().window(), Some((0, 0)), "pending");
    for block in 0..blocks {
        pace.at(block, blocks);
        let outcomes = walk.take(block, &flag).unwrap();
        assert!(
            matches!(outcomes.as_slice(), [(Outcome::Sequential, _)]),
            "block {block} is observed hour by hour"
        );
        assert_eq!(pace.snapshot().window(), Some((block, block)));
    }
    assert_eq!(pace.snapshot().claimed, None, "nothing was claimed");
    pace.at(blocks, blocks);
    drop(walk);
    drop(ended);
    assert_eq!(
        pace.history(),
        [Lifecycle::Pending, Lifecycle::Running, Lifecycle::Terminal]
    );
}

fn until(pool: &Pool, what: &str, done: &dyn Fn(&PoolState) -> bool) {
    let mut state = pool.shared.lock();
    let failure = loop {
        if done(&state) {
            break None;
        }
        if state.alive == 0 {
            break Some("every worker left");
        }
        state = pool
            .shared
            .changed
            .wait(state)
            .unwrap_or_else(|error| error.into_inner());
    };
    drop(state);
    if let Some(failure) = failure {
        panic!("waiting for {what}: {failure}");
    }
}

#[test]
fn paced_family_lanes_claim_nothing_beyond_the_walks_window_from_their_workers_start() {
    let replay = recording(5);
    let blocks = replay.index.len();
    let flag = AtomicBool::new(false);
    let _ = take_family_claims();
    let (pace, ended) = WalkPace::start(0, blocks);
    let pool = Pool::start_with_walk(
        Arc::clone(&replay),
        vec![(&HOURS, 0, false)],
        4,
        Knobs {
            pace: Some(Arc::clone(&pace)),
            ..Knobs::default()
        },
        None,
        None,
    );
    let extracted = |through: usize| {
        move |state: &PoolState| (0..=through).all(|block| state.lanes[0].done.contains_key(&block))
    };
    until(&pool, "block 0's extract", &extracted(0));
    assert_eq!(
        pool.shared.lock().lanes[0].next,
        1,
        "a pending walk admits its first block alone"
    );
    pace.at(0, blocks);
    pace.claimed(2);
    until(&pool, "blocks 1 and 2's extracts", &extracted(2));
    assert_eq!(
        pool.shared.lock().lanes[0].next,
        3,
        "a running walk admits its cursor through its lanes' claims"
    );
    drop(ended);
    until(&pool, "every block's extract", &extracted(blocks - 1));
    for block in 0..blocks {
        facts(pool.take(0, block, &flag).unwrap().0);
    }
    drop(pool);
    let claims = take_family_claims();
    assert_eq!(claims.len(), blocks, "each block claimed once");
    for (block, snapshot) in &claims {
        assert!(
            snapshot.window().is_none_or(|(_, last)| *block <= last),
            "block {block} claimed beyond the walk's window {snapshot:?}"
        );
    }
    // Unpaced, the same pool claims ahead of a pending walk.
    let (pending, _ended) = WalkPace::start(0, blocks);
    let pool = Pool::start_with_walk(
        Arc::clone(&replay),
        vec![(&HOURS, 0, false)],
        4,
        Knobs::default(),
        None,
        None,
    );
    until(&pool, "the unpaced lane's claims", &|state| {
        state.lanes[0].next == blocks
    });
    assert_eq!(pending.snapshot().window(), Some((0, 0)));
    for block in 0..blocks {
        facts(pool.take(0, block, &flag).unwrap().0);
    }
}

#[test]
fn a_walk_behind_every_paced_family_lane_keeps_the_pool() {
    let replay = recording(6);
    let blocks = replay.index.len();
    let flag = AtomicBool::new(false);
    let _ = take_family_claims();
    let (pace, ended) = WalkPace::start(0, blocks);
    let pool = Pool::start_with_walk(
        Arc::clone(&replay),
        vec![(&HOURS, 3, false), (&HOURS, 3, false)],
        2,
        Knobs {
            pace: Some(Arc::clone(&pace)),
            ..Knobs::default()
        },
        None,
        Some(0),
    );
    pool.pace_walk(Arc::clone(&pace));
    let walk = pool.walk().unwrap();
    for block in 0..blocks {
        pace.at(block, blocks);
        walk.take(block, &flag).unwrap();
    }
    pace.at(blocks, blocks);
    drop(walk);
    for lane in 0..2 {
        for block in 3..blocks {
            facts(pool.take(lane, block, &flag).unwrap().0);
        }
    }
    drop(pool);
    drop(ended);
    let claims = take_family_claims();
    assert_eq!(
        claims.len(),
        2 * (blocks - 3),
        "each family block claimed once"
    );
    for (block, snapshot) in &claims {
        assert!(
            snapshot.window().is_none_or(|(_, last)| *block <= last),
            "block {block} claimed beyond the walk's window {snapshot:?}"
        );
    }
}

#[test]
fn production_prepass_worker_policy_is_independent_of_measurement_session() {
    for cores in [1, 2, 16, 1500] {
        assert_eq!(workers(cores), 1);
    }
    let session = runtime::Session::start().unwrap();
    for cores in [1, 2, 16, 1500] {
        assert_eq!(workers(cores), 1);
    }
    drop(session);
}
