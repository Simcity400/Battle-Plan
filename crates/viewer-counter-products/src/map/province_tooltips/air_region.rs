//! Air region statistics: the received-damage grids the strategic air view
//! draws in its default state for the
//! hovered province's strategic region and the viewing country, over the
//! last day. The rows come from AirRegionCombat at the frame's hour after the
//! latest daily prune that visited the viewing country
//! ([`replay_derive::air_region_derive::visiting_pass`]), found from the recorded hourly
//! AirDailyRoster of the frame's branch ([`PassRosters`], read on a worker; the hover
//! shows no statistics until the answer is ready); each side lists its air grid
//! and its ground grid for our missions and the enemy's. Each line uses the
//! item's tooltip key and its value as the received-damage formatter
//! writes it, green for ours and red for the enemy's.
use job_priority::SpawnCarried as _;
use super::{Arg, Context, line};
use replay::{
    campaign::Replay,
    columnar_natives::{Cursor, Natives},
};
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_derive::air_region_derive as derive;
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
use viewer_foundation::tooltip_text::Tooltip;

/// The daily air roster at a recorded sample: whether a country (canonical slot) is in
/// the roster the daily pass visits.
pub trait RosterSamples {
    /// The branch's first sample hour, or None for a branch the recording lacks.
    fn first_hour(&self, branch: BranchId) -> Option<Hour>;
    /// The country's AirDailyRoster eligibility at the branch's sample `hour`.
    fn eligible(&self, branch: BranchId, hour: Hour, slot: u32) -> Result<bool, String>;
}

/// The hover's view of the roster history.
pub trait Rosters {
    /// The latest daily pass of `branch` at or before `hour` that visited the country in
    /// `slot` ([`visiting_pass`]), or None while it is still being read.
    fn visiting_pass(
        &self,
        branch: BranchId,
        hour: Hour,
        slot: u32,
    ) -> Option<Result<Option<Hour>, String>>;
}

/// One sample's roster: which canonical slots it lists and which of them are eligible,
/// as bit sets. Equal rosters share one interned snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Roster {
    listed: Vec<u64>,
    eligible: Vec<u64>,
}

impl Roster {
    fn set(bits: &mut Vec<u64>, slot: u32) {
        let (word, bit) = (slot as usize / 64, slot % 64);
        if bits.len() <= word {
            bits.resize(word + 1, 0);
        }
        bits[word] |= 1 << bit;
    }

    fn get(bits: &[u64], slot: u32) -> bool {
        bits.get(slot as usize / 64)
            .is_some_and(|word| word & (1 << (slot % 64)) != 0)
    }

    fn eligible(&self, slot: u32) -> Option<bool> {
        Self::get(&self.listed, slot).then(|| Self::get(&self.eligible, slot))
    }

    /// The roster AirDailyRoster's rows list, each raw country turned into its canonical
    /// slot by CountryLinks (key-sorted rows, as a frame holds them).
    pub fn of(
        rows: &[replay_data::rows::AirDailyRosterRow],
        links: &[replay_data::rows::CountryLinkRow],
    ) -> Result<Self, String> {
        let mut roster = Self::default();
        for row in rows {
            let slot = links
                .binary_search_by_key(&row.country, |link| link.id)
                .map(|at| links[at].canonical_slot)
                .map_err(|_| format!("AirDailyRoster country {} has no CountryLink", row.country))?;
            Self::set(&mut roster.listed, slot);
            if row.eligible {
                Self::set(&mut roster.eligible, slot);
            }
        }
        Ok(roster)
    }

    fn bytes(&self) -> usize {
        (self.listed.capacity() + self.eligible.capacity()) * size_of::<u64>()
    }
}

type Sample = Result<Arc<Roster>, String>;

/// Why a search stopped: a newer demand or the service closing superseded it.
pub const SUPERSEDED: &str = "the air roster search was superseded";

/// What a source read for an asked hour: the span of hours it read, which holds that hour,
/// and the roster at each sample of it around a daily pass's tick (hours ≡ 23 and ≡ 0 mod
/// 24), or why that sample is unreadable.
pub struct Span {
    pub first: Hour,
    pub last: Hour,
    pub samples: Vec<(Hour, Result<Roster, String>)>,
}

/// Whether `hour` is a sample around a daily pass's tick: the hour before it or the pass's own.
pub fn sample_hour(hour: Hour) -> bool {
    matches!(hour.rem_euclid(derive::DAY), 0 | 23)
}

/// Where the roster samples are read from: today's block reader, today's columnar cursor,
/// or lanes of the history's own (the viewer's).
pub trait SampleSource: Send + Sync {
    /// The span holding `hour` of `branch`, an empty one where nothing is recorded;
    /// `Err(SUPERSEDED)` once `superseded` says to stop.
    fn span(&self, branch: BranchId, hour: Hour, superseded: &dyn Fn() -> bool) -> Result<Span, String>;
}

/// Today's reader: the whole block holding the hour, decoded, its samples kept.
struct Blocks(Arc<Replay>);

