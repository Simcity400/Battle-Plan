use super::*;
use crate::columnar_frames::{
    Limits, Reads,
    fixtures::{churned, every, recording},
};
use replay_columnar::envelope::{Extent, FOOTER_BYTES, UNIT_ORDINALS};
use std::{collections::HashMap, path::PathBuf, sync::atomic::AtomicUsize};

/// Bytes alive at once, as the values counting themselves into it hold them.
struct Gauge {
    id: usize,
    now: AtomicUsize,
    peak: AtomicUsize,
}

/// Every gauge, by id, for items read back from the spool to count themselves in again.
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

/// An hour a probe saw, holding bytes counted alive in its gauge: written to the spool, its
/// pad by its length.
struct Seen {
    saw: Saw,
    pad: Vec<u8>,
    gauge: &'static Gauge,
}

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

impl Spooled for Seen {
    fn spool(&self, out: &mut replay_spool::Writer<'_>) -> io::Result<()> {
        self.gauge.id.spool(out)?;
        self.saw.spool(out)?;
        self.pad.len().spool(out)
    }

    fn unspool(input: &mut replay_spool::Reader<'_>) -> io::Result<Self> {
        let gauge = Gauge::of(usize::unspool(input)?);
        let saw = Saw::unspool(input)?;
        Ok(Seen::new(saw, usize::unspool(input)?, gauge))
    }
}

/// Where a probe fails: as its walk starts, at an hour, as its walk ends, or in its emit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum When {
    Start,
    Hour,
    End,
    Emit,
}

/// How a probe fails.
#[derive(Clone, Copy, PartialEq, Eq)]
enum How {
    Refuse,
    Fail,
    Panic,
    /// What it works with fails it, as a store refusing a write: the run's own failure.
    Fatal,
}

impl How {
    fn trip(self) -> io::Result<()> {
        match self {
            How::Refuse => Err(io::Error::new(io::ErrorKind::InvalidData, "probe refused")),
            How::Fail => Err(io::Error::other("probe failed")),
            How::Panic => panic!("probe panicked"),
            How::Fatal => Err(fatal(io::Error::new(
                io::ErrorKind::InvalidData,
                "store refused",
            ))),
        }
    }
}

/// A probe's failure: at `hour`, or as the walk of the segment it starts or ends starts or
/// ends, or as its emit takes the segment it starts.
#[derive(Clone, Copy)]
struct Trip {
    when: When,
    how: How,
    hour: i32,
}

/// A family of probes: each starts where `starts` lets it, sees each hour as a [`Seen`] of
/// `pad` bytes, and carries the hours it saw, and `weight` bytes counted in `operators`,
/// into the segments it cannot start; `spills` when it can write them; fails as `trip` says.
struct Probes {
    starts: fn(Start<'_>) -> bool,
    pad: usize,
    weight: usize,
    spills: bool,
    trip: Option<Trip>,
    items: &'static Gauge,
    operators: &'static Gauge,
    /// The first hour of each walk it started, and of each run it crossed into.
    started: Mutex<Vec<i32>>,
    crossed: Mutex<Vec<i32>>,
    /// Each segment that reached its emit short, and how.
    ends: Mutex<Vec<(Segment, &'static str)>>,
    /// What each segment its operator went on past handed its emit, by run: None when not
    /// saved.
    carries: Mutex<Vec<(u32, Option<Vec<u8>>)>>,
}

impl Probes {
    fn new(starts: fn(Start<'_>) -> bool) -> Self {
        Self {
            starts,
            pad: 8,
            weight: 0,
            spills: true,
            trip: None,
            items: Gauge::new(),
            operators: Gauge::new(),
            started: Mutex::default(),
            crossed: Mutex::default(),
            ends: Mutex::default(),
            carries: Mutex::default(),
        }
    }

    fn make(&self, _: &Segment) -> io::Result<Probe<'_>> {
        self.operators.add(self.weight);
        Ok(Probe {
            family: self,
            walked: 0,
            last: None,
            weight: vec![0; self.weight],
        })
    }

    /// Fails as `when` says, when its trip does.
    fn trips(&self, when: When, hour: i32) -> io::Result<()> {
        match self.trip {
            Some(trip) if trip.when == when && trip.hour == hour => trip.how.trip(),
            _ => Ok(()),
        }
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
    /// The last hour of its walk it saw.
    last: Option<i32>,
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
        self.family.trips(When::Start, first)?;
        lock(&self.family.started).push(first);
        Ok(())
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        lock(&self.family.crossed).push(first);
        Ok(())
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut dyn Sink<Seen>) -> io::Result<()> {
        self.family.trips(When::Hour, hour.hour)?;
        let saw = (
            hour.branch,
            hour.hour,
            hour.run,
            hour.ordinal,
            hour.starts,
            self.walked,
        );
        self.walked += 1;
        self.last = Some(hour.hour);
        out.push(Seen::new(saw, self.family.pad, self.family.items))
    }

    fn end(&mut self, _: &mut dyn Sink<Seen>) -> io::Result<()> {
        match self.last.take() {
            Some(last) => self.family.trips(When::End, last),
            None => Ok(()),
        }
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

/// A lane of `probes` whose emit keeps what each hour saw, and each segment ending short,
/// checking each segment's chunks come together and end with its last; it fails as its trip
/// says at the segment whose first hour it names.
fn probing<'a>(name: &'a str, probes: &'a Probes, seen: &'a mut Vec<Saw>) -> Lane<'a> {
    let mut open: Option<Segment> = None;
    Lane::new(
        name,
        move |segment: &Segment| probes.make(segment),
        move |chunk: Chunk<'_, Seen>| {
            if let Some(open) = open {
                assert_eq!(open, *chunk.segment);
            }
            if open.is_none()
                && let Some(first) = chunk.items.first()
            {
                probes.trips(When::Emit, first.saw.1)?;
            }
            open = (!chunk.last).then_some(*chunk.segment);
            seen.extend(chunk.items.iter().map(|item| item.saw));
            assert!(chunk.last || chunk.carry.is_none());
            if let Some(carry) = chunk.carry {
                let saved = match carry {
                    Carry::Saved(bytes) => Some(bytes.to_vec()),
                    Carry::Unsaved => None,
                };
                lock(&probes.carries).push((chunk.segment.run, saved));
            }
            if let Some(ending) = chunk.ending {
                let how = match ending {
                    Ending::Refused { .. } => "refused",
                    Ending::Lost(_) => "lost",
                };
                lock(&probes.ends).push((*chunk.segment, how));
            }
            Ok(())
        },
    )
}

/// The stats of a run every family went through whole, uncancelled.
fn whole(ran: Ran) -> Stats {
    assert!(!ran.cancelled);
    for outcome in &ran.outcomes {
        assert!(
            outcome.ended.is_empty() && outcome.failure.is_none(),
            "{outcome:?}"
        );
    }
    ran.stats
}

/// The index of the segment holding `saw`'s hour.
fn segment_of(file: &Recording, saw: &Saw) -> usize {
    file.segments()
        .iter()
        .position(|segment| segment.run == saw.2 && (segment.first..=segment.last).contains(&saw.3))
        .unwrap()
}

/// The first and last hours of segment `at`.
fn hours_of(file: &Recording, at: usize) -> (i32, i32) {
    let segment = file.segments()[at];
    let first = file.run(segment.run).unwrap().first_hour;
    (first + segment.first as i32, first + segment.last as i32)
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

fn settings(threads: usize) -> Settings {
    Settings::new(threads)
}

/// A spool folder of the run's own: tests run at once, and a run empties its folder.
fn spool() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let next = NEXT.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("segments-spool-{}-{next}", std::process::id()))
}

fn opened(runs: &[(u64, i32, u32)]) -> Recording {
    let (bytes, _) = recording(runs, every(runs), 1, |_| {});
    loaded(bytes)
}

fn loaded(bytes: Vec<u8>) -> Recording {
    Recording::open(bytes, Limits::default()).unwrap()
}

/// Runs of `branches` branches in turn, `rounds` times, each of two segments.
fn alternating(branches: u64, rounds: u64) -> Vec<(u64, i32, u32)> {
    (0..rounds * branches)
        .map(|at| (at % branches + 1, at as i32 * 10_000, UNIT_ORDINALS + 1))
        .collect()
}

/// Every segment reaches its family in order, each with every hour of its own once, its
/// first starting it, while no walk holds what it projected: only the items an emit is
/// handed at once and each worker's item being written are alive. Of two runs, and of six
/// runs of twenty segments, three runs of one branch.
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
        for threads in [1, 3, 8] {
            let probes = Probes::new(anywhere);
            let mut seen = Vec::new();
            let stats = whole(
                run(
                    &file,
                    &settings(threads),
                    &spool(),
                    &Cancellation::default(),
                    vec![probing("probe", &probes, &mut seen)],
                )
                .unwrap(),
            );
            assert_eq!(seen, saw);
            assert_eq!(
                probes.started(),
                starts_and_crossings(&saw).0,
                "each segment starts once"
            );
            assert_eq!(stats.walks, file.segments().len());
            assert_eq!((stats.deferred, stats.parked), (0, 0));
            assert_eq!(probes.items.now(), 0);
            let segment = file
                .segments()
                .iter()
                .map(|s| (s.last - s.first + 1) as usize)
                .max();
            assert!(probes.items.peak() <= (segment.unwrap() + threads) * probes.pad);
            assert!(stats.spooled > 0 && stats.spool_bytes > 0);
        }
    }
    let file = opened(&many);
    assert_eq!((file.run_count(), file.segments().len()), (6, 20));
}

