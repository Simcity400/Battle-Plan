use super::*;
use crate::columnar_frames::{
    Limits,
    fixtures::{every, recording},
};
use replay_columnar::envelope::UNIT_ORDINALS;
use std::{collections::HashMap, mem::size_of, sync::atomic::AtomicUsize};

/// Bytes alive at once, as the values counting themselves into it hold them.
struct Gauge {
    id: usize,
    now: AtomicUsize,
    peak: AtomicUsize,
}

/// Every gauge, by id, for items read back from a spill to count themselves in again.
static GAUGES: Mutex<Vec<&'static Gauge>> = Mutex::new(Vec::new());

impl Gauge {
    fn new() -> &'static Self {
        let mut gauges = lock(&GAUGES);
        let gauge: &'static Self = Box::leak(Box::new(Self {
            id: gauges.len(),
            now: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }));
        gauges.push(gauge);
        gauge
    }

    fn of(id: usize) -> &'static Self {
        lock(&GAUGES)[id]
    }

    fn add(&self, bytes: usize) {
        let now = self.now.fetch_add(bytes, Ordering::SeqCst) + bytes;
        self.peak.fetch_max(now, Ordering::SeqCst);
    }

    fn sub(&self, bytes: usize) {
        self.now.fetch_sub(bytes, Ordering::SeqCst);
    }

    fn now(&self) -> usize {
        self.now.load(Ordering::SeqCst)
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

/// What a probe saw of an hour: its branch, hour, run, ordinal, whether it started there,
/// and the hours its operator saw before it since it started.
type Saw = (u64, i32, u32, u32, bool, u64);

/// An hour a probe saw, holding bytes counted alive in its gauge.
struct Seen {
    saw: Saw,
    pad: Vec<u8>,
    gauge: &'static Gauge,
}

/// A spilled [`Seen`]: its gauge, branch, hour, run, ordinal, start, hours before and pad.
const SPILLED_SEEN: usize = 8 + 8 + 4 + 4 + 4 + 1 + 8 + 8;

impl Seen {
    fn new(saw: Saw, pad: usize, gauge: &'static Gauge) -> Self {
        gauge.add(pad);
        Self {
            saw,
            pad: vec![0; pad],
            gauge,
        }
    }
}

impl Drop for Seen {
    fn drop(&mut self) {
        self.gauge.sub(self.pad.len());
    }
}

fn read<const N: usize>(bytes: &[u8], at: &mut usize) -> [u8; N] {
    let read = bytes[*at..*at + N].try_into().unwrap();
    *at += N;
    read
}

impl Item for Seen {
    const SPILLS: bool = true;

    fn heap(&self) -> usize {
        self.pad.capacity()
    }

    fn save(items: &[Self], out: &mut Vec<u8>) -> io::Result<()> {
        for item in items {
            let (branch, hour, run, ordinal, starts, walked) = item.saw;
            out.extend_from_slice(&(item.gauge.id as u64).to_le_bytes());
            out.extend_from_slice(&branch.to_le_bytes());
            out.extend_from_slice(&hour.to_le_bytes());
            out.extend_from_slice(&run.to_le_bytes());
            out.extend_from_slice(&ordinal.to_le_bytes());
            out.push(u8::from(starts));
            out.extend_from_slice(&walked.to_le_bytes());
            out.extend_from_slice(&(item.pad.len() as u64).to_le_bytes());
        }
        Ok(())
    }

    fn load(bytes: &[u8]) -> io::Result<Vec<Self>> {
        assert_eq!(bytes.len() % SPILLED_SEEN, 0);
        Ok(bytes
            .chunks_exact(SPILLED_SEEN)
            .map(|item| {
                let at = &mut 0;
                let gauge = Gauge::of(u64::from_le_bytes(read(item, at)) as usize);
                let saw = (
                    u64::from_le_bytes(read(item, at)),
                    i32::from_le_bytes(read(item, at)),
                    u32::from_le_bytes(read(item, at)),
                    u32::from_le_bytes(read(item, at)),
                    read::<1>(item, at)[0] != 0,
                    u64::from_le_bytes(read(item, at)),
                );
                let pad = u64::from_le_bytes(read(item, at)) as usize;
                Seen::new(saw, pad, gauge)
            })
            .collect())
    }
}

/// A family of probes: each starts where `starts` lets it, sees each hour as a [`Seen`] of
/// `pad` bytes, and carries the hours it saw, and `weight` bytes counted in `operators`,
/// into the segments it cannot start; `spills` when it can write them.
struct Probes {
    starts: fn(Start<'_>) -> bool,
    pad: usize,
    weight: usize,
    spills: bool,
    items: &'static Gauge,
    operators: &'static Gauge,
    /// The first hour of each walk it started, and of each run it crossed into.
    started: Mutex<Vec<i32>>,
    crossed: Mutex<Vec<i32>>,
}

impl Probes {
    fn new(starts: fn(Start<'_>) -> bool) -> Self {
        Self {
            starts,
            pad: 8,
            weight: 0,
            spills: true,
            items: Gauge::new(),
            operators: Gauge::new(),
            started: Mutex::default(),
            crossed: Mutex::default(),
        }
    }

    fn make(&self, _: &Segment) -> io::Result<Probe<'_>> {
        self.operators.add(self.weight);
        Ok(Probe {
            family: self,
            walked: 0,
            weight: vec![0; self.weight],
        })
    }

    fn started(&self) -> Vec<i32> {
        let mut started = lock(&self.started).clone();
        started.sort_unstable();
        started
    }

    fn crossed(&self) -> Vec<i32> {
        let mut crossed = lock(&self.crossed).clone();
        crossed.sort_unstable();
        crossed
    }
}

struct Probe<'p> {
    family: &'p Probes,
    walked: u64,
    weight: Vec<u8>,
}

impl Drop for Probe<'_> {
    fn drop(&mut self) {
        self.family.operators.sub(self.weight.len());
    }
}

impl Operator for Probe<'_> {
    type Output = Seen;

    fn starts_at(&self, start: Start<'_>) -> bool {
        (self.family.starts)(start)
    }

    fn start(&mut self, _: Start<'_>, _: u64, first: i32) -> io::Result<()> {
        lock(&self.family.started).push(first);
        Ok(())
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        lock(&self.family.crossed).push(first);
        Ok(())
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut dyn Sink<Seen>) -> io::Result<()> {
        let saw = (
            hour.branch,
            hour.hour,
            hour.run,
            hour.ordinal,
            hour.starts,
            self.walked,
        );
        self.walked += 1;
        out.push(Seen::new(saw, self.family.pad, self.family.items))
    }

    fn carried(&self) -> Option<usize> {
        Some(self.weight.capacity())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        if !self.family.spills {
            return Ok(false);
        }
        out.extend_from_slice(&self.walked.to_le_bytes());
        out.extend_from_slice(&(self.weight.len() as u64).to_le_bytes());
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let at = &mut 0;
        self.walked = u64::from_le_bytes(read(bytes, at));
        assert_eq!(
            u64::from_le_bytes(read(bytes, at)) as usize,
            self.weight.len()
        );
        Ok(())
    }
}

