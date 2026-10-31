use super::{
    Commits, Ran, Walk, ahead_first,
    backend::{Bound, Member, Native, Projected, Taken},
    content::{Once, load_all},
    run_walk,
    shape::Shape,
    status, taken, walk_all,
};
use crate::prepared::{
    State as Phase,
    driver::{
        plan::{self, Backend, Plan},
        publication::{Family, Publisher, Resume, Stage, Stopped},
        registry::{self, Descriptor, FamilyId},
    },
    pipeline,
};
use dashboard_base::prepared::{
    gates,
    roots::{Binding, Published},
};
use dashboard_world::{Needs, World};
use replay::{
    columnar_frames::{Limits, Recording, Segment, fixtures, schema},
    segments::{self, Hour, Item, Manifest, Operator, Settings, Sink, Start, State},
};
use replay_data::{format::FormatVersion, rows::TableId, scope::CaptureMode};
use std::{
    collections::BTreeMap,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    path::PathBuf,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// Two branches, the first coming back after the second: every unit ends in a restart.
const RUNS: [(u64, i32, u32); 3] = [(1, 0, 6), (2, 50_000, 4), (1, 100_000, 3)];
/// The same runs, then a third branch.
const GROWN: [(u64, i32, u32); 4] = [(1, 0, 6), (2, 50_000, 4), (1, 100_000, 3), (3, 150_000, 2)];
const TABLES: &[TableId] = &[TableId::Countries, TableId::WorldTension];

static ALPHA: Descriptor = Descriptor {
    queries: &["alpha-product"],
    gates: &["alpha-product"],
    sections: &["alpha"],
    ..registry::family("alpha")
};
static BETA: Descriptor = Descriptor {
    queries: &["beta-product"],
    gates: &["beta-product"],
    sections: &["beta"],
    ..registry::family("beta")
};
static GAMMA: Descriptor = Descriptor {
    queries: &["gamma-product"],
    gates: &["gamma-product"],
    sections: &["gamma"],
    ..registry::family("gamma")
};

/// The registry's families and three more, alpha and beta with native backends.
fn test_plan() -> &'static Plan {
    plan::test_native(&[&ALPHA, &BETA, &GAMMA], &["alpha", "beta"]).unwrap()
}

/// A store of its own, bound to a synthetic recording file, and scratch beside it.
struct Fixture {
    root: PathBuf,
    store: store::Store,
    key: store::SourceKey,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("hoi4-native-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let recording = root.join("recording");
        std::fs::write(&recording, b"synthetic").unwrap();
        Self {
            key: store::SourceKey::capture(&recording, &[]).unwrap(),
            store: store::Store::local(uuid::Uuid::new_v4()).unwrap(),
            root,
        }
    }

    /// The publisher of `names` under `plan`, taking up what `recording` still holds.
    fn open<'s>(
        &'s self,
        plan: &'static Plan,
        names: &[&str],
        recording: &Recording,
        shape: &Shape,
    ) -> (Publisher<'s>, Resume) {
        Publisher::open(
            &self.store,
            self.key.clone(),
            Binding::new(b"synthetic recording", b"synthetic content"),
            plan,
            &Family::planned(plan, names).unwrap(),
            &|_, branch, frontier| shape.accepts(recording, branch, frontier),
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.clear();
        let _ = std::fs::remove_dir_all(self.store.directory());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn opened(runs: &[(u64, i32, u32)]) -> (Recording, Vec<u8>) {
    opened_every(runs, 1)
}

/// [`opened`], a restart ending every `restart_units`th unit.
fn opened_every(runs: &[(u64, i32, u32)], restart_units: u32) -> (Recording, Vec<u8>) {
    let (bytes, _) = fixtures::churned(runs, restart_units);
    let mut recording = Recording::open(bytes.clone(), Limits::default()).unwrap();
    recording.select(TABLES);
    (recording, bytes)
}

/// What a walk shows of one hour of [`TABLES`]: each one's status, sample hour, and the slots
/// it read and changed, with the hour's read bits.
#[derive(Clone, Debug, PartialEq)]
struct Fact {
    branch: u64,
    hour: i32,
    statuses: Vec<String>,
    sampled: Vec<Option<i32>>,
    read: Vec<u64>,
    changes: Vec<Option<[Vec<u32>; 4]>>,
    truncated: Vec<Option<bool>>,
}

impl Fact {
    fn of(state: &State, manifest: &Manifest<'_>) -> Self {
        let at = |table: &TableId| table.index();
        Self {
            branch: manifest.branch,
            hour: manifest.hour,
            statuses: TABLES
                .iter()
                .map(|table| format!("{:?}", state.status(at(table))))
                .collect(),
            sampled: TABLES
                .iter()
                .map(|table| state.sampled_at(at(table)))
                .collect(),
            read: manifest.read.clone(),
            changes: TABLES
                .iter()
                .map(|table| {
                    state.changes(at(table)).map(|changes| {
                        [
                            changes.read.clone(),
                            changes.changed.clone(),
                            changes.deleted.clone(),
                            changes.inserted.clone(),
                        ]
                    })
                })
                .collect(),
            truncated: TABLES
                .iter()
                .map(|table| state.changes(at(table)).map(|changes| changes.truncated))
                .collect(),
        }
    }
}

impl Item for Fact {
    fn heap(&self) -> usize {
        self.statuses.iter().map(String::capacity).sum::<usize>()
            + self.read.capacity() * 8
            + self
                .changes
                .iter()
                .flatten()
                .flatten()
                .map(|slots| slots.capacity() * 4)
                .sum::<usize>()
    }
}

impl Projected for Fact {
    fn hour(&self) -> i32 {
        self.hour
    }
}

/// Every hour of `bytes` as one walk of the whole recording sees it, run after run.
fn reference(bytes: Vec<u8>) -> Vec<Fact> {
    let mut recording = Recording::open(bytes, Limits::default()).unwrap();
    recording.select(TABLES);
    let mut state = schema().and_then(State::new).unwrap();
    let mut seen = Vec::new();
    recording
        .walk(&mut state, |state, manifest, _| {
            seen.push(Fact::of(state, manifest));
            Ok(true)
        })
        .unwrap();
    seen
}

/// Where a probe trips.
#[derive(Clone, Copy)]
enum Trip {
    /// Its operator refuses the hour's data.
    Refuse(i32),
    /// Its emit refuses the hour's data.
    Reject(i32),
    /// Its emit fails at the hour.
    Fail(i32),
    /// Its operator cancels the job at the hour.
    Cancel(i32),
}

/// A family seeing each hour of [`TABLES`] as a [`Fact`], publishing their hours under its name.
struct Probe {
    name: &'static str,
    trip: Option<Trip>,
    cancelled: Arc<AtomicBool>,
    /// What its emit kept, in order.
    kept: Arc<Mutex<Vec<Fact>>>,
}

impl Probe {
    fn new(name: &'static str, trip: Option<Trip>, cancelled: &Arc<AtomicBool>) -> Self {
        Self {
            name,
            trip,
            cancelled: Arc::clone(cancelled),
            kept: Arc::default(),
        }
    }
}

struct Looking {
    trip: Option<Trip>,
    cancelled: Arc<AtomicBool>,
}

impl Operator<World> for Looking {
    type Output = Fact;

    fn hour(&mut self, hour: &Hour<'_, World>, out: &mut dyn Sink<Fact>) -> io::Result<()> {
        match self.trip {
            Some(Trip::Refuse(at)) if at == hour.hour => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "probe refused"));
            }
            Some(Trip::Cancel(at)) if at == hour.hour => {
                self.cancelled.store(true, Ordering::SeqCst);
            }
            _ => {}
        }
        out.push(Fact::of(hour.state, hour.manifest))
    }
}