/// A family passing a branch is handed none of its segments, alone or beside one walking
/// every segment, whether it starts anywhere or goes on through its branch; segments no family
/// needs are never walked.
#[test]
fn a_family_passing_a_branch_walks_and_emits_none_of_its_segments() {
    let file = opened(&alternating(2, 3));
    let own = file
        .segments()
        .iter()
        .filter(|segment| file.run(segment.run).unwrap().branch == 2)
        .count();
    for (starts, cold) in [
        (anywhere as fn(Start<'_>) -> bool, true),
        (branch_start, false),
    ] {
        let saw = expected(&file, starts);
        let second: Vec<Saw> = saw.iter().copied().filter(|saw| saw.0 == 2).collect();
        for threads in [1, 4] {
            let alone = Probes::new(starts);
            let mut seen = Vec::new();
            let stats = whole(
                run(
                    &file,
                    &settings(threads),
                    &spool(),
                    &Cancellation::default(),
                    vec![probing("alone", &alone, &mut seen).passes(&[1])],
                )
                .unwrap(),
            );
            assert_eq!(seen, second);
            if cold {
                assert_eq!(stats.walks, own);
            }
            let (beside, every) = (Probes::new(starts), Probes::new(starts));
            let (mut passing, mut walking) = (Vec::new(), Vec::new());
            whole(
                run(
                    &file,
                    &settings(threads),
                    &spool(),
                    &Cancellation::default(),
                    vec![
                        probing("beside", &beside, &mut passing).passes(&[1]),
                        probing("every", &every, &mut walking),
                    ],
                )
                .unwrap(),
            );
            assert_eq!(passing, second);
            assert_eq!(walking, saw);
        }
    }
}

/// What a probe carries having seen `walked` hours.
fn carry(walked: u64) -> Vec<u8> {
    [walked.to_le_bytes(), 0_u64.to_le_bytes()].concat()
}

/// A family going on in a branch from where an earlier run left it is handed none of the hours
/// before, nor walked any segment before: one starting anywhere starts again where it goes on,
/// past its segment's first hour as at a restart there, and one going on through its branch
/// takes up its carry there and sees the rest as one walk of the whole recording does. The
/// last segment of each branch hands its emit what an operator that cannot start past it
/// carries, saved, and every other it goes on past says it carries what is not; one that
/// starts anywhere carries nothing.
#[test]
fn a_family_goes_on_from_its_carry_where_an_earlier_run_left_it() {
    let units = UNIT_ORDINALS;
    let file = opened(&[
        (1, 0, 3 * units + 9),
        (2, 50_000, 2 * units),
        (1, 90_000, units + 5),
    ]);
    // Branch 1 goes on past its third segment's first hour, branch 2 at its second segment.
    let joins = [(1, 0, 2 * units + 100), (2, 1, units)];
    let after = |saw: &Saw| {
        joins
            .iter()
            .any(|&(branch, run, ordinal)| saw.0 == branch && (saw.2, saw.3) >= (run, ordinal))
    };
    let walks = file
        .segments()
        .iter()
        .filter(|segment| {
            let branch = file.run(segment.run).unwrap().branch;
            joins.iter().any(|&(of, run, ordinal)| {
                of == branch && (segment.run, segment.last) >= (run, ordinal)
            })
        })
        .count();
    assert_eq!(walks, 5);
    for (starts, carried) in [
        (anywhere as fn(Start<'_>) -> bool, false),
        (branch_start, true),
    ] {
        let mut saw: Vec<Saw> = expected(&file, starts).into_iter().filter(after).collect();
        if !carried {
            // Started anew where it goes on, it has seen nothing of its segment before.
            let first = saw.iter().position(|saw| saw.0 == 1).unwrap();
            for (at, saw) in saw[first..].iter_mut().enumerate() {
                if saw.3 >= 3 * units {
                    break;
                }
                (saw.4, saw.5) = (at == 0, at as u64);
            }
        }
        let carries = [carry(u64::from(2 * units + 100)), carry(u64::from(units))];
        let resumes = [
            Resume {
                branch: 1,
                run: 0,
                ordinal: 2 * units + 100,
                carry: carried.then_some(&carries[0][..]),
            },
            Resume {
                branch: 2,
                run: 1,
                ordinal: units,
                carry: carried.then_some(&carries[1][..]),
            },
        ];
        for threads in [1, 4] {
            let probes = Probes::new(starts);
            let mut seen = Vec::new();
            let stats = whole(
                run(
                    &file,
                    &settings(threads),
                    &spool(),
                    &Cancellation::default(),
                    vec![probing("going on", &probes, &mut seen).resumes(&resumes)],
                )
                .unwrap(),
            );
            assert_eq!(seen, saw);
            if !carried {
                assert_eq!(stats.walks, walks);
            }
            let mut kept = lock(&probes.carries).clone();
            kept.sort_unstable();
            let ends = if carried {
                vec![
                    (0, None),
                    (0, None),
                    (1, Some(carry(u64::from(2 * units)))),
                    (2, None),
                    (2, Some(carry(u64::from(4 * units + 14)))),
                ]
            } else {
                Vec::new()
            };
            assert_eq!(kept, ends);
        }
    }
    // One that cannot start where it goes on, and carries nothing there, fails there.
    let probes = Probes::new(branch_start);
    let mut seen = Vec::new();
    let resumes = [Resume {
        branch: 2,
        run: 1,
        ordinal: units,
        carry: None,
    }];
    let ran = run(
        &file,
        &settings(2),
        &spool(),
        &Cancellation::default(),
        vec![probing("going on", &probes, &mut seen).resumes(&resumes)],
    )
    .unwrap();
    let failure = ran.outcomes[0].failure.as_ref().unwrap();
    assert_eq!(failure.operation, Operation::Decide);
    // Resumes out of order are refused before any walk.
    let backwards = [
        resumes[0],
        Resume {
            branch: 1,
            ..resumes[0]
        },
    ];
    let error = run(
        &file,
        &settings(1),
        &spool(),
        &Cancellation::default(),
        vec![probing("going on", &probes, &mut Vec::new()).resumes(&backwards)],
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

/// The dictionary size of the state each segment starts from.
struct Started(Option<usize>);

#[derive(replay_spool::Spooled)]
struct Size(Option<usize>);

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
    whole(
        run(
            &file,
            &settings(2),
            &spool(),
            &Cancellation::default(),
            vec![lane],
        )
        .unwrap(),
    );
    let ordinals = UNIT_ORDINALS as usize;
    assert_eq!(sizes, [Some(0), Some(ordinals), Some(2 * ordinals)]);
}

/// Walks of a recording read from its file size their buffers for their segment's reads, its
/// stored bytes too: checking the digest chain or not, every hour reaches its family in order.
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
        for threads in [1, 3] {
            let probes = Probes::new(anywhere);
            let mut seen = Vec::new();
            whole(
                run(
                    &file,
                    &settings(threads),
                    &spool(),
                    &Cancellation::default(),
                    vec![probing("probe", &probes, &mut seen)],
                )
                .unwrap(),
            );
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
    for threads in [1, 4] {
        let probes = Probes::new(not_second);
        let mut seen = Vec::new();
        let stats = whole(
            run(
                &file,
                &settings(threads),
                &spool(),
                &Cancellation::default(),
                vec![probing("picky", &probes, &mut seen)],
            )
            .unwrap(),
        );
        assert_eq!(seen, saw);
        assert_eq!(
            (probes.started(), probes.crossed()),
            starts_and_crossings(&saw)
        );
        // Each operator it parked was written out and read back.
        assert_eq!((stats.spilled, stats.loaded), (stats.parked, stats.parked));
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
        whole(
            run(
                &file,
                &settings(threads),
                &spool(),
                &Cancellation::default(),
                vec![probing("continuing", &probes, &mut seen)],
            )
            .unwrap(),
        );
        assert_eq!(seen, saw);
        assert_eq!(
            (probes.started(), probes.crossed()),
            starts_and_crossings(&saw)
        );
    }
}

/// Over branches in turn, a family going on through every segment of its branch and one
/// starting everywhere: on one worker and on two, each hour reaches each once, in order; each
/// branch's operator starts once and crosses once into each of its later runs; and every
/// operator parked, never a walker's tables, is written out and read back, going on as it was.
#[test]
fn alternating_branches_go_on_parked_on_disk() {
    let weight = 64;
    for branches in [2, 5] {
        let file = opened(&alternating(branches, 4));
        let (cold, going) = (expected(&file, anywhere), expected(&file, branch_start));
        let (started, crossed) = starts_and_crossings(&going);
        assert_eq!(
            (started.len(), crossed.len()),
            (branches as usize, 3 * branches as usize)
        );
        for threads in [1, 2] {
            let anywhere_probes = Probes::new(anywhere);
            let going_probes = Probes {
                weight,
                ..Probes::new(branch_start)
            };
            let (mut cold_seen, mut going_seen) = (Vec::new(), Vec::new());
            let stats = whole(
                run(
                    &file,
                    &settings(threads),
                    &spool(),
                    &Cancellation::default(),
                    vec![
                        probing("cold", &anywhere_probes, &mut cold_seen),
                        probing("going", &going_probes, &mut going_seen),
                    ],
                )
                .unwrap(),
            );
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
            assert_eq!((stats.spilled, stats.loaded), (stats.parked, stats.parked));
            if stats.parked > 0 {
                // Its walked hours and its weight's length, written and read back.
                assert!(stats.carry_scratch >= 16);
            }
            // An operator per worker at most, and those being parked.
            assert!(going_probes.operators.peak() <= (threads + branches as usize) * weight);
            assert_eq!(going_probes.operators.now(), 0);
            assert_eq!(going_probes.items.now(), 0);
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
    for threads in [1, 4] {
        let (anywhere_probes, picky_probes) = (Probes::new(anywhere), Probes::new(not_second));
        let (mut cold_seen, mut picky_seen) = (Vec::new(), Vec::new());
        let stats = whole(
            run(
                &file,
                &settings(threads),
                &spool(),
                &Cancellation::default(),
                vec![
                    probing("cold", &anywhere_probes, &mut cold_seen),
                    probing("picky", &picky_probes, &mut picky_seen),
                ],
            )
            .unwrap(),
        );
        assert_eq!(cold_seen, cold);
        assert_eq!(picky_seen, picky);
        assert_eq!(anywhere_probes.started(), starts_and_crossings(&cold).0);
        assert_eq!(picky_probes.started(), starts_and_crossings(&picky).0);
        let segments = file.segments().len();
        assert!(stats.walks >= segments);
        assert!(stats.walks <= segments + stats.deferred);
    }
}

/// A family whose first emit is slow holds that family alone, not the walks nor the other
/// family's emits: segments go on being walked, and emitted by the other family, while the
/// first waits.
#[test]
fn a_slow_emit_holds_its_family_not_the_walks_or_the_other_families() {
    let file = opened(&[(1, 0, 6 * UNIT_ORDINALS)]);
    let waits = 3;
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
                while *count < waits {
                    count = emitted.1.wait(count).unwrap();
                }
                assert!(slow.started().len() >= waits);
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
    let stats = whole(
        run(
            &file,
            &settings(2),
            &spool(),
            &Cancellation::default(),
            vec![slow_lane, fast_lane],
        )
        .unwrap(),
    );
    assert!(stats.segments >= waits);
    assert_eq!((slow_seen, fast_seen), (saw.clone(), saw));
}

/// Goes on through its branch, parked between its runs and spilled at once: in branch 1, as
/// its parked operator is written out, says so, waits until `concluded`, then cancels the run.
struct Carrying<'t> {
    branch: u64,
    writing: &'t Flag,
    concluded: &'t Flag,
    cancellation: &'t Cancellation,
}

impl Operator for Carrying<'_> {
    type Output = ();

    fn starts_at(&self, start: Start<'_>) -> bool {
        branch_start(start)
    }

    fn start(&mut self, _: Start<'_>, branch: u64, _: i32) -> io::Result<()> {
        self.branch = branch;
        Ok(())
    }

    fn hour(&mut self, _: &Hour<'_>, _: &mut dyn Sink<()>) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Option<usize> {
        Some(0)
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        if self.branch == 1 {
            self.writing.set();
            self.concluded.wait();
            self.cancellation.cancel();
        }
        out.extend_from_slice(&self.branch.to_le_bytes());
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.branch = u64::from_le_bytes(read(bytes, &mut 0));
        Ok(())
    }
}

/// Starts anywhere; when it `waits`, its walk waits as it ends until `writing`, and says it
/// concluded once dropped.
struct Concluding<'t> {
    waits: bool,
    writing: &'t Flag,
    concluded: &'t Flag,
}

impl Drop for Concluding<'_> {
    fn drop(&mut self) {
        if self.waits {
            self.concluded.set();
        }
    }
}

impl Operator for Concluding<'_> {
    type Output = ();

    fn hour(&mut self, _: &Hour<'_>, _: &mut dyn Sink<()>) -> io::Result<()> {
        Ok(())
    }

    fn end(&mut self, _: &mut dyn Sink<()>) -> io::Result<()> {
        if self.waits {
            self.writing.wait();
        }
        Ok(())
    }
}

/// A family's code runs with no lock of the run held. One worker writes out an operator
/// parked between branch 1's runs, whose save waits until a walk of branch 2 has concluded
/// and then cancels the run; meanwhile that walk, on the other worker, concludes, a family it
/// started anew checked against what was parked for it, and drops its operator. The run
/// returns cancelled.
#[test]
fn family_code_runs_with_no_lock_of_the_run_held() {
    let file = opened(&alternating(2, 2));
    let segments = file.segments();
    let (writing, concluded) = (Flag::default(), Flag::default());
    let cancellation = Cancellation::default();
    let carrying = Lane::new(
        "carrying",
        |_: &Segment| {
            Ok(Carrying {
                branch: 0,
                writing: &writing,
                concluded: &concluded,
                cancellation: &cancellation,
            })
        },
        |_: Chunk<'_, ()>| Ok(()),
    );
    // Branch 2's second segment: its walk starts it anew, after the segment before.
    let concluding = Lane::new(
        "concluding",
        |segment: &Segment| {
            Ok(Concluding {
                waits: *segment == segments[3],
                writing: &writing,
                concluded: &concluded,
            })
        },
        |_: Chunk<'_, ()>| Ok(()),
    );
    let ran = run(
        &file,
        &settings(2),
        &spool(),
        &cancellation,
        vec![carrying, concluding],
    )
    .unwrap();
    assert!(ran.cancelled);
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
/// segment before fails there, by name, rather than waiting: what it emitted before stands.
#[test]
fn a_family_deciding_otherwise_at_a_segment_s_start_fails_there() {
    let file = opened(&[(1, 0, 2 * UNIT_ORDINALS)]);
    let mut emitted = Vec::new();
    let lane = Lane::new(
        "fickle",
        |_: &Segment| -> io::Result<Fickle> { Ok(Fickle { seen: false }) },
        |chunk: Chunk<'_, ()>| {
            emitted.push(*chunk.segment);
            Ok(())
        },
    );
    let ran = run(
        &file,
        &settings(1),
        &spool(),
        &Cancellation::default(),
        vec![lane],
    )
    .unwrap();
    let failure = ran.outcomes[0].failure.as_ref().unwrap();
    assert_eq!((failure.at, failure.operation), (1, Operation::Decide));
    assert!(!failure.panicked);
    assert!(failure.error.to_string().contains("fickle"), "{failure:?}");
    assert_eq!(emitted, [file.segments()[0]]);
    assert_eq!(ran.outcomes[0].settled, 2);
}

/// A family refusing its branch's data at an hour, as the row path's families refuse it,
/// keeps every item it projected of the hours before, the refusing segment's too, which
/// reach its emit with the refusal; nothing of the branch after reaches it, its later run
/// included, whether it starts every segment or goes on through its branch. Its other branch,
/// and another family, go on whole.
#[test]
fn a_family_refusing_an_hour_keeps_its_hours_before_and_ends_its_branch() {
    let file = opened(&[
        (1, 0, 3 * UNIT_ORDINALS),
        (2, 50_000, 2 * UNIT_ORDINALS),
        (1, 90_000, UNIT_ORDINALS),
    ]);
    let refused = UNIT_ORDINALS as i32 + 100;
    let starts: [fn(Start<'_>) -> bool; 2] = [anywhere, branch_start];
    for starts in starts {
        let kept: Vec<Saw> = expected(&file, starts)
            .into_iter()
            .filter(|saw| saw.0 != 1 || saw.1 < refused)
            .collect();
        for threads in [1, 4] {
            let refusing = Probes {
                trip: Some(Trip {
                    when: When::Hour,
                    how: How::Refuse,
                    hour: refused,
                }),
                ..Probes::new(starts)
            };
            let other = Probes::new(anywhere);
            let (mut refusing_seen, mut other_seen) = (Vec::new(), Vec::new());
            let ran = run(
                &file,
                &settings(threads),
                &spool(),
                &Cancellation::default(),
                vec![
                    probing("refusing", &refusing, &mut refusing_seen),
                    probing("other", &other, &mut other_seen),
                ],
            )
            .unwrap();
            assert_eq!(refusing_seen, kept);
            assert_eq!(other_seen, expected(&file, anywhere));
            let outcome = &ran.outcomes[0];
            assert!(outcome.failure.is_none() && !ran.cancelled);
            assert_eq!(outcome.settled, file.segments().len());
            match &outcome.ended[..] {
                [
                    Ended {
                        at: 1,
                        branch: 1,
                        ending: Ending::Refused { hour, error },
                    },
                ] => {
                    assert_eq!(*hour, refused);
                    assert_eq!(error.to_string(), "probe refused");
                }
                ended => panic!("{ended:?}"),
            }
            assert_eq!(*lock(&refusing.ends), [(file.segments()[1], "refused")]);
            assert!(ran.outcomes[1].ended.is_empty() && ran.outcomes[1].failure.is_none());
            assert_eq!((refusing.items.now(), refusing.operators.now()), (0, 0));
        }
    }
}

/// An emit's refusal naming its hour ends its branch there; the emit goes on with its other
/// branch.
#[test]
fn an_emit_s_refusal_ends_its_branch_at_the_hour_it_names() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS), (2, 50_000, 2 * UNIT_ORDINALS)]);
    let refused = UNIT_ORDINALS as i32 + 7;
    let saw = expected(&file, anywhere);
    for threads in [1, 3] {
        let probes = Probes::new(anywhere);
        let mut seen = Vec::new();
        let lane = Lane::new(
            "refusing",
            |segment: &Segment| probes.make(segment),
            |chunk: Chunk<'_, Seen>| {
                for item in &chunk.items {
                    if item.saw.1 == refused {
                        return Err(refused_at(refused));
                    }
                    seen.push(item.saw);
                }
                Ok(())
            },
        );
        let ran = run(
            &file,
            &settings(threads),
            &spool(),
            &Cancellation::default(),
            vec![lane],
        )
        .unwrap();
        let kept: Vec<Saw> = saw
            .iter()
            .copied()
            .filter(|saw| saw.0 != 1 || saw.1 < refused)
            .collect();
        assert_eq!(seen, kept);
        match &ran.outcomes[0].ended[..] {
            [
                Ended {
                    at: 1,
                    branch: 1,
                    ending: Ending::Refused { hour, error },
                },
            ] => {
                assert_eq!(*hour, refused);
                assert_eq!(error.to_string(), "emit refused");
            }
            ended => panic!("{ended:?}"),
        }
        assert!(ran.outcomes[0].failure.is_none());
        assert_eq!(probes.items.now(), 0);
    }
}