/// Starts anywhere.
fn anywhere(_: Start<'_>) -> bool {
    true
}

/// Starts only a branch's first run: goes on through every restart and later run.
fn branch_start(start: Start<'_>) -> bool {
    matches!(start, Start::Run { continues: false })
}

/// Cannot start a later run of a branch.
fn not_continuing(start: Start<'_>) -> bool {
    !matches!(start, Start::Run { continues: true })
}

/// Cannot start at a run's second restart, which holds one string per hour before it.
fn not_second(start: Start<'_>) -> bool {
    match start {
        Start::Restart(state) => state.dictionary().len() != UNIT_ORDINALS as usize,
        Start::Run { .. } => true,
    }
}

/// A lane of `probes` whose emit keeps what each hour saw, checking each segment's chunks
/// come together and end with its last.
fn probing<'a>(name: &'a str, probes: &'a Probes, seen: &'a mut Vec<Saw>) -> Lane<'a> {
    let mut open: Option<Segment> = None;
    Lane::new(
        name,
        move |segment: &Segment| probes.make(segment),
        move |chunk: Chunk<'_, Seen>| {
            if let Some(open) = open {
                assert_eq!(open, *chunk.segment);
            }
            open = (!chunk.last).then_some(*chunk.segment);
            seen.extend(chunk.items.iter().map(|item| item.saw));
            Ok(())
        },
    )
}

/// What a probe that starts where `starts` lets it sees, hour by hour in segment order.
fn expected(file: &Recording, starts: fn(Start<'_>) -> bool) -> Vec<Saw> {
    let mut walked: HashMap<u64, u64> = HashMap::new();
    let mut saw = Vec::new();
    for segment in file.segments() {
        let run = file.run(segment.run).unwrap();
        let cold = match segment.restart {
            Some(page) => {
                let mut state = State::new(schema().unwrap()).unwrap();
                file.restore(segment.run, page, &mut state).unwrap();
                starts(Start::Restart(&state))
            }
            None => starts(Start::Run {
                continues: walked.contains_key(&run.branch),
            }),
        };
        let count = walked.entry(run.branch).or_default();
        if cold {
            *count = 0;
        }
        for ordinal in segment.first..=segment.last {
            let first = ordinal == segment.first;
            saw.push((
                run.branch,
                run.first_hour + ordinal as i32,
                segment.run,
                ordinal,
                first && (cold || segment.restart.is_none()),
                *count,
            ));
            *count += 1;
        }
    }
    saw
}

/// The first hour of each walk a probe seeing `saw` started, and of each run it crossed into.
fn starts_and_crossings(saw: &[Saw]) -> (Vec<i32>, Vec<i32>) {
    let mut started: Vec<i32> = saw
        .iter()
        .filter(|saw| saw.5 == 0)
        .map(|saw| saw.1)
        .collect();
    let mut crossed: Vec<i32> = saw
        .iter()
        .filter(|saw| saw.3 == 0 && saw.5 > 0)
        .map(|saw| saw.1)
        .collect();
    started.sort_unstable();
    crossed.sort_unstable();
    (started, crossed)
}

/// Scratch in memory, counting its spills and what it holds.
#[derive(Default)]
struct Memory {
    spills: AtomicUsize,
    held: Arc<AtomicUsize>,
}

struct Kept {
    bytes: Vec<u8>,
    held: Arc<AtomicUsize>,
}

impl Scratch for Memory {
    fn spill(&self, bytes: &[u8]) -> io::Result<Box<dyn Spilled>> {
        self.spills.fetch_add(1, Ordering::SeqCst);
        self.held.fetch_add(bytes.len(), Ordering::SeqCst);
        Ok(Box::new(Kept {
            bytes: bytes.to_vec(),
            held: Arc::clone(&self.held),
        }))
    }
}

impl Spilled for Kept {
    fn read(&self, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes);
        Ok(())
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        self.held.fetch_sub(self.bytes.len(), Ordering::SeqCst);
    }
}

fn settings(threads: usize, window: usize) -> Settings {
    Settings {
        window,
        slots: window,
        ..Settings::new(threads)
    }
}

fn opened(runs: &[(u64, i32, u32)]) -> Recording {
    let (bytes, _) = recording(runs, every(runs), 1, |_| {});
    Recording::open(bytes, Limits::default()).unwrap()
}

/// Runs of `branches` branches in turn, `rounds` times, each of two segments.
fn alternating(branches: u64, rounds: u64) -> Vec<(u64, i32, u32)> {
    (0..rounds * branches)
        .map(|at| (at % branches + 1, at as i32 * 10_000, UNIT_ORDINALS + 1))
        .collect()
}

/// Every segment reaches its family in order, each with every hour of its own once, its
/// first starting it, while no more segments are held than the window allows: of two runs,
/// and of six runs of twenty segments, three runs of one branch.
#[test]
fn segments_reach_their_family_in_order_with_their_own_hours() {
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
        let saw = expected(&file, anywhere);
        for (threads, window) in [(1, 1), (3, 1), (8, 2), (8, 64)] {
            let probes = Probes::new(anywhere);
            let mut seen = Vec::new();
            let stats = run(
                &file,
                &settings(threads, window),
                &Memory::default(),
                &Cancellation::default(),
                vec![probing("probe", &probes, &mut seen)],
            )
            .unwrap();
            assert_eq!(seen, saw);
            assert_eq!(
                probes.started(),
                starts_and_crossings(&saw).0,
                "each segment starts once"
            );
            assert_eq!(stats.walks, file.segments().len());
            assert!(stats.segments <= window);
            assert_eq!((stats.deferred, stats.parked), (0, 0));
            assert_eq!(probes.items.now(), 0);
            assert!(probes.items.peak() <= window * UNIT_ORDINALS as usize * probes.pad);
        }
    }
    let file = opened(&many);
    assert_eq!((file.run_count(), file.segments().len()), (6, 20));
}

/// The dictionary size of the state each segment starts from.
struct Started(Option<usize>);

struct Size(Option<usize>);

impl Item for Size {
    fn heap(&self) -> usize {
        0
    }
}

impl Operator for Started {
    type Output = Size;

    fn start(&mut self, start: Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.0 = Some(match start {
            Start::Restart(state) => state.dictionary().len(),
            Start::Run { .. } => 0,
        });
        Ok(())
    }