impl Native for Probe {
    type Operator = Looking;
    type Branch = Vec<i32>;

    fn tables(&self) -> &'static [TableId] {
        TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(Needs::default())
    }

    fn operator(&self, _: &Segment) -> io::Result<Looking> {
        Ok(Looking {
            trip: self.trip,
            cancelled: Arc::clone(&self.cancelled),
        })
    }

    fn branch(&self, _: u64, _: i32) -> io::Result<Vec<i32>> {
        Ok(Vec::new())
    }

    fn observe(
        &self,
        state: &mut Vec<i32>,
        _: &mut Stage<'_, '_>,
        _: u64,
        fact: Fact,
    ) -> io::Result<()> {
        match self.trip {
            Some(Trip::Reject(at)) if at == fact.hour => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "probe rejected"));
            }
            Some(Trip::Fail(at)) if at == fact.hour => {
                return Err(io::Error::other("probe failed"));
            }
            _ => {}
        }
        state.push(fact.hour);
        self.kept.lock().unwrap().push(fact);
        Ok(())
    }

    fn flush(
        &self,
        state: &mut Vec<i32>,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        let hours: Vec<u8> = state.drain(..).flat_map(i32::to_le_bytes).collect();
        stage
            .pages()?
            .write(&format!("{}-{branch}", self.name), hours.as_slice())
            .map(drop)
    }
}

/// `native`'s member of a run, bound to its stage of `publisher`, taking up what `resume` says
/// of `shape`'s branches as preparation does.
fn member<'p, N: Native + 'p>(
    publisher: &'p Publisher<'p>,
    resume: &Resume,
    shape: &Shape,
    (name, native): (&'static str, N),
) -> Box<dyn Member + 'p> {
    let id = FamilyId::new(name);
    let mut stage = publisher.stage(id).unwrap();
    let frontiers = resume.families.get(&id);
    let (passes, going) = taken(publisher, &mut stage, shape, id, frontiers).unwrap();
    let taken = Taken {
        passes,
        going: Vec::leak(going),
    };
    Box::new(Bound::new(id, native, stage, taken))
}

/// Every probe's member of a run ([`member`]).
fn bind<'p>(
    publisher: &'p Publisher<'p>,
    resume: &Resume,
    shape: &Shape,
    probes: Vec<Probe>,
) -> Vec<Box<dyn Member + 'p>> {
    probes
        .into_iter()
        .map(|probe| member(publisher, resume, shape, (probe.name, probe)))
        .collect()
}