/// An emit's refusal of its branch's data from `hour` on.
fn refused_at(hour: i32) -> io::Error {
    super::refused(
        hour,
        io::Error::new(io::ErrorKind::InvalidData, "emit refused"),
    )
}

/// A family whose operator panics or fails as its walk starts, at an hour, as its walk ends,
/// or whose emit panics or fails, fails alone from that segment on: every segment before it
/// reached its emit whole, of both branches, and none after it does. Another family going on
/// through its branch, walking the same segments, goes on whole, its walk begun again past
/// the panic from where it was; its parked operators and spooled items, and the failed
/// family's, are all given back, the spool folder with them.
#[test]
fn a_family_failing_fails_alone_from_its_segment() {
    let file = opened(&alternating(2, 3));
    let failing = 4;
    let (first, last) = hours_of(&file, failing);
    let going = expected(&file, branch_start);
    let trips = [
        (When::Start, first, Operation::Start),
        (When::Hour, first + 5, Operation::Hour),
        (When::End, last, Operation::End),
        (When::Emit, first, Operation::Emit),
    ];
    for (when, hour, operation) in trips {
        for how in [How::Panic, How::Fail] {
            for threads in [1, 2, 4] {
                for starts in [anywhere as fn(Start<'_>) -> bool, branch_start] {
                    // An operator going on through its branch does not start there.
                    if when == When::Start && !starts(Start::Run { continues: true }) {
                        continue;
                    }
                    let faulty = Probes {
                        trip: Some(Trip { when, how, hour }),
                        weight: 32,
                        ..Probes::new(starts)
                    };
                    let steady = Probes {
                        weight: 32,
                        ..Probes::new(branch_start)
                    };
                    let folder = spool();
                    let (mut faulty_seen, mut steady_seen) = (Vec::new(), Vec::new());
                    let ran = run(
                        &file,
                        &settings(threads),
                        &folder,
                        &Cancellation::default(),
                        vec![
                            probing("faulty", &faulty, &mut faulty_seen),
                            probing("steady", &steady, &mut steady_seen),
                        ],
                    )
                    .unwrap();
                    let before: Vec<Saw> = expected(&file, starts)
                        .into_iter()
                        .filter(|saw| segment_of(&file, saw) < failing)
                        .collect();
                    assert_eq!(faulty_seen, before);
                    assert_eq!(steady_seen, going);
                    assert_eq!(steady.started(), starts_and_crossings(&going).0);
                    let outcome = &ran.outcomes[0];
                    let failure = outcome.failure.as_ref().unwrap();
                    assert_eq!(
                        (failure.at, failure.operation, failure.panicked),
                        (failing, operation, how == How::Panic)
                    );
                    let said = if how == How::Panic {
                        "probe panicked"
                    } else {
                        "probe failed"
                    };
                    assert_eq!(failure.error.to_string(), said);
                    assert!(outcome.ended.is_empty());
                    assert_eq!(outcome.settled, file.segments().len());
                    assert!(ran.outcomes[1].ended.is_empty() && ran.outcomes[1].failure.is_none());
                    if (when, how, threads) == (When::Hour, How::Panic, 1) {
                        assert_eq!(ran.stats.rewalked, 1);
                    }
                    for probes in [&faulty, &steady] {
                        assert_eq!((probes.items.now(), probes.operators.now()), (0, 0));
                    }
                    assert!(!folder.exists());
                }
            }
        }
    }
}

/// Refuses its branch at hour `refused`, once the operator failing at hour `failing` was
/// dropped, its walk ended, when it `waits` for it. It fails by panicking or, given `cancels`,
/// with an error whose drop cancels that run.
struct Racing<'t> {
    refused: i32,
    failing: i32,
    waits: bool,
    failed: &'t Flag,
    cancels: Option<Arc<Cancellation>>,
    gave_up: bool,
}