    fn hour(&mut self, _: &Hour<'_>, _: &mut dyn Sink<Size>) -> io::Result<()> {
        Ok(())
    }

    fn end(&mut self, out: &mut dyn Sink<Size>) -> io::Result<()> {
        out.push(Size(self.0.take()))
    }
}

/// Each segment's operators see the state its first hour applies to: empty at a run's start,
/// else what its restart holds (here, one dictionary string per hour before it).
#[test]
fn operators_start_from_the_state_before_the_first_hour() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let mut sizes = Vec::new();
    let lane = Lane::new(
        "started",
        |_: &Segment| -> io::Result<Started> { Ok(Started(None)) },
        |chunk: Chunk<'_, Size>| {
            sizes.extend(chunk.items.iter().map(|size| size.0));
            Ok(())
        },
    );
    run(
        &file,
        &settings(2, 2),
        &Memory::default(),
        &Cancellation::default(),
        vec![lane],
    )
    .unwrap();
    let ordinals = UNIT_ORDINALS as usize;
    assert_eq!(sizes, [Some(0), Some(ordinals), Some(2 * ordinals)]);
}

/// Walks of a recording read from its file take, as they start, the whole share of the
/// walkers' budget their segment's reads need, its stored bytes too: under a budget admitting
/// one walk at a time or any number, checking the digest chain or not, every hour reaches its
/// family in order.
#[test]
fn walks_of_a_file_read_within_their_segment_s_share() {
    let runs = [
        (1, 0, 3 * UNIT_ORDINALS + 5),
        (2, 50_000, 2 * UNIT_ORDINALS),
    ];
    let (bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let path = std::env::temp_dir().join(format!("segments-shares-{}", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    for verify in [false, true] {
        let mut file = Recording::open_path(&path, Limits::default()).unwrap();
        file.verify(verify);
        let saw = expected(&file, anywhere);
        for walker_bytes in [1, usize::MAX] {
            let probes = Probes::new(anywhere);
            let mut seen = Vec::new();
            run(
                &file,
                &Settings {
                    walker_bytes,
                    ..settings(3, 4)
                },
                &Memory::default(),
                &Cancellation::default(),
                vec![probing("probe", &probes, &mut seen)],
            )
            .unwrap();
            assert_eq!(seen, saw);
        }
    }
    std::fs::remove_file(&path).unwrap();
}

/// A segment a family cannot start goes on from the operator that walked the segment before
/// in its run: every hour reaches it once, in order, and only a walk's first hour starts it.
#[test]
fn a_segment_its_family_cannot_start_goes_on_from_the_one_before() {
    let file = opened(&[(1, 0, 4 * UNIT_ORDINALS), (2, 9_000, 2 * UNIT_ORDINALS)]);
    let saw = expected(&file, not_second);
    // The second restart of each run is gone on through.
    let starts: Vec<bool> = file
        .segments()
        .iter()
        .map(|segment| {
            saw.iter()
                .find(|saw| saw.2 == segment.run && saw.3 == segment.first)
                .unwrap()
                .4
        })
        .collect();
    assert_eq!(starts, [true, false, true, true, true, false]);
    for (threads, window) in [(1, 1), (1, 2), (4, 1), (4, 2)] {
        let probes = Probes::new(not_second);
        let mut seen = Vec::new();
        let stats = run(
            &file,
            &settings(threads, window),
            &Memory::default(),
            &Cancellation::default(),
            vec![probing("picky", &probes, &mut seen)],
        )
        .unwrap();
        assert_eq!(seen, saw);
        assert_eq!(
            (probes.started(), probes.crossed()),
            starts_and_crossings(&saw)
        );
        assert!(stats.segments <= window);
    }
}

/// A run of a branch an earlier run is of, which its family cannot start, goes on from that
/// earlier run's last segment, from empty tables, crossing into it once; the rest of the run
/// starts from its restarts.
#[test]
fn a_branch_s_later_run_its_family_cannot_start_goes_on_from_its_earlier_run() {
    let file = opened(&[
        (1, 0, 2 * UNIT_ORDINALS),
        (2, 50_000, UNIT_ORDINALS),
        (1, 90_000, UNIT_ORDINALS + 5),
    ]);
    let saw = expected(&file, not_continuing);
    let units = UNIT_ORDINALS as i32;
    assert_eq!(
        starts_and_crossings(&saw),
        (vec![0, units, 50_000, 90_000 + units], vec![90_000])
    );
    for threads in [1, 4] {
        let probes = Probes::new(not_continuing);
        let mut seen = Vec::new();
        run(
            &file,
            &settings(threads, 2),
            &Memory::default(),
            &Cancellation::default(),
            vec![probing("continuing", &probes, &mut seen)],
        )
        .unwrap();
        assert_eq!(seen, saw);
        assert_eq!(
            (probes.started(), probes.crossed()),
            starts_and_crossings(&saw)
        );
    }
}

/// Over branches in turn, a family going on through every segment of its branch and one
/// starting everywhere: on one worker and a window of one, and on two workers and a window
/// of two, each hour reaches each once, in order; each branch's operator starts once and
/// crosses once into each of its later runs; no more segments are held than the window
/// allows, nor projected items than the window's segments hold; and parked operators, never
/// a walker's tables, stay within their budget, but for the one being parked, however many
/// branches wait, spilled past it and going on as they were.
#[test]
fn alternating_branches_go_on_within_the_window_and_the_continuation_budget() {
    let weight = 64;
    let carried = size_of::<Probe<'static>>() + weight;
    for branches in [2, 5] {
        let file = opened(&alternating(branches, 4));
        let (cold, going) = (expected(&file, anywhere), expected(&file, branch_start));
        let (started, crossed) = starts_and_crossings(&going);
        assert_eq!(
            (started.len(), crossed.len()),
            (branches as usize, 3 * branches as usize)
        );
        for (threads, window) in [(1, 1), (2, 2)] {
            for carry_bytes in [0, usize::MAX] {
                let anywhere_probes = Probes::new(anywhere);
                let going_probes = Probes {
                    weight,
                    ..Probes::new(branch_start)
                };
                let scratch = Memory::default();
                let (mut cold_seen, mut going_seen) = (Vec::new(), Vec::new());
                let stats = run(
                    &file,
                    &Settings {
                        carry_bytes,
                        ..settings(threads, window)
                    },
                    &scratch,
                    &Cancellation::default(),
                    vec![
                        probing("cold", &anywhere_probes, &mut cold_seen),
                        probing("going", &going_probes, &mut going_seen),
                    ],
                )
                .unwrap();
                assert_eq!(cold_seen, cold);
                assert_eq!(going_seen, going);
                assert_eq!(
                    (going_probes.started(), going_probes.crossed()),
                    (started.clone(), crossed.clone())
                );
                assert_eq!(
                    (anywhere_probes.started(), anywhere_probes.crossed()),
                    starts_and_crossings(&cold)
                );
                assert!(stats.segments <= window);
                let segment = UNIT_ORDINALS as usize * anywhere_probes.pad;
                assert!(anywhere_probes.items.peak() <= window * segment);
                assert!(going_probes.items.peak() <= window * segment);
                assert!(stats.parked > 0);
                if carry_bytes == 0 {
                    // Each held only as it is parked, until it spills.
                    assert_eq!(stats.carry_bytes, carried);
                    // Its walked hours and its weight's length, written and read back.
                    assert!(stats.carry_scratch >= 16);
                    assert_eq!((stats.spilled, stats.loaded), (stats.parked, stats.parked));
                    assert_eq!(scratch.spills.load(Ordering::SeqCst), stats.spilled);
                    // An operator per worker, and none parked.
                    assert!(going_probes.operators.peak() <= threads * weight);
                } else {
                    assert_eq!((stats.spilled, stats.carry_scratch), (0, 0));
                    // At most one parked per branch, each its operator alone.
                    assert!(stats.carry_bytes <= branches as usize * carried);
                    assert!(
                        going_probes.operators.peak() <= (threads + branches as usize) * weight
                    );
                }
                assert_eq!(scratch.held.load(Ordering::SeqCst), 0);
                assert_eq!(going_probes.operators.now(), 0);
                assert_eq!(going_probes.items.now(), 0);
            }
        }
    }
}

/// A restart one family cannot start leaves that family alone to go on from the segment
/// before: the other starts every segment, each hour reaching each family once, and a
/// segment is walked again only for a family it deferred.
#[test]
fn a_restart_one_family_cannot_start_defers_that_family_alone() {
    let file = opened(&[(1, 0, 4 * UNIT_ORDINALS), (2, 9_000, 3 * UNIT_ORDINALS)]);
    let (cold, picky) = (expected(&file, anywhere), expected(&file, not_second));
    for (threads, window) in [(1, 1), (1, 4), (4, 4)] {
        let (anywhere_probes, picky_probes) = (Probes::new(anywhere), Probes::new(not_second));
        let (mut cold_seen, mut picky_seen) = (Vec::new(), Vec::new());
        let stats = run(
            &file,
            &settings(threads, window),
            &Memory::default(),
            &Cancellation::default(),
            vec![
                probing("cold", &anywhere_probes, &mut cold_seen),
                probing("picky", &picky_probes, &mut picky_seen),
            ],
        )
        .unwrap();
        assert_eq!(cold_seen, cold);
        assert_eq!(picky_seen, picky);
        assert_eq!(anywhere_probes.started(), starts_and_crossings(&cold).0);
        assert_eq!(picky_probes.started(), starts_and_crossings(&picky).0);
        let segments = file.segments().len();
        assert!(stats.walks >= segments);
        assert!(stats.walks <= segments + stats.deferred);
        assert!(stats.segments <= window);
    }
}

/// A family whose first emit is slow holds the window, not the walks within it nor the other
/// family's emits: every segment the window admits is walked and emitted by the other family
/// while the first waits, and none past it starts.
#[test]
fn a_slow_emit_holds_its_family_not_the_walks_or_the_other_families() {
    let file = opened(&[(1, 0, 6 * UNIT_ORDINALS)]);
    let window = 3;
    let saw = expected(&file, anywhere);
    let (slow, fast) = (Probes::new(anywhere), Probes::new(anywhere));
    let emitted = (Mutex::new(0_usize), Condvar::new());
    let (mut slow_seen, mut fast_seen) = (Vec::new(), Vec::new());
    let slow_lane = Lane::new(
        "slow",
        |segment: &Segment| slow.make(segment),
        |chunk: Chunk<'_, Seen>| {
            if chunk.segment.first == 0 {
                // Waits for other tasks, as no emit may: the run's second worker runs them.
                let mut count = lock(&emitted.0);
                while *count < window {
                    count = emitted.1.wait(count).unwrap();
                }
                assert_eq!(slow.started().len(), window);
            }
            slow_seen.extend(chunk.items.iter().map(|item| item.saw));
            Ok(())
        },
    );
    let fast_lane = Lane::new(
        "fast",
        |segment: &Segment| fast.make(segment),
        |chunk: Chunk<'_, Seen>| {
            fast_seen.extend(chunk.items.iter().map(|item| item.saw));
            if chunk.last {
                *lock(&emitted.0) += 1;
                emitted.1.notify_all();
            }
            Ok(())
        },
    );
    let stats = run(
        &file,
        &settings(2, window),
        &Memory::default(),
        &Cancellation::default(),
        vec![slow_lane, fast_lane],
    )
    .unwrap();
    assert_eq!(stats.segments, window);
    assert_eq!((slow_seen, fast_seen), (saw.clone(), saw));
}

/// In run 1, at its walk's first hour, holding the walkers' only share: says so, then waits
/// for run 0's emit and for every other worker to wait for work.
struct Sharing<'t> {
    run: u32,
    held: &'t Flag,
    emitted: &'t Flag,
    cancellation: &'t Cancellation,
    others: usize,
}