impl SampleSource for Blocks {
    fn span(&self, branch: BranchId, hour: Hour, _: &dyn Fn() -> bool) -> Result<Span, String> {
        let Some(index) = self.0.index.iter().find(|index| {
            index.branch == branch && index.first_hour <= hour && hour <= index.last_hour
        }) else {
            return Ok(Span { first: hour, last: hour, samples: Vec::new() });
        };
        let mut samples = Vec::new();
        let tables = [TableId::AirDailyRoster, TableId::CountryLinks];
        let read = self.0.read_block_alone(
            index,
            &tables,
            |_, decoded| {
                let at = decoded.hour;
                if sample_hour(at) {
                    samples.push((at, roster(decoded)));
                }
                Ok(())
            },
            || Ok(()),
        );
        if let Err(error) = read {
            samples.push((hour, Err(format!("hour {hour} is unreadable: {error}"))));
        }
        Ok(Span { first: index.first_hour, last: index.last_hour, samples })
    }
}

/// Today's columnar cursor over the two tables, moved to each sample asked.
struct Columnar {
    replay: Arc<Replay>,
    natives: Arc<Natives>,
    cursor: OnceLock<Result<Mutex<Cursor>, String>>,
}

impl SampleSource for Columnar {
    fn span(&self, branch: BranchId, hour: Hour, superseded: &dyn Fn() -> bool) -> Result<Span, String> {
        let cursor = self
            .cursor
            .get_or_init(|| {
                Cursor::new(
                    Arc::clone(&self.natives),
                    &[TableId::AirDailyRoster, TableId::CountryLinks],
                    self.replay.format,
                    self.replay.capture_mode,
                )
                .map(Mutex::new)
                .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let mut cursor = cursor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if superseded() {
            return Err(SUPERSEDED.into());
        }
        let only = |sample| Ok(Span { first: hour, last: hour, samples: vec![(hour, sample)] });
        let Some(segment) = self.replay.segment_at(branch, hour) else {
            return only(Err(format!("hour {hour} of branch {branch} is not recorded")));
        };
        match cursor.move_to(segment, branch, hour, superseded) {
            Err(_) if superseded() => Err(SUPERSEDED.into()),
            Err(error) => only(Err(error.to_string())),
            Ok(false) => only(Err(format!("hour {hour} of branch {branch} is not recorded"))),
            Ok(true) => only(cursor.build(None).map_err(|error| error.to_string()).and_then(|decoded| roster(&decoded))),
        }
    }
}

fn roster(decoded: &DecodedFrame) -> Result<Roster, String> {
    let rows = decoded
        .complete::<tables::AirDailyRoster>()
        .map_err(|error| error.to_string())?;
    let links = decoded
        .complete::<tables::CountryLinks>()
        .map_err(|error| error.to_string())?;
    Roster::of(rows, links)
}

/// The spans whose samples are kept: the newest few, so what is held does not grow with
/// hours played. An eligible country's question reads the head's span and at most the one
/// before it; a search further back reads and drops.
const KEPT: usize = 8;

/// The recording's daily air rosters at the samples around each daily pass, read through
/// one source and shared by every reader of the history (the province hover, the combat
/// window's air panel, the air-mission counters). Each sample holds an interned snapshot,
/// so a history of unchanged rosters keeps one copy.
pub struct Sampler {
    replay: Arc<Replay>,
    columnar: Option<Arc<Natives>>,
    source: Box<dyn SampleSource>,
    hours: Mutex<BTreeMap<(BranchId, Hour), Sample>>,
    /// The spans read, oldest first.
    spans: Mutex<VecDeque<(BranchId, Hour, Hour)>>,
    distinct: Mutex<HashSet<Arc<Roster>>>,
    /// Spans read, for the cancellation checks.
    blocks: AtomicUsize,
    /// One span read at a time: a second asker of the same span waits and finds it held.
    reading: Mutex<()>,
}

impl Sampler {
    pub fn new(replay: Arc<Replay>, source: Box<dyn SampleSource>) -> Self {
        Self::with(replay, None, source)
    }

    /// Today's sources: the columnar cursor when `columnar` is given, else the block reader.
    pub fn today(replay: Arc<Replay>, columnar: Option<Arc<Natives>>) -> Self {
        let source: Box<dyn SampleSource> = match &columnar {
            Some(natives) => Box::new(Columnar { replay: Arc::clone(&replay), natives: Arc::clone(natives), cursor: OnceLock::new() }),
            None => Box::new(Blocks(Arc::clone(&replay))),
        };
        Self::with(replay, columnar, source)
    }

    fn with(replay: Arc<Replay>, columnar: Option<Arc<Natives>>, source: Box<dyn SampleSource>) -> Self {
        Self {
            replay,
            columnar,
            source,
            hours: Mutex::default(),
            spans: Mutex::default(),
            distinct: Mutex::default(),
            blocks: AtomicUsize::new(0),
            reading: Mutex::default(),
        }
    }

    pub fn replay(&self) -> &Arc<Replay> {
        &self.replay
    }

    pub fn columnar(&self) -> Option<&Arc<Natives>> {
        self.columnar.as_ref()
    }

    /// The samples, spans and distinct rosters held, and their bytes.
    pub fn held(&self) -> (usize, usize, usize, usize) {
        let hours = self.hours.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let distinct = self.distinct.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let spans = self.spans.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len();
        let bytes = hours.len() * (size_of::<(BranchId, Hour)>() + size_of::<Sample>()) + distinct.iter().map(|roster| roster.bytes() + size_of::<Roster>()).sum::<usize>();
        (hours.len(), spans, distinct.len(), bytes)
    }

    fn intern(&self, roster: Roster) -> Arc<Roster> {
        let mut distinct = self
            .distinct
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(held) = distinct.get(&roster) {
            return Arc::clone(held);
        }
        let roster = Arc::new(roster);
        distinct.insert(Arc::clone(&roster));
        roster
    }

    /// Keeps `span`'s samples, and the newest [`KEPT`] spans' only.
    fn keep(&self, branch: BranchId, span: Span) {
        let mut hours = self.hours.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for (at, sample) in span.samples {
            hours.entry((branch, at)).or_insert_with(|| sample.map(|roster| self.intern(roster)));
        }
        let mut spans = self.spans.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        spans.push_back((branch, span.first, span.last));
        if spans.len() <= KEPT {
            return;
        }
        while spans.len() > KEPT {
            spans.pop_front();
        }
        hours.retain(|&(of, at), _| spans.iter().any(|&(branch, first, last)| branch == of && first <= at && at <= last));
        drop(hours);
        self.distinct
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|roster| Arc::strong_count(roster) > 1);
    }

    fn eligible(&self, branch: BranchId, hour: Hour, slot: u32, superseded: &dyn Fn() -> bool) -> Result<bool, String> {
        let held = || {
            self.hours
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&(branch, hour))
                .cloned()
        };
        let roster = match held() {
            Some(roster) => roster,
            None => {
                if superseded() {
                    return Err(SUPERSEDED.into());
                }
                let _reading = self.reading.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                match held() {
                    Some(roster) => roster,
                    None => {
                        self.blocks.fetch_add(1, Ordering::AcqRel);
                        let mut span = self.source.span(branch, hour, superseded)?;
                        if !span.samples.iter().any(|(at, _)| *at == hour) {
                            span.samples.push((hour, Err(format!("hour {hour} of branch {branch} is not recorded"))));
                            span.first = span.first.min(hour);
                            span.last = span.last.max(hour);
                        }
                        self.keep(branch, span);
                        held().unwrap_or_else(|| Err(format!("hour {hour} of branch {branch} is not recorded")))
                    }
                }
            }
        };
        roster?
            .eligible(slot)
            .ok_or_else(|| format!("the daily air roster at hour {hour} lacks country slot {slot}"))
    }
}