impl Drop for Racing<'_> {
    fn drop(&mut self) {
        if self.gave_up {
            self.failed.set();
        }
    }
}

impl Operator for Racing<'_> {
    type Output = Seen;

    fn hour(&mut self, hour: &Hour<'_>, _: &mut dyn Sink<Seen>) -> io::Result<()> {
        if hour.hour == self.refused {
            if self.waits {
                self.failed.wait();
            }
            return Err(io::Error::new(io::ErrorKind::InvalidData, "racing refused"));
        }
        if hour.hour == self.failing {
            self.gave_up = true;
            if let Some(cancellation) = &self.cancels {
                return Err(io::Error::other(Cancelling(Arc::clone(cancellation))));
            }
            panic!("racing panicked");
        }
        Ok(())
    }
}

/// An error whose drop cancels its run: a family's destructor calling back into the run.
struct Cancelling(Arc<Cancellation>);

impl fmt::Debug for Cancelling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cancelling")
    }
}

impl fmt::Display for Cancelling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("racing failed")
    }
}

impl std::error::Error for Cancelling {}

impl Drop for Cancelling {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// A family whose walk of a later segment of a branch fails before its walk of an earlier one
/// refuses that branch is not failed: in segment order the branch ended first, and the failing
/// segment is never walked. Its other branch goes on whole, as when nothing runs ahead.
#[test]
fn a_failure_overtaken_by_an_earlier_end_of_its_branch_is_not_the_family_s() {
    let file = opened(&alternating(2, 3));
    let segments = file.segments();
    let (refused, failing) = (hours_of(&file, 0).0 + 1, hours_of(&file, 4).0 + 1);
    // Branch 2's segments.
    let other = [2, 3, 6, 7, 10, 11].map(|at| segments[at]);
    for (threads, waits) in [(1, false), (2, true)] {
        let failed = Flag::default();
        let mut emitted = Vec::new();
        let lane = Lane::new(
            "racing",
            |_: &Segment| {
                Ok(Racing {
                    refused,
                    failing,
                    waits,
                    failed: &failed,
                    cancels: None,
                    gave_up: false,
                })
            },
            |chunk: Chunk<'_, Seen>| {
                if chunk.last {
                    emitted.push(*chunk.segment);
                }
                Ok(())
            },
        );
        let ran = run(
            &file,
            &settings(threads),
            &spool(),
            &Cancellation::default(),
            vec![lane],
        )
        .unwrap();
        let outcome = &ran.outcomes[0];
        assert!(outcome.failure.is_none(), "{outcome:?}");
        match &outcome.ended[..] {
            [
                Ended {
                    at: 0,
                    branch: 1,
                    ending: Ending::Refused { hour, .. },
                },
            ] => assert_eq!(*hour, refused),
            ended => panic!("{ended:?}"),
        }
        assert_eq!(outcome.settled, segments.len());
        assert_eq!(emitted[0], segments[0]);
        assert_eq!(emitted[1..], other);
    }
}

/// The error a family's walk failed with, overtaken by an earlier end of its branch, drops
/// with no lock of the run held, though its drop cancels the run: the run returns cancelled,
/// and the failure is none of the family's.
#[test]
fn an_overtaken_failure_s_error_drops_with_no_lock_of_the_run_held() {
    let file = opened(&alternating(2, 3));
    let segments = file.segments();
    let (refused, failing) = (hours_of(&file, 0).0 + 1, hours_of(&file, 4).0 + 1);
    let failed = Flag::default();
    let cancellation = Arc::new(Cancellation::default());
    let lane = Lane::new(
        "racing",
        |_: &Segment| {
            Ok(Racing {
                refused,
                failing,
                waits: true,
                failed: &failed,
                cancels: Some(Arc::clone(&cancellation)),
                gave_up: false,
            })
        },
        |_: Chunk<'_, Seen>| Ok(()),
    );
    // Segment 4's walk fails first, its error pending; segment 0's then refuses branch 1.
    let ran = run(&file, &settings(2), &spool(), &cancellation, vec![lane]).unwrap();
    assert!(ran.cancelled);
    assert!(ran.outcomes[0].failure.is_none(), "{:?}", ran.outcomes[0]);
}

/// Never written out: its parked operator's save fails.
struct Unwritten;

impl Operator for Unwritten {
    type Output = ();

    fn starts_at(&self, start: Start<'_>) -> bool {
        branch_start(start)
    }

    fn hour(&mut self, _: &Hour<'_>, _: &mut dyn Sink<()>) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Option<usize> {
        Some(8)
    }

    fn save(&self, _: &mut Vec<u8>) -> io::Result<bool> {
        Err(io::Error::other("save failed"))
    }
}

/// An operator parked for a segment whose segment before has sealed, failing as it is written
/// out, fails its family at that segment before any walk may take it again: a walk asking
/// for it the moment it is let go finds it closed, and the family's failure is the save's, not
/// a walk's finding nothing handed on.
#[test]
fn a_failed_write_out_fails_its_family_before_its_segment_can_be_taken() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let segments = file.segments();
    let lanes = vec![Lane::new(
        "unwritten",
        |_: &Segment| Ok(Unwritten),
        |_: Chunk<'_, ()>| Ok(()),
    )];
    let signal = Signal {
        schedule: Mutex::new(Schedule::new(segments.len(), 1).unwrap()),
        board: Board::default(),
        stopped: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
    };
    let (root, cancellation) = (spool(), Cancellation::default());
    let folder = Folder::open(&root, &["unwritten"]).unwrap();
    let run = Run {
        recording: &file,
        segments,
        plan: Plan::new(&file, segments).unwrap(),
        lanes: &lanes,
        worlds: &worldless,
        signal: &signal,
        budget: Arc::new(Budget::new(usize::MAX)),
        folder: &folder,
        store: Store::new(),
        resumed: Mutex::new(Vec::new()),
        joins: std::collections::BTreeMap::new(),
        cancellation: &cancellation,
    };
    let claim = |schedule: &mut Schedule| {
        schedule
            .pick(&run.plan, true, &mut |_| true)
            .unwrap()
            .map(|task| match task {
                Task::Walk { at, .. } => ("walk", at),
                Task::Emit { at, .. } => ("emit", at),
                Task::Measure { at } => ("measure", at),
            })
    };
    // Segment 0 is walked and sealed; its operator, going on into segment 1, parks for it.
    {
        let mut schedule = signal.lock();
        schedule.measured(0, 0, Some(Reads::default()), &[(0, Reads::default())]);
        assert_eq!(claim(&mut *schedule), Some(("walk", 0)));
        schedule.done();
        schedule.sealed(0, &[0]);
    }
    let operator = lanes[0]
        .erased
        .fresh(0, &segments[1], &folder, None)
        .unwrap();
    let bytes = operator.carried().unwrap();
    // Parked, it is written out at once, and its save fails.
    run.park(1, 0, operator, bytes).unwrap();
    let mut schedule = signal.lock();
    assert_eq!(schedule.status(1, 0), Status::Closed);
    assert_eq!(claim(&mut *schedule), Some(("emit", 0)));
    schedule.done();
    let mut gone = Gone::default();
    schedule
        .emitted(&run.plan, 0, 0, 1, None, &mut gone)
        .unwrap();
    let failure = schedule.outcomes().remove(0).failure.unwrap();
    assert_eq!(
        (failure.at, failure.operation, failure.panicked),
        (1, Operation::Save, false)
    );
    assert_eq!(failure.error.to_string(), "save failed");
    drop(schedule);
    run.drop_gone(gone);
}

/// `table`'s unit on page `at` of run `run`.
fn unit_of(file: &Recording, run: u32, at: usize, table: TableId) -> Extent {
    *file
        .page(run, at)
        .unwrap()
        .units
        .iter()
        .find(|extent| extent.stream as usize == table.index() + 1)
        .unwrap()
}