impl Operator for Sharing<'_> {
    type Output = ();

    fn hour(&mut self, hour: &Hour<'_>, _: &mut dyn Sink<()>) -> io::Result<()> {
        if hour.starts && self.run == 1 {
            self.held.set();
            self.emitted.wait();
            self.cancellation.wait_for_idle(self.others);
        }
        Ok(())
    }
}

/// Under a walkers' budget admitting one walk at a time, on sixteen workers, no worker waits
/// inside a walk for that share: while run 1's walk holds it, run 0's emit runs on another
/// worker, and every other worker waits for work though runs 2 and 3 are ready to walk.
#[test]
fn a_one_walker_budget_leaves_the_other_workers_free_for_emits() {
    let file = opened(&[(1, 0, 10), (2, 100, 10), (3, 200, 10), (4, 300, 10)]);
    let threads = 16;
    let (held, emitted, cancellation) = (Flag::default(), Flag::default(), Cancellation::default());
    let mut runs = Vec::new();
    let lane = Lane::new(
        "sharing",
        |segment: &Segment| {
            Ok(Sharing {
                run: segment.run,
                held: &held,
                emitted: &emitted,
                cancellation: &cancellation,
                others: threads - 1,
            })
        },
        |chunk: Chunk<'_, ()>| {
            if chunk.segment.run == 0 {
                // Waits for another task, as no emit may: run 1's walk, on another worker.
                held.wait();
                emitted.set();
            }
            runs.push(chunk.segment.run);
            Ok(())
        },
    );
    let stats = run(
        &file,
        &Settings {
            walker_bytes: 1,
            ..settings(threads, threads)
        },
        &Memory::default(),
        &cancellation,
        vec![lane],
    )
    .unwrap();
    assert_eq!(runs, [0, 1, 2, 3]);
    assert_eq!(stats.walks, 4);
}