/// A run of `members` over `recording`, on `threads` workers with a window of `window`.
fn run(
    fixture: &Fixture,
    recording: &Recording,
    shape: &Shape,
    members: &mut [Box<dyn Member + '_>],
    (threads, window): (usize, usize),
    cancelled: &AtomicBool,
    commits: &Commits,
) -> Ran {
    let scratch = segments::Folder::new(fixture.root.join("spill")).unwrap();
    let walk = Walk {
        shape,
        members,
        world: (FormatVersion::CURRENT, CaptureMode::Full),
        settings: &Settings {
            window,
            slots: window,
            ..Settings::new(threads)
        },
        scratch: &scratch,
        cancelled,
        measuring: true,
        commits,
        progress: &mut || {},
    };
    run_walk(recording, walk).unwrap()
}

/// The segments of `branch`.
fn segments_of(shape: &Shape, branch: u64) -> u64 {
    (0..shape.len())
        .filter(|&at| shape.get(at).branch == branch)
        .count() as u64
}

/// Every member's last batch committed, as a run's end leaves them.
fn settle(members: &mut [Box<dyn Member + '_>]) {
    for member in members {
        member.settle().unwrap();
    }
}

/// One walk of each segment carries every family: each keeps every hour once, in the recording's
/// order, with the statuses, sample hours and read and changed slots one walk of the whole
/// recording shows, at segment boundaries as within them; and each publishes every branch whole.
#[test]
fn one_walk_of_each_segment_serves_every_family_each_hour_once() {
    let plan = test_plan();
    let (recording, bytes) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    let expected = reference(bytes);
    assert!(shape.len() > RUNS.len());
    for settings in [(1, 1), (4, 2), (8, 16)] {
        let fixture = Fixture::new();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (alpha, beta) = (
            Probe::new("alpha", None, &cancelled),
            Probe::new("beta", None, &cancelled),
        );
        let (alpha_kept, beta_kept) = (Arc::clone(&alpha.kept), Arc::clone(&beta.kept));
        let (publisher, resume) = fixture.open(plan, &["alpha", "beta"], &recording, &shape);
        let mut members = bind(&publisher, &resume, &shape, vec![alpha, beta]);
        let ran = run(
            &fixture,
            &recording,
            &shape,
            &mut members,
            settings,
            &cancelled,
            &Commits::default(),
        );
        assert!(!ran.cancelled);
        assert_eq!(ran.stats.walks, shape.len(), "one walk a segment");
        assert_eq!((ran.stats.rewalked, ran.stats.derived), (0, 0));
        for outcome in ran.outcomes.iter().flatten() {
            assert!(outcome.ended.is_empty() && outcome.failure.is_none());
        }
        assert_eq!(*alpha_kept.lock().unwrap(), expected);
        assert_eq!(*beta_kept.lock().unwrap(), expected);
        settle(&mut members);
        for name in ["alpha", "beta"] {
            for (&branch, extent) in shape.branches() {
                let frontier = publisher
                    .committed(FamilyId::new(name), branch)
                    .unwrap()
                    .frontier;
                assert!(frontier.complete && frontier.end.is_none());
                assert_eq!(
                    (frontier.first_hour, frontier.last_hour, frontier.segments),
                    (
                        extent.first_hour,
                        extent.last_hour,
                        segments_of(&shape, branch)
                    )
                );
            }
        }
    }
}

/// A store whose recording grew while its content came to be bound otherwise, as when the
/// game's language changed, is prepared anew under the new key, never added to.
#[test]
fn a_grown_recording_bound_to_other_content_is_prepared_anew() {
    let plan = test_plan();
    let fixture = Fixture::new();
    let (recording, _) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    let mut writer = fixture.store.replace(fixture.key.clone()).unwrap();
    writer.write("synthetic", &b"prepared"[..]).unwrap();
    writer.commit().unwrap();
    let path = fixture.root.join("recording");
    std::fs::write(&path, b"synthetic and more hours").unwrap();
    let settings = fixture.root.join("settings.txt");
    std::fs::write(&settings, b"language=\"l_german\"").unwrap();
    let key = store::SourceKey::capture(&path, &[settings]).unwrap();
    assert!(!key.appends(&fixture.key));
    let (publisher, resume) = Publisher::open(
        &fixture.store,
        key.clone(),
        Binding::new(b"synthetic recording", b"synthetic content"),
        plan,
        &Family::planned(plan, &["alpha"]).unwrap(),
        &|_, branch, frontier| shape.accepts(&recording, branch, frontier),
    )
    .unwrap();
    assert!(resume.replaced);
    drop(publisher);
}

/// A proof covers every hour through its frontier: a recording grown from another proves the
/// same through the hours they share, and one whose hours on the frontier's own page differ
/// proves otherwise, though no page comes before it.
#[test]
fn a_proof_covers_every_hour_through_its_frontier() {
    let (recording, _) = opened(&[(1, 0, 2)]);
    let shape = Shape::new(&recording).unwrap();
    let first = shape.proof(&recording, 0);
    assert_ne!(first, [0; 32]);
    let (grown, _) = opened(&[(1, 0, 3)]);
    let grown_shape = Shape::new(&grown).unwrap();
    assert_eq!(grown_shape.get(0).last, shape.get(0).last);
    assert_eq!(grown_shape.proof(&grown, 0), first);
    assert_eq!(
        grown_shape.proof_through(&grown, 0, 3),
        shape.proof_through(&recording, 0, 3)
    );
    let (other, _) = opened(&[(1, 1, 2)]);
    let other_shape = Shape::new(&other).unwrap();
    assert_eq!(other_shape.get(0).last, shape.get(0).last);
    assert_ne!(other_shape.proof(&other, 0), first);
    assert_ne!(
        other_shape.proof_through(&other, 0, 3),
        shape.proof_through(&recording, 0, 3)
    );
    // A branch's later run proves its earlier runs too: only the first run of branch 1 differs.
    let (runs, _) = opened(&RUNS);
    let (earlier, _) = opened(&[(1, 1, 6), (2, 50_000, 4), (1, 100_000, 3)]);
    let (runs_shape, earlier_shape) = (Shape::new(&runs).unwrap(), Shape::new(&earlier).unwrap());
    let last_of = |shape: &Shape, run: u32| {
        (0..shape.len())
            .rfind(|&at| shape.get(at).run == run)
            .unwrap()
    };
    let (third, second) = (last_of(&runs_shape, 2), last_of(&runs_shape, 1));
    assert_eq!(last_of(&earlier_shape, 2), third);
    assert_ne!(
        earlier_shape.proof(&earlier, third),
        runs_shape.proof(&runs, third)
    );
    assert_eq!(
        earlier_shape.proof(&earlier, second),
        runs_shape.proof(&runs, second)
    );
}

/// A family takes up every branch its committed publication finished: a run of the same
/// recording walks nothing, and one of the recording grown by a branch walks that branch alone.
#[test]
fn a_family_goes_on_only_where_its_publication_left_work() {
    let plan = test_plan();
    let fixture = Fixture::new();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (recording, _) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    {
        let (publisher, resume) = fixture.open(plan, &["alpha"], &recording, &shape);
        let mut members = bind(
            &publisher,
            &resume,
            &shape,
            vec![Probe::new("alpha", None, &cancelled)],
        );
        let commits = Commits::default();
        run(
            &fixture,
            &recording,
            &shape,
            &mut members,
            (4, 4),
            &cancelled,
            &commits,
        );
        settle(&mut members);
    }
    {
        let (publisher, resume) = fixture.open(plan, &["alpha"], &recording, &shape);
        assert_eq!(resume.families[&FamilyId::new("alpha")].len(), 2);
        let mut members = bind(
            &publisher,
            &resume,
            &shape,
            vec![Probe::new("alpha", None, &cancelled)],
        );
        assert!(members[0].idle(&shape));
        let commits = Commits::default();
        let ran = run(
            &fixture,
            &recording,
            &shape,
            &mut members,
            (4, 4),
            &cancelled,
            &commits,
        );
        assert_eq!(ran.stats.walks, 0);
        assert!(ran.outcomes[0].is_none());
    }
    let (grown, bytes) = opened(&GROWN);
    let shape = Shape::new(&grown).unwrap();
    let (publisher, resume) = fixture.open(plan, &["alpha"], &grown, &shape);
    assert!(resume.again.is_empty());
    let probe = Probe::new("alpha", None, &cancelled);
    let kept = Arc::clone(&probe.kept);
    let mut members = bind(&publisher, &resume, &shape, vec![probe]);
    let ran = run(
        &fixture,
        &grown,
        &shape,
        &mut members,
        (4, 4),
        &cancelled,
        &Commits::default(),
    );
    settle(&mut members);
    let third: Vec<Fact> = reference(bytes)
        .into_iter()
        .filter(|fact| fact.branch == 3)
        .collect();
    assert_eq!(*kept.lock().unwrap(), third);
    assert_eq!(ran.stats.walks as u64, segments_of(&shape, 3));
    assert!(
        publisher
            .committed(FamilyId::new("alpha"), 3)
            .unwrap()
            .frontier
            .complete
    );
}

/// A refusal of a branch's data, by a family's operator or its emit, keeps the hours before it
/// and publishes the branch's end there with its reason: its other branches, and the other
/// family, go on whole.
#[test]
fn a_refusal_ends_its_family_s_branch_there_by_name() {
    let plan = test_plan();
    let (recording, bytes) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    let expected = reference(bytes);
    // An hour inside the first branch's second segment.
    let refused = shape.get(1).first_hour + 1;
    for (trip, reason) in [
        (Trip::Refuse(refused), "probe refused"),
        (Trip::Reject(refused), "probe rejected"),
    ] {
        let fixture = Fixture::new();
        let cancelled = Arc::new(AtomicBool::new(false));
        let alpha = Probe::new("alpha", Some(trip), &cancelled);
        let beta = Probe::new("beta", None, &cancelled);
        let (alpha_kept, beta_kept) = (Arc::clone(&alpha.kept), Arc::clone(&beta.kept));
        let (publisher, resume) = fixture.open(plan, &["alpha", "beta"], &recording, &shape);
        let mut members = bind(&publisher, &resume, &shape, vec![alpha, beta]);
        let ran = run(
            &fixture,
            &recording,
            &shape,
            &mut members,
            (4, 4),
            &cancelled,
            &Commits::default(),
        );
        settle(&mut members);
        assert!(ran.outcomes[0].as_ref().unwrap().failure.is_none());
        let first: Vec<Fact> = expected
            .iter()
            .filter(|fact| fact.branch != 1 || fact.hour < refused)
            .cloned()
            .collect();
        assert_eq!(*alpha_kept.lock().unwrap(), first);
        assert_eq!(*beta_kept.lock().unwrap(), expected);
        let ended = publisher
            .committed(FamilyId::new("alpha"), 1)
            .unwrap()
            .frontier;
        let end = ended.end.unwrap();
        assert_eq!((end.hour, end.reason.as_str()), (refused, reason));
        assert_eq!(ended.last_hour, shape.get(1).last_hour);
        assert!(
            publisher
                .committed(FamilyId::new("alpha"), 2)
                .unwrap()
                .frontier
                .complete
        );
    }
}

/// A family whose emit fails fails alone: what it queued before stands, what it held since goes
/// uncommitted, and the other family publishes every branch whole.
#[test]
fn a_failing_family_fails_alone() {
    let plan = test_plan();
    let fixture = Fixture::new();
    let (recording, _) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    let failing = shape.get(shape.len() - 1).first_hour;
    let cancelled = Arc::new(AtomicBool::new(false));
    let alpha = Probe::new("alpha", Some(Trip::Fail(failing)), &cancelled);
    let beta = Probe::new("beta", None, &cancelled);
    let (publisher, resume) = fixture.open(plan, &["alpha", "beta"], &recording, &shape);
    let mut members = bind(&publisher, &resume, &shape, vec![alpha, beta]);
    let ran = run(
        &fixture,
        &recording,
        &shape,
        &mut members,
        (4, 4),
        &cancelled,
        &Commits::default(),
    );
    settle(&mut members);
    let failure = ran.outcomes[0].as_ref().unwrap().failure.as_ref().unwrap();
    assert_eq!(failure.at, shape.len() - 1);
    assert!(ran.outcomes[1].as_ref().unwrap().failure.is_none());
    let alpha = publisher.committed(FamilyId::new("alpha"), 1);
    assert!(alpha.is_none_or(|published| !published.frontier.terminal()));
    for &branch in shape.branches().keys() {
        let beta = publisher.committed(FamilyId::new("beta"), branch).unwrap();
        assert!(beta.frontier.complete);
    }
}

/// Cancellation reaches the run from wherever the job's flag is first seen, waking every wait in
/// it: set before the run starts, by an operator at an hour, or as a commit is made. The run ends
/// cancelled, its stages go, and only committed batches stand.
#[test]
fn cancellation_stops_a_run_at_every_wait() {
    let plan = test_plan();
    let (recording, _) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    let middle = shape.get(shape.len() / 2).first_hour;
    for when in ["before", "operator", "commit"] {
        let fixture = Fixture::new();
        let cancelled = Arc::new(AtomicBool::new(when == "before"));
        let trip = (when == "operator").then_some(Trip::Cancel(middle));
        let (publisher, resume) = fixture.open(plan, &["alpha", "beta"], &recording, &shape);
        let flag = Arc::clone(&cancelled);
        let publisher = if when == "commit" {
            publisher.with_commits(move || flag.store(true, Ordering::SeqCst))
        } else {
            publisher
        };
        let probes = vec![
            Probe::new("alpha", trip, &cancelled),
            Probe::new("beta", None, &cancelled),
        ];
        let mut members = bind(&publisher, &resume, &shape, probes);
        let ran = run(
            &fixture,
            &recording,
            &shape,
            &mut members,
            (4, 4),
            &cancelled,
            &Commits::default(),
        );
        assert!(ran.cancelled, "{when}");
        drop(members);
        publisher.stop(Stopped::Cancelled);
        for name in ["alpha", "beta"] {
            let terminal = shape.branches().keys().all(|&branch| {
                publisher
                    .committed(FamilyId::new(name), branch)
                    .is_some_and(|published| published.frontier.terminal())
            });
            assert!(!terminal, "{when}: {name} finished a cancelled run");
        }
    }
}

#[test]
fn content_is_loaded_once_under_concurrent_consumers() {
    let loads = AtomicUsize::new(0);
    let piece: Once<Vec<u32>> = Once::new();
    let failing: Once<u32> = Once::new();
    let consumers = 16;
    let barrier = Barrier::new(consumers);
    std::thread::scope(|scope| {
        for _ in 0..consumers {
            scope.spawn(|| {
                barrier.wait();
                let value = piece
                    .get(|| {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok(vec![1, 2, 3])
                    })
                    .unwrap();
                assert_eq!(value, &[1, 2, 3]);
                let error = failing
                    .get(|| Err(io::Error::new(io::ErrorKind::NotFound, "no such content")))
                    .unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::NotFound);
                assert_eq!(error.to_string(), "no such content");
            });
        }
    });
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    let ran = AtomicUsize::new(0);
    let made = load_all::<u32>(
        3,
        (0..10u32)
            .map(|at| {
                let ran = &ran;
                Box::new(move || {
                    ran.fetch_add(1, Ordering::SeqCst);
                    match at {
                        4 => Err(io::Error::other("content missing")),
                        7 => panic!("content panicked"),
                        _ => Ok(at),
                    }
                }) as Box<dyn FnOnce() -> io::Result<u32> + Send + '_>
            })
            .collect(),
    );
    assert_eq!(ran.load(Ordering::SeqCst), 10);
    for (at, made) in made.iter().enumerate() {
        match at {
            4 => assert_eq!(made.as_ref().unwrap_err().to_string(), "content missing"),
            7 => assert_eq!(made.as_ref().unwrap_err().to_string(), "content panicked"),
            _ => assert_eq!(*made.as_ref().unwrap(), at as u32),
        }
    }
}

/// A native plan prepares each family by its one backend or refuses it by name: its coordinator
/// marks the store and publishes, on every branch from its first hour, the refusal of every
/// family and recording-wide job without a native backend, which every query of theirs answers
/// with; the families with one answer from their own pages. A family that gains a backend loses
/// the refusal it published before.
#[test]
fn a_native_plan_refuses_a_family_without_a_backend_by_name() {
    let plan = test_plan();
    assert_eq!(plan.backend("alpha"), Some(Backend::Segments));
    assert_eq!(plan.backend("gamma"), Some(Backend::Unavailable));
    assert!(plan.apart("gamma") && plan.apart("alpha"));
    for family in plan.families() {
        assert!(
            plan.backend(family.id.name()).is_some(),
            "{}",
            family.id.name()
        );
    }
    assert!(plan::test_native(&[&ALPHA], &["alpha", "alpha"]).is_err());
    assert!(plan::test_native(&[], &["alpha"]).is_err());
    let fixture = Fixture::new();
    let (recording, _) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    let mark = status::mark(plan, &shape);
    let (publisher, _) = fixture.open(plan, &["alpha"], &recording, &shape);
    publisher
        .coordinate(|writer| {
            // A refusal of alpha from before it had a backend.
            writer.write_at(
                &crate::prepared::coverage::family_section("alpha", 1),
                0,
                &b"{}"[..],
            )?;
            status::coordinate(writer, plan, &shape, &mark)
        })
        .unwrap();
    let published = gates::decode(&fixture.store, None).unwrap().unwrap();
    assert_eq!(published.plan(), Some(&mark[..]));
    assert!(
        published
            .gate(1, "alpha")
            .is_none_or(|gate| gate.id().is_none())
    );
    plan::with_test_native(plan, || {
        for (&branch, extent) in shape.branches() {
            let (hour, error) = pipeline::refusal_from(&published, branch, "gamma-product")
                .unwrap()
                .unwrap();
            assert_eq!(hour, extent.first_hour);
            assert_eq!(error, "gamma is not prepared from native recordings yet");
            let (_, error) = pipeline::refusal_from(&published, branch, "production-lines")
                .unwrap()
                .unwrap();
            assert!(error.contains("production-lines"), "{error}");
            assert!(
                pipeline::refusal_from(&published, branch, "alpha-product")
                    .unwrap()
                    .is_none()
            );
        }
    });
    let accepted = status::accepted(
        plan,
        &shape,
        &[FamilyId::new("alpha")],
        &|_, _| None,
        &Default::default(),
        0,
    );
    let focus = accepted
        .sections
        .iter()
        .find(|section| section.id == "focus")
        .unwrap();
    assert_eq!(focus.state, Phase::Failed);
    assert!(
        focus
            .error
            .as_deref()
            .is_some_and(|error| error.ends_with("is not prepared from native recordings yet"))
    );
    let map = accepted
        .sections
        .iter()
        .find(|section| section.id == "map")
        .unwrap();
    assert_eq!(map.state, Phase::Ready);
}

/// A family's read of another's state is its own: under the native plan its stage writes the
/// read under its own names, and never the sections of the family it reads, nor another's.
#[test]
fn dependencies_are_served_privately() {
    let plan = plan::native();
    let fixture = Fixture::new();
    let (recording, _) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    let (publisher, _) = fixture.open(plan, &["territory"], &recording, &shape);
    let mut stage = publisher.stage(FamilyId::new("territory")).unwrap();
    let pages = stage.pages().unwrap();
    pages
        .write("territory-identities-1", &b"own read"[..])
        .unwrap();
    for other in ["country-identities-1", "army-values-1-0", "map.checkpoints"] {
        let error = pages.write(other, &b"another's"[..]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{other}");
    }
    let names = super::names();
    for (at, name) in names.iter().enumerate() {
        assert_eq!(plan.backend(name), Some(Backend::Segments), "{name}");
        assert!(
            !names[..at].contains(name),
            "{name} has two native backends"
        );
    }
}

/// What a tally's operator hands its emit of an hour: a digest of what the walk shows of it,
/// and, for one going on through its branch, the hours its operator walked before it.
struct Tick {
    hour: i32,
    seen: u64,
    walked: Option<u64>,
}

impl Item for Tick {
    fn heap(&self) -> usize {
        0
    }
}

impl Projected for Tick {
    fn hour(&self) -> i32 {
        self.hour
    }
}

fn digest(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// A tally's operator: one starting anywhere, or one going on from its branch's first run
/// through every restart and later run, carrying the hours it walked.
struct Counting {
    hands: bool,
    walked: u64,
    hold: Option<Arc<Hold>>,
}

/// Holds the recording's first hour, as its family's operator walks it, until the test lets it go.
struct Hold {
    met: Barrier,
    released: Barrier,
    held: AtomicBool,
}

impl Operator<World> for Counting {
    type Output = Tick;

    fn starts_at(&self, start: Start<'_>) -> bool {
        !self.hands || matches!(start, Start::Run { continues: false })
    }

    fn hour(&mut self, hour: &Hour<'_, World>, out: &mut dyn Sink<Tick>) -> io::Result<()> {
        if let Some(hold) = &self.hold
            && (hour.run, hour.ordinal) == (0, 0)
            && !hold.held.swap(true, Ordering::SeqCst)
        {
            hold.met.wait();
            hold.released.wait();
        }
        let fact = Fact::of(hour.state, hour.manifest);
        let seen = digest(&(
            &fact.statuses,
            &fact.sampled,
            &fact.read,
            &fact.changes,
            &fact.truncated,
        ));
        let walked = self.hands.then_some(self.walked);
        self.walked += 1;
        out.push(Tick {
            hour: hour.hour,
            seen,
            walked,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(0)
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(&self.walked.to_le_bytes());
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let bytes = bytes
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Not a saved count"))?;
        self.walked = u64::from_le_bytes(bytes);
        Ok(())
    }
}

/// A family folding every hour it keeps into a running digest, which it publishes whole and
/// saves to go on with: what it serves depends on every hour it kept, in order, once.
struct Tally {
    name: &'static str,
    hands: bool,
    /// The hours its emit kept, in order.
    kept: Arc<Mutex<Vec<i32>>>,
    hold: Option<Arc<Hold>>,
}

impl Tally {
    fn new(name: &'static str, hands: bool) -> Self {
        Self {
            name,
            hands,
            kept: Arc::default(),
            hold: None,
        }
    }
}

impl Native for Tally {
    type Operator = Counting;
    type Branch = (u64, u64);

    fn tables(&self) -> &'static [TableId] {
        TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(Needs::default())
    }

    fn operator(&self, _: &Segment) -> io::Result<Counting> {
        Ok(Counting {
            hands: self.hands,
            walked: 0,
            hold: self.hold.clone(),
        })
    }

    fn branch(&self, _: u64, _: i32) -> io::Result<(u64, u64)> {
        Ok((0, 0))
    }

    fn observe(
        &self,
        state: &mut (u64, u64),
        _: &mut Stage<'_, '_>,
        _: u64,
        tick: Tick,
    ) -> io::Result<()> {
        *state = (
            state.0 + 1,
            digest(&(state.1, tick.hour, tick.seen, tick.walked)),
        );
        self.kept.lock().unwrap().push(tick.hour);
        Ok(())
    }

    fn flush(
        &self,
        state: &mut (u64, u64),
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        let page = [state.0.to_le_bytes(), state.1.to_le_bytes()].concat();
        stage
            .pages()?
            .write_at(&format!("{}-{branch}", self.name), 0, page.as_slice())
    }

    fn save(&self, state: &mut (u64, u64)) -> io::Result<Option<Vec<u8>>> {
        Ok(Some(
            [state.0.to_le_bytes(), state.1.to_le_bytes()].concat(),
        ))
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        _: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<(u64, u64)> {
        let half = |at: usize| u64::from_le_bytes(saved[at..at + 8].try_into().unwrap());
        Ok((half(0), half(8)))
    }
}

/// What a family serves of each branch: its coverage, its end, and its published page.
type Served = BTreeMap<u64, ((i32, i32, bool, bool), Vec<u8>)>;

fn served(
    fixture: &Fixture,
    publisher: &Publisher<'_>,
    name: &'static str,
    shape: &Shape,
) -> Served {
    let mut served = Served::new();
    for &branch in shape.branches().keys() {
        let frontier = publisher
            .committed(FamilyId::new(name), branch)
            .unwrap()
            .frontier;
        let mut page = Vec::new();
        io::Read::read_to_end(
            &mut fixture
                .store
                .read(&fixture.key, &format!("{name}-{branch}"), 0)
                .unwrap()
                .unwrap(),
            &mut page,
        )
        .unwrap();
        let coverage = (
            frontier.first_hour,
            frontier.last_hour,
            frontier.complete,
            frontier.end.is_some(),
        );
        served.insert(branch, (coverage, page));
    }
    served
}

/// A recording a live recorder grows: its last branch's open segment grows, then a segment
/// past its restart, then a later run of the branch. Each append prepares only the segments
/// holding its new hours, each family keeping only those hours, one starting anywhere joining
/// the walk where its publication left off and one going on through its branch from its
/// carry; and what each serves equals what one preparation of the grown recording serves.
#[test]
fn a_growing_branch_prepares_only_its_new_hours() {
    let plan = test_plan();
    let appends: [&[(u64, i32, u32)]; 4] = [
        &[(1, 0, 3), (2, 50_000, 3)],
        &[(1, 0, 3), (2, 50_000, 4)],
        &[(1, 0, 3), (2, 50_000, 5)],
        &[(1, 0, 3), (2, 50_000, 5), (2, 60_000, 2)],
    ];
    let names = ["alpha", "beta"];
    let tallies = || (Tally::new("alpha", false), Tally::new("beta", true));
    let cancelled = Arc::new(AtomicBool::new(false));
    let grown = Fixture::new();
    let mut last: Option<i32> = None;
    for (append, runs) in appends.iter().enumerate() {
        let (recording, _) = opened_every(runs, 2);
        let shape = Shape::new(&recording).unwrap();
        let (publisher, resume) = grown.open(plan, &names, &recording, &shape);
        assert!(resume.again.is_empty());
        let (alpha, beta) = tallies();
        let kept = [Arc::clone(&alpha.kept), Arc::clone(&beta.kept)];
        let mut members = vec![
            member(&publisher, &resume, &shape, ("alpha", alpha)),
            member(&publisher, &resume, &shape, ("beta", beta)),
        ];
        let ran = run(
            &grown,
            &recording,
            &shape,
            &mut members,
            (2, 2),
            &cancelled,
            &Commits::default(),
        );
        settle(&mut members);
        drop(members);
        for outcome in ran.outcomes.iter().flatten() {
            assert!(outcome.ended.is_empty() && outcome.failure.is_none());
        }
        let extent = shape.branches()[&2];
        let new: Vec<i32> = (0..shape.len())
            .map(|at| *shape.get(at))
            .filter(|placed| placed.branch == 2 || last.is_none())
            .flat_map(|placed| placed.first_hour..=placed.last_hour)
            .filter(|&hour| last.is_none_or(|last| hour > last))
            .collect();
        if append > 0 {
            let walked = (0..shape.len())
                .map(|at| *shape.get(at))
                .filter(|placed| placed.branch == 2)
                .filter(|placed| last.is_some_and(|last| placed.last_hour > last))
                .count();
            assert_eq!(ran.stats.walks, walked, "append {append}");
            // The first append's hours go on within the segment the one before left open.
            let opens = (0..shape.len()).any(|at| Some(&shape.get(at).first_hour) == new.first());
            assert_eq!(opens, append != 1, "append {append}");
        }
        for kept in &kept {
            assert_eq!(*kept.lock().unwrap(), new, "append {append}");
        }
        last = Some(extent.last_hour);
        let whole = Fixture::new();
        let (once, _) = whole.open(plan, &names, &recording, &shape);
        let (alpha, beta) = tallies();
        let mut members = vec![
            member(&once, &Resume::default(), &shape, ("alpha", alpha)),
            member(&once, &Resume::default(), &shape, ("beta", beta)),
        ];
        run(
            &whole,
            &recording,
            &shape,
            &mut members,
            (4, 4),
            &cancelled,
            &Commits::default(),
        );
        settle(&mut members);
        drop(members);
        for name in names {
            assert_eq!(
                served(&grown, &publisher, name, &shape),
                served(&whole, &once, name, &shape),
                "{name} after append {append}"
            );
        }
    }
}

/// A family prepared ahead walks the whole recording in a run of its own before the union run,
/// which the families' registry asks for country styles alone: what it serves of every branch's
/// last hour is published while the union run is held at its first hour.
#[test]
fn a_family_prepared_ahead_serves_the_last_hour_before_the_union_run() {
    let ahead: Vec<&str> = registry::FAMILIES
        .iter()
        .filter(|family| family.ahead)
        .map(|family| family.id.name())
        .collect();
    assert_eq!(ahead, ["country-styles"]);
    static DELTA: Descriptor = Descriptor {
        queries: &["delta-product"],
        gates: &["delta-product"],
        sections: &["delta"],
        ahead: true,
        ..registry::family("delta")
    };
    let plan = plan::test_native(&[&ALPHA, &DELTA], &["alpha", "delta"]).unwrap();
    let fixture = Fixture::new();
    let (mut recording, _) = opened(&RUNS);
    let shape = Shape::new(&recording).unwrap();
    let (publisher, resume) = fixture.open(plan, &["alpha", "delta"], &recording, &shape);
    let hold = Arc::new(Hold {
        met: Barrier::new(2),
        released: Barrier::new(2),
        held: AtomicBool::new(false),
    });
    let alpha = Tally {
        hold: Some(Arc::clone(&hold)),
        ..Tally::new("alpha", false)
    };
    let delta = Tally::new("delta", false);
    let kept = Arc::clone(&delta.kept);
    let mut members = vec![
        member(&publisher, &resume, &shape, ("alpha", alpha)),
        member(&publisher, &resume, &shape, ("delta", delta)),
    ];
    let first = ahead_first(plan, &mut members);
    assert_eq!(first, 1);
    assert_eq!(members[0].id(), FamilyId::new("delta"));
    let (cancelled, commits) = (AtomicBool::new(false), Commits::default());
    let scratch = segments::Folder::new(fixture.root.join("spill")).unwrap();
    let settings = Settings {
        window: 4,
        slots: 4,
        ..Settings::new(4)
    };
    let (walks, kept, seen) = std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            let walk = Walk {
                shape: &shape,
                members: &mut members,
                world: (FormatVersion::CURRENT, CaptureMode::Full),
                settings: &settings,
                scratch: &scratch,
                cancelled: &cancelled,
                measuring: true,
                commits: &commits,
                progress: &mut || {},
            };
            let walks = walk_all(&mut recording, first, walk);
            // A run that never held lets the test go all the same.
            if !hold.held.swap(true, Ordering::SeqCst) {
                hold.met.wait();
                hold.released.wait();
            }
            walks
        });
        // The union run holds at the recording's first hour: the narrow run ended, and its
        // last batch was committed.
        hold.met.wait();
        let kept = kept.lock().unwrap().len() as u64;
        let seen: Vec<_> = shape
            .branches()
            .iter()
            .map(|(&branch, &extent)| {
                let delta = publisher
                    .committed(FamilyId::new("delta"), branch)
                    .map(|published| published.frontier);
                let page = fixture
                    .store
                    .read(&fixture.key, &format!("delta-{branch}"), 0)
                    .is_ok_and(|page| page.is_some());
                let alpha = publisher.committed(FamilyId::new("alpha"), branch);
                (extent, delta, page, alpha.is_some())
            })
            .collect();
        hold.released.wait();
        (running.join().unwrap().unwrap(), kept, seen)
    });
    settle(&mut members);
    assert_eq!(kept, recording_hours(&shape));
    for (extent, delta, page, alpha) in seen {
        let delta = delta.unwrap();
        assert!(delta.complete && delta.end.is_none());
        assert_eq!(
            (delta.first_hour, delta.last_hour),
            (extent.first_hour, extent.last_hour)
        );
        assert!(page && !alpha);
    }
    assert!(!walks.ran.cancelled);
    for outcome in walks.ran.outcomes.iter().flatten() {
        assert!(outcome.ended.is_empty() && outcome.failure.is_none());
    }
    let (narrow, _, _) = walks.ahead.unwrap();
    assert_eq!(
        (narrow.walks, walks.ran.stats.walks),
        (shape.len(), shape.len())
    );
    for (&branch, extent) in shape.branches() {
        let frontier = publisher
            .committed(FamilyId::new("alpha"), branch)
            .unwrap()
            .frontier;
        assert_eq!(frontier.last_hour, extent.last_hour);
    }
}

/// Every hour `shape` places.
fn recording_hours(shape: &Shape) -> u64 {
    (0..shape.len())
        .map(|at| shape.get(at))
        .map(|placed| u64::from(placed.last - placed.first) + 1)
        .sum()
}