/// A sampler asked on behalf of one search, which `superseded` may stop between reads.
struct Asking<'a>(&'a Sampler, &'a dyn Fn() -> bool);

impl RosterSamples for Asking<'_> {
    fn first_hour(&self, branch: BranchId) -> Option<Hour> {
        self.0
            .replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| start.first_hour)
    }

    fn eligible(&self, branch: BranchId, hour: Hour, slot: u32) -> Result<bool, String> {
        self.0.eligible(branch, hour, slot, self.1)
    }
}

/// The recording's daily air rosters read on the asking thread, for an hour builder that
/// answers within the hour it builds; samples kept as the sampler bounds them.
pub struct RosterHistory(Arc<Sampler>);

impl RosterHistory {
    /// Another reader of `sampler`'s history.
    pub fn of(sampler: Arc<Sampler>) -> Self {
        Self(sampler)
    }

    pub fn sampler(&self) -> &Arc<Sampler> {
        &self.0
    }

    /// Whether the daily pass at `pass` visited the country in `slot`
    /// ([`derive::pass_visited`]). A pass at or before the branch's first sample counts as not
    /// visiting: every row read since already holds it ([`derive::visiting_pass`]).
    pub fn visited(&self, branch: BranchId, pass: Hour, slot: u32) -> Result<bool, String> {
        let asking = Asking(&self.0, &|| false);
        let first = asking
            .first_hour(branch)
            .ok_or_else(|| format!("branch {branch} is not recorded"))?;
        if pass <= first {
            return Ok(false);
        }
        derive::pass_visited(
            pass,
            asking.eligible(branch, pass - 1, slot)?,
            asking.eligible(branch, pass, slot)?,
        )
    }

    /// The latest daily pass of `branch` at or before `hour` that visited the country in
    /// `slot`, read on this thread.
    pub fn visiting_pass(&self, branch: BranchId, hour: Hour, slot: u32) -> Result<Option<Hour>, String> {
        visiting_pass(&Asking(&self.0, &|| false), branch, hour, slot)
    }
}

type Key = (BranchId, Hour, u32);
type Answer = Result<Option<Hour>, String>;

/// The worker's demand: only the latest question is searched; an older one still
/// unanswered is dropped and searched again if the hover asks for it again.
#[derive(Default)]
struct Demand {
    latest: Option<Key>,
    /// The question the worker is searching.
    searching: Option<Key>,
    closed: bool,
}