/// With no output budget to spare, once a walk has sealed and so says what a segment needs,
/// only the first segment some family has not emitted starts: while the first waits for its
/// emit, the other workers find nothing to do. One segment is held at a time, however many
/// workers, and every chunk spills, the first segment's too.
#[test]
fn a_spent_output_budget_admits_only_the_first_segment_not_emitted() {
    let file = opened(&[(1, 0, 4 * UNIT_ORDINALS), (2, 9_000, 2 * UNIT_ORDINALS)]);
    let saw = expected(&file, anywhere);
    let first = file.segments()[0];
    for threads in [1, 3] {
        let cancellation = Cancellation::default();
        let probes = Probes::new(anywhere);
        let mut seen = Vec::new();
        let lane = Lane::new(
            "probe",
            |segment: &Segment| probes.make(segment),
            |chunk: Chunk<'_, Seen>| {
                if *chunk.segment == first {
                    cancellation.wait_for_idle(threads - 1);
                    assert_eq!(probes.started(), [0]);
                }
                seen.extend(chunk.items.iter().map(|item| item.saw));
                Ok(())
            },
        );
        let stats = run(
            &file,
            &Settings {
                output_bytes: 0,
                ..settings(threads, 8)
            },
            &Memory::default(),
            &cancellation,
            vec![lane],
        )
        .unwrap();
        assert_eq!(seen, saw);
        assert_eq!(stats.segments, 1);
        assert_eq!(stats.walks, file.segments().len());
        assert_eq!(stats.chunks_spilled, file.segments().len());
        assert!(probes.items.peak() <= UNIT_ORDINALS as usize * probes.pad);
        assert_eq!(probes.items.now(), 0);
    }
}

/// A family holding all its slots holds the walks past the first segment it has not
/// emitted: the other worker finds nothing to do.
#[test]
fn a_family_holding_its_slots_holds_the_walks() {
    let file = opened(&[(1, 0, 4 * UNIT_ORDINALS)]);
    let saw = expected(&file, anywhere);
    let cancellation = Cancellation::default();
    let probes = Probes::new(anywhere);
    let mut seen = Vec::new();
    let lane = Lane::new(
        "probe",
        |segment: &Segment| probes.make(segment),
        |chunk: Chunk<'_, Seen>| {
            if chunk.segment.first == 0 {
                cancellation.wait_for_idle(1);
                // Segment 0, and one the other worker started before it sealed.
                assert!(probes.started().len() <= 2);
            }
            seen.extend(chunk.items.iter().map(|item| item.saw));
            Ok(())
        },
    );
    let stats = run(
        &file,
        &Settings {
            slots: 1,
            ..settings(2, 4)
        },
        &Memory::default(),
        &cancellation,
        vec![lane],
    )
    .unwrap();
    assert!(stats.segments <= 2);
    assert_eq!(seen, saw);
}

/// The walks of two runs, each waiting at its first hour until the other's runs too, or
/// every other worker waits for work; and the most seen at once.
struct Together<'t> {
    walking: AtomicUsize,
    most: AtomicUsize,
    threads: usize,
    cancellation: &'t Cancellation,
}

impl Together<'_> {
    fn meet(&self) {
        let walking = self.walking.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(walking, Ordering::SeqCst);
        self.cancellation.poke();
        self.cancellation.wait_for_idle_until(|idle| {
            let walking = self.walking.load(Ordering::SeqCst);
            walking >= 2 || idle + walking >= self.threads
        });
    }
}

/// Projects a [`Seen`] of `pad` bytes each hour; `together`, meets it at its walk's first.
struct Padding<'t> {
    pad: usize,
    items: &'static Gauge,
    together: Option<&'t Together<'t>>,
}

impl Operator for Padding<'_> {
    type Output = Seen;

    fn hour(&mut self, hour: &Hour<'_>, out: &mut dyn Sink<Seen>) -> io::Result<()> {
        if hour.starts
            && let Some(together) = self.together
        {
            together.meet();
        }
        let saw = (
            hour.branch,
            hour.hour,
            hour.run,
            hour.ordinal,
            hour.starts,
            0,
        );
        out.push(Seen::new(saw, self.pad, self.items))
    }
}

/// A segment projecting far past the output budget leaves the walks after it of a family
/// whose items spill to start together, each taking a chunk of the budget, not what that
/// segment projected.
#[test]
fn one_oversized_segment_leaves_later_spilling_walks_together() {
    let file = opened(&[
        (1, 0, 10),
        (2, 100, 10),
        (3, 200, 10),
        (4, 300, 10),
        (5, 400, 10),
    ]);
    let (threads, output_bytes) = (4, 64 << 10);
    let cancellation = Cancellation::default();
    let together = Together {
        walking: AtomicUsize::new(0),
        most: AtomicUsize::new(0),
        threads,
        cancellation: &cancellation,
    };
    let items = Gauge::new();
    let mut hours = Vec::new();
    let lane = Lane::new(
        "padding",
        |segment: &Segment| {
            Ok(Padding {
                pad: if segment.run == 0 { output_bytes } else { 8 },
                items,
                together: matches!(segment.run, 1 | 2).then_some(&together),
            })
        },
        |chunk: Chunk<'_, Seen>| {
            hours.extend(chunk.items.iter().map(|item| item.saw.1));
            Ok(())
        },
    );
    let stats = run(
        &file,
        &Settings {
            output_bytes,
            chunk_bytes: 1 << 10,
            ..settings(threads, 8)
        },
        &Memory::default(),
        &cancellation,
        vec![lane],
    )
    .unwrap();
    let all: Vec<i32> = [0, 100, 200, 300, 400]
        .into_iter()
        .flat_map(|first| first..first + 10)
        .collect();
    assert_eq!(hours, all);
    assert!(stats.chunks_spilled > 0);
    assert_eq!(together.most.load(Ordering::SeqCst), 2);
    assert_eq!(items.now(), 0);
}

/// Pushes `count` items of `pad` bytes at its segment's end.
struct Ending {
    count: usize,
    pad: usize,
    gauge: &'static Gauge,
}

impl Operator for Ending {
    type Output = Seen;

    fn hour(&mut self, _: &Hour<'_>, _: &mut dyn Sink<Seen>) -> io::Result<()> {
        Ok(())
    }