/// A table's unit that cannot be read loses its segment to the families reading that table,
/// and to one naming no tables, which reads them all: one resumes at the next segment, the
/// other's branch ends. A family reading another table walks the segment whole. The segment
/// is measured, its share taken and its walk handed out as any other.
#[test]
fn a_damaged_table_unit_loses_its_segment_to_that_table_s_readers_alone() {
    let (bytes, _) = churned(&[(1, 0, 4)], 1);
    let clean = loaded(bytes.clone());
    let damaged = unit_of(&clean, 0, 1, TableId::WorldTension);
    let mut bytes = bytes;
    bytes[damaged.offset as usize] ^= 1;
    let file = loaded(bytes);
    assert!(file.damage().is_empty());
    assert_eq!(file.segments().len(), 4);
    let saw = expected(&clean, anywhere);
    let lost = |at: usize| {
        saw.iter()
            .copied()
            .filter(|saw| segment_of(&clean, saw) != at)
            .collect::<Vec<Saw>>()
    };
    let ended = |saw: &[Saw]| {
        saw.iter()
            .copied()
            .filter(|saw| segment_of(&clean, saw) < 1)
            .collect::<Vec<Saw>>()
    };
    for threads in [1, 3] {
        let (countries, tension, unnamed) = (
            Probes::new(anywhere),
            Probes::new(anywhere),
            Probes::new(anywhere),
        );
        let (mut countries_seen, mut tension_seen, mut unnamed_seen) =
            (Vec::new(), Vec::new(), Vec::new());
        let ran = run(
            &file,
            &settings(threads),
            &spool(),
            &Cancellation::default(),
            vec![
                probing("countries", &countries, &mut countries_seen).reads(&[TableId::Countries]),
                probing("tension", &tension, &mut tension_seen)
                    .reads(&[TableId::WorldTension])
                    .recovers(Recovery::Resumes),
                probing("unnamed", &unnamed, &mut unnamed_seen),
            ],
        )
        .unwrap();
        assert_eq!(countries_seen, saw);
        assert_eq!(tension_seen, lost(1));
        assert_eq!(unnamed_seen, ended(&saw));
        assert!(ran.outcomes[0].ended.is_empty() && ran.outcomes[0].failure.is_none());
        for outcome in &ran.outcomes[1..] {
            match &outcome.ended[..] {
                [
                    Ended {
                        at: 1,
                        branch: 1,
                        ending: Ending::Lost(Loss::Damaged(error)),
                    },
                ] => assert_eq!(
                    Damage::of(error).unwrap().scope,
                    DamageScope::Extent(damaged)
                ),
                ended => panic!("{ended:?}"),
            }
            assert!(outcome.failure.is_none());
        }
        for probes in [&tension, &unnamed] {
            assert_eq!(*lock(&probes.ends), [(file.segments()[1], "lost")]);
        }
        assert!(lock(&countries.ends).is_empty());
    }
}

/// A family's panic in a walk once another family lost a table there to the table's own
/// damage ends the panicking family alone: the other loses the segment to that damage, as it
/// would have had nothing panicked, though no family is left to walk the segment again.
#[test]
fn a_panic_after_another_family_lost_a_table_fails_the_panicking_family_alone() {
    let (bytes, _) = churned(&[(1, 0, 4)], 1);
    let clean = loaded(bytes.clone());
    let damaged = unit_of(&clean, 0, 1, TableId::WorldTension);
    let mut bytes = bytes;
    bytes[damaged.offset as usize] ^= 1;
    let file = loaded(bytes);
    let saw = expected(&clean, anywhere);
    let only = |kept: &dyn Fn(usize) -> bool| {
        saw.iter()
            .copied()
            .filter(|saw| kept(segment_of(&clean, saw)))
            .collect::<Vec<Saw>>()
    };
    let (first, _) = hours_of(&clean, 1);
    for threads in [1, 3] {
        let tension = Probes::new(anywhere);
        let panicking = Probes {
            trip: Some(Trip {
                when: When::Hour,
                how: How::Panic,
                hour: first + 2,
            }),
            ..Probes::new(anywhere)
        };
        let (mut tension_seen, mut panicking_seen) = (Vec::new(), Vec::new());
        let ran = run(
            &file,
            &settings(threads),
            &spool(),
            &Cancellation::default(),
            vec![
                probing("tension", &tension, &mut tension_seen)
                    .reads(&[TableId::WorldTension])
                    .recovers(Recovery::Resumes),
                probing("panicking", &panicking, &mut panicking_seen).reads(&[TableId::Countries]),
            ],
        )
        .unwrap();
        assert_eq!(tension_seen, only(&|at| at != 1));
        assert_eq!(panicking_seen, only(&|at| at < 1));
        match &ran.outcomes[0].ended[..] {
            [
                Ended {
                    at: 1,
                    ending: Ending::Lost(Loss::Damaged(error)),
                    ..
                },
            ] => assert_eq!(
                Damage::of(error).unwrap().scope,
                DamageScope::Extent(damaged)
            ),
            ended => panic!("{ended:?}"),
        }
        assert!(ran.outcomes[0].failure.is_none());
        let failure = ran.outcomes[1].failure.as_ref().unwrap();
        assert_eq!(
            (failure.at, failure.operation, failure.panicked),
            (1, Operation::Hour, true)
        );
        assert_eq!(ran.stats.rewalked, 1);
        for probes in [&tension, &panicking] {
            assert_eq!((probes.items.now(), probes.operators.now()), (0, 0));
        }
    }
}

/// A segment whose manifests cannot be read is lost to every family walking it: a family
/// resuming starts again at the next segment it can start, one whose branch ends there sees
/// nothing more of it, and one going on through its branch loses each segment after that it
/// cannot start. Another branch goes on whole.
#[test]
fn a_damaged_manifest_unit_loses_its_segment_to_every_family() {
    let runs = [(1, 0, 3 * UNIT_ORDINALS), (2, 50_000, 2 * UNIT_ORDINALS)];
    let (bytes, _) = recording(&runs, every(&runs), 1, |page| {
        if page.run == 0 && page.first == UNIT_ORDINALS {
            page.units[0].digest[0] ^= 1;
        }
    });
    let file = loaded(bytes);
    let (cold, going) = (expected(&file, anywhere), expected(&file, branch_start));
    let without = |saw: &[Saw], lost: &dyn Fn(usize) -> bool| {
        saw.iter()
            .copied()
            .filter(|saw| !lost(segment_of(&file, saw)))
            .collect::<Vec<Saw>>()
    };
    for threads in [1, 4] {
        let (resuming, ending, continuing) = (
            Probes::new(anywhere),
            Probes::new(anywhere),
            Probes::new(branch_start),
        );
        let (mut resuming_seen, mut ending_seen, mut continuing_seen) =
            (Vec::new(), Vec::new(), Vec::new());
        let ran = run(
            &file,
            &settings(threads),
            &spool(),
            &Cancellation::default(),
            vec![
                probing("resuming", &resuming, &mut resuming_seen).recovers(Recovery::Resumes),
                probing("ending", &ending, &mut ending_seen),
                probing("continuing", &continuing, &mut continuing_seen)
                    .recovers(Recovery::Resumes),
            ],
        )
        .unwrap();
        assert_eq!(resuming_seen, without(&cold, &|at| at == 1));
        assert_eq!(ending_seen, without(&cold, &|at| matches!(at, 1 | 2)));
        assert_eq!(continuing_seen, without(&going, &|at| matches!(at, 1 | 2)));
        let manifests = |ended: &Ended| match &ended.ending {
            Ending::Lost(Loss::Damaged(error)) => matches!(
                Damage::of(error).unwrap().scope,
                DamageScope::Extent(extent) if (extent.stream, extent.first) == (0, UNIT_ORDINALS)
            ),
            _ => false,
        };
        for outcome in &ran.outcomes {
            assert!(outcome.failure.is_none());
            assert_eq!((outcome.ended[0].at, outcome.ended[0].branch), (1, 1));
            assert!(manifests(&outcome.ended[0]));
        }
        assert_eq!(ran.outcomes[0].ended.len(), 1);
        assert_eq!(ran.outcomes[1].ended.len(), 1);
        match &ran.outcomes[2].ended[1..] {
            [
                Ended {
                    at: 2,
                    branch: 1,
                    ending: Ending::Lost(Loss::After(1)),
                },
            ] => {}
            ended => panic!("{ended:?}"),
        }
        let segments = file.segments();
        assert_eq!(
            *lock(&continuing.ends),
            [(segments[1], "lost"), (segments[2], "lost")]
        );
    }
}

/// A restart that cannot be restored loses no hour: its segment's walk goes back to begin at
/// the segment before, under a share measured and taken for both before it is handed out
/// again, and every family, starting there or going on through it, sees every hour as from
/// an intact recording.
#[test]
fn a_damaged_restart_loses_no_hours() {
    let runs = [(1, 0, 3 * UNIT_ORDINALS)];
    let countries = TableId::Countries.index() as u32;
    let (bytes, _) = recording(&runs, every(&runs), 1, |page| {
        if page.first == UNIT_ORDINALS
            && let Some(restart) = &mut page.restart
        {
            for table in restart
                .tables
                .iter_mut()
                .filter(|table| table.table == countries)
            {
                table.extent.digest[0] ^= 1;
            }
        }
    });
    let file = loaded(bytes);
    let clean = loaded(recording(&runs, every(&runs), 1, |_| {}).0);
    let (cold, going) = (expected(&clean, anywhere), expected(&clean, branch_start));
    for threads in [1, 2, 4] {
        let (anywhere_probes, going_probes) = (Probes::new(anywhere), Probes::new(branch_start));
        let (mut cold_seen, mut going_seen) = (Vec::new(), Vec::new());
        let stats = whole(
            run(
                &file,
                &settings(threads),
                &spool(),
                &Cancellation::default(),
                vec![
                    probing("cold", &anywhere_probes, &mut cold_seen),
                    probing("going", &going_probes, &mut going_seen),
                ],
            )
            .unwrap(),
        );
        assert_eq!((cold_seen, going_seen), (cold.clone(), going.clone()));
        // Its start is made once at most, where its segment is not walked on from the one
        // before.
        assert!(stats.derived <= 1);
    }
}

/// One run with restarts after pages `1..=count` damaged, plus its intact copy.
fn damaged_restarts(segments: u32, count: u32) -> (Recording, Recording) {
    let runs = [(1, 0, segments * UNIT_ORDINALS)];
    let countries = TableId::Countries.index() as u32;
    let (bytes, _) = recording(&runs, every(&runs), 1, |page| {
        if (1..=count).contains(&(page.first / UNIT_ORDINALS))
            && let Some(restart) = &mut page.restart
        {
            for table in restart
                .tables
                .iter_mut()
                .filter(|table| table.table == countries)
            {
                table.extent.digest[0] ^= 1;
            }
        }
    });
    let clean = recording(&runs, every(&runs), 1, |_| {}).0;
    (loaded(bytes), loaded(clean))
}

