use job_priority::SpawnCarried as _;
use super::{Arg, Context, line};
use replay::campaign::Replay;
use replay_recording::columnar_natives::{Cursor, Natives};
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

pub trait RosterSamples {
    fn first_hour(&self, branch: BranchId) -> Option<Hour>;
    fn eligible(&self, branch: BranchId, hour: Hour, slot: u32) -> Result<bool, String>;
}

pub trait Rosters {
    fn visiting_pass(
        &self,
        branch: BranchId,
        hour: Hour,
        slot: u32,
    ) -> Option<Result<Option<Hour>, String>>;
}

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

pub const SUPERSEDED: &str = "the air roster search was superseded";

pub struct Span {
    pub first: Hour,
    pub last: Hour,
    pub samples: Vec<(Hour, Result<Roster, String>)>,
}

pub fn sample_hour(hour: Hour) -> bool {
    matches!(hour.rem_euclid(derive::DAY), 0 | 23)
}

pub trait SampleSource: Send + Sync {
    fn span(&self, branch: BranchId, hour: Hour, superseded: &dyn Fn() -> bool) -> Result<Span, String>;
}

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
            Ok(true) => only(cursor.build().map_err(|error| error.to_string()).and_then(|decoded| roster(&decoded))),
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

const KEPT: usize = 8;

pub struct Sampler {
    replay: Arc<Replay>,
    columnar: Option<Arc<Natives>>,
    source: Box<dyn SampleSource>,
    hours: Mutex<BTreeMap<(BranchId, Hour), Sample>>,
    spans: Mutex<VecDeque<(BranchId, Hour, Hour)>>,
    distinct: Mutex<HashSet<Arc<Roster>>>,
    blocks: AtomicUsize,
    reading: Mutex<()>,
}

impl Sampler {
    pub fn new(replay: Arc<Replay>, source: Box<dyn SampleSource>) -> Self {
        Self::with(replay, None, source)
    }

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

pub struct RosterHistory(Arc<Sampler>);

impl RosterHistory {
    pub fn of(sampler: Arc<Sampler>) -> Self {
        Self(sampler)
    }

    pub fn sampler(&self) -> &Arc<Sampler> {
        &self.0
    }

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

    pub fn visiting_pass(&self, branch: BranchId, hour: Hour, slot: u32) -> Result<Option<Hour>, String> {
        visiting_pass(&Asking(&self.0, &|| false), branch, hour, slot)
    }
}

type Key = (BranchId, Hour, u32);
type Answer = Result<Option<Hour>, String>;

#[derive(Default)]
struct Demand {
    latest: Option<Key>,
    searching: Option<Key>,
    closed: bool,
}

struct Shared {
    samples: Arc<Sampler>,
    superseded: Box<dyn Fn() -> bool + Send + Sync>,
    answers: Mutex<BTreeMap<Key, Answer>>,
    demand: Arc<(Mutex<Demand>, Condvar)>,
    generation: AtomicU64,
    answered: (Mutex<()>, Condvar),
    stopped: Mutex<Option<String>>,
    wake: Box<dyn Fn() + Send + Sync>,
    #[cfg(test)]
    fail: AtomicBool,
}

impl Shared {
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

    fn keep(&self, key: Key, answer: Answer) {
        let mut answers = self
            .answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        answers.retain(|&(branch, _, slot), _| (branch, slot) != (key.0, key.2));
        answers.insert(key, answer);
    }

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

pub struct PassRosters {
    shared: Arc<Shared>,
    started: AtomicBool,
    spawn: bool,
}

impl PassRosters {
    pub fn new(replay: Arc<Replay>, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_worker(replay, wake, true)
    }

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

    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }

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
                viewer_runtime::worker_priority::lowest_priority();
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

fn fixed_round(value: i64) -> i32 {
    let biased = if value < 0 {
        value.wrapping_sub(50_000)
    } else {
        value.wrapping_add(50_000)
    };
    (biased / 100_000) as i32
}

fn value(context: &Context<'_>, item: derive::Item) -> Result<String, String> {
    let text = match item.cause {
        0 | 1 | 2 | 6 | 7 => content_reader_parser::fixed5::fixed5_to_string(item.amount, 2),
        3 => context.loc(
            "AIRWING_DISRUPTED_VAL",
            &[("VAL", Arg::Fixed5(item.amount))],
        )?,
        _ => fixed_round(item.amount).to_string(),
    };
    let colour = if item.ours() { 'G' } else { 'R' };
    Ok(format!("\u{11}{colour}{text}\u{11}!"))
}

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
    let Some(pass) = context
        .rosters
        .visiting_pass(context.frame.branch, context.frame.hour, ours)
    else {
        return Ok(None);
    };
    Ok(Some(derive::retained_through(row, pass?, timeout)))
}

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