    fn end(&mut self, out: &mut dyn Sink<Seen>) -> io::Result<()> {
        for at in 0..self.count {
            out.push(Seen::new(
                (0, at as i32, 0, 0, false, 0),
                self.pad,
                self.gauge,
            ))?;
        }
        Ok(())
    }
}

/// Chunks close at their size; past the output budget, those of every segment spill, the
/// first some family has not emitted's too, and come back to the emit in order, each once
/// what reading it back takes is held.
#[test]
fn chunks_close_at_their_size_and_spill_past_the_budget() {
    let gauge = Gauge::new();
    let signal = Signal {
        schedule: Mutex::new(Schedule::new(2, vec![true]).unwrap()),
        changed: Condvar::new(),
        stopped: AtomicBool::new(false),
    };
    let scratch = Memory::default();
    let settings = Settings {
        output_bytes: 0,
        chunk_bytes: 1,
        ..Settings::new(1)
    };
    let shared = Shared::new(settings, &scratch, &signal);
    let mut emitted = Vec::new();
    let lane = Lane::new(
        "ending",
        |_: &Segment| -> io::Result<Ending> {
            Ok(Ending {
                count: 3,
                pad: 16,
                gauge,
            })
        },
        |chunk: Chunk<'_, Seen>| {
            let hours: Vec<i32> = chunk.items.iter().map(|item| item.saw.1).collect();
            emitted.push((hours, chunk.last));
            Ok(())
        },
    );
    let segment = Segment {
        run: 0,
        restart: None,
        first: 0,
        last: 0,
    };
    let reserved = shared.credit.reserve(0);
    for at in [0, 1] {
        let mut walking = lane.0.fresh(&segment).unwrap();
        let context = Context {
            shared: &shared,
            at,
            reserved: &reserved,
        };
        walking.end(&context).unwrap();
        assert!(walking.seal(&context).unwrap() >= 3 * 16);
    }
    assert_eq!(scratch.spills.load(Ordering::SeqCst), 6);
    assert_eq!(shared.chunks_spilled.load(Ordering::SeqCst), 6);
    assert_eq!((gauge.now(), shared.credit.held()), (0, 0));
    for at in [0, 1] {
        // Each spilled chunk stops the emit until what reading it back takes is held for it.
        let mut restore = None;
        while let Emitted::Spilled(need) = lane.0.emit(at, &segment, &shared, restore).unwrap() {
            assert!(need >= SPILLED_SEEN + 16);
            restore = Some(shared.credit.hold(need));
        }
    }
    drop(lane);
    let chunks = [(vec![0], false), (vec![1], false), (vec![2], true)];
    assert_eq!(emitted, [chunks.clone(), chunks].concat());
    assert_eq!(shared.credit.held(), 0);
    assert_eq!(gauge.now(), 0);
    assert_eq!(scratch.held.load(Ordering::SeqCst), 0);
}