/// A family's panic in a walk gone on from the one before past its segment's damaged restart
/// costs the other families nothing: the walk goes back to begin before that restart, under
/// a share measured and taken for it, and each family goes on from where it was, every hour
/// seen once, as from an intact recording, as when the walk began cold.
#[test]
fn a_panic_in_a_walk_gone_on_past_a_damaged_restart_costs_the_others_nothing() {
    let (file, clean) = damaged_restarts(3, 1);
    let (cold, going) = (expected(&clean, anywhere), expected(&clean, branch_start));
    let before: Vec<Saw> = cold
        .iter()
        .copied()
        .filter(|saw| segment_of(&clean, saw) < 1)
        .collect();
    let (first, _) = hours_of(&clean, 1);
    for threads in [1, 2] {
        let (cold_probes, going_probes) = (Probes::new(anywhere), Probes::new(branch_start));
        let panicking = Probes {
            trip: Some(Trip {
                when: When::Hour,
                how: How::Panic,
                hour: first + 5,
            }),
            ..Probes::new(anywhere)
        };
        let (mut cold_seen, mut panicking_seen, mut going_seen) =
            (Vec::new(), Vec::new(), Vec::new());
        let ran = run(
            &file,
            &settings(threads),
            &spool(),
            &Cancellation::default(),
            vec![
                probing("cold", &cold_probes, &mut cold_seen),
                probing("panicking", &panicking, &mut panicking_seen),
                probing("going", &going_probes, &mut going_seen),
            ],
        )
        .unwrap();
        assert_eq!((&cold_seen, &going_seen), (&cold, &going));
        assert_eq!(panicking_seen, before);
        assert_eq!(going_probes.started(), starts_and_crossings(&going).0);
        let failure = ran.outcomes[1].failure.as_ref().unwrap();
        assert_eq!(
            (failure.at, failure.operation, failure.panicked),
            (1, Operation::Hour, true)
        );
        for at in [0, 2] {
            let outcome = &ran.outcomes[at];
            assert!(outcome.ended.is_empty() && outcome.failure.is_none());
        }
        if threads == 1 {
            // The walk of the segment went on from the one before, past its restart.
            assert!(ran.stats.chained >= 1 && ran.stats.derived >= 1);
        }
        for probes in [&cold_probes, &panicking, &going_probes] {
            assert_eq!((probes.items.now(), probes.operators.now()), (0, 0));
        }
    }
}

/// Projects one item of `pad` bytes at hour `at`, and panics at hour `panics`.
struct Lump {
    at: i32,
    pad: usize,
    panics: Option<i32>,
    gauge: &'static Gauge,
}

impl Operator for Lump {
    type Output = Seen;

    fn hour(&mut self, hour: &Hour<'_>, out: &mut dyn Sink<Seen>) -> io::Result<()> {
        if Some(hour.hour) == self.panics {
            panic!("lump panicked");
        }
        if hour.hour == self.at {
            let saw = (
                hour.branch,
                hour.hour,
                hour.run,
                hour.ordinal,
                hour.starts,
                0,
            );
            out.push(Seen::new(saw, self.pad, self.gauge))?;
        }
        Ok(())
    }
}

/// A walk gone on past a damaged restart, given back after a family's panic, holds nothing
/// that family projected: what it spooled of the segment is dropped with its fate, segment 0's
/// million bytes reach their emit through the spool, and the run goes on, the panic ending
/// its family alone.
#[test]
fn a_walk_given_back_after_a_panic_holds_nothing_of_the_failed_family() {
    let (file, _) = damaged_restarts(3, 1);
    let going = Probes::new(branch_start);
    let mut going_seen = Vec::new();
    let first = hours_of(&file, 1).0;
    let (large, small) = (Gauge::new(), Gauge::new());
    let lump =
        |name: &'static str, at: i32, pad: usize, panics: Option<i32>, gauge: &'static Gauge| {
            Lane::new(
                name,
                move |_: &Segment| {
                    Ok(Lump {
                        at,
                        pad,
                        panics,
                        gauge,
                    })
                },
                |_: Chunk<'_, Seen>| Ok(()),
            )
        };
    let folder = spool();
    let ran = run(
        &file,
        &settings(1),
        &folder,
        &Cancellation::default(),
        vec![
            lump("spilling", 0, 1_000_000, None, large),
            lump("failing", first + 1, 90_000, Some(first + 2), small),
            // Goes on through its branch: the walk of segment 1 goes on from segment 0's.
            probing("going", &going, &mut going_seen),
        ],
    )
    .unwrap();
    assert!(!ran.cancelled);
    assert!(ran.outcomes[0].ended.is_empty() && ran.outcomes[0].failure.is_none());
    let failure = ran.outcomes[1].failure.as_ref().unwrap();
    assert_eq!(
        (failure.at, failure.operation, failure.panicked),
        (1, Operation::Hour, true)
    );
    // The walk of segment 1 went on from segment 0's, past its restart.
    assert!(ran.stats.chained >= 1 && ran.stats.derived >= 1);
    assert!(ran.stats.spooled > 1_000_000);
    assert_eq!((large.now(), small.now()), (0, 0));
    assert!(!folder.exists());
}

/// Damaged restarts reuse the first intact restart and measure every segment once.
#[test]
fn restarts_damaged_one_after_another_are_measured_once_each() {
    let (file, clean) = damaged_restarts(5, 4);
    let segments = file.segments().len();
    assert_eq!(segments, 5);
    let probes = Probes::new(anywhere);
    let mut seen = Vec::new();
    let stats = whole(
        run(
            &file,
            &settings(1),
            &spool(),
            &Cancellation::default(),
            vec![probing("probe", &probes, &mut seen)],
        )
        .unwrap(),
    );
    assert_eq!(seen, expected(&clean, anywhere));
    assert_eq!(stats.measured, segments);
    assert_eq!(stats.chained, 0);
    // Page 0's restart is intact: segments 2, 3 and 4 derive from segment 1.
    assert_eq!(stats.derived, (1..segments - 1).sum::<usize>());
}

/// A family holding its branch's history (each hour counts the hours its operator saw since
/// it started) is not healed by healthy segments after a loss: when its branch ends there,
/// nothing more of it reaches its emit; when it resumes but its operator can start only at
/// its branch's start, every later segment of the branch, its later run's included, reaches
/// its emit lost, and no count goes on across the gap.
#[test]
fn a_history_is_not_healed_by_healthy_segments_after_a_loss() {
    let runs = [
        (1, 0, 4 * UNIT_ORDINALS),
        (2, 50_000, UNIT_ORDINALS),
        (1, 90_000, UNIT_ORDINALS),
    ];
    let (bytes, _) = recording(&runs, every(&runs), 1, |page| {
        if page.run == 0 && page.first == UNIT_ORDINALS {
            page.units[0].digest[0] ^= 1;
        }
    });
    let file = loaded(bytes);
    let going = expected(&file, branch_start);
    let kept: Vec<Saw> = going
        .iter()
        .copied()
        .filter(|saw| saw.0 != 1 || saw.1 < UNIT_ORDINALS as i32)
        .collect();
    let segments = file.segments();
    // Branch 1's segments after the lost one: run 0's last two, and run 2's.
    let later = [2, 3, 5];
    for recovery in [Recovery::Ends, Recovery::Resumes] {
        for threads in [1, 3] {
            let probes = Probes::new(branch_start);
            let mut seen = Vec::new();
            let ran = run(
                &file,
                &settings(threads),
                &spool(),
                &Cancellation::default(),
                vec![probing("history", &probes, &mut seen).recovers(recovery)],
            )
            .unwrap();
            assert_eq!(seen, kept);
            let outcome = &ran.outcomes[0];
            assert!(outcome.failure.is_none());
            assert_eq!(outcome.settled, segments.len());
            assert!(matches!(
                outcome.ended[0],
                Ended {
                    at: 1,
                    branch: 1,
                    ending: Ending::Lost(Loss::Damaged(_)),
                }
            ));
            let after: Vec<(usize, usize)> = outcome.ended[1..]
                .iter()
                .map(|ended| match ended.ending {
                    Ending::Lost(Loss::After(before)) => (ended.at, before),
                    _ => panic!("{ended:?}"),
                })
                .collect();
            let mut ends = vec![(segments[1], "lost")];
            if recovery == Recovery::Resumes {
                assert_eq!(after, [(2, 1), (3, 2), (5, 3)]);
                ends.extend(later.iter().map(|&at| (segments[at], "lost")));
            } else {
                assert!(after.is_empty());
            }
            assert_eq!(*lock(&probes.ends), ends);
            assert_eq!((probes.items.now(), probes.operators.now()), (0, 0));
        }
    }
}

/// Cancelling a run leaves every segment its families had not emitted pending: what reached
/// each emit is whole segments in order, exactly what an uncancelled run emits first, and
/// none is reported lost, refused or failed.
#[test]
fn cancelling_leaves_what_was_not_emitted_pending() {
    let file = opened(&[(1, 0, 4 * UNIT_ORDINALS), (2, 9_000, 2 * UNIT_ORDINALS)]);
    let segments = file.segments();
    let saw = expected(&file, anywhere);
    let cancellation = Cancellation::default();
    let probes = Probes::new(anywhere);
    let mut seen = Vec::new();
    let lane = Lane::new(
        "probe",
        |segment: &Segment| probes.make(segment),
        |chunk: Chunk<'_, Seen>| {
            seen.extend(chunk.items.iter().map(|item| item.saw));
            if chunk.last && *chunk.segment == segments[2] {
                cancellation.cancel();
            }
            Ok(())
        },
    );
    let ran = run(&file, &settings(1), &spool(), &cancellation, vec![lane]).unwrap();
    assert!(ran.cancelled);
    let outcome = &ran.outcomes[0];
    assert_eq!(outcome.settled, 3);
    assert!(outcome.ended.is_empty() && outcome.failure.is_none());
    let emitted: Vec<Saw> = saw
        .iter()
        .copied()
        .filter(|saw| segment_of(&file, saw) < 3)
        .collect();
    assert_eq!(seen, emitted);
    assert_eq!((probes.items.now(), probes.operators.now()), (0, 0));
}

/// Damage that leaves hours where no branch places them refuses the run before it starts,
/// naming it, and a recording whose root cannot be read does not open: neither reports
/// anything prepared.
#[test]
fn damage_no_branch_places_never_reports_a_run_complete() {
    let runs = [(1, 0, UNIT_ORDINALS), (2, 9_000, UNIT_ORDINALS)];
    let (mut bytes, pages) = recording(&runs, every(&runs), 1, |_| {});
    // Run 0's directory follows its last page.
    let page = pages[0].last().unwrap();
    bytes[(page.offset + page.stored) as usize] ^= 1;
    let file = loaded(bytes);
    assert_eq!(file.damage().len(), 1);
    assert_eq!(file.segments().len(), 1);
    let probes = Probes::new(anywhere);
    let mut seen = Vec::new();
    let refused = run(
        &file,
        &settings(2),
        &spool(),
        &Cancellation::default(),
        vec![probing("probe", &probes, &mut seen)],
    )
    .unwrap_err();
    assert_eq!(Damage::of(&refused).unwrap().scope, DamageScope::Run(0));
    assert!(seen.is_empty() && probes.started().is_empty());
    let (mut bytes, _) = recording(&runs, every(&runs), 1, |_| {});
    let root = bytes.len() - FOOTER_BYTES - 1;
    bytes[root] ^= 1;
    match Recording::open(bytes, Limits::default()) {
        Ok(_) => panic!("a recording whose root cannot be read opened"),
        Err(refused) => assert_eq!(Damage::of(&refused).unwrap().scope, DamageScope::Unlocated),
    }
}