struct Shared {
    samples: Arc<Sampler>,
    /// Whether the search in hand is still the latest demand.
    superseded: Box<dyn Fn() -> bool + Send + Sync>,
    answers: Mutex<BTreeMap<Key, Answer>>,
    demand: Arc<(Mutex<Demand>, Condvar)>,
    /// Advances with every answer, so a held hover built while one was pending rebuilds.
    generation: AtomicU64,
    /// Signalled after each answer advances the generation.
    answered: (Mutex<()>, Condvar),
    /// Why the worker stopped while the service was open; every later question gets
    /// this refusal instead of waiting for an answer that cannot come.
    stopped: Mutex<Option<String>>,
    wake: Box<dyn Fn() + Send + Sync>,
    /// Makes the next search panic, to stop the worker as a failed read would.
    #[cfg(test)]
    fail: AtomicBool,
}

impl Shared {
    /// Search the latest demand, if any, and keep its answer unless a newer demand or
    /// closing superseded it. False once the service is closed.
    fn run_one(&self) -> bool {
        #[cfg(test)]
        if self.fail.load(Ordering::Acquire) {
            panic!("injected air roster worker failure");
        }
        let key = {
            let (demand, _) = &*self.demand;
            let mut demand = demand
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if demand.closed {
                return false;
            }
            match demand.latest {
                Some(key) => {
                    demand.searching = Some(key);
                    key
                }
                None => return true,
            }
        };
        let (branch, hour, slot) = key;
        let answer = visiting_pass(&self.asking(), branch, hour, slot);
        let (demand, _) = &*self.demand;
        let mut demand = demand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        demand.searching = None;
        if demand.closed {
            return false;
        }
        if answer.as_ref().err().map(String::as_str) == Some(SUPERSEDED) {
            return true;
        }
        if demand.latest == Some(key) {
            demand.latest = None;
        }
        drop(demand);
        self.keep(key, answer);
        self.advance();
        true
    }

    fn asking(&self) -> Asking<'_> {
        Asking(&self.samples, &*self.superseded)
    }

    /// Keeps `answer`, the newest per (branch, slot): what is held does not grow with the
    /// hours a hover is asked at.
    fn keep(&self, key: Key, answer: Answer) {
        let mut answers = self
            .answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        answers.retain(|&(branch, _, slot), _| (branch, slot) != (key.0, key.2));
        answers.insert(key, answer);
    }

    /// Failed spawns must stop the already-started service, or waiters would hang.
    fn launched(&self, spawned: std::io::Result<()>) -> Result<(), String> {
        spawned.map_err(|error| {
            let reason = format!("the air roster worker failed to start: {error}");
            *self
                .stopped
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason.clone());
            self.advance();
            reason
        })
    }

    /// Count an answer, signal waiters and wake the hover.
    fn advance(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        {
            let (gate, signal) = &self.answered;
            let _held = gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            signal.notify_all();
        }
        (self.wake)();
    }
}

/// Held by the worker thread. If the worker ends while the service is still open (a
/// panicking read, or any other exit), it records why and advances the generation, so a
/// hover held while pending rebuilds with the refusal and a waiter returns.
struct Exit(Arc<Shared>);
impl Drop for Exit {
    fn drop(&mut self) {
        let closed = self
            .0
            .demand
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed;
        if closed {
            return;
        }
        *self
            .0
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(
            if std::thread::panicking() {
                "the air roster worker panicked"
            } else {
                "the air roster worker stopped"
            }
            .into(),
        );
        self.0.advance();
    }
}

/// The roster history service: a hover asks for a visiting pass and gets its answer once
/// a lowest-priority worker has read the recording's blocks, so drawing decodes nothing.
/// The worker searches only the latest question, stops between block reads when a newer
/// one or closing supersedes it, and ends when the service is dropped.
pub struct PassRosters {
    shared: Arc<Shared>,
    started: AtomicBool,
    spawn: bool,
}

impl PassRosters {
    /// Rosters of `replay`; `wake` is called after each answer so the hover is asked again.
    pub fn new(replay: Arc<Replay>, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_worker(replay, wake, true)
    }

    /// Another reader of `sampler`'s history, with a worker of its own.
    pub fn of(sampler: Arc<Sampler>, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_sampler(sampler, wake, true)
    }

    fn with_worker(
        replay: Arc<Replay>,
        wake: impl Fn() + Send + Sync + 'static,
        spawn: bool,
    ) -> Self {
        Self::with_sampler(Arc::new(Sampler::today(replay, None)), wake, spawn)
    }

    fn with_sampler(
        samples: Arc<Sampler>,
        wake: impl Fn() + Send + Sync + 'static,
        spawn: bool,
    ) -> Self {
        let demand: Arc<(Mutex<Demand>, Condvar)> = Arc::default();
        let superseded = {
            let demand = Arc::clone(&demand);
            // The search in hand must still be the latest demand.
            move || {
                let demand = demand
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                demand.closed || demand.latest != demand.searching
            }
        };
        Self {
            shared: Arc::new(Shared {
                samples,
                superseded: Box::new(superseded),
                answers: Mutex::default(),
                demand,
                generation: AtomicU64::new(0),
                answered: Default::default(),
                stopped: Mutex::default(),
                wake: Box::new(wake),
                #[cfg(test)]
                fail: AtomicBool::new(false),
            }),
            started: AtomicBool::new(false),
            spawn,
        }
    }

    pub fn sampler(&self) -> &Arc<Sampler> {
        &self.shared.samples
    }

    /// How many answers have completed.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }

    /// Blocks until an answer completes after `generation` answers had, or the worker
    /// stops, which counts as an answer refusing every question.
    pub fn wait_beyond(&self, generation: u64) {
        let (gate, signal) = &self.shared.answered;
        let mut held = gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while self.generation() <= generation {
            held = signal
                .wait(held)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn start(&self) -> Result<(), String> {
        if !self.spawn || self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("air roster history".into())
            .spawn_carried(move || {
                let _exit = Exit(Arc::clone(&shared));
                viewer_foundation::worker_priority::lowest_priority();
                loop {
                    {
                        let (demand, signal) = &*shared.demand;
                        let mut held = demand
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        while held.latest.is_none() && !held.closed {
                            held = signal
                                .wait(held)
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                        }
                    }
                    if !shared.run_one() {
                        return;
                    }
                }
            })
            .map(drop);
        self.shared.launched(spawned)
    }
}

impl Drop for PassRosters {
    fn drop(&mut self) {
        let (demand, signal) = &*self.shared.demand;
        demand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed = true;
        signal.notify_all();
    }
}

impl Rosters for PassRosters {
    fn visiting_pass(
        &self,
        branch: BranchId,
        hour: Hour,
        slot: u32,
    ) -> Option<Result<Option<Hour>, String>> {
        let key = (branch, hour, slot);
        if let Some(answer) = self
            .shared
            .answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return Some(answer.clone());
        }
        if let Some(reason) = self
            .shared
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Some(Err(reason));
        }
        if let Err(reason) = self.start() {
            return Some(Err(reason));
        }
        let (demand, signal) = &*self.shared.demand;
        let mut demand = demand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if demand.latest != Some(key) {
            demand.latest = Some(key);
            signal.notify_all();
        }
        None
    }
}

/// Each cause's tooltip keys, ours and the enemy's.
const KEYS: [(u32, &str, &str); 9] = [
    (
        0,
        "AIR_VIEW_DAMAGE_TO_BUILDINGS_BY_US",
        "AIR_VIEW_DAMAGE_TO_BUILDINGS_BY_ENEMY",
    ),
    (
        1,
        "AIR_VIEW_DAMAGE_TO_SHIPS_BY_US",
        "AIR_VIEW_DAMAGE_TO_SHIPS_BY_ENEMY",
    ),
    (
        2,
        "AIR_VIEW_DAMAGE_TO_DIVISIONS_BY_US",
        "AIR_VIEW_DAMAGE_TO_DIVISIONS_BY_ENEMY",
    ),
    (
        3,
        "AIR_VIEW_DAMAGE_DISRUPTED_PLANES_BY_US",
        "AIR_VIEW_DAMAGE_DISRUPTED_PLANES_BY_ENEMY",
    ),
    (
        4,
        "AIR_VIEW_DAMAGE_FROM_PLANES_BY_US",
        "AIR_VIEW_DAMAGE_FROM_PLANES_BY_ENEMY",
    ),
    (
        5,
        "AIR_VIEW_DAMAGE_FROM_AA_BY_US",
        "AIR_VIEW_DAMAGE_FROM_AA_BY_ENEMY",
    ),
    (
        6,
        "AIR_VIEW_DAMAGE_TO_LOGISTICS_BY_US",
        "AIR_VIEW_DAMAGE_TO_LOGISTICS_BY_ENEMY",
    ),
    (
        7,
        "AIR_VIEW_DAMAGE_TO_RAILWAY_GUNS_BY_US",
        "AIR_VIEW_DAMAGE_TO_RAILWAY_GUNS_BY_ENEMY",
    ),
    (
        10,
        "AIR_VIEW_DAMAGE_FROM_TRAINS_BY_US",
        "AIR_VIEW_DAMAGE_FROM_TRAINS_BY_ENEMY",
    ),
];

/// Fixed-point rounding: half a unit away from zero, then
/// the integer part, as an `i32`.
fn fixed_round(value: i64) -> i32 {
    let biased = if value < 0 {
        value.wrapping_sub(50_000)
    } else {
        value.wrapping_add(50_000)
    };
    (biased / 100_000) as i32
}

/// Received-damage value text: causes 0, 1, 2, 6 and 7 through
/// fixed5 formatting with precision 2, cause 3 through `AIRWING_DISRUPTED_VAL`'s
/// `VAL`, every other cause rounded to an integer.
fn value(context: &Context<'_>, item: derive::Item) -> Result<String, String> {
    let text = match item.cause {
        0 | 1 | 2 | 6 | 7 => installed_content_core::fixed5::fixed5_to_string(item.amount, 2),
        3 => context.loc(
            "AIRWING_DISRUPTED_VAL",
            &[("VAL", Arg::Fixed5(item.amount))],
        )?,
        _ => fixed_round(item.amount).to_string(),
    };
    let colour = if item.ours() { 'G' } else { 'R' };
    Ok(format!("\u{11}{colour}{text}\u{11}!"))
}

/// One grid line: the tooltip's first line, then the value.
fn item_line(context: &Context<'_>, item: derive::Item) -> Result<String, String> {
    let (_, ours, theirs) = KEYS
        .iter()
        .find(|(cause, _, _)| *cause == item.cause)
        .ok_or_else(|| {
            format!(
                "no strategic air view tooltip for damage cause {}",
                item.cause
            )
        })?;
    let label = context.loc(if item.ours() { ours } else { theirs }, &[])?;
    let label = label.split('\n').next().unwrap_or_default().to_owned();
    Ok(format!("{label}: {}", value(context, item)?))
}