/// Sixteen families' emits on sixteen workers, each reading back the spilled chunks of the
/// same segment, hold no more at once than the working allowance, their reading back counted
/// in: the rest wait for room on no worker, and every hour reaches its family in order.
#[test]
fn sixteen_emits_reading_back_spilled_chunks_stay_within_the_allowance() {
    let file = opened(&[(1, 0, 10)]);
    let saw = expected(&file, anywhere);
    let threads = 16;
    let working_bytes = 4 << 10;
    let cancellation = Cancellation::default();
    let (inside, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
    // The bytes of the chunks read back that emits hold at once.
    let restored = Gauge::new();
    let families: Vec<(String, Probes)> = (0..threads)
        .map(|lane| {
            let probes = Probes {
                pad: 1 << 10,
                ..Probes::new(anywhere)
            };
            (format!("probe {lane}"), probes)
        })
        .collect();
    let mut seen: Vec<Vec<Saw>> = (0..threads).map(|_| Vec::new()).collect();
    let (inside, most, cancellation_ref) = (&inside, &most, &cancellation);
    let lanes = families
        .iter()
        .zip(seen.iter_mut())
        .map(|((name, probes), seen)| {
            Lane::new(
                name,
                move |segment: &Segment| probes.make(segment),
                move |chunk: Chunk<'_, Seen>| {
                    let bytes = chunk
                        .items
                        .iter()
                        .fold(chunk.items.capacity() * size_of::<Seen>(), |bytes, item| {
                            bytes + item.pad.capacity()
                        });
                    restored.add(bytes);
                    most.fetch_max(inside.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                    cancellation_ref.poke();
                    // Every worker emits or waits for work: no other chunk is being read back.
                    cancellation_ref.wait_for_idle_until(|idle| {
                        idle + inside.load(Ordering::SeqCst) >= threads
                    });
                    seen.extend(chunk.items.iter().map(|item| item.saw));
                    inside.fetch_sub(1, Ordering::SeqCst);
                    restored.sub(bytes);
                    cancellation_ref.poke();
                    Ok(())
                },
            )
        })
        .collect();
    let stats = run(
        &file,
        &Settings {
            output_bytes: 0,
            working_bytes,
            chunk_bytes: 1,
            ..settings(threads, threads)
        },
        &Memory::default(),
        &cancellation,
        lanes,
    )
    .unwrap();
    for seen in &seen {
        assert_eq!(*seen, saw);
    }
    assert_eq!(stats.chunks_spilled, threads * saw.len());
    assert!(restored.peak() <= working_bytes, "{}", restored.peak());
    assert!(
        stats.output_bytes <= working_bytes,
        "{}",
        stats.output_bytes
    );
    assert!(most.load(Ordering::SeqCst) < threads);
    assert_eq!(restored.now(), 0);
    for (_, probes) in &families {
        assert_eq!(probes.items.now(), 0);
    }
}

/// A spilled chunk that reading back would take past the output budget and its working
/// allowance, with nothing left to make room, stops the run, its family named, rather than
/// leaving it waiting.
#[test]
fn a_spilled_chunk_too_large_to_read_back_stops_the_run() {
    let file = opened(&[(1, 0, 10)]);
    for threads in [1, 3] {
        let probes = Probes::new(anywhere);
        let mut seen = Vec::new();
        let error = run(
            &file,
            &Settings {
                output_bytes: 0,
                working_bytes: 0,
                chunk_bytes: 1,
                ..settings(threads, 2)
            },
            &Memory::default(),
            &Cancellation::default(),
            vec![probing("unread", &probes, &mut seen)],
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        assert!(error.to_string().contains("unread"), "{error}");
        assert!(seen.is_empty());
        assert_eq!(probes.items.now(), 0);
    }
}

/// An item whose codec reads back eight times the bytes it saved.
struct Inflated {
    pad: Vec<u8>,
}

impl Item for Inflated {
    const SPILLS: bool = true;

    fn heap(&self) -> usize {
        self.pad.capacity()
    }

    fn save(items: &[Self], out: &mut Vec<u8>) -> io::Result<()> {
        for item in items {
            out.extend_from_slice(&(item.pad.len() as u64).to_le_bytes());
        }
        Ok(())
    }

    fn load(bytes: &[u8]) -> io::Result<Vec<Self>> {
        Ok(bytes
            .chunks_exact(8)
            .map(|len| Inflated {
                pad: vec![0; 8 * u64::from_le_bytes(len.try_into().unwrap()) as usize],
            })
            .collect())
    }
}

/// Projects an [`Inflated`] of 64 bytes each hour.
struct Inflating;

impl Operator for Inflating {
    type Output = Inflated;

    fn hour(&mut self, _: &Hour<'_>, out: &mut dyn Sink<Inflated>) -> io::Result<()> {
        out.push(Inflated { pad: vec![0; 64] })
    }
}

/// A chunk read back larger than it was when it spilled stops the run, its family named,
/// rather than holding bytes its admission never counted.
#[test]
fn a_chunk_read_back_larger_than_it_spilled_stops_the_run() {
    let file = opened(&[(1, 0, 10)]);
    for threads in [1, 3] {
        let mut emitted = 0;
        let lane = Lane::new(
            "inflating",
            |_: &Segment| -> io::Result<Inflating> { Ok(Inflating) },
            |chunk: Chunk<'_, Inflated>| {
                emitted += chunk.items.len();
                Ok(())
            },
        );
        let error = run(
            &file,
            &Settings {
                output_bytes: 0,
                working_bytes: usize::MAX,
                chunk_bytes: 1,
                ..settings(threads, 2)
            },
            &Memory::default(),
            &Cancellation::default(),
            vec![lane],
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("inflating"), "{error}");
        assert_eq!(emitted, 0);
    }
}

/// An item that cannot spill, holding bytes counted alive in its gauge.
struct Pinned {
    pad: Vec<u8>,
    gauge: &'static Gauge,
}

impl Drop for Pinned {
    fn drop(&mut self) {
        self.gauge.sub(self.pad.len());
    }
}

impl Item for Pinned {
    fn heap(&self) -> usize {
        self.pad.capacity()
    }
}

/// Projects a [`Pinned`] of `pad` bytes each hour.
struct Pinning {
    pad: usize,
    gauge: &'static Gauge,
}

impl Operator for Pinning {
    type Output = Pinned;

    fn hour(&mut self, _: &Hour<'_>, out: &mut dyn Sink<Pinned>) -> io::Result<()> {
        self.gauge.add(self.pad);
        out.push(Pinned {
            pad: vec![0; self.pad],
            gauge: self.gauge,
        })
    }
}

/// Items that cannot spill stay resident past the output budget within its working
/// allowance, a segment at a time once the budget is spent, however many workers; past the
/// allowance they stop the run, the family named, every item dropped.
#[test]
fn items_that_cannot_spill_past_the_working_allowance_stop_the_run() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let pad = 64;
    let segment = UNIT_ORDINALS as usize * pad;
    for threads in [1, 3] {
        for working_bytes in [usize::MAX, segment / 2] {
            let gauge = Gauge::new();
            let mut emitted = 0;
            let lane = Lane::new(
                "pinned",
                |_: &Segment| -> io::Result<Pinning> { Ok(Pinning { pad, gauge }) },
                |chunk: Chunk<'_, Pinned>| {
                    emitted += chunk.items.len();
                    Ok(())
                },
            );
            let ran = run(
                &file,
                &Settings {
                    output_bytes: 0,
                    working_bytes,
                    ..settings(threads, 4)
                },
                &Memory::default(),
                &Cancellation::default(),
                vec![lane],
            );
            if working_bytes == usize::MAX {
                let stats = ran.unwrap();
                assert_eq!(emitted, 3 * UNIT_ORDINALS as usize);
                assert_eq!((stats.segments, stats.chunks_spilled), (1, 0));
                assert!(gauge.peak() <= segment);
            } else {
                let error = ran.unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
                assert!(error.to_string().contains("pinned"), "{error}");
                assert_eq!(emitted, 0);
            }
            assert_eq!(gauge.now(), 0);
        }
    }
}

/// Parked operators past the continuation budget that cannot be spilled stop the run, the
/// family named, every operator dropped.
#[test]
fn parked_operators_that_cannot_spill_past_the_budget_stop_the_run() {
    let file = opened(&alternating(3, 2));
    let probes = Probes {
        weight: 64,
        spills: false,
        ..Probes::new(branch_start)
    };
    let mut seen = Vec::new();
    let error = run(
        &file,
        &Settings {
            carry_bytes: 0,
            ..settings(1, 1)
        },
        &Memory::default(),
        &Cancellation::default(),
        vec![probing("unspilled", &probes, &mut seen)],
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert!(error.to_string().contains("unspilled"), "{error}");
    assert_eq!((probes.operators.now(), probes.items.now()), (0, 0));
}

/// Refuses restarts only before it has seen an hour: it decides otherwise than the operator
/// that walked the segment before.
struct Fickle {
    seen: bool,
}

impl Operator for Fickle {
    type Output = ();

    fn starts_at(&self, start: Start<'_>) -> bool {
        self.seen || !matches!(start, Start::Restart(_))
    }

    fn hour(&mut self, _: &Hour<'_>, _: &mut dyn Sink<()>) -> io::Result<()> {
        self.seen = true;
        Ok(())
    }
}

/// A family whose operators decide otherwise at a segment's start than at the end of the
/// segment before is refused, by name, rather than left waiting.
#[test]
fn a_family_deciding_otherwise_at_a_segment_s_start_is_refused() {
    let file = opened(&[(1, 0, 2 * UNIT_ORDINALS)]);
    let lane = Lane::new(
        "fickle",
        |_: &Segment| -> io::Result<Fickle> { Ok(Fickle { seen: false }) },
        |_: Chunk<'_, ()>| Ok(()),
    );
    let error = run(
        &file,
        &settings(1, 2),
        &Memory::default(),
        &Cancellation::default(),
        vec![lane],
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("fickle"), "{error}");
}

/// Fails, or panics, at `hour`.
struct Failing {
    hour: i32,
    panics: bool,
}

impl Operator for Failing {
    type Output = ();

    fn hour(&mut self, hour: &Hour<'_>, _: &mut dyn Sink<()>) -> io::Result<()> {
        if hour.hour == self.hour {
            assert!(!self.panics, "operator panicked");
            return Err(io::Error::other("operator failed"));
        }
        Ok(())
    }
}

fn failing<'a>(
    hour: i32,
    panics: bool,
    emit: impl FnMut(Chunk<'_, ()>) -> io::Result<()> + Send + 'a,
) -> Lane<'a> {
    Lane::new(
        "failing",
        move |_: &Segment| -> io::Result<Failing> { Ok(Failing { hour, panics }) },
        emit,
    )
}