/// A world whose advance panics at hour `at`: the run's own code.
struct Panicking {
    at: i32,
}

impl World for Panicking {
    fn hour(&mut self, _: &State, manifest: &Manifest<'_>) -> io::Result<()> {
        assert!(manifest.hour != self.at, "world panicked");
        Ok(())
    }
}

/// Reads nothing of its world.
struct Blind;

impl Operator<Panicking> for Blind {
    type Output = ();

    fn hour(&mut self, _: &Hour<'_, Panicking>, _: &mut dyn Sink<()>) -> io::Result<()> {
        Ok(())
    }
}

/// A panic in the run's own code is no family's: it stops the run and reaches the caller.
#[test]
fn a_panic_of_the_run_s_own_stops_it() {
    let file = opened(&[(1, 0, 2 * UNIT_ORDINALS)]);
    let at = hours_of(&file, 1).0 + 3;
    let worlds = |_: &Segment, _: &[usize]| -> io::Result<Panicking> { Ok(Panicking { at }) };
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_in(
            &file,
            &settings(2),
            &spool(),
            &Cancellation::default(),
            &worlds,
            vec![Lane::new(
                "blind",
                |_: &Segment| Ok(Blind),
                |_: Chunk<'_, ()>| Ok(()),
            )],
        )
    }));
    assert!(ran.is_err());
}

/// An item that cannot be written: its codec panics.
struct Brittle;

impl Spooled for Brittle {
    fn spool(&self, _: &mut replay_spool::Writer<'_>) -> io::Result<()> {
        panic!("brittle codec panicked")
    }

    fn unspool(_: &mut replay_spool::Reader<'_>) -> io::Result<Self> {
        Ok(Brittle)
    }
}

/// Pushes a [`Brittle`] each hour.
struct Breaking;

impl Operator for Breaking {
    type Output = Brittle;

    fn hour(&mut self, _: &Hour<'_>, out: &mut dyn Sink<Brittle>) -> io::Result<()> {
        out.push(Brittle)
    }
}

/// An item whose codec panics as it is spooled, at an hour's push, fails its family as a
/// save that panicked, not as the hour's.
#[test]
fn an_item_s_codec_panicking_as_it_is_spooled_fails_its_family_as_a_save() {
    let file = opened(&[(1, 0, 2 * UNIT_ORDINALS)]);
    let lane = Lane::new(
        "breaking",
        |_: &Segment| Ok(Breaking),
        |_: Chunk<'_, Brittle>| Ok(()),
    );
    let ran = run(
        &file,
        &settings(1),
        &spool(),
        &Cancellation::default(),
        vec![lane],
    )
    .unwrap();
    let failure = ran.outcomes[0].failure.as_ref().unwrap();
    assert_eq!(
        (failure.at, failure.operation, failure.panicked),
        (0, Operation::Save, true)
    );
    assert_eq!(failure.error.to_string(), "brittle codec panicked");
}

/// An item whose codec meets the run's own failure ([`fatal`]), as a store refusing a write,
/// counted in its gauge while alive.
struct Doomed(&'static Gauge);

impl Doomed {
    fn new(gauge: &'static Gauge) -> Self {
        gauge.add(1);
        Self(gauge)
    }
}

impl Drop for Doomed {
    fn drop(&mut self) {
        self.0.sub(1);
    }
}

impl Spooled for Doomed {
    fn spool(&self, _: &mut replay_spool::Writer<'_>) -> io::Result<()> {
        Err(fatal(io::Error::other("store full")))
    }

    fn unspool(_: &mut replay_spool::Reader<'_>) -> io::Result<Self> {
        unreachable!("never written")
    }
}

/// Pushes an hour's item, counted in its gauge, going on past whatever its push met.
struct Ignoring(&'static Gauge);

impl Operator for Ignoring {
    type Output = Doomed;

    fn hour(&mut self, _: &Hour<'_>, out: &mut dyn Sink<Doomed>) -> io::Result<()> {
        let _ = out.push(Doomed::new(self.0));
        Ok(())
    }
}

/// An operator going on past what its push met, the run's own failure as its item was
/// spooled, does not hide that failure, though its walk of the segment is lost to damage
/// before it ends: the failure is the run's own, and stops the run, which returns it.
#[test]
fn a_run_s_failure_a_push_met_stops_the_run_though_its_operator_goes_on() {
    // One segment of two pages, the second's manifests damaged.
    let runs = [(1, 0, 2 * UNIT_ORDINALS)];
    let (bytes, _) = recording(&runs, every(&runs), 2, |page| {
        if page.first == UNIT_ORDINALS {
            page.units[0].digest[0] ^= 1;
        }
    });
    let file = loaded(bytes);
    assert_eq!(file.segments().len(), 1);
    let gauge = Gauge::new();
    let lane = Lane::new(
        "ignoring",
        move |_: &Segment| Ok(Ignoring(gauge)),
        |_: Chunk<'_, Doomed>| Ok(()),
    );
    let error = run(
        &file,
        &settings(1),
        &spool(),
        &Cancellation::default(),
        vec![lane],
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "store full");
    assert_eq!(gauge.now(), 0);
}

/// An error a family's operator or emit says is the run's own ([`fatal`]), as the store it
/// writes to refusing a write, stops the run, which returns that error: whatever its kind, it
/// neither refuses the branch nor fails the family alone.
#[test]
fn an_error_a_family_s_code_says_is_the_run_s_stops_the_run() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let (first, last) = hours_of(&file, 1);
    let trips = [
        (When::Start, first),
        (When::Hour, first + 3),
        (When::End, last),
        (When::Emit, first),
    ];
    for (when, hour) in trips {
        for threads in [1, 3] {
            let probes = Probes {
                trip: Some(Trip {
                    when,
                    how: How::Fatal,
                    hour,
                }),
                ..Probes::new(anywhere)
            };
            let mut seen = Vec::new();
            let error = run(
                &file,
                &settings(threads),
                &spool(),
                &Cancellation::default(),
                vec![probing("storing", &probes, &mut seen)],
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), "store refused");
            assert!(error.get_ref().is_none_or(|inner| !inner.is::<Fatal>()));
        }
    }
}

/// An emit dating as its refusal the run's own error, as one its reference store met, refuses
/// nothing: the run stops, and returns that error.
#[test]
fn a_refusal_dating_the_run_s_own_error_stops_the_run() {
    let file = opened(&[(1, 0, 2 * UNIT_ORDINALS)]);
    let (first, _) = hours_of(&file, 1);
    let probes = Probes::new(anywhere);
    let lane = Lane::new(
        "refusing",
        |segment: &Segment| probes.make(segment),
        |chunk: Chunk<'_, Seen>| {
            if chunk.items.first().is_some_and(|item| item.saw.1 == first) {
                let error = io::Error::new(io::ErrorKind::InvalidData, "reference refused");
                return Err(refused(first, fatal(error)));
            }
            Ok(())
        },
    );
    let error = run(
        &file,
        &settings(1),
        &spool(),
        &Cancellation::default(),
        vec![lane],
    )
    .unwrap_err();
    assert_eq!(
        (error.kind(), error.to_string()),
        (io::ErrorKind::InvalidData, "reference refused".to_owned())
    );
}

/// Cancels its run as its walk of the segment whose last hour is `last` ends, and says it was
/// interrupted.
struct Interrupting<'c> {
    cancellation: &'c Cancellation,
    last: i32,
    saw: Option<i32>,
}

impl Operator for Interrupting<'_> {
    type Output = ();

    fn hour(&mut self, hour: &Hour<'_>, _: &mut dyn Sink<()>) -> io::Result<()> {
        self.saw = Some(hour.hour);
        Ok(())
    }

    fn end(&mut self, _: &mut dyn Sink<()>) -> io::Result<()> {
        if self.saw.take() != Some(self.last) {
            return Ok(());
        }
        self.cancellation.cancel();
        Err(io::Error::new(io::ErrorKind::Interrupted, "interrupted"))
    }
}

/// A fault a family meets as its run is cancelled, here as its walk ends, is none of its own:
/// the run returns cancelled, what was not emitted pending, and nothing failed.
#[test]
fn a_fault_met_as_the_run_is_cancelled_is_no_family_s() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let (_, last) = hours_of(&file, 1);
    let cancellation = Cancellation::default();
    let mut emitted = Vec::new();
    let lane = Lane::new(
        "interrupting",
        |_: &Segment| {
            Ok(Interrupting {
                cancellation: &cancellation,
                last,
                saw: None,
            })
        },
        |chunk: Chunk<'_, ()>| {
            emitted.push(*chunk.segment);
            Ok(())
        },
    );
    let ran = run(&file, &settings(1), &spool(), &cancellation, vec![lane]).unwrap();
    assert!(ran.cancelled);
    let outcome = &ran.outcomes[0];
    assert!(
        outcome.failure.is_none() && outcome.ended.is_empty(),
        "{outcome:?}"
    );
    assert_eq!(outcome.settled, 1);
    assert_eq!(emitted, [file.segments()[0]]);
}

/// A failure waiting behind a segment whose emit cancels the run, and then goes through, is
/// none of its family's: the emit settles its own segment, and nothing past it, the failure's
/// pending with the rest.
#[test]
fn a_failure_pending_as_an_emit_cancels_the_run_stays_pending() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let segments = file.segments();
    let (first, _) = hours_of(&file, 1);
    let probes = Probes {
        trip: Some(Trip {
            when: When::Hour,
            how: How::Panic,
            hour: first + 3,
        }),
        // Goes on through its branch: segment 0's walk goes on into segment 1.
        ..Probes::new(branch_start)
    };
    let cancellation = Cancellation::default();
    let lane = Lane::new(
        "probe",
        |segment: &Segment| probes.make(segment),
        |chunk: Chunk<'_, Seen>| {
            if chunk.last && *chunk.segment == segments[0] {
                cancellation.cancel();
            }
            Ok(())
        },
    );
    let ran = run(&file, &settings(1), &spool(), &cancellation, vec![lane]).unwrap();
    assert!(ran.cancelled);
    let outcome = &ran.outcomes[0];
    assert!(
        outcome.failure.is_none() && outcome.ended.is_empty(),
        "{outcome:?}"
    );
    assert_eq!(outcome.settled, 1);
    // Segment 1 failed as segment 0's walk went on into it, before segment 0 was emitted.
    assert_eq!((ran.stats.chained, ran.stats.rewalked), (1, 1));
}