/// The viewing country's holder of the province's strategic region at the
/// frame's hour, or `None` when the region or the holder has no records.
fn retained<'a>(
    context: &'a Context<'_>,
    rows: &'a [replay_data::rows::AirRegionCombatRow],
) -> Result<Option<derive::Retained<'a>>, String> {
    let Some(region) = context
        .content
        .region_of
        .get(&(context.province.id))
        .copied()
    else {
        return Ok(None);
    };
    let ours = context.vision.ours();
    let Some(row) = rows
        .iter()
        .find(|row| row.region == region && context.slot(row.country) == Ok(ours))
    else {
        return Ok(None);
    };
    let timeout = *context
        .content
        .province_hover
        .air_log_timeout
        .as_ref()
        .map_err(Clone::clone)?;
    // Until the worker answers, the hover shows no statistics; the cache rebuilds it then.
    let Some(pass) = context
        .rosters
        .visiting_pass(context.frame.branch, context.frame.hour, ours)
    else {
        return Ok(None);
    };
    Ok(Some(derive::retained_through(row, pass?, timeout)))
}

/// The latest daily pass of `branch` at or before `hour` that visited the country in
/// `slot`, from the roster samples on either side of each pass's tick.
fn visiting_pass(
    samples: &dyn RosterSamples,
    branch: BranchId,
    hour: Hour,
    slot: u32,
) -> Result<Option<Hour>, String> {
    let first = samples
        .first_hour(branch)
        .ok_or_else(|| format!("branch {branch} is not recorded"))?;
    derive::visiting_pass(hour, first, |pass| {
        derive::pass_visited(
            pass,
            samples.eligible(branch, pass - 1, slot)?,
            samples.eligible(branch, pass, slot)?,
        )
    })
}