/// A panic in an operator or in an emit stops the run instead of leaving it waiting, and
/// reaches the caller.
#[test]
fn a_panic_stops_the_run() {
    let file = opened(&[(1, 0, 4 * UNIT_ORDINALS)]);
    let walk = |lane: Lane<'_>, threads: usize| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(
                &file,
                &settings(threads, 1),
                &Memory::default(),
                &Cancellation::default(),
                vec![lane],
            )
        }))
    };
    assert!(walk(failing(800, true, |_| Ok(())), 2).is_err());
    assert!(walk(failing(-1, false, |_| panic!("emit panicked")), 4).is_err());
}

/// An operator's failure, or an emit's, stops the run and is what it returns.
#[test]
fn the_first_failure_stops_the_run() {
    let file = opened(&[(1, 0, 4 * UNIT_ORDINALS)]);
    let mut emitted = 0;
    let lane = failing(800, false, |chunk| {
        emitted += usize::from(chunk.last);
        Ok(())
    });
    let error = run(
        &file,
        &settings(2, 1),
        &Memory::default(),
        &Cancellation::default(),
        vec![lane],
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "operator failed");
    assert!(emitted <= 1);
    let lane = failing(-1, false, |_| Err(io::Error::other("emit failed")));
    let error = run(
        &file,
        &settings(2, 1),
        &Memory::default(),
        &Cancellation::default(),
        vec![lane],
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "emit failed");
}

/// Cancelling a run whose other worker waits for the window to move stops it: it returns
/// as interrupted, every operator and item dropped.
#[test]
fn cancelling_a_run_waiting_for_its_window_stops_it() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let cancellation = Cancellation::default();
    let probes = Probes::new(anywhere);
    let lane = Lane::new(
        "probe",
        |segment: &Segment| probes.make(segment),
        |_: Chunk<'_, Seen>| {
            cancellation.wait_for_idle(1);
            cancellation.cancel();
            Ok(())
        },
    );
    let error = run(
        &file,
        &settings(2, 1),
        &Memory::default(),
        &cancellation,
        vec![lane],
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!((probes.operators.now(), probes.items.now()), (0, 0));
    assert_eq!(probes.started(), [0]);
}

/// A run handed a cancelled cancellation starts nothing.
#[test]
fn a_cancelled_run_starts_nothing() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let probes = Probes::new(anywhere);
    let mut seen = Vec::new();
    let error = run(
        &file,
        &settings(2, 2),
        &Memory::default(),
        &cancellation,
        vec![probing("probe", &probes, &mut seen)],
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(probes.started().is_empty());
    assert!(seen.is_empty());
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

#[derive(Clone, Copy)]
enum Stop {
    Fail,
    Panic,
    Cancel,
}

/// In run 1, at its walk's first hour, holding the walkers' only share: says so, then waits
/// until the run is stopped.
struct Holding<'t> {
    run: u32,
    held: &'t Flag,
    stopped: &'t Flag,
}

impl Operator for Holding<'_> {
    type Output = ();

    fn hour(&mut self, hour: &Hour<'_>, _: &mut dyn Sink<()>) -> io::Result<()> {
        if hour.starts && self.run == 1 {
            self.held.set();
            self.stopped.wait();
        }
        Ok(())
    }
}

/// Four runs on three workers under a walkers' budget admitting one walk at a time: while
/// run 1's walk holds it, and the third worker waits for work, run 0's emit, on the other,
/// stops the run. What the run returned.
fn stopped_while_a_walk_holds_the_budget(stop: Stop) -> std::thread::Result<io::Result<Stats>> {
    let file = opened(&[(1, 0, 10), (2, 100, 10), (3, 200, 10), (4, 300, 10)]);
    let (held, stopped, cancellation) = (Flag::default(), Flag::default(), Cancellation::default());
    let lane = Lane::new(
        "holding",
        |segment: &Segment| {
            Ok(Holding {
                run: segment.run,
                held: &held,
                stopped: &stopped,
            })
        },
        |chunk: Chunk<'_, ()>| {
            if chunk.segment.run != 0 {
                return Ok(());
            }
            // Waits for other tasks, as no emit may: run 1's walk, and the third worker.
            held.wait();
            cancellation.wait_for_idle(1);
            stopped.set();
            match stop {
                Stop::Cancel => {
                    cancellation.cancel();
                    Ok(())
                }
                Stop::Fail | Stop::Panic => {
                    assert!(!matches!(stop, Stop::Panic), "emit panicked");
                    Err(io::Error::other("emit failed"))
                }
            }
        },
    );
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(
            &file,
            &Settings {
                walker_bytes: 1,
                ..settings(3, 3)
            },
            &Memory::default(),
            &cancellation,
            vec![lane],
        )
    }))
}

/// A failure while a walk holds the walkers' only share and a worker waits for work stops
/// the run: the walk ends, the waiting worker leaves, and the failure is returned.
#[test]
fn a_failure_while_a_walk_holds_the_walkers_budget_stops_the_run() {
    let error = stopped_while_a_walk_holds_the_budget(Stop::Fail)
        .unwrap()
        .unwrap_err();
    assert_eq!(error.to_string(), "emit failed");
}

/// A panic while a walk holds the walkers' only share stops the run as a failure does, and
/// reaches the caller.
#[test]
fn a_panic_while_a_walk_holds_the_walkers_budget_stops_the_run() {
    assert!(stopped_while_a_walk_holds_the_budget(Stop::Panic).is_err());
}

/// Cancelling while a walk holds the walkers' only share wakes the worker waiting for work:
/// the run returns as interrupted.
#[test]
fn cancelling_while_a_walk_holds_the_walkers_budget_stops_the_run() {
    let error = stopped_while_a_walk_holds_the_budget(Stop::Cancel)
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

/// A folder reads back what it spilled, and removes each spill once dropped.
#[test]
fn a_folder_reads_back_its_spills_and_removes_them() {
    let path = std::env::temp_dir().join(format!("segments-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let folder = Folder::new(path.clone()).unwrap();
    let (carried, empty) = (
        folder.spill(b"carried").unwrap(),
        folder.spill(b"").unwrap(),
    );
    let mut back = Vec::new();
    carried.read(&mut back).unwrap();
    assert_eq!(back, b"carried");
    empty.read(&mut back).unwrap();
    assert!(back.is_empty());
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 2);
    drop((carried, empty));
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    std::fs::remove_dir(&path).unwrap();
}