/// A run handed a cancelled cancellation starts nothing, and leaves everything pending.
#[test]
fn a_cancelled_run_starts_nothing() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let probes = Probes::new(anywhere);
    let mut seen = Vec::new();
    let ran = run(
        &file,
        &settings(2),
        &spool(),
        &cancellation,
        vec![probing("probe", &probes, &mut seen)],
    )
    .unwrap();
    assert!(ran.cancelled);
    assert_eq!(ran.outcomes[0].settled, 0);
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
    /// The run's own failure: a lack of memory.
    Fail,
    /// The family's own panic, which ends it alone.
    Panic,
    Cancel,
}

/// In run 1, at its walk's first hour: says so, then waits until the run is stopped.
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

/// Four runs on three workers: while run 1's walk waits, and the third worker waits for work
/// once it walked the rest, run 0's emit, on the other, stops the run or fails its family.
/// What the run returned.
fn stopped_while_a_walk_waits(stop: Stop) -> std::thread::Result<io::Result<Ran>> {
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
                Stop::Fail => Err(io::Error::new(io::ErrorKind::OutOfMemory, "emit failed")),
                Stop::Panic => panic!("emit panicked"),
            }
        },
    );
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&file, &settings(3), &spool(), &cancellation, vec![lane])
    }))
}

/// The run's own failure while a walk waits and a worker waits for work stops the run: the
/// walk ends, the waiting worker leaves, and the failure is returned.
#[test]
fn a_failure_of_the_run_s_own_while_a_walk_waits_stops_the_run() {
    let error = stopped_while_a_walk_waits(Stop::Fail).unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert_eq!(error.to_string(), "emit failed");
}

/// An emit's panic while a walk waits ends its family alone from that segment on: the waiting
/// walk ends and gives back what it walked, the waiting worker leaves, and the run returns
/// the family failed.
#[test]
fn a_panic_while_a_walk_waits_fails_its_family() {
    let ran = stopped_while_a_walk_waits(Stop::Panic).unwrap().unwrap();
    assert!(!ran.cancelled);
    let failure = ran.outcomes[0].failure.as_ref().unwrap();
    assert_eq!(
        (failure.at, failure.operation, failure.panicked),
        (0, Operation::Emit, true)
    );
    assert_eq!(ran.outcomes[0].settled, 4);
}

/// Cancelling while a walk waits wakes the worker waiting for work: the run returns cancelled.
#[test]
fn cancelling_while_a_walk_waits_stops_the_run() {
    let ran = stopped_while_a_walk_waits(Stop::Cancel).unwrap().unwrap();
    assert!(ran.cancelled);
    assert!(ran.outcomes[0].ended.is_empty() && ran.outcomes[0].failure.is_none());
}

/// A world keeping the hours it was advanced to, with the segment and lanes it was built for.
struct Hours {
    segment: Segment,
    lanes: Vec<usize>,
    seen: Vec<i32>,
}

impl World for Hours {
    fn hour(&mut self, _: &State, manifest: &Manifest<'_>) -> io::Result<()> {
        self.seen.push(manifest.hour);
        Ok(())
    }
}

/// What an operator read of its walk's world at an hour.
#[derive(replay_spool::Spooled)]
struct Read {
    segment: Segment,
    lanes: Vec<usize>,
    hours: usize,
}

struct Reading;

impl Operator<Hours> for Reading {
    type Output = Read;

    fn hour(&mut self, hour: &Hour<'_, Hours>, out: &mut dyn Sink<Read>) -> io::Result<()> {
        assert_eq!(
            hour.world.seen.last(),
            Some(&hour.hour),
            "advanced before the lanes"
        );
        out.push(Read {
            segment: hour.world.segment,
            lanes: hour.world.lanes.clone(),
            hours: hour.world.seen.len(),
        })
    }
}

/// A lane of [`Reading`] whose emit keeps what each hour read of its world.
fn reading<'a>(name: &'a str, seen: &'a mut Vec<(Segment, Vec<usize>, usize)>) -> Lane<'a, Hours> {
    Lane::new(
        name,
        |_: &Segment| -> io::Result<Reading> { Ok(Reading) },
        move |chunk: Chunk<'_, Read>| {
            for read in chunk.items {
                assert_eq!(read.segment, *chunk.segment);
                seen.push((read.segment, read.lanes, read.hours));
            }
            Ok(())
        },
    )
}

/// Every walk builds one world for its segment and the lanes it carries, both lanes read it,
/// and it starts at the walk's first hour: advanced once an hour, before the lanes.
#[test]
fn a_walk_s_lanes_share_one_world_built_for_the_walk() {
    let file = opened(&[
        (1, 0, 3 * UNIT_ORDINALS + 5),
        (2, 50_000, 2 * UNIT_ORDINALS),
    ]);
    let built = AtomicUsize::new(0);
    let worlds = |segment: &Segment, lanes: &[usize]| -> io::Result<Hours> {
        built.fetch_add(1, Ordering::SeqCst);
        Ok(Hours {
            segment: *segment,
            lanes: lanes.to_vec(),
            seen: Vec::new(),
        })
    };
    let (mut first, mut second) = (Vec::new(), Vec::new());
    let stats = whole(
        run_in(
            &file,
            &settings(3),
            &spool(),
            &Cancellation::default(),
            &worlds,
            vec![reading("first", &mut first), reading("second", &mut second)],
        )
        .unwrap(),
    );
    assert_eq!(stats.walks, file.segments().len());
    assert_eq!(built.load(Ordering::SeqCst), file.segments().len());
    assert_eq!(first, second);
    let mut at = 0;
    for segment in file.segments() {
        for ordinal in segment.first..=segment.last {
            let (read, lanes, hours) = &first[at];
            assert_eq!((read, lanes.as_slice()), (segment, &[0, 1][..]));
            assert_eq!(*hours, (ordinal - segment.first) as usize + 1);
            at += 1;
        }
    }
    assert_eq!(at, first.len());
}

#[test]
fn a_lane_payload_policy_dates_emit_refusals_but_keeps_undated_family_failures() {
    let file = opened(&[(1, 0, 2 * UNIT_ORDINALS), (2, 50_000, UNIT_ORDINALS)]);
    for family in [false, true] {
        let probes = Probes::new(anywhere);
        let mut seen = Vec::new();
        let lane = Lane::new(
            "refusing",
            |segment: &Segment| probes.make(segment),
            |chunk: Chunk<'_, Seen>| {
                for item in chunk.items {
                    if item.saw.0 == 1 {
                        let error = io::Error::other("synthetic emit refusal");
                        return Err(if family { family_failure(error) } else { error });
                    }
                    seen.push(item.saw);
                }
                Ok(())
            },
        )
        .refuses(|_| true);
        let ran = run(
            &file,
            &settings(2),
            &spool(),
            &Cancellation::default(),
            vec![lane],
        )
        .unwrap();
        let outcome = &ran.outcomes[0];
        assert_eq!(outcome.failure.is_some(), family);
        if family {
            assert!(seen.is_empty());
            assert!(outcome.ended.is_empty());
        } else {
            assert_eq!(
                seen,
                expected(&file, anywhere)
                    .into_iter()
                    .filter(|saw| saw.0 == 2)
                    .collect::<Vec<_>>()
            );
            assert!(matches!(
                &outcome.ended[..],
                [Ended {
                    branch: 1,
                    ending: Ending::Refused { hour: 0, .. },
                    ..
                }]
            ));
        }
    }
}

/// A run empties its spool folder of what a run that stopped left, and removes it once done.
#[test]
fn a_run_empties_its_spool_as_it_starts_and_removes_it_as_it_ends() {
    let file = opened(&[(1, 0, 2 * UNIT_ORDINALS)]);
    let folder = spool();
    std::fs::create_dir_all(folder.join("0-probe")).unwrap();
    std::fs::write(
        folder.join("0-probe").join("0"),
        b"left by a run that stopped",
    )
    .unwrap();
    let probes = Probes::new(anywhere);
    let mut seen = Vec::new();
    whole(
        run(
            &file,
            &settings(2),
            &folder,
            &Cancellation::default(),
            vec![probing("probe", &probes, &mut seen)],
        )
        .unwrap(),
    );
    assert_eq!(seen, expected(&file, anywhere));
    assert!(!folder.exists());
}

/// What a sharing operator hands its emit of an hour: the allocation its walk holds throughout,
/// and one of the hour's own.
#[derive(replay_spool::Spooled)]
struct Shares {
    held: Arc<Vec<u32>>,
    own: Arc<Vec<u32>>,
}

/// Holds one allocation from its walk's start, handed on with it through its branch.
struct Holder {
    held: Arc<Vec<u32>>,
}

impl Operator for Holder {
    type Output = Shares;

    fn starts_at(&self, start: Start<'_>) -> bool {
        branch_start(start)
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut dyn Sink<Shares>) -> io::Result<()> {
        out.push(Shares {
            held: Arc::clone(&self.held),
            own: Arc::new(vec![hour.ordinal]),
        })
    }
}

/// Items sharing an allocation reach their emit sharing it, through the segments their
/// operator goes on into too, and an item's own reaches it its own: what compared by address
/// before the spool compares alike after it.
#[test]
fn items_sharing_an_allocation_reach_their_emit_sharing_it() {
    let file = opened(&[(1, 0, 3 * UNIT_ORDINALS)]);
    for threads in [1, 3] {
        let mut held: Option<Arc<Vec<u32>>> = None;
        let mut own: Vec<Arc<Vec<u32>>> = Vec::new();
        let lane = Lane::new(
            "sharing",
            |_: &Segment| {
                Ok(Holder {
                    held: Arc::new(vec![7]),
                })
            },
            |chunk: Chunk<'_, Shares>| {
                for item in chunk.items {
                    let first = held.get_or_insert_with(|| Arc::clone(&item.held));
                    assert!(Arc::ptr_eq(first, &item.held));
                    assert!(own.iter().all(|seen| !Arc::ptr_eq(seen, &item.own)));
                    assert_eq!(item.own.len(), 1);
                    own.push(item.own);
                }
                Ok(())
            },
        );
        whole(
            run(
                &file,
                &settings(threads),
                &spool(),
                &Cancellation::default(),
                vec![lane],
            )
            .unwrap(),
        );
        assert_eq!(own.len(), 3 * UNIT_ORDINALS as usize);
        assert_eq!(*held.unwrap(), [7]);
    }
}