    #[test]
    fn the_latest_pass_that_visited_the_country_comes_from_the_recorded_roster() {
        let hour = 24 * 5 + 7;
        let eligible = |until: Hour| Fixture((23..=hour).map(|at| (at, at <= until)).collect());
        assert_eq!(visiting_pass(&eligible(80), 1, hour, 5), Ok(Some(72)));
        assert_eq!(visiting_pass(&eligible(hour), 1, hour, 5), Ok(Some(120)));
        assert_eq!(visiting_pass(&eligible(0), 1, hour, 5), Ok(None));
        assert!(visiting_pass(&eligible(95), 1, hour, 5).is_err());
        let mut gap = eligible(0);
        gap.0.remove(&71);
        assert_eq!(
            visiting_pass(&gap, 1, hour, 5),
            Err("hour 71 not recorded".into())
        );
        assert!(visiting_pass(&eligible(80), 2, hour, 5).is_err());
    }

    pub(in super::super) fn recording(until: Hour, missing: &[Hour]) -> Arc<Replay> {
        use replay_capture_spool::spool::Record;
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
        Arc::new(Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &[records]).unwrap())
    }

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
        assert_eq!(rosters.visiting_pass(1, 127, 5), Some(Ok(Some(72))));
        assert_eq!(rosters.generation(), 1);
        let never = PassRosters::new(recording(0, &[]), || {});
        assert_eq!(answered(&never, 127, 5), Ok(None));
        let gap = PassRosters::new(recording(0, &[95]), || {});
        assert_eq!(
            answered(&gap, 127, 5),
            Err("hour 95 of branch 1 is not recorded".into())
        );
        assert!(
            answered(&rosters, 127, 6)
                .unwrap_err()
                .contains("lacks country slot 6")
        );
    }

    #[test]
    fn a_worker_that_stops_unanswered_refuses_instead_of_leaving_the_hover_pending() {
        let woken = Arc::new(AtomicU64::new(0));
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

    #[test]
    fn a_newer_question_supersedes_pending_and_running_searches() {
        let rosters = PassRosters::with_worker(recording(0, &[]), || {}, false);
        assert_eq!(rosters.visiting_pass(1, 127, 5), None);
        assert_eq!(rosters.visiting_pass(1, 103, 5), None);
        assert!(rosters.shared.run_one());
        assert_eq!(rosters.visiting_pass(1, 103, 5), Some(Ok(None)));
        assert_eq!(rosters.visiting_pass(1, 127, 5), None);
        assert_eq!(
            rosters.shared.demand.0.lock().unwrap().latest,
            Some((1, 127, 5))
        );
        let fresh = PassRosters::with_worker(recording(0, &[]), || {}, false);
        assert_eq!(fresh.visiting_pass(1, 127, 5), None);
        fresh.shared.demand.0.lock().unwrap().searching = Some((1, 127, 5));
        assert_eq!(fresh.visiting_pass(1, 103, 5), None);
        assert_eq!(
            visiting_pass(&fresh.shared.asking(), 1, 127, 5),
            Err(SUPERSEDED.into())
        );
        assert_eq!(fresh.shared.samples.blocks.load(Ordering::Acquire), 0);
        let closed = PassRosters::with_worker(recording(0, &[]), || {}, false);
        assert_eq!(closed.visiting_pass(1, 127, 5), None);
        let shared = Arc::clone(&closed.shared);
        drop(closed);
        assert!(!shared.run_one());
        assert!(shared.answers.lock().unwrap().is_empty());
    }

    #[test]
    fn unchanged_rosters_share_one_snapshot() {
        let rosters = PassRosters::with_worker(recording(80, &[]), || {}, false);
        let samples = &rosters.shared.samples;
        assert_eq!(rosters.visiting_pass(1, 127, 5), None);
        assert!(rosters.shared.run_one());
        assert_eq!(rosters.visiting_pass(1, 127, 5), Some(Ok(Some(72))));
        let held = samples.hours.lock().unwrap();
        assert!(held.len() >= 6, "{}", held.len());
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