pub(super) fn append(context: &Context<'_>, out: &mut Tooltip) {
    let rows = match context.frame.complete::<tables::AirRegionCombat>() {
        Ok(rows) => rows,
        Err(e) => return line(out, "air region statistics", 0, Err(e.to_string())),
    };
    let state = match retained(context, rows) {
        Ok(Some(state)) => state,
        Ok(None) => return,
        Err(reason) => return line(out, "air region statistics", 0, Err(reason)),
    };
    for (enemy, header) in [
        (false, "AIR_VIEW_SHOW_OUR_MISSIONS"),
        (true, "AIR_VIEW_SHOW_ENEMY_MISSIONS"),
    ] {
        let grids = match derive::grids(state, context.frame.hour, enemy) {
            Ok(grids) => grids,
            Err(reason) => {
                line(out, "air region statistics", 0, Err(reason.into()));
                continue;
            }
        };
        if grids.air.is_empty() && grids.ground.is_empty() {
            continue;
        }
        line(
            out,
            "air region statistics header",
            0,
            context
                .loc(header, &[])
                .map(|text| text.split('\n').next().unwrap_or_default().to_owned()),
        );
        for item in grids.air.into_iter().chain(grids.ground) {
            line(
                out,
                "air region statistics line",
                1,
                item_line(context, item),
            );
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Eligibility by hour for slot 5 on branch 1, which starts at hour 24.
    struct Fixture(BTreeMap<Hour, bool>);
    impl RosterSamples for Fixture {
        fn first_hour(&self, branch: BranchId) -> Option<Hour> {
            (branch == 1).then_some(24)
        }
        fn eligible(&self, _: BranchId, hour: Hour, slot: u32) -> Result<bool, String> {
            if slot != 5 {
                return Err("other slot".into());
            }
            self.0
                .get(&hour)
                .copied()
                .ok_or_else(|| format!("hour {hour} not recorded"))
        }
    }

    /// A country without a state now, which last owned one at the pass of hour 72, is
    /// pruned by that pass; a country the branch's passes never visited keeps its row; a
    /// gap or a roster that changed within a pass's tick refuses.
    #[test]
    fn the_latest_pass_that_visited_the_country_comes_from_the_recorded_roster() {
        let hour = 24 * 5 + 7;
        let eligible = |until: Hour| Fixture((23..=hour).map(|at| (at, at <= until)).collect());
        assert_eq!(visiting_pass(&eligible(80), 1, hour, 5), Ok(Some(72)));
        assert_eq!(visiting_pass(&eligible(hour), 1, hour, 5), Ok(Some(120)));
        assert_eq!(visiting_pass(&eligible(0), 1, hour, 5), Ok(None));
        // Eligibility lost in the tick of the pass at 96 leaves that pass unknown.
        assert!(visiting_pass(&eligible(95), 1, hour, 5).is_err());
        let mut gap = eligible(0);
        gap.0.remove(&71);
        assert_eq!(
            visiting_pass(&gap, 1, hour, 5),
            Err("hour 71 not recorded".into())
        );
        assert!(visiting_pass(&eligible(80), 2, hour, 5).is_err());
    }

    /// A recording of branch 1 from hour 20 to 130 whose roster holds country 7 (slot 5)
    /// until hour `until`, and lacks the hours in `missing`.
    pub(in super::super) fn recording(until: Hour, missing: &[Hour]) -> Arc<Replay> {
        use replay::spool::Record;
        use replay_data::{
            identity::Identity, identity::Roster as Verified, identity::SessionStart,
            identity::Verification, rows::AirDailyRosterRow, rows::CampaignRow,
            rows::CountryLinkRow, rows::TableRows,
        };
        use replay_frame::{
            branches::BranchReason, branches::BranchStart, branches::Parentage,
            frame::FrameEncoder, frame::Metrics, frame::TableView,
        };
        let mut records = vec![
            Record::SessionStart(SessionStart {
                id: 1,
                process_id: 1,
                process_start_time: 1,
                qpc_frequency: 10_000_000,
                sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
                capture_mode: replay_data::identity::CaptureMode::Full,
                identity: Identity {
                    game_version: "synthetic".into(),
                    exe_hash: [1; 32],
                    exe_size: 1,
                    vanilla_hash: [2; 32],
                    dlc: Vec::new(),
                    mods: Vec::new(),
                    coverage_policy: 1,
                    language: "l_english".into(),
                    recorder_version: "test".into(),
                    collector_hash: [3; 32],
                    verification: Verification::Verified {
                        launcher: Verified::default(),
                        memory: Verified::default(),
                    },
                },
            }),
            Record::BranchStart(BranchStart {
                id: 1,
                session: 1,
                loaded_hour: Some(19),
                first_hour: 20,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
        ];
        let mut encoder = FrameEncoder::new();
        for hour in (20..=130).filter(|hour| !missing.contains(hour)) {
            let mut tables: [TableView; replay_data::rows::TableId::ALL.len()] =
                std::array::from_fn(|_| TableView::Unavailable {
                    reason: "outside the roster fixture".into(),
                });
            for rows in [
                TableRows::Campaign(vec![CampaignRow {
                    id: b"campaign".to_vec(),
                }]),
                TableRows::CountryLinks(vec![CountryLinkRow {
                    id: 7,
                    canonical_slot: 5,
                }]),
                TableRows::AirDailyRoster(vec![AirDailyRosterRow {
                    country: 7,
                    eligible: hour <= until,
                }]),
            ] {
                let index = rows.table().index();
                tables[index] = TableView::Complete(rows);
            }
            let frame = replay_frame::frame::DecodedFrame {
                branch: 1,
                hour,
                tables: tables.map(Into::into),
                metrics: Metrics::default(),
            };
            records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
        }
        Arc::new(Replay::from_records(b"campaign".to_vec(), &[records]).unwrap())
    }

    /// The answer of `rosters` once its worker has it; the first question is pending.
    fn answered(rosters: &PassRosters, hour: Hour, slot: u32) -> Answer {
        assert_eq!(
            rosters.visiting_pass(1, hour, slot),
            None,
            "asked on the worker"
        );
        let started = std::time::Instant::now();
        loop {
            if let Some(answer) = rosters.visiting_pass(1, hour, slot) {
                return answer;
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(30));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// The hover's question is answered on the worker from the recording's real blocks:
    /// the first ask returns pending without decoding, and the answer then wakes the
    /// hover and advances the generation.
    #[test]
    fn the_worker_answers_from_the_recorded_roster_blocks() {
        let woken = Arc::new(AtomicU64::new(0));
        let wake = {
            let woken = Arc::clone(&woken);
            move || {
                woken.fetch_add(1, Ordering::AcqRel);
            }
        };
        let rosters = PassRosters::new(recording(80, &[]), wake);
        assert_eq!(answered(&rosters, 127, 5), Ok(Some(72)));
        assert_eq!(rosters.generation(), 1);
        assert_eq!(woken.load(Ordering::Acquire), 1);
        // A held answer is returned without asking the worker again.
        assert_eq!(rosters.visiting_pass(1, 127, 5), Some(Ok(Some(72))));
        assert_eq!(rosters.generation(), 1);
        // Never eligible since the branch's first sample: no pass prunes the row.
        let never = PassRosters::new(recording(0, &[]), || {});
        assert_eq!(answered(&never, 127, 5), Ok(None));
        // A missing sample before any visit refuses by name.
        let gap = PassRosters::new(recording(0, &[95]), || {});
        assert_eq!(
            answered(&gap, 127, 5),
            Err("hour 95 of branch 1 is not recorded".into())
        );
        // A country the roster lacks refuses by name.
        assert!(
            answered(&rosters, 127, 6)
                .unwrap_err()
                .contains("lacks country slot 6")
        );
    }

    /// A worker that stops before answering (here a panicking search) still releases a
    /// waiter: the stop advances the generation, wakes the hover, and every question from
    /// then on refuses by name instead of staying pending.
    #[test]
    fn a_worker_that_stops_unanswered_refuses_instead_of_leaving_the_hover_pending() {
        let woken = Arc::new(AtomicU64::new(0));
        // The stop signals waiters before it wakes the hover, so the wake has its own signal.
        let (rang, rung) = std::sync::mpsc::channel();
        let wake = {
            let woken = Arc::clone(&woken);
            move || {
                woken.fetch_add(1, Ordering::AcqRel);
                rang.send(()).unwrap();
            }
        };
        let rosters = Arc::new(PassRosters::new(recording(80, &[]), wake));
        rosters.shared.fail.store(true, Ordering::Release);
        assert_eq!(
            rosters.visiting_pass(1, 127, 5),
            None,
            "asked on the worker"
        );
        let (done, waited) = std::sync::mpsc::channel();
        let waiter = {
            let rosters = Arc::clone(&rosters);
            job_priority::spawn(move || {
                rosters.wait_beyond(0);
                done.send(()).unwrap();
            })
        };
        waited
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the waiter returns once the worker stops");
        waiter.join().unwrap();
        assert_eq!(rosters.generation(), 1);
        rung.recv_timeout(std::time::Duration::from_secs(30))
            .expect("the stop wakes the hover");
        assert_eq!(woken.load(Ordering::Acquire), 1);
        let refusal = Some(Err("the air roster worker panicked".to_owned()));
        assert_eq!(rosters.visiting_pass(1, 127, 5), refusal);
        assert_eq!(rosters.visiting_pass(1, 103, 5), refusal);
    }

    #[test]
    fn a_worker_that_cannot_start_refuses_every_later_question() {
        let rosters = PassRosters::with_worker(recording(80, &[]), || {}, false);
        let reason = "the air roster worker failed to start: no threads".to_owned();
        assert_eq!(
            rosters
                .shared
                .launched(Err(std::io::Error::other("no threads"))),
            Err(reason.clone())
        );
        rosters.wait_beyond(0);
        assert_eq!(rosters.generation(), 1);
        assert_eq!(rosters.visiting_pass(1, 127, 5), Some(Err(reason.clone())));
        assert_eq!(rosters.visiting_pass(1, 103, 5), Some(Err(reason)));
    }

    /// Only the latest question is searched: an older one still pending is dropped, not
    /// queued behind, and a search stops before its next block once superseded.
    #[test]
    fn a_newer_question_supersedes_pending_and_running_searches() {
        let rosters = PassRosters::with_worker(recording(0, &[]), || {}, false);
        assert_eq!(rosters.visiting_pass(1, 127, 5), None);
        assert_eq!(rosters.visiting_pass(1, 103, 5), None);
        assert!(rosters.shared.run_one());
        assert_eq!(rosters.visiting_pass(1, 103, 5), Some(Ok(None)));
        // The older question was never searched; asking again demands it afresh.
        assert_eq!(rosters.visiting_pass(1, 127, 5), None);
        assert_eq!(
            rosters.shared.demand.0.lock().unwrap().latest,
            Some((1, 127, 5))
        );
        // A search whose demand is withdrawn stops before reading another block.
        let fresh = PassRosters::with_worker(recording(0, &[]), || {}, false);
        assert_eq!(fresh.visiting_pass(1, 127, 5), None);
        fresh.shared.demand.0.lock().unwrap().searching = Some((1, 127, 5));
        assert_eq!(fresh.visiting_pass(1, 103, 5), None);
        assert_eq!(
            visiting_pass(&fresh.shared.asking(), 1, 127, 5),
            Err(SUPERSEDED.into())
        );
        assert_eq!(fresh.shared.samples.blocks.load(Ordering::Acquire), 0);
        // Closing ends the worker without keeping an answer.
        let closed = PassRosters::with_worker(recording(0, &[]), || {}, false);
        assert_eq!(closed.visiting_pass(1, 127, 5), None);
        let shared = Arc::clone(&closed.shared);
        drop(closed);
        assert!(!shared.run_one());
        assert!(shared.answers.lock().unwrap().is_empty());
    }

    /// Unchanged rosters share one snapshot however long the searched history.
    #[test]
    fn unchanged_rosters_share_one_snapshot() {
        let rosters = PassRosters::with_worker(recording(80, &[]), || {}, false);
        let samples = &rosters.shared.samples;
        assert_eq!(rosters.visiting_pass(1, 127, 5), None);
        assert!(rosters.shared.run_one());
        assert_eq!(rosters.visiting_pass(1, 127, 5), Some(Ok(Some(72))));
        let held = samples.hours.lock().unwrap();
        assert!(held.len() >= 6, "{}", held.len());
        // Eligible and no longer eligible: two snapshots.
        assert_eq!(samples.distinct.lock().unwrap().len(), 2);
        let first = held[&(1, 23)].as_ref().unwrap();
        let later = held[&(1, 48)].as_ref().unwrap();
        assert!(Arc::ptr_eq(first, later));
    }

    #[test]
    fn values_round_as_the_native_helpers_do() {
        assert_eq!(fixed_round(149_999), 1);
        assert_eq!(fixed_round(150_000), 2);
        assert_eq!(fixed_round(-150_000), -2);
        assert_eq!(fixed_round(-149_999), -1);
        assert_eq!(fixed_round(0), 0);
        // Every cause the grids list has a tooltip pair; 8 and 9 never show.
        for cause in [0, 1, 2, 3, 4, 5, 6, 7, 10] {
            assert!(KEYS.iter().any(|(listed, _, _)| *listed == cause));
        }
        assert!(
            !KEYS
                .iter()
                .any(|(listed, _, _)| *listed == 9 || *listed == 8)
        );
    }
}
