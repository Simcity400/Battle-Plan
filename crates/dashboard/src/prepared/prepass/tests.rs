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
