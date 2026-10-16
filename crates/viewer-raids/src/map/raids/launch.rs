//! Whether an offer is launchable (bit 15), enabling its frame:
//!
//! 1. Affordable: command power within capacity, plus nukes and every
//!    essential archetype in the stockpile.
//! 2. Off cooldown.
//! 3. `available`.
//! 4. Some country unit matches the type.
//! 5. Some source and unit can reach the target.
//!
//! Units, sources and range limits depend only on the country and type, so
//! they are prepared once per type; a target then costs one distance per source.
//! Every table a gate reads is indexed at most once per country and hour, on
//! first use and only for the rows of the country it reads.
//!
//! A [`Launcher`] holds what the hour's evaluations share: those indexes and
//! each type's preparation, built once whichever thread asks first. What a
//! target's search reads and writes as it goes (per-search answers, reach and
//! air distances, and the path searches' asked pairs) is a [`Local`]'s, one
//! per thread's part of a batch, so a part takes no lock per target.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, OnceLock},
};
use viewer_foundation::map::memo::Recent;
use viewer_foundation::map::memo::TableKey;
use viewer_foundation::parallel::Beside;
use viewer_foundation::parallel::Workers;

use installed_content::raids::{
    EquipmentRequirement, RaidType, SourceKind, StartingPoint, UnitRequirement,
};
use replay::rows::{
    AceRangeInputRow, AirBaseRow, AirWingRow, CountryCommandPowerRow, CountryRow, DivisionRow,
    DivisionTemplateIconRow, NukeRow, RaidAirSiteRow, RaidCountryInputRow, RaidPathDistanceRow,
    RaidScope, ReservedCountryNamingRow, ReservedLaunchSources, TableId, TableSchema, TaskForceRow,
    VariantDefinitionRow, tables,
};

pub use super::paths::Asked;
use super::{
    RaidAssets,
    script::{Eval, Scopes, World},
};
use viewer_foundation::map::air_range::{self, finite, registered, to_map};

/// A naval-base or building source (kinds 2 and 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathSource {
    pub kind: u8,
    pub province: u32,
}

/// The path searches this module does not reconstruct. A batch's parts
/// on other threads share the hour's ([`Launcher`]); each part's view of
/// them is its own ([`Local`]).
pub trait RaidPaths {
    /// Distance for a naval-base or building source: `Some(fixed5)`, or `None`
    /// when no distance is found. `asked` says whether it was certainly queried here.
    fn distance(
        &self,
        world: &World<'_>,
        source: &PathSource,
        target: &RaidScope,
        asked: Asked,
    ) -> Result<Option<i64>, String>;
    /// The game asked, or may have asked, a distance whose answer is not
    /// needed: the session memo keeps its first kind all the same.
    fn note(&self, source: &PathSource, target: &RaidScope, asked: Asked);
    /// The recorded session memo in (source, target) order; `None` when it
    /// is not recorded.
    fn recorded_rows(&self) -> Option<&[RaidPathDistanceRow]>;
    /// The province a path source of `kind` measures `target` to.
    fn path_target(&self, kind: u8, target: &RaidScope) -> Result<Option<u32>, String>;
    /// Whether the division (`Divisions.id`) can move to the source province.
    fn army_reaches(&self, world: &World<'_>, division: u64, source_province: u32) -> Eval;
    /// The hour's searches, when a batch may be split across threads with
    /// them ([`super::paths::PartPaths`]); `None` evaluates every batch in
    /// order on one thread.
    fn hour(&self) -> Option<&super::paths::HourPaths<'_>> {
        None
    }
}

/// Answers no path search, for tests of the other gates.
#[cfg(test)]
pub struct Unreconstructed;

#[cfg(test)]
impl RaidPaths for Unreconstructed {
    fn distance(
        &self,
        _: &World<'_>,
        _: &PathSource,
        _: &RaidScope,
        _: Asked,
    ) -> Result<Option<i64>, String> {
        Err("path search not reconstructed".into())
    }
    fn note(&self, _: &PathSource, _: &RaidScope, _: Asked) {}
    fn recorded_rows(&self) -> Option<&[RaidPathDistanceRow]> {
        None
    }
    fn path_target(&self, _: u8, _: &RaidScope) -> Result<Option<u32>, String> {
        Err("path search not reconstructed".into())
    }
    fn army_reaches(&self, _: &World<'_>, _: u64, _: u32) -> Eval {
        Eval::unknown("path search not reconstructed")
    }
}

/// `(x * 100000) / 300000`, the raid distance scale, or `0xFFFFFFFF`.
fn third(value: i64) -> i64 {
    if finite(value) {
        value.wrapping_mul(100_000) / 300_000
    } else {
        0xffff_ffff
    }
}

fn table<'a, T: TableSchema>(world: &World<'a>) -> Result<&'a [T::Row], String> {
    world
        .frame()
        .complete::<T>()
        .map_err(|error| error.to_string())
}

/// An index built on first use; a failed build answers every later use.
type Lazy<T> = OnceLock<Result<T, String>>;

fn lazy<T>(cell: &Lazy<T>, build: impl FnOnce() -> Result<T, String>) -> Result<&T, String> {
    cell.get_or_init(build).as_ref().map_err(Clone::clone)
}

/// A raid unit: an army or an air wing.
#[derive(Clone, Copy, Debug)]
enum Unit<'a> {
    Division(&'a DivisionRow),
    Wing(&'a AirWingRow),
}

impl Unit<'_> {
    fn id(self) -> u64 {
        match self {
            Self::Division(division) => division.id,
            Self::Wing(wing) => wing.id,
        }
    }
}

/// A unit's range limit; `None` means no limit.
type Limit = Option<i64>;

/// Wider limit: `None` (no limit) is widest.
fn wider(a: Limit, b: Limit) -> Limit {
    match (a, b) {
        (None, _) | (_, None) => None,
        (Some(a), Some(b)) => Some(a.max(b)),
    }
}

/// A listed unit, whether it is proved to match, and its range limit.
type Candidate<'a> = (Unit<'a>, Eval, Result<Limit, String>);

/// One source: where it is, whether it is visited, and for an air site the
/// base and the country's container capacity there. Which listed unit can
/// reach it is asked only for a unit whose range admits the target
/// ([`Launcher::usable`]).
struct Source<'a> {
    province: Result<u32, String>,
    path: Option<PathSource>,
    site: Option<(&'a AirBaseRow, Option<i64>)>,
    valid: Eval,
    /// A source that may exist but whose inputs are unknown.
    unknown: Option<String>,
}

impl Source<'_> {
    fn unknown(reason: String) -> Self {
        Self {
            province: Err(reason.clone()),
            path: None,
            site: None,
            valid: Eval::unknown(reason.clone()),
            unknown: Some(reason),
        }
    }
}

/// A type's country-only gates, sources and listed units.
struct Prepared<'a> {
    resources: Eval,
    units: Eval,
    /// Shared by the types with the same starting point.
    sources: Arc<Sources<'a>>,
    /// Each listed unit, whether it is proved to match, and its range limit.
    candidates: Vec<Candidate<'a>>,
    /// `unlimited_ai_range` with an AI-controlled country.
    bypass: Eval,
    /// Shared with the types whose source search reads the same inputs.
    shared: Arc<Shared>,
    /// The type's path sources grouped for the recorded memo, built on first
    /// use and shared by the types with the same sources and widest limit.
    recorded: OnceLock<Option<Arc<Recorded>>>,
    /// The memo's index for them and the type's `max_distance`, shared
    /// across hours; `None` when the rows read are not the frame's memo
    /// table.
    index: OnceLock<Option<Arc<TargetIndexes>>>,
}

/// What the types with the same sources, listed units (each with its match
/// and limit), AI bypass and `max_distance` share, since
/// [`Launcher::feasible`] reads nothing else of the type. Its answers are a
/// [`Local`]'s ([`Memo`]), by `id`.
#[derive(Default)]
struct Shared {
    /// The search's position among the launcher's.
    id: usize,
    /// No source is a path source: the search then reads only the target
    /// province and asks the memo nothing, so each target province's
    /// answer is kept.
    pathless: bool,
    /// For a search with neither path sources, an AI bypass nor an
    /// unbounded range: the sources that can be in range of a target.
    near: Option<Near>,
}

/// One [`Shared`] search's answers so far in a [`Local`].
#[derive(Default)]
struct Memo {
    /// Cache by (source, candidate), on first need: army reach is the gate's
    /// costliest input.
    reach: HashMap<(usize, usize), Eval>,
    /// Each target province's answer, for a pathless search.
    answers: HashMap<Option<u32>, Eval>,
}

/// What one thread's part of a batch keeps as it evaluates targets: the hour's
/// path searches as the part sees them, each type's preparation it has
/// looked up, its searches' answers and its air distances. Every answer here
/// depends on the hour alone, except the path searches' asked pairs, which
/// `paths` keeps in the order the part asks them.
pub struct Local<'l, 'a> {
    paths: &'l dyn RaidPaths,
    prepared: RefCell<BTreeMap<String, Arc<Prepared<'a>>>>,
    memos: RefCell<Vec<Memo>>,
    /// Map distances between an air source and a target province, which
    /// every type from air sources measures.
    distances: RefCell<HashMap<(u32, u32), Distance>>,
}

impl<'l, 'a> Local<'l, 'a> {
    /// A part's memos, with `paths` its view of the hour's path searches.
    pub fn new(paths: &'l dyn RaidPaths) -> Self {
        Self {
            paths,
            prepared: RefCell::default(),
            memos: RefCell::default(),
            distances: RefCell::default(),
        }
    }

    fn memo<R>(&self, shared: &Shared, read: impl FnOnce(&mut Memo) -> R) -> R {
        let mut memos = self.memos.borrow_mut();
        if memos.len() <= shared.id {
            memos.resize_with(shared.id + 1, Memo::default);
        }
        read(&mut memos[shared.id])
    }
}

/// A search's sources by province and its range, `max_distance` or the
/// widest unit limit. A source farther than that is unusable for every unit
/// and asks nothing, so the search visits only the others; without path
/// sources it reads no memo, so which of the others it visits first changes
/// neither its answer nor anything it records.
struct Near {
    range: i64,
    /// Sources measured from a nonzero province, by province.
    by_province: HashMap<u32, Vec<usize>>,
    /// The other sources, always visited.
    always: Vec<usize>,
    distances: Arc<SourceDistances>,
}

/// Map distances from a set of source provinces to each target province,
/// nearest first: geometry only, so shared by every hour with the same set.
pub(crate) struct SourceDistances {
    provinces: Vec<u32>,
    by_target: Mutex<HashMap<u32, Arc<Nearest>>>,
}

/// Source provinces by distance (saturated to `u32`) and those without a
/// map centroid.
struct Nearest {
    measured: Vec<(u32, u32)>,
    unmeasured: Vec<u32>,
}

impl SourceDistances {
    fn nearest(&self, assets: &RaidAssets, target: u32) -> Arc<Nearest> {
        let mut by_target = self
            .by_target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(by_target.entry(target).or_insert_with(|| {
            let mut nearest = Nearest {
                measured: Vec::new(),
                unmeasured: Vec::new(),
            };
            for &province in &self.provinces {
                match assets.geometry.wrapped_distance(province, target) {
                    Some(pixels) => nearest
                        .measured
                        .push((u32::try_from(third(pixels)).unwrap_or(u32::MAX), province)),
                    None => nearest.unmeasured.push(province),
                }
            }
            nearest.measured.sort_unstable();
            Arc::new(nearest)
        }))
    }
}

/// Recent [`SourceDistances`], by source provinces.
pub(crate) struct SourceDistanceSets {
    recent: Recent<Vec<u32>, SourceDistances>,
}

impl Default for SourceDistanceSets {
    fn default() -> Self {
        Self {
            // Air bases change seldom; carrier provinces every hour.
            recent: Recent::new(4),
        }
    }
}

/// Which [`Shared`] a type's search uses: its sources (by identity), each
/// listed unit (kind and id) with its match and limit, its AI bypass and its
/// `max_distance`.
struct SharedKey<'a> {
    sources: Arc<Sources<'a>>,
    candidates: Vec<(bool, u64, Eval, Result<Limit, String>)>,
    bypass: Eval,
    max_distance: Option<i64>,
}

impl<'a> SharedKey<'a> {
    fn of(kind: &RaidType, prepared: &Prepared<'a>) -> Self {
        Self {
            sources: Arc::clone(&prepared.sources),
            candidates: prepared
                .candidates
                .iter()
                .map(|(unit, matched, limit)| {
                    (
                        matches!(unit, Unit::Wing(_)),
                        unit.id(),
                        matched.clone(),
                        limit.clone(),
                    )
                })
                .collect(),
            bypass: prepared.bypass.clone(),
            max_distance: kind.max_distance_raw,
        }
    }

    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sources, &other.sources)
            && self.candidates == other.candidates
            && self.bypass == other.bypass
            && self.max_distance == other.max_distance
    }
}

/// A source list's path sources grouped by (kind, source province), each
/// with its source indices, and the widest unit limit, which with a type's
/// `max_distance` bounds the range. `None` in [`Prepared::recorded`] when a
/// path source cannot be answered from the memo (a null province or an
/// undecided visit).
struct Recorded {
    /// Kind, province, source indices, and the province's rows in the memo.
    provinces: Vec<(u8, u32, Vec<usize>, std::ops::Range<usize>)>,
    /// The air and unknown sources, which every target visits.
    others: Vec<usize>,
    widest: Limit,
}

/// The [`Recorded`] groups of the hour's source lists, by list (identity)
/// and widest limit.
type RecordedGroups<'a> = Vec<(
    Arc<Sources<'a>>,
    Limit,
    Arc<OnceLock<Option<Arc<Recorded>>>>,
)>;

/// Per kind (naval, building): each target province's in-range path
/// sources, as positions in [`Recorded::provinces`], or `None` when the memo
/// lacks a pair of that kind to it. Filled a target at a time, as targets
/// are evaluated, and kept while the memo table is the same. A missing
/// target is computed outside the lock: its positions depend on the memo
/// and the sources alone, so a part that computes them again finds the same.
type TargetIndexes = Mutex<[HashMap<u32, Option<Arc<[u32]>>>; 2]>;

/// What a [`TargetIndexes`] is built from, every input it reads: the memo
/// table's identity, the path sources' (kind, province) in order, the
/// widest unit limit and the type's `max_distance`.
#[derive(Clone, PartialEq)]
struct IndexKey {
    memo: TableKey,
    provinces: Vec<(u8, u32)>,
    widest: Limit,
    max_distance: Option<i64>,
}

/// The position of target `to` in `rows`, one source's memo rows in target
/// order, searched outward from `hint`: sources asked of the same targets
/// hold a target at the same position, so the last source's usually answers.
pub(super) fn target_row(rows: &[RaidPathDistanceRow], to: u32, hint: usize) -> Option<usize> {
    let hint = hint.min(rows.len().checked_sub(1)?);
    let at = rows[hint].target;
    let (low, high) = if at == to {
        return Some(hint);
    } else if at < to {
        let (mut low, mut step) = (hint + 1, 1);
        loop {
            let probe = hint + step;
            if probe >= rows.len() {
                break (low, rows.len());
            }
            if rows[probe].target >= to {
                break (low, probe + 1);
            }
            (low, step) = (probe + 1, step * 2);
        }
    } else {
        let (mut high, mut step) = (hint, 1);
        loop {
            if step > hint {
                break (0, high);
            }
            let probe = hint - step;
            if rows[probe].target <= to {
                break (probe, high);
            }
            (high, step) = (probe, step * 2);
        }
    };
    rows[low..high]
        .binary_search_by_key(&to, |row| row.target)
        .ok()
        .map(|at| low + at)
}

/// Recent memo indexes. The session memo only grows, and a decoded hour that
/// leaves it unchanged shares its table with the hour before, so every hour
/// between two memo changes reuses one index.
pub(crate) struct RecordedIndexes {
    recent: Recent<IndexKey, TargetIndexes>,
}

impl Default for RecordedIndexes {
    fn default() -> Self {
        Self {
            // The distinct path source lists of a catalogue's types, and a
            // memo change's worth more.
            recent: Recent::new(16),
        }
    }
}

/// The launch gate for one country and hour, shared by the threads that
/// evaluate a batch's parts.
pub struct Launcher<'w, 'a> {
    assets: &'w RaidAssets,
    world: &'w World<'a>,
    /// The hour's path searches, as an evaluation outside a batch part
    /// sees them.
    paths: &'w (dyn RaidPaths + Sync),
    ours: u32,
    context: OnceLock<Result<Context<'w, 'a>, String>>,
    /// Each type's preparation by name: types are prepared side by side,
    /// each once, by the first part that needs it.
    prepared: Mutex<BTreeMap<String, Arc<OnceLock<Arc<Prepared<'a>>>>>>,
    shared: Mutex<Vec<(SharedKey<'a>, Arc<Shared>)>>,
    recorded: Mutex<RecordedGroups<'a>>,
    /// Our cooldowns by raid type name.
    cooldowns: Lazy<Cooldowns<'a>>,
    /// The types and divisions [`Self::prefetch`] has built for.
    prefetched: Mutex<(BTreeSet<String>, BTreeSet<u64>)>,
}

/// One task of [`Launcher::prefetch`].
enum Prefetch<'k> {
    /// The reachable components for armies that do or do not exfiltrate.
    Reachable(bool),
    /// A type's preparation and its recorded-memo group.
    Prepare(&'k RaidType),
}

/// Rows by owner slot, and (row, raw owner) of those whose non-empty owner
/// links to no slot.
type Owned = (BTreeMap<u32, Vec<u32>>, Vec<(u32, u32)>);

/// Actual Country0 differs from a positive reference linked to slot zero.
#[derive(Clone, Copy)]
enum LaunchCountry {
    Reserved,
    Linked(u32),
}

type LaunchMembers = Option<Vec<Result<LaunchCountry, String>>>;

/// Province building counts by type and province, and each content
/// state's buildings: (province, type, count) in the state's content
/// province order, a province's types ascending.
#[derive(Default)]
struct ProvinceBuildings {
    counts: BTreeMap<u32, BTreeMap<u32, usize>>,
    by_state: HashMap<u32, Vec<(u32, u32, usize)>>,
}

/// The whole-table indexes the source lists read, each kept while the
/// tables it reads are the same ([`viewer_foundation::map::memo::TableKey`]): the same
/// for every hour and viewing country that leaves those tables alone.
pub(crate) struct TableIndexes {
    /// From `NavalBases` and `CountryLinks`.
    naval_bases: Recent<TableKey, Owned>,
    /// From `States` and `CountryLinks`.
    owned_states: Recent<TableKey, Owned>,
    /// From `ProvinceBuildings` and the content's states.
    province_buildings: Recent<TableKey, ProvinceBuildings>,
}

impl Default for TableIndexes {
    fn default() -> Self {
        // A few hours in flight and a step back.
        Self {
            naval_bases: Recent::new(4),
            owned_states: Recent::new(4),
            province_buildings: Recent::new(4),
        }
    }
}

/// A measured distance: `None` when the game finds none.
type Distance = Result<Option<i64>, String>;

/// Our cooldowns by raid type content key, and those whose type token is
/// refused, with why: such a cooldown may be of any type.
type Cooldowns<'a> = (
    BTreeMap<String, Vec<&'a RaidScope>>,
    Vec<(String, &'a RaidScope)>,
);

impl<'w, 'a> Launcher<'w, 'a> {
    /// `paths` holds the hour's path searches.
    pub fn new(
        assets: &'w RaidAssets,
        world: &'w World<'a>,
        ours: u32,
        paths: &'w (dyn RaidPaths + Sync),
    ) -> Self {
        Self {
            assets,
            world,
            paths,
            ours,
            context: OnceLock::new(),
            prepared: Mutex::default(),
            shared: Mutex::default(),
            recorded: Mutex::default(),
            cooldowns: OnceLock::new(),
            prefetched: Mutex::default(),
        }
    }

    /// Builds on `workers`, before a batch's parts evaluate its launch
    /// gates, what every part would otherwise wait for in turn: each of
    /// `kinds`' preparation and recorded-memo group, one task per type, and
    /// the hour's reachable components for its listed divisions' army
    /// searches. Each is built once whoever asks first, from the hour alone,
    /// so what the parts evaluate does not change. What was built for an
    /// earlier batch of the hour is not asked again.
    ///
    /// A batch asks nearly all its reach of its types' first listed
    /// divisions, whose search runs as far as the nearest usable sources and
    /// cannot be split. While the tasks run, a pool thread that is free runs
    /// those searches ahead ([`viewer_foundation::map::movement_reads::Moves::run_ahead`]), so the
    /// parts find them run further; they stop with the tasks, since running
    /// them past what the batch asks costs several times the batch.
    pub fn prefetch(
        &self,
        kinds: &[&RaidType],
        hour: &super::paths::HourPaths<'_>,
        workers: &Workers,
    ) -> usize {
        let Ok(context) = self
            .context
            .get_or_init(|| Context::new(self.assets, self.world, self.ours))
        else {
            return 0;
        };
        let mut done = self
            .prefetched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let kinds: Vec<&RaidType> = kinds
            .iter()
            .copied()
            .filter(|kind| done.0.insert(kind.name.clone()))
            .collect();
        // The divisions a launch gate of these types may ask to reach a
        // source: the listed ones, where the country can afford the type;
        // and the first of each type's, which a gate asks first.
        let (mut divisions, mut leads) = (Vec::new(), Vec::new());
        for kind in &kinds {
            if context.resources(kind) == Eval::False {
                continue;
            }
            if let Ok(units) = &*context.shared_units(kind) {
                let mut first = true;
                for (unit, matched) in units {
                    if let Unit::Division(division) = unit
                        && *matched != Eval::False
                    {
                        if std::mem::take(&mut first) && !leads.contains(&division.id) {
                            leads.push(division.id);
                        }
                        if done.1.insert(division.id) {
                            divisions.push(division.id);
                        }
                    }
                }
            }
        }
        drop(done);
        let moves = hour.moves(self.world);
        // Types with the same starting point share one source list, which
        // the first to ask builds while the others wait: one type of each
        // starting point first, so the lists are built side by side.
        let mut starts: Vec<&StartingPoint> = Vec::new();
        let (first, rest): (Vec<&RaidType>, Vec<&RaidType>) = kinds.iter().partition(|kind| {
            let new = !starts.contains(&&kind.starting_point);
            if new {
                starts.push(&kind.starting_point);
            }
            new
        });
        let tasks: Vec<Prefetch<'_>> = moves
            .exfiltrates(divisions)
            .into_iter()
            .map(Prefetch::Reachable)
            .chain(first.into_iter().chain(rest).map(Prefetch::Prepare))
            .collect();
        let ahead = |done: &dyn Fn() -> bool| moves.run_ahead(&leads, done);
        let ahead: Option<&Beside<'_>> = (!leads.is_empty()).then_some(&ahead);
        workers.run_beside(
            tasks.len(),
            &|at| match tasks[at] {
                Prefetch::Reachable(exfiltrates) => moves.reachable(exfiltrates),
                Prefetch::Prepare(kind) => {
                    let local = self.local();
                    let prepared = self.prepare(&local, kind);
                    // As `Self::search` reads it first.
                    if prepared.bypass == Eval::False
                        && !prepared.shared.pathless
                        && let Some(Ok(sources)) = prepared.sources.cached()
                    {
                        prepared
                            .recorded
                            .get_or_init(|| self.recorded(&local, &prepared, sources));
                    }
                }
            },
            ahead,
        );
        tasks.len()
    }

    /// Memos for evaluations outside a batch part.
    pub fn local(&self) -> Local<'w, 'a> {
        Local::new(self.paths)
    }

    fn diagnostic_context(&self) -> Result<&Context<'w, 'a>, String> {
        self.context
            .get_or_init(|| Context::new(self.assets, self.world, self.ours))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Diagnostic mode asks equipment even after command power fails.
    /// Reuse the icon admission gate's stored operands.
    pub(super) fn command_power(&self, kind: &RaidType) -> Eval {
        match self.diagnostic_context() {
            Ok(c) => c.command_power(kind),
            Err(e) => Eval::unknown(e),
        }
    }
    pub(super) fn stockpile(&self, name: &str) -> Result<i32, String> {
        self.diagnostic_context()?.essential_available(name)
    }
    pub(super) fn nuke_count(&self, kind: &RaidType) -> Result<i32, String> {
        self.diagnostic_context()?.nuke_count(kind)
    }
    pub(super) fn valid_units(&self, local: &Local<'_, 'a>, kind: &RaidType) -> Eval {
        self.prepare(local, kind).units.clone()
    }
    pub(super) fn reachable(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        scope: &RaidScope,
    ) -> Eval {
        self.feasible(
            local,
            kind,
            &self.prepare(local, kind),
            scope,
            Asked::Certain,
        )
    }

    /// [`Self::launchable_in`] with memos of its own.
    #[cfg(test)]
    pub fn launchable(
        &self,
        kind: &RaidType,
        scope: &RaidScope,
        scopes: Scopes,
        asked: Asked,
    ) -> Eval {
        self.launchable_in(&self.local(), kind, scope, scopes, asked)
    }

    /// Launchable bit 15 for this type and target. Run path searches only after
    /// all earlier gates pass. `asked` says whether this row is certainly evaluated;
    /// an undecided shown bit leaves its searches maybe asked. `local` holds the
    /// evaluating part's memos and path searches.
    pub fn launchable_in(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        scope: &RaidScope,
        scopes: Scopes,
        asked: Asked,
    ) -> Eval {
        let prepared = self.prepare(local, kind);
        let gates = prepared
            .resources
            .clone()
            .and(|| self.cooldown(kind, scope))
            .and(|| self.world.evaluate(&kind.available, scopes))
            .and(|| prepared.units.clone());
        match gates {
            Eval::False => Eval::False,
            Eval::True => self.feasible(local, kind, &prepared, scope, asked),
            // Whether the game searched is unknown, and so is the answer:
            // the searches are skipped, and the pairs the game may have
            // asked are noted so that a later query of another kind is
            // decided only when both kinds agree.
            Eval::Unknown(reasons) => {
                self.note_all(local, kind, &prepared, scope);
                Eval::Unknown(reasons)
            }
        }
    }

    fn prepare(&self, local: &Local<'_, 'a>, kind: &RaidType) -> Arc<Prepared<'a>> {
        if let Some(prepared) = local.prepared.borrow().get(&kind.name) {
            return Arc::clone(prepared);
        }
        let cell = Arc::clone(
            self.prepared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(kind.name.clone())
                .or_default(),
        );
        let prepared = Arc::clone(cell.get_or_init(|| Arc::new(self.prepare_uncached(kind))));
        local
            .prepared
            .borrow_mut()
            .insert(kind.name.clone(), Arc::clone(&prepared));
        prepared
    }

    fn prepare_uncached(&self, kind: &RaidType) -> Prepared<'a> {
        let context = self
            .context
            .get_or_init(|| Context::new(self.assets, self.world, self.ours));
        let mut prepared = match context {
            Ok(context) => context.prepare(kind),
            Err(reason) => Prepared {
                resources: Eval::unknown(reason.clone()),
                units: Eval::unknown(reason.clone()),
                sources: Arc::new(Sources::Cached(Err(reason.clone()))),
                candidates: Vec::new(),
                bypass: Eval::unknown(reason.clone()),
                shared: Arc::default(),
                recorded: OnceLock::new(),
                index: OnceLock::new(),
            },
        };
        let key = SharedKey::of(kind, &prepared);
        let mut searches = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let known = searches
            .iter()
            .find(|(other, _)| other.same(&key))
            .map(|(_, shared)| Arc::clone(shared));
        prepared.shared = known.unwrap_or_else(|| {
            let pathless = prepared.sources.cached().is_some_and(|sources| {
                sources
                    .as_ref()
                    .is_ok_and(|sources| sources.iter().all(|source| source.path.is_none()))
            });
            let shared = Arc::new(Shared {
                id: searches.len(),
                pathless,
                near: pathless.then(|| self.near(kind, &prepared)).flatten(),
            });
            searches.push((key, Arc::clone(&shared)));
            shared
        });
        prepared
    }

    /// [`Near`] for a search without path sources, or `None` when every
    /// source may be in range.
    fn near(&self, kind: &RaidType, prepared: &Prepared<'a>) -> Option<Near> {
        let sources = prepared.sources.cached()?.as_ref().ok()?;
        if prepared.bypass != Eval::False {
            return None;
        }
        // The widest unit limit; an undecided limit counts as none.
        let widest = prepared
            .candidates
            .iter()
            .try_fold(Some(0), |widest: Limit, (_, _, limit)| {
                limit.as_ref().ok().map(|limit| wider(widest, *limit))
            })
            .unwrap_or(None);
        let range = match (kind.max_distance_raw, widest) {
            (None, None) => return None,
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
        };
        let mut by_province: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut always = Vec::new();
        for (at, source) in sources.iter().enumerate() {
            match (&source.unknown, &source.province) {
                (None, Ok(province)) if *province != 0 => {
                    by_province.entry(*province).or_default().push(at);
                }
                _ => always.push(at),
            }
        }
        let mut provinces: Vec<u32> = by_province.keys().copied().collect();
        provinces.sort_unstable();
        let distances = self
            .assets
            .source_distances
            .recent
            .get_or_insert_with(provinces.clone(), || {
                Ok::<_, ()>(Arc::new(SourceDistances {
                    provinces,
                    by_target: Default::default(),
                }))
            })
            .ok()?;
        Some(Near {
            range,
            by_province,
            always,
            distances,
        })
    }

    /// Visit prepared sources and units in order. Measure each visited source on
    /// its first unit, whether any unit can reach it or not. Query unit reach
    /// only once the target is in range, stopping at the first usable source.
    fn feasible(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        prepared: &Prepared<'a>,
        scope: &RaidScope,
        asked: Asked,
    ) -> Eval {
        let sources = match &*prepared.sources {
            Sources::Cached(Ok(sources)) => sources,
            Sources::Cached(Err(reason)) => return Eval::unknown(reason.clone()),
            Sources::Walk => return self.search_walk(local, kind, prepared, scope, asked),
        };
        let target = self.assets.province(scope);
        let shared = &prepared.shared;
        if shared.pathless {
            if let Some(known) = local.memo(shared, |memo| memo.answers.get(&target).cloned()) {
                return known;
            }
            let result = self.search(local, kind, prepared, sources, scope, target, asked);
            local.memo(shared, |memo| memo.answers.insert(target, result.clone()));
            return result;
        }
        self.search(local, kind, prepared, sources, scope, target, asked)
    }

    /// [`Self::feasible`]'s search over the sources.
    #[allow(clippy::too_many_arguments)]
    fn search(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        prepared: &Prepared<'a>,
        sources: &[Source<'a>],
        scope: &RaidScope,
        target: Option<u32>,
        asked: Asked,
    ) -> Eval {
        let bypass = &prepared.bypass;
        let pathless = prepared.shared.pathless;
        // With every path pair recorded, only in-range path sources can be
        // usable, and measuring the rest neither searches nor memoises.
        // Without path sources every source is visited.
        let near = prepared
            .shared
            .near
            .as_ref()
            .zip(target)
            .map(|(near, target)| (near, near.distances.nearest(self.assets, target)));
        let visited: Box<dyn Iterator<Item = usize> + '_> = match &near {
            // The sources that can be in range, nearest first.
            Some((near, nearest)) => Box::new(
                near.always.iter().copied().chain(
                    nearest
                        .measured
                        .iter()
                        .take_while(|(distance, _)| i64::from(*distance) <= near.range)
                        .map(|(_, province)| province)
                        .chain(&nearest.unmeasured)
                        .flat_map(|province| near.by_province[province].iter().copied()),
                ),
            ),
            None => match (*bypass == Eval::False && !pathless)
                .then(|| self.recorded_visits(local, kind, prepared, sources, scope))
                .flatten()
            {
                Some(visited) => Box::new(visited.into_iter()),
                None => Box::new(0..sources.len()),
            },
        };
        let mut asked = asked;
        let mut result = Eval::False;
        for at in visited {
            let source = &sources[at];
            let usable =
                self.search_one(local, kind, prepared, at, source, scope, target, &mut asked);
            result = result.or(|| usable);
            if result == Eval::True {
                break;
            }
        }
        result
    }

    fn search_walk(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        prepared: &Prepared<'a>,
        scope: &RaidScope,
        mut asked: Asked,
    ) -> Eval {
        let context = match self.context.get() {
            Some(Ok(context)) => context,
            Some(Err(reason)) => return Eval::unknown(reason.clone()),
            None => return Eval::unknown("the launch gate's inputs are not indexed"),
        };
        let target = self.assets.province(scope);
        let mut result = Eval::False;
        let mut at = 0;
        context.visit_sources(kind, &mut |source| {
            // This is a deterministic memo position, including refused family
            // boundaries; it is never exported as a native source ordinal.
            let usable = self.search_one(
                local, kind, prepared, at, &source, scope, target, &mut asked,
            );
            at += 1;
            result = std::mem::replace(&mut result, Eval::False).or(|| usable);
            result != Eval::True
        });
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn search_one(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        prepared: &Prepared<'a>,
        at: usize,
        source: &Source<'a>,
        scope: &RaidScope,
        target: Option<u32>,
        asked: &mut Asked,
    ) -> Eval {
        // Do not visit a busy carrier.
        if source.valid == Eval::False {
            return Eval::False;
        }
        let bypass = &prepared.bypass;
        let usable = match &source.unknown {
            Some(reason) => Eval::unknown(reason.clone()),
            None => {
                let this = if source.valid == Eval::True && !matches!(bypass, Eval::Unknown(_)) {
                    *asked
                } else {
                    Asked::Maybe
                };
                let distance = (*bypass != Eval::True)
                    .then(|| self.distance(local, source, scope, target, this));
                self.usable(local, kind, prepared, at, source, distance)
            }
        };
        if matches!(usable, Eval::Unknown(_)) {
            // The game may have stopped here.
            *asked = Asked::Maybe;
        }
        usable
    }

    /// The sources [`Self::feasible`] must visit for `scope` when the
    /// recorded memo holds the pair of every path source: the air and
    /// unknown sources and the in-range path sources, in source order. A
    /// path source out of range is unusable for every unit, its reach is
    /// never asked, and its recorded pair answers every later query of it,
    /// so leaving it out changes nothing. `None` when the memo lacks a pair.
    fn recorded_visits(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        prepared: &Prepared<'a>,
        sources: &[Source<'a>],
        scope: &RaidScope,
    ) -> Option<Vec<usize>> {
        let recorded = prepared
            .recorded
            .get_or_init(|| self.recorded(local, prepared, sources))
            .as_ref()?;
        let memo = local.paths.recorded_rows()?;
        let within = |distance: i64| {
            kind.max_distance_raw.is_none_or(|max| distance <= max)
                && recorded.widest.is_none_or(|limit| distance <= limit)
        };
        let index = prepared
            .index
            .get_or_init(|| {
                let frame = self.world.frame();
                // The index keys on the frame's memo table: the rows read must
                // be that table's.
                frame
                    .complete::<tables::RaidPathDistances>()
                    .ok()
                    .filter(|table| std::ptr::eq(table.as_ptr(), memo.as_ptr()))?;
                let key = IndexKey {
                    memo: TableKey::of(frame, &[replay::rows::TableId::RaidPathDistances]),
                    provinces: recorded
                        .provinces
                        .iter()
                        .map(|(kind, province, _, _)| (*kind, *province))
                        .collect(),
                    widest: recorded.widest,
                    max_distance: kind.max_distance_raw,
                };
                self.assets
                    .recorded_indexes
                    .recent
                    .get_or_insert_with(key, || Ok::<_, ()>(Arc::default()))
                    .ok()
            })
            .as_ref()?;
        let mut out = recorded.others.clone();
        for (at, path_kind) in [super::paths::NAVAL_BASE, super::paths::BUILDING]
            .into_iter()
            .enumerate()
        {
            if !recorded
                .provinces
                .iter()
                .any(|(known, _, _, _)| *known == path_kind)
            {
                continue;
            }
            // No target province measures none and asks nothing.
            let Some(to) = local.paths.path_target(path_kind, scope).ok()? else {
                continue;
            };
            let lock = || {
                index
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            };
            let known = lock()[at].get(&to).cloned();
            let positions = known.unwrap_or_else(|| {
                let positions = (|| {
                    let (mut positions, mut hint) = (Vec::new(), 0);
                    for (position, (_, _, _, range)) in recorded
                        .provinces
                        .iter()
                        .enumerate()
                        .filter(|(_, (known, _, _, _))| *known == path_kind)
                    {
                        let rows = &memo[range.clone()];
                        hint = target_row(rows, to, hint)?;
                        let row = &rows[hint];
                        if row.ok != 0 && within(row.distance) {
                            positions.push(position as u32);
                        }
                    }
                    Some(Arc::from(positions))
                })();
                lock()[at].entry(to).or_insert(positions).clone()
            });
            for &position in positions.as_deref()? {
                out.extend(&recorded.provinces[position as usize].2);
            }
        }
        out.sort_unstable();
        Some(out)
    }

    /// The type's path sources grouped by (kind, province), once per source
    /// list, widest limit and hour.
    fn recorded(
        &self,
        local: &Local<'_, 'a>,
        prepared: &Prepared<'a>,
        sources: &[Source<'a>],
    ) -> Option<Arc<Recorded>> {
        // The widest unit limit; an undecided limit counts as none.
        let widest = prepared
            .candidates
            .iter()
            .try_fold(Some(0), |widest: Limit, (_, _, limit)| {
                limit.as_ref().ok().map(|limit| wider(widest, *limit))
            })
            .unwrap_or(None);
        let group = {
            let mut groups = self
                .recorded
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let known = groups
                .iter()
                .find(|(list, limit, _)| Arc::ptr_eq(list, &prepared.sources) && *limit == widest)
                .map(|(_, _, group)| Arc::clone(group));
            known.unwrap_or_else(|| {
                let group = Arc::default();
                groups.push((Arc::clone(&prepared.sources), widest, Arc::clone(&group)));
                group
            })
        };
        group
            .get_or_init(|| Self::group(local, sources, widest).map(Arc::new))
            .clone()
    }

    /// [`Self::recorded`]'s groups.
    fn group(local: &Local<'_, 'a>, sources: &[Source<'a>], widest: Limit) -> Option<Recorded> {
        let memo = local.paths.recorded_rows()?;
        let mut by_province: BTreeMap<(u8, u32), Vec<usize>> = BTreeMap::new();
        for (at, source) in sources.iter().enumerate() {
            if let Some(path) = &source.path {
                // A null province measures zero without the memo.
                if path.province == 0 || source.valid != Eval::True || !matches!(path.kind, 2 | 5) {
                    return None;
                }
                by_province
                    .entry((path.kind, path.province))
                    .or_default()
                    .push(at);
            }
        }
        Some(Recorded {
            provinces: by_province
                .into_iter()
                .map(|((kind, province), indices)| {
                    let start = memo.partition_point(|row| row.source < province);
                    let end = start + memo[start..].partition_point(|row| row.source == province);
                    (kind, province, indices, start..end)
                })
                .collect(),
            others: sources
                .iter()
                .enumerate()
                .filter(|(_, source)| source.path.is_none())
                .map(|(at, _)| at)
                .collect(),
            widest,
        })
    }

    /// Whether some listed unit is in range of the target from `source`
    /// (`distance`, `None` under an unlimited AI range) and can reach it.
    fn usable(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        prepared: &Prepared<'a>,
        at: usize,
        source: &Source<'a>,
        distance: Option<Result<Option<i64>, String>>,
    ) -> Eval {
        let bypass = &prepared.bypass;
        let within = |limit: Limit| match &distance {
            None => Eval::False,
            Some(Err(reason)) => Eval::unknown(reason.clone()),
            Some(Ok(None)) => Eval::False,
            Some(Ok(Some(d))) => Eval::from_bool(
                kind.max_distance_raw.is_none_or(|max| *d <= max)
                    && limit.is_none_or(|limit| *d <= limit),
            ),
        };
        let mut usable = Eval::False;
        for (index, (unit, matched, limit)) in prepared.candidates.iter().enumerate() {
            let in_range = match limit {
                Ok(limit) => bypass.clone().or(|| within(*limit)),
                Err(reason) => bypass
                    .clone()
                    .or(|| within(None).and(|| Eval::unknown(reason.clone()))),
            };
            let unit_usable = source
                .valid
                .clone()
                .and(|| matched.clone())
                .and(|| in_range)
                .and(|| self.reach(local, prepared, at, index, *unit, source));
            usable = usable.or(|| unit_usable);
            if usable == Eval::True {
                break;
            }
        }
        usable
    }

    /// Source reach, once per source and unit.
    fn reach(
        &self,
        local: &Local<'_, 'a>,
        prepared: &Prepared<'a>,
        at: usize,
        index: usize,
        unit: Unit<'a>,
        source: &Source<'a>,
    ) -> Eval {
        let shared = &prepared.shared;
        let known = local.memo(shared, |memo| memo.reach.get(&(at, index)).cloned());
        known.unwrap_or_else(|| {
            let answer = self.reach_uncached(local, unit, source);
            local.memo(shared, |memo| {
                memo.reach.insert((at, index), answer.clone())
            });
            answer
        })
    }

    fn reach_uncached(&self, local: &Local<'_, 'a>, unit: Unit<'a>, source: &Source<'a>) -> Eval {
        match unit {
            Unit::Division(division) => match &source.province {
                Ok(0) => Eval::False,
                Ok(province) => local.paths.army_reaches(self.world, division.id, *province),
                Err(reason) => Eval::unknown(reason.clone()),
            },
            // Naval-base and building sources have no air base.
            Unit::Wing(wing) => match (source.site, self.context.get()) {
                (None, _) => Eval::False,
                (Some((base, capacity)), Some(Ok(context))) => {
                    context.wing_reaches(wing, base, capacity)
                }
                (Some(_), Some(Err(reason))) => Eval::unknown(reason.clone()),
                (Some(_), None) => Eval::unknown("the launch gate's inputs are not indexed"),
            },
        }
    }

    /// Notes every path pair the game may have measured for this target.
    fn note_all(
        &self,
        local: &Local<'_, 'a>,
        kind: &RaidType,
        prepared: &Prepared<'a>,
        scope: &RaidScope,
    ) {
        if prepared.bypass == Eval::True {
            return;
        }
        let note = |source: &Source<'a>| {
            if source.valid != Eval::False
                && let Some(path) = &source.path
            {
                local.paths.note(path, scope, Asked::Maybe);
            }
        };
        match &*prepared.sources {
            Sources::Cached(sources) => {
                for source in sources.iter().flatten() {
                    note(source);
                }
            }
            Sources::Walk => {
                if let Some(Ok(context)) = self.context.get() {
                    context.visit_sources(kind, &mut |source| {
                        note(&source);
                        true
                    });
                }
            }
        }
    }

    fn distance(
        &self,
        local: &Local<'_, 'a>,
        source: &Source<'a>,
        scope: &RaidScope,
        target: Option<u32>,
        asked: Asked,
    ) -> Result<Option<i64>, String> {
        if let Some(path) = &source.path {
            return local.paths.distance(self.world, path, scope, asked);
        }
        let from = source.province.clone()?;
        // A missing source province is distance zero; a missing target none.
        if from == 0 {
            return Ok(Some(0));
        }
        let Some(to) = target else {
            return Ok(None);
        };
        local
            .distances
            .borrow_mut()
            .entry((from, to))
            .or_insert_with(|| {
                self.assets
                    .geometry
                    .wrapped_distance(from, to)
                    .map(|pixels| Some(third(pixels)))
                    .ok_or_else(|| format!("province {from} or {to} has no map centroid"))
            })
            .clone()
    }
}

impl<'a> Launcher<'_, 'a> {
    /// Cooldown of this type with matching building, province, state and leader
    /// arms; ignore the auxiliary scope word.
    fn cooldown(&self, kind: &RaidType, scope: &RaidScope) -> Eval {
        let cooldowns = match lazy(&self.cooldowns, || {
            let mut out = Cooldowns::default();
            for row in table::<tables::RaidCooldowns>(self.world)? {
                if row.owner_slot != self.ours {
                    continue;
                }
                let catalog = &self.assets.catalog;
                let key = match self.assets.raid_type(self.world, row.raid_type) {
                    Ok(super::TypeToken::Type(index)) => catalog.types[index].name.clone(),
                    Ok(super::TypeToken::Refused(at)) => catalog.uncertain[at].name.clone(),
                    Err(reason) => {
                        out.1.push((reason, &row.scope));
                        continue;
                    }
                };
                out.0.entry(key).or_default().push(&row.scope);
            }
            Ok(out)
        }) {
            Ok(cooldowns) => cooldowns,
            Err(reason) => return Eval::unknown(reason),
        };
        let matches = |row: &RaidScope| {
            row.building_type == scope.building_type
                && (scope.building_type == 0
                    || (row.building_location == scope.building_location
                        && row.building_flags == scope.building_flags))
                && row.province == scope.province
                && row.state == scope.state
                && row.leader_reference == scope.leader_reference
        };
        let blocked = cooldowns
            .0
            .get(&kind.name)
            .into_iter()
            .flatten()
            .any(|row| matches(row));
        if blocked {
            return Eval::False;
        }
        // A refused cooldown on this target may be of this type.
        match cooldowns.1.iter().find(|(_, row)| matches(row)) {
            Some((reason, _)) => Eval::unknown(reason.clone()),
            None => Eval::True,
        }
    }
}

type Equipment = BTreeMap<u64, Vec<(u64, i64)>>;

/// The recorded rows the gates read, each indexed once per country and hour
/// on first use, and only for the rows the country's gates read.
struct Context<'w, 'a> {
    assets: &'w RaidAssets,
    world: &'w World<'a>,
    ours: u32,
    country: &'a CountryRow,
    /// Every raw country reference that links to `ours`.
    raws: BTreeSet<u32>,
    inputs: Lazy<BTreeMap<u32, &'a RaidCountryInputRow>>,
    bases: Lazy<BTreeMap<u64, &'a AirBaseRow>>,
    wings: Lazy<BTreeMap<u64, &'a AirWingRow>>,
    aces: Lazy<BTreeMap<u64, &'a AceRangeInputRow>>,
    templates: Lazy<BTreeMap<u64, &'a DivisionTemplateIconRow>>,
    definitions: Lazy<BTreeMap<u64, &'a VariantDefinitionRow>>,
    /// Our divisions, then wings, not held by a live raid, in table order.
    units: Lazy<Vec<Unit<'a>>>,
    /// Pool variants of each of our wings (hourly) and divisions (daily).
    wing_equipment: Lazy<Equipment>,
    division_equipment: Lazy<Equipment>,
    /// Installed module names of each variant.
    modules: Lazy<BTreeMap<u64, Vec<&'a [u8]>>>,
    /// Ships to their task forces.
    ships: Lazy<BTreeMap<u64, &'a TaskForceRow>>,
    /// Units held by a live raid and ships from which it launches. Assignment
    /// and source changes set matching back-references; ending clears both.
    held: Lazy<(BTreeSet<u64>, BTreeSet<u64>)>,
    /// Our nukes of each type, and our stockpile's archetype totals.
    nukes: Lazy<[Option<&'a NukeRow>; 2]>,
    archetypes: Lazy<BTreeMap<u32, i32>>,
    /// The best stockpiled transport plane's range.
    transport: Lazy<Option<i64>>,
    faction: Lazy<LaunchMembers>,
    reserved_launch: Lazy<&'a ReservedLaunchSources>,
    /// Each country's naval base provinces, in `NavalBases` order.
    naval_bases: Lazy<Arc<Owned>>,
    /// Province building counts by type and province, and their states.
    province_buildings: Lazy<Arc<ProvinceBuildings>>,
    /// Each country's owned states, ascending.
    owned_states: Lazy<Arc<Owned>>,
    /// Whether a state's recorded provinces are the content's
    /// ([`World::state_order`]), for the states the building sources visit.
    orderly: Mutex<HashMap<u32, bool>>,
    /// Our air and missile sites in their native order.
    air_sites: Lazy<Vec<&'a RaidAirSiteRow>>,
    demilitarized: Lazy<BTreeSet<u32>>,
    /// Volunteer links as (country, host) slots.
    volunteers: Lazy<BTreeSet<(Option<u32>, Option<u32>)>>,
    /// Sources by starting point and matched units by requirements: the
    /// vanilla catalogue's types share a handful of each, and both read only
    /// the country's hour and these fields.
    /// Each is built outside the list's lock, so types with other starting
    /// points or requirements are prepared side by side.
    sources_by_start: Mutex<Vec<(StartingPoint, Shared1<Sources<'a>>)>>,
    units_by_requirements: Mutex<Vec<(Vec<UnitRequirement>, Shared1<UnitList<'a>>)>>,
}

type SourceList<'a> = Result<Vec<Source<'a>>, String>;
/// Mixed dependency families must stop construction at the first usable
/// source. The existing shared handle still identifies their search memo.
enum Sources<'a> {
    Cached(SourceList<'a>),
    Walk,
}

impl<'a> Sources<'a> {
    fn cached(&self) -> Option<&SourceList<'a>> {
        match self {
            Self::Cached(sources) => Some(sources),
            Self::Walk => None,
        }
    }
}
/// A value built once, by whichever thread asks first.
type Shared1<T> = Arc<OnceLock<Arc<T>>>;
type UnitList<'a> = Result<Vec<(Unit<'a>, Eval)>, String>;

impl<'w, 'a> Context<'w, 'a> {
    fn new(assets: &'w RaidAssets, world: &'w World<'a>, ours: u32) -> Result<Self, String> {
        let country = table::<tables::Countries>(world)?
            .iter()
            .find(|row| row.canonical_slot == ours)
            .ok_or("the raiding country has no Countries row")?;
        Ok(Self {
            assets,
            world,
            ours,
            country,
            raws: world
                .links
                .iter()
                .filter(|&(_, &slot)| slot == ours)
                .map(|(&raw, _)| raw)
                .collect(),
            inputs: Lazy::new(),
            bases: Lazy::new(),
            wings: Lazy::new(),
            aces: Lazy::new(),
            templates: Lazy::new(),
            definitions: Lazy::new(),
            units: Lazy::new(),
            wing_equipment: Lazy::new(),
            division_equipment: Lazy::new(),
            modules: Lazy::new(),
            ships: Lazy::new(),
            held: Lazy::new(),
            nukes: Lazy::new(),
            archetypes: Lazy::new(),
            transport: Lazy::new(),
            faction: Lazy::new(),
            reserved_launch: Lazy::new(),
            naval_bases: Lazy::new(),
            province_buildings: Lazy::new(),
            owned_states: Lazy::new(),
            orderly: Mutex::default(),
            air_sites: Lazy::new(),
            demilitarized: Lazy::new(),
            volunteers: Lazy::new(),
            sources_by_start: Mutex::default(),
            units_by_requirements: Mutex::default(),
        })
    }

    fn ours(&self, raw: u32) -> bool {
        self.raws.contains(&raw)
    }

    fn inputs(&self, slot: u32) -> Result<&'a RaidCountryInputRow, String> {
        lazy(&self.inputs, || {
            Ok(table::<tables::RaidCountryInputs>(self.world)?
                .iter()
                .map(|row| (row.canonical_slot, row))
                .collect())
        })?
        .get(&slot)
        .copied()
        .ok_or_else(|| format!("country slot {slot} has no RaidCountryInputs row"))
    }

    /// The country's hourly command power operands and ace country modifiers
    /// (CountryCommandPower).
    fn command_power_row(&self, slot: u32) -> Result<&'a CountryCommandPowerRow, String> {
        let rows = table::<tables::CountryCommandPower>(self.world)?;
        rows.binary_search_by_key(&slot, |row| row.canonical_slot)
            .map(|at| &rows[at])
            .map_err(|_| format!("country slot {slot} has no CountryCommandPower row"))
    }

    fn bases(&self) -> Result<&BTreeMap<u64, &'a AirBaseRow>, String> {
        lazy(&self.bases, || {
            Ok(table::<tables::AirBases>(self.world)?
                .iter()
                .map(|row| (row.id, row))
                .collect())
        })
    }

    fn wing(&self, id: u64) -> Result<Option<&'a AirWingRow>, String> {
        Ok(lazy(&self.wings, || {
            Ok(table::<tables::AirWings>(self.world)?
                .iter()
                .map(|row| (row.id, row))
                .collect())
        })?
        .get(&id)
        .copied())
    }

    fn ace(&self, id: u64) -> Result<Option<&'a AceRangeInputRow>, String> {
        Ok(lazy(&self.aces, || {
            Ok(table::<tables::AceRangeInputs>(self.world)?
                .iter()
                .map(|row| (row.ace, row))
                .collect())
        })?
        .get(&id)
        .copied())
    }

    /// A template's hourly counter and model inputs, whose grouped sub-units
    /// the battalion requirements count.
    fn template(&self, id: u64) -> Result<Option<&'a DivisionTemplateIconRow>, String> {
        Ok(lazy(&self.templates, || {
            Ok(table::<tables::DivisionTemplateIcons>(self.world)?
                .iter()
                .map(|row| (row.template, row))
                .collect())
        })?
        .get(&id)
        .copied())
    }

    /// The variant dictionary.
    fn definitions(&self) -> Result<&BTreeMap<u64, &'a VariantDefinitionRow>, String> {
        lazy(&self.definitions, || {
            Ok(table::<tables::VariantDefinitions>(self.world)?
                .iter()
                .map(|row| (row.id, row))
                .collect())
        })
    }

    fn held(&self) -> Result<&(BTreeSet<u64>, BTreeSet<u64>), String> {
        lazy(&self.held, || {
            let mut assigned = BTreeSet::new();
            let mut busy_ships = BTreeSet::new();
            for raid in table::<tables::Raids>(self.world)?
                .iter()
                .filter(|raid| raid.phase != 5)
            {
                assigned.extend([raid.unit, raid.air_wing].into_iter().filter(|&id| id != 0));
                let ship = (u64::from(raid.source.ship_reference_id) << 32)
                    | u64::from(raid.source.ship_reference_type);
                if ship != 0 {
                    busy_ships.insert(ship);
                }
            }
            Ok((assigned, busy_ships))
        })
    }

    fn ships(&self) -> Result<&BTreeMap<u64, &'a TaskForceRow>, String> {
        lazy(&self.ships, || {
            let forces: BTreeMap<u64, &'a TaskForceRow> = table::<tables::TaskForces>(self.world)?
                .iter()
                .map(|row| (row.id, row))
                .collect();
            Ok(table::<tables::Ships>(self.world)?
                .iter()
                .filter_map(|ship| Some((ship.id, *forces.get(&ship.task_force)?)))
                .collect())
        })
    }

    fn prepare(&self, kind: &RaidType) -> Prepared<'a> {
        let units = self.shared_units(kind);
        let (units_gate, sources, candidates) = match &*units {
            Ok(units) => (
                units.iter().fold(Eval::False, |gate, (_, matched)| {
                    gate.or(|| matched.clone())
                }),
                self.shared_sources(kind),
                units
                    .iter()
                    .map(|(unit, matched)| (*unit, matched.clone(), self.limit(kind, *unit)))
                    .collect(),
            ),
            Err(reason) => (
                Eval::unknown(reason.clone()),
                Arc::new(Sources::Cached(Err(reason.clone()))),
                Vec::new(),
            ),
        };
        Prepared {
            resources: self.resources(kind),
            units: units_gate,
            sources,
            candidates,
            bypass: if kind.unlimited_ai_range {
                self.ai_controlled()
            } else {
                Eval::False
            },
            shared: Arc::default(),
            recorded: OnceLock::new(),
            index: OnceLock::new(),
        }
    }

    // --- Resources ---------------------------------------------------------

    fn resources(&self, kind: &RaidType) -> Eval {
        self.command_power(kind).and(|| self.equipment(kind))
    }

    fn command_power(&self, kind: &RaidType) -> Eval {
        let cost = kind.command_power_raw;
        if self.country.command_power_raw < cost {
            return Eval::False;
        }
        let inputs = match self.command_power_row(self.ours).and_then(|inputs| {
            Ok((
                inputs,
                registered(inputs.max_command_power, "max_command_power")?,
                registered(inputs.max_command_power_factor, "max_command_power_mult")?,
            ))
        }) {
            Ok(inputs) => inputs,
            Err(reason) => return Eval::unknown(reason),
        };
        let (inputs, max, factor) = inputs;
        let capacity = (self
            .assets
            .defines
            .base_max_command_power
            .wrapping_add(max)
            .wrapping_mul(100_000_i64.wrapping_add(factor))
            / 100_000)
            .wrapping_sub(inputs.allocated_command_power_raw);
        Eval::from_bool(capacity >= cost)
    }

    fn equipment(&self, kind: &RaidType) -> Eval {
        let mut result = self.nukes(kind);
        // The native map keeps the last amount of a repeated key.
        let mut amounts: BTreeMap<&str, i32> = BTreeMap::new();
        for (name, amount) in &kind.essential_equipment {
            amounts.insert(name, *amount);
        }
        for (name, amount) in amounts {
            if result == Eval::False {
                break;
            }
            result = result.and(|| match self.essential_available(name) {
                Ok(have) => Eval::from_bool(have >= amount),
                Err(reason) => Eval::unknown(reason),
            });
        }
        result
    }

    fn nukes(&self, kind: &RaidType) -> Eval {
        match self.nuke_count(kind) {
            Ok(have) => Eval::from_bool(have >= kind.nukes),
            Err(e) => Eval::unknown(e),
        }
    }

    fn nuke_count(&self, kind: &RaidType) -> Result<i32, String> {
        let ours = lazy(&self.nukes, || {
            let mut out = [None; 2];
            for row in table::<tables::Nukes>(self.world)? {
                if self.ours(row.country)
                    && let Some(slot) = out.get_mut(row.nuke_type as usize)
                    && slot.is_none()
                {
                    *slot = Some(row);
                }
            }
            Ok(out)
        })?;
        let Some(row) = ours[usize::from(kind.thermonuclear)] else {
            return Err("the raiding country has no Nukes row".into());
        };
        let have = if finite(row.amount_raw) {
            (row.amount_raw.wrapping_mul(100_000) / 10_000_000_000_000) as i32
        } else {
            0xa7c5
        };
        Ok(have)
    }

    /// Whole units of an essential equipment type in the stockpile, as
    /// StockpileArchetypes records them: capped at the largest amount any
    /// raid type requires, which every comparison and printed shortfall
    /// reads exactly. Convoys count the larger of the stockpile and the
    /// available convoys.
    fn essential_available(&self, name: &str) -> Result<i32, String> {
        let (equipment, _) = self.assets.equipment()?;
        let kind = equipment
            .named(name)
            .ok_or_else(|| format!("essential equipment {name} is not in the content"))?;
        let archetype = kind
            .archetype
            .map(|archetype| &equipment.equipment[archetype])
            .ok_or_else(|| format!("essential equipment {name} has no archetype in the content"))?;
        let mask = equipment.category(kind);
        let have = match self.world.token_of(&archetype.name)? {
            None => 0,
            Some(token) => lazy(&self.archetypes, || {
                let mut out = BTreeMap::new();
                for row in table::<tables::StockpileArchetypes>(self.world)? {
                    if row.canonical_slot == self.ours {
                        out.insert(row.archetype, row.units);
                    }
                }
                Ok(out)
            })?
            .get(&token)
            .copied()
            .unwrap_or(0),
        };
        if mask & 1 == 0 {
            return Ok(have);
        }
        Ok(have.max(self.convoys()?))
    }

    /// Available convoys at `NAVAL_INVASION_PRIORITY`.
    fn convoys(&self) -> Result<i32, String> {
        let cache = self
            .country
            .convoys_raw
            .ok_or("the convoy cache is not valid this hour")?;
        let priority = self.assets.defines.naval_invasion_priority;
        let mut available = (cache / 100_000) as i32;
        let inputs = self.inputs(self.ours)?;
        for pair in inputs.convoy_reservations.chunks_exact(2) {
            if pair[0] as i32 <= priority {
                available = available.wrapping_sub(pair[1] as i32);
            }
        }
        Ok(available)
    }

    // --- Units -------------------------------------------------------------

    /// [`Self::units`], shared by the types with equal requirements.
    fn shared_units(&self, kind: &RaidType) -> Arc<UnitList<'a>> {
        let cell = {
            let mut known = self
                .units_by_requirements
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match known
                .iter()
                .find(|(requirements, _)| *requirements == kind.unit_requirements)
            {
                Some((_, cell)) => Arc::clone(cell),
                None => {
                    let cell = Shared1::default();
                    known.push((kind.unit_requirements.clone(), Arc::clone(&cell)));
                    cell
                }
            }
        };
        Arc::clone(cell.get_or_init(|| Arc::new(self.units(kind))))
    }

    /// [`Self::sources`], shared by the types with equal starting points.
    fn shared_sources(&self, kind: &RaidType) -> Arc<Sources<'a>> {
        let cell = {
            let mut known = self
                .sources_by_start
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match known
                .iter()
                .find(|(start, _)| *start == kind.starting_point)
            {
                Some((_, cell)) => Arc::clone(cell),
                None => {
                    let cell = Shared1::default();
                    known.push((kind.starting_point.clone(), Arc::clone(&cell)));
                    cell
                }
            }
        };
        Arc::clone(cell.get_or_init(|| {
            let air = Self::has_air_sources(kind);
            let naval = kind.starting_point.types.contains(&SourceKind::NavalBase);
            let buildings = !kind.starting_point.building_types.is_empty();
            Arc::new(if (air && (naval || buildings)) || (naval && buildings) {
                Sources::Walk
            } else {
                Sources::Cached(self.sources(kind))
            })
        }))
    }

    /// The type's units: country divisions, then wings, not held by a raid and
    /// admitted by some requirement block, each with whether that is proved.
    fn units(&self, kind: &RaidType) -> Result<Vec<(Unit<'a>, Eval)>, String> {
        let mut out = Vec::new();
        if kind.unit_requirements.is_empty() {
            return Ok(out);
        }
        let units = lazy(&self.units, || {
            let assigned = &self.held()?.0;
            let divisions = table::<tables::Divisions>(self.world)?
                .iter()
                .filter(|row| self.ours(row.roster_owner))
                .map(Unit::Division);
            let wings = table::<tables::AirWings>(self.world)?
                .iter()
                .filter(|row| self.ours(row.owner))
                .map(Unit::Wing);
            Ok(divisions
                .chain(wings)
                .filter(|unit| !assigned.contains(&unit.id()))
                .collect())
        })?;
        for &unit in units {
            let matched = self.admits(kind, unit);
            if matched != Eval::False {
                out.push((unit, matched));
            }
        }
        Ok(out)
    }

    fn admits(&self, kind: &RaidType, unit: Unit<'a>) -> Eval {
        let mut result = Eval::False;
        for block in &kind.unit_requirements {
            result = result.or(|| self.block_admits(block, unit));
            if result == Eval::True {
                break;
            }
        }
        result
    }

    fn block_admits(&self, block: &UnitRequirement, unit: Unit<'a>) -> Eval {
        let mut result = Eval::True;
        if !block.battalions.is_empty() {
            let Unit::Division(division) = unit else {
                return Eval::False;
            };
            result = self.battalions(block, division);
        }
        for requirement in &block.equipment {
            if result == Eval::False {
                break;
            }
            result = result.and(|| match self.pool_matches(requirement, unit) {
                Ok(value) => Eval::from_bool(value),
                Err(reason) => Eval::unknown(reason),
            });
        }
        result
    }

    /// Standard-template sub-unit counts grouped over the three grids.
    fn battalions(&self, block: &UnitRequirement, division: &DivisionRow) -> Eval {
        let template = match self.template(division.template) {
            Ok(Some(template)) => template,
            Ok(None) => {
                return Eval::unknown("a division template has no hourly template inputs row");
            }
            Err(reason) => return Eval::unknown(reason),
        };
        for requirement in &block.battalions {
            let token = match self.world.token_of(requirement.name.as_bytes()) {
                Ok(Some(token)) => token,
                Ok(None) => return Eval::False,
                Err(reason) => return Eval::unknown(reason),
            };
            if template.model_unreadable != 0 {
                return Eval::unknown("a division template's sub-units were unreadable this hour");
            }
            // A country-local clone counts under its definition's token.
            let count = template
                .subunits
                .iter()
                .filter(|unit| unit.token == token)
                .fold(0_i32, |count, unit| count.saturating_add(unit.count));
            if count == 0
                || requirement.min.is_some_and(|min| count < min)
                || requirement.max.is_some_and(|max| count > max)
            {
                return Eval::False;
            }
        }
        Eval::True
    }

    /// A listed unit's pool variants: only our listed units ask, so only
    /// their pools are indexed.
    fn pool(&self, unit: Unit<'a>) -> Result<&[(u64, i64)], String> {
        let pools = match unit {
            Unit::Wing(_) => self.pools(
                &self.wing_equipment,
                table::<tables::WingEquipment>(self.world)?
                    .iter()
                    .map(|row| (row.wing, row.variant, row.amount_raw)),
            )?,
            Unit::Division(_) => self.pools(
                &self.division_equipment,
                table::<tables::DivisionEquipment>(self.world)?
                    .iter()
                    .map(|row| (row.division, row.variant_handle, row.amount_raw)),
            )?,
        };
        Ok(pools.get(&unit.id()).map_or(&[][..], Vec::as_slice))
    }

    fn pools<'s>(
        &'s self,
        cell: &'s Lazy<Equipment>,
        rows: impl Iterator<Item = (u64, u64, i64)>,
    ) -> Result<&'s Equipment, String> {
        lazy(cell, || {
            let listed: BTreeSet<u64> = lazy(&self.units, || Err("no units listed".into()))?
                .iter()
                .map(|unit| unit.id())
                .collect();
            let mut out = Equipment::new();
            for (id, variant, amount) in rows {
                if listed.contains(&id) {
                    out.entry(id).or_default().push((variant, amount));
                }
            }
            Ok(out)
        })
    }

    /// A unit pool's archetype totals are sums of its variants in the first array.
    fn pool_matches(
        &self,
        requirement: &EquipmentRequirement,
        unit: Unit<'a>,
    ) -> Result<bool, String> {
        let entries = self.pool(unit)?;
        let definitions = self.definitions()?;
        let archetype_of = |variant: u64| {
            definitions
                .get(&variant)
                .map(|row| row.archetype_token)
                .ok_or("a pool variant is not in the daily variant dictionary")
        };
        let mut totals: BTreeMap<u32, i64> = BTreeMap::new();
        for &(variant, amount) in entries {
            let total = totals.entry(archetype_of(variant)?).or_default();
            *total = total.wrapping_add(amount);
        }
        let (equipment, groups) = self.assets.equipment()?;
        let members = groups.expand(&requirement.types)?;
        let mut admitted = BTreeSet::new();
        for (token, total) in totals {
            if total <= 0 {
                continue;
            }
            let name = self
                .world
                .token(token)?
                .and_then(|name| std::str::from_utf8(name).ok())
                .ok_or("a pool archetype token has no literal")?;
            let admits = members.is_empty()
                || equipment.group_admits(
                    &members,
                    equipment
                        .get(name)
                        .ok_or_else(|| format!("archetype {name} is not in the content"))?,
                );
            if admits {
                admitted.insert(token);
            }
        }
        if admitted.is_empty() {
            return Ok(false);
        }
        let mut sum = 0_i64;
        for &(variant, amount) in entries {
            if amount <= 0 || !admitted.contains(&archetype_of(variant)?) {
                continue;
            }
            if !requirement.modules.is_empty() {
                let installed = lazy(&self.modules, || {
                    let mut out: BTreeMap<u64, Vec<&'a [u8]>> = BTreeMap::new();
                    // Modules are recorded by lexer token; the requirement
                    // names them.
                    for row in table::<tables::VariantModules>(self.world)? {
                        let name = self
                            .world
                            .token(row.definition_token)?
                            .ok_or("an installed module token has no literal")?;
                        out.entry(row.variant).or_default().push(name);
                    }
                    Ok(out)
                })?
                .get(&variant)
                .map_or(&[][..], Vec::as_slice);
                let satisfied = requirement.modules.iter().all(|alternatives| {
                    alternatives
                        .iter()
                        // Module names are tokens: ASCII case folds.
                        .any(|name| {
                            installed
                                .iter()
                                .any(|held| held.eq_ignore_ascii_case(name.as_bytes()))
                        })
                });
                if !satisfied {
                    continue;
                }
            }
            sum = sum.wrapping_add(amount);
        }
        let count = (sum / 100_000) as i32;
        Ok(count >= 1
            && requirement.min.is_none_or(|min| count >= min)
            && requirement.max.is_none_or(|max| count <= max))
    }

    // --- Range -------------------------------------------------------------

    fn limit(&self, kind: &RaidType, unit: Unit<'a>) -> Result<Limit, String> {
        let factor = kind.range_factor_raw.wrapping_add(100_000);
        let scale = self.assets.defines.map_scale;
        if let Unit::Wing(wing) = unit {
            return Ok(Some(
                factor.wrapping_mul(third(self.wing_range(wing)?)) / 100_000,
            ));
        }
        let has = |bit: u64| -> Result<bool, String> {
            let mut found = false;
            for (name, _) in &kind.essential_equipment {
                found |= self.assets.equipment_category(name)? & bit != 0;
            }
            Ok(found)
        };
        if has(0x10_0000)? {
            return Ok(self
                .transport_range()?
                .map(|best| factor.wrapping_mul(third(best)) / 100_000));
        }
        if has(1)? {
            let inputs = self.inputs(self.ours)?;
            let kilometres = (registered(
                inputs.naval_commando_range_factor,
                "naval_commando_raid_distance",
            )?
            .wrapping_add(100_000)
            .wrapping_mul(self.assets.defines.naval_commando_distance)
                / 100_000)
                .wrapping_mul(factor)
                / 100_000;
            if scale == 0 || !finite(kilometres) {
                return Ok(Some(0xffff_ffff_i64.wrapping_mul(100_000) / 300_000));
            }
            let pixels = kilometres.wrapping_mul(100_000) / scale;
            if !(-0x7fff_ffff_ffff_fffe..=0x7fff_ffff_ffff_fffe).contains(&pixels) {
                return Ok(Some(0xffff_ffff));
            }
            return Ok(Some(pixels.wrapping_mul(100_000) / 300_000));
        }
        Ok(None)
    }

    fn wing_range(&self, wing: &AirWingRow) -> Result<i64, String> {
        air_range::wing_range(wing, self.range_defines(), self)
    }

    fn range_defines(&self) -> air_range::Defines {
        let defines = &self.assets.defines;
        air_range::Defines {
            map_scale: defines.map_scale,
            ace_wing_size: defines.ace_wing_size,
            ace_wing_size_max_bonus: defines.ace_wing_size_max_bonus,
        }
    }

    /// Best stockpiled transport plane from daily stockpile and variant tables,
    /// once per country and hour.
    fn transport_range(&self) -> Result<Option<i64>, String> {
        lazy(&self.transport, || {
            let definitions = self.definitions()?;
            let mut transports = BTreeSet::new();
            for row in table::<tables::Stockpiles>(self.world)? {
                if row.amount_raw == 0 || !self.ours(row.country) {
                    continue;
                }
                let definition = definitions
                    .get(&row.variant_handle)
                    .ok_or("a stockpiled variant is not in the daily variant dictionary")?;
                if definition.classifier_flags & (1 << 20) != 0 {
                    transports.insert(row.variant_handle);
                }
            }
            if transports.is_empty() {
                return Ok(None);
            }
            // Read the first stat, range.
            let mut stats = BTreeMap::new();
            for row in table::<tables::VariantStats>(self.world)? {
                if transports.contains(&row.variant) {
                    stats.entry(row.variant).or_insert(row.values[0]);
                }
            }
            let air = registered(self.inputs(self.ours)?.air_range_factor, "air_range_factor")?;
            let scale = self.assets.defines.map_scale;
            let mut best: Option<i64> = None;
            for variant in &transports {
                let stat = stats
                    .get(variant)
                    .ok_or("a stockpiled transport variant has no recorded stats")?;
                let range = air.wrapping_add(100_000).wrapping_mul(to_map(*stat, scale)) / 100_000;
                if best.is_none_or(|best| best < range) {
                    best = Some(range);
                }
            }
            Ok(best)
        })
        .copied()
    }

    fn ai_controlled(&self) -> Eval {
        let features = match table::<tables::SimulationFeatures>(self.world) {
            Ok(rows) => rows,
            Err(reason) => return Eval::unknown(reason),
        };
        if features.first().is_some_and(|row| row.human_ai != 0) {
            return Eval::True;
        }
        let inputs = match self.inputs(self.ours) {
            Ok(inputs) => inputs,
            Err(reason) => return Eval::unknown(reason),
        };
        match inputs.autonomous {
            Some(flag) => {
                let humans = match table::<tables::HumanControllers>(self.world) {
                    Ok(rows) => rows,
                    Err(reason) => return Eval::unknown(reason),
                };
                match humans
                    .iter()
                    .find(|row| self.world.slot(row.country) == Some(self.ours))
                {
                    Some(row) => Eval::from_bool(flag != 0 && row.count == 0),
                    None => Eval::unknown("the raiding country has no HumanControllers row"),
                }
            }
            None => match table::<tables::Players>(self.world) {
                Ok(players) => Eval::from_bool(
                    !players
                        .iter()
                        .any(|player| self.world.slot(player.country) == Some(self.ours)),
                ),
                Err(reason) => Eval::unknown(reason),
            },
        }
    }

    // --- Sources -----------------------------------------------------------

    fn reserved(&self) -> Result<&'a ReservedCountryNamingRow, String> {
        let [row] = table::<tables::ReservedCountryNaming>(self.world)? else {
            return Err("ReservedCountryNaming does not hold one reserved country".into());
        };
        Ok(row)
    }

    fn reserved_launch(&self) -> Result<&'a ReservedLaunchSources, String> {
        lazy(&self.reserved_launch, || {
            let packet = self
                .reserved()?
                .launch_sources
                .require("ReservedCountryNaming.launch_sources")?
                .as_ref()
                .ok_or("ReservedCountryNaming.launch_sources refused this hour")?;
            if !packet.sample_hour.fresh_at(self.world.frame().hour) {
                return Err(format!(
                    "ReservedCountryNaming.launch_sources sampled hour {}, expected {}",
                    packet.sample_hour,
                    self.world.frame().hour
                ));
            }
            Ok(packet)
        })
        .copied()
    }

    fn reserved_owned_states(&self) -> Result<&'a [u32], String> {
        let count = self
            .reserved()?
            .owned_state_count
            .require("ReservedCountryNaming.owned_state_count")?
            .ok_or("ReservedCountryNaming.owned_state_count refused this hour")?;
        // The native empty loop demands no packet, state or building join.
        if count == 0 {
            return Ok(&[]);
        }
        if count < 0 {
            return Err("ReservedCountryNaming.owned_state_count is negative".into());
        }
        let states = self
            .reserved_launch()?
            .owned_states
            .as_deref()
            .ok_or("ReservedCountryNaming.launch_sources.owned_states refused this hour")?;
        if states.len() != count as usize {
            return Err(
                "ReservedCountryNaming.launch_sources.owned_states contradict their count".into(),
            );
        }
        Ok(states)
    }

    /// Resolve the base province through a signed-positive provider. A complete
    /// hourly census proves a positive slot absent; a partial census cannot prove
    /// null. Neither branch selects province zero.
    fn reserved_base_province(&self, stored: u32) -> Result<u32, String> {
        if (stored as i32) <= 0 {
            return Ok(0);
        }
        table::<tables::Provinces>(self.world)?;
        Ok(if self.world.provinces.contains_key(&stored) {
            stored
        } else {
            0
        })
    }

    fn reserved_naval_sources(
        &self,
        visit: &mut impl FnMut(Source<'a>) -> bool,
    ) -> Result<bool, String> {
        let bases = match &self.reserved_launch()?.naval_bases {
            replay::rows::LaunchNavalBases::Bases(bases) => bases,
            replay::rows::LaunchNavalBases::NativeNull => {
                return Err(
                    "ReservedCountryNaming.launch_sources.naval_bases native provider is null"
                        .into(),
                );
            }
            replay::rows::LaunchNavalBases::Unavailable => {
                return Err(
                    "ReservedCountryNaming.launch_sources.naval_bases refused this hour".into(),
                );
            }
        };
        for base in bases {
            let source = match self.reserved_base_province(base.base_province) {
                Ok(0) => Source {
                    province: Ok(0),
                    path: None,
                    site: None,
                    valid: Eval::True,
                    unknown: None,
                },
                // Distance and division reach do not read the source's raw holder country.
                // Keep its recorded bits without demanding CountryLinks here.
                Ok(province) => path_source(2, province),
                Err(reason) => Source::unknown(reason),
            };
            if !visit(source) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn has_air_sources(kind: &RaidType) -> bool {
        [
            SourceKind::AirBase,
            SourceKind::Carrier,
            SourceKind::RocketSite,
            SourceKind::Submarine,
        ]
        .iter()
        .any(|source| kind.starting_point.types.contains(source))
    }

    fn air_sources(
        &self,
        kind: &RaidType,
        visit: &mut impl FnMut(Source<'a>) -> bool,
    ) -> Result<bool, String> {
        let has = |source| kind.starting_point.types.contains(&source);
        let bases = self.bases()?;
        for site in self.air_sites()? {
            // List 0 holds air bases and carriers; list 1 holds rocket sites and submarines.
            let wanted = |stated: bool| match (site.list, stated) {
                (0, true) => has(SourceKind::AirBase),
                (0, false) => has(SourceKind::Carrier),
                (_, true) => has(SourceKind::RocketSite),
                (_, false) => has(SourceKind::Submarine),
            };
            let Some(&base) = bases.get(&site.base) else {
                if (wanted(true) || wanted(false))
                    && !visit(Source::unknown(
                        "a raid air site is not in the hourly AirBases".into(),
                    ))
                {
                    return Ok(false);
                }
                continue;
            };
            if wanted(base.state.is_some())
                && !visit(self.air_source(base, site.container_capacity_raw))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn naval_sources(
        &self,
        faction: &LaunchMembers,
        visit: &mut impl FnMut(Source<'a>) -> bool,
    ) -> Result<bool, String> {
        let own = [Ok(LaunchCountry::Linked(self.ours))];
        let mut noted_unlinked = false;
        for country in faction.as_deref().unwrap_or(&own) {
            match country {
                Ok(LaunchCountry::Reserved) => match self.reserved_naval_sources(visit) {
                    Ok(false) => return Ok(false),
                    Ok(true) => {}
                    Err(reason) => {
                        if !visit(Source::unknown(reason)) {
                            return Ok(false);
                        }
                    }
                },
                Ok(LaunchCountry::Linked(country)) => match self.naval_bases() {
                    Ok(bases) => {
                        // Retain the positive-country hash law: an unresolved
                        // hash owner may be this member's.
                        if !noted_unlinked {
                            if let Some(&(province, raw)) = bases.1.first()
                                && !visit(Source::unknown(format!(
                                    "the naval base in province {province} belongs to country {raw}, which has no CountryLinks row"
                                )))
                            {
                                return Ok(false);
                            }
                            noted_unlinked = true;
                        }
                        for &province in bases.0.get(country).into_iter().flatten() {
                            if !visit(path_source(2, province)) {
                                return Ok(false);
                            }
                        }
                    }
                    Err(reason) => {
                        if !visit(Source::unknown(reason)) {
                            return Ok(false);
                        }
                    }
                },
                Err(reason) => {
                    if !visit(Source::unknown(reason.clone())) {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// A boolean walk demands each family only after the previous one continues.
    /// Refused families occupy an uncertainty boundary, with unknown cardinality.
    fn visit_sources(&self, kind: &RaidType, visit: &mut impl FnMut(Source<'a>) -> bool) -> bool {
        if Self::has_air_sources(kind) {
            let continued = match self.air_sources(kind, visit) {
                Ok(continued) => continued,
                Err(reason) => visit(Source::unknown(reason)),
            };
            if !continued {
                return false;
            }
        }
        if kind.starting_point.types.contains(&SourceKind::NavalBase) {
            let continued = match self
                .faction()
                .and_then(|faction| self.naval_sources(faction, visit))
            {
                Ok(continued) => continued,
                Err(reason) => visit(Source::unknown(reason)),
            };
            if !continued {
                return false;
            }
        }
        if !kind.starting_point.building_types.is_empty() {
            let faction = if kind.starting_point.allow_faction_buildings {
                self.faction()
            } else {
                Ok(&None)
            };
            let continued = match faction
                .and_then(|faction| self.building_sources(kind, faction, visit, true))
            {
                Ok(continued) => continued,
                Err(reason) => visit(Source::unknown(reason)),
            };
            if !continued {
                return false;
            }
        }
        true
    }

    /// Enumerate sources in their stored order.
    fn sources(&self, kind: &RaidType) -> Result<Vec<Source<'a>>, String> {
        let has = |source| kind.starting_point.types.contains(&source);
        let mut out = Vec::new();
        if Self::has_air_sources(kind) {
            self.air_sources(kind, &mut |source| {
                out.push(source);
                true
            })?;
        }
        // Enumerate faction members only for naval sources or authored faction
        // building sources.
        let faction = if has(SourceKind::NavalBase)
            || (!kind.starting_point.building_types.is_empty()
                && kind.starting_point.allow_faction_buildings)
        {
            self.faction()?
        } else {
            &None
        };
        if has(SourceKind::NavalBase) {
            self.naval_sources(faction, &mut |source| {
                out.push(source);
                true
            })?;
        }
        if !kind.starting_point.building_types.is_empty() {
            // Checked against the full walk in the debug test runs; the
            // release bench runs without the check.
            #[cfg(all(test, debug_assertions))]
            let full = {
                let mut full = Vec::new();
                self.building_sources(
                    kind,
                    faction,
                    &mut |source| {
                        full.push(source);
                        true
                    },
                    false,
                )
                .map(|_| described(&full))
            };
            #[cfg(all(test, debug_assertions))]
            let start = out.len();
            let walked = self.building_sources(
                kind,
                faction,
                &mut |source| {
                    out.push(source);
                    true
                },
                true,
            );
            #[cfg(all(test, debug_assertions))]
            assert_eq!(
                walked.clone().map(|_| described(&out[start..])),
                full,
                "the building index walk gave other sources than the full walk"
            );
            walked?;
        }
        Ok(out)
    }

    /// Province buildings of owned states, or faction members' states when the
    /// type allows. With `indexed`, states whose provinces match content supply
    /// wanted buildings through the content-ordered index without scanning other
    /// provinces. Check that province match once per hour.
    fn building_sources(
        &self,
        kind: &RaidType,
        faction: &LaunchMembers,
        visit: &mut impl FnMut(Source<'a>) -> bool,
        indexed: bool,
    ) -> Result<bool, String> {
        let own = [Ok(LaunchCountry::Linked(self.ours))];
        let countries = match faction {
            Some(members) if kind.starting_point.allow_faction_buildings => members.as_slice(),
            _ => &own,
        };
        let building_inputs = Lazy::new();
        let mut noted_unlinked = false;
        let orderly = |state: u32| {
            let known = self
                .orderly
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&state)
                .copied();
            known.unwrap_or_else(|| {
                let ok = self.world.state_order(state).is_ok();
                self.orderly
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(state, ok);
                ok
            })
        };
        for country in countries {
            let states = match country {
                Ok(LaunchCountry::Reserved) => match self.reserved_owned_states() {
                    Ok(states) => states,
                    Err(reason) => {
                        if !visit(Source::unknown(reason)) {
                            return Ok(false);
                        }
                        continue;
                    }
                },
                Ok(LaunchCountry::Linked(country)) => match self.owned_states() {
                    Ok(owned) => owned.0.get(country).map_or(&[][..], Vec::as_slice),
                    Err(reason) => {
                        if !visit(Source::unknown(reason)) {
                            return Ok(false);
                        }
                        continue;
                    }
                },
                Err(reason) => {
                    if !visit(Source::unknown(reason.clone())) {
                        return Ok(false);
                    }
                    continue;
                }
            };
            let unresolved = matches!(country, Ok(LaunchCountry::Linked(_)))
                && !noted_unlinked
                && !self.owned_states()?.1.is_empty();
            if states.is_empty() && !unresolved {
                continue;
            }
            // A partial census retaining this state's provinces cannot prove
            // that its runtime list is the complete content list. Count zero
            // above remains lazy and does not demand the provider census.
            if matches!(country, Ok(LaunchCountry::Reserved))
                && let Err(reason) = table::<tables::Provinces>(self.world)
            {
                if !visit(Source::unknown(reason)) {
                    return Ok(false);
                }
                continue;
            }
            let (tokens, buildings, wanted) = lazy(&building_inputs, || {
                let mut tokens = BTreeSet::new();
                for name in &kind.starting_point.building_types {
                    if let Some(token) = self.world.token_of(name.as_bytes())? {
                        tokens.insert(token);
                    }
                }
                let buildings = self.province_buildings()?;
                let wanted: Vec<_> = tokens
                    .iter()
                    .filter_map(|token| buildings.counts.get(token))
                    .collect();
                Ok((tokens, buildings, wanted))
            })?;
            // Keep the existing positive hash-owner caution. Actual0's
            // ordered roster does not derive membership from this index.
            if matches!(country, Ok(LaunchCountry::Linked(_))) && !noted_unlinked {
                for &(state, raw) in &self.owned_states()?.1 {
                    let holds = match self.world.state_order(state) {
                        Ok(provinces) => provinces.iter().any(|province| {
                            wanted.iter().any(|counts| counts.contains_key(province))
                        }),
                        Err(_) => true,
                    };
                    if holds {
                        if !visit(Source::unknown(format!(
                            "state {state} belongs to country {raw}, which has no CountryLinks row"
                        ))) {
                            return Ok(false);
                        }
                        break;
                    }
                }
                noted_unlinked = true;
            }
            for &state in states {
                if matches!(country, Ok(LaunchCountry::Reserved)) {
                    if let Err(reason) = table::<tables::States>(self.world) {
                        if !visit(Source::unknown(reason)) {
                            return Ok(false);
                        }
                        continue;
                    }
                    if !self.world.states.contains_key(&state) {
                        if !visit(Source::unknown(format!(
                            "ReservedCountryNaming.launch_sources.owned_states state {state} has no States row"
                        ))) {
                            return Ok(false);
                        }
                        continue;
                    }
                }
                // The runtime province list matches the content's; its sources are
                // its wanted buildings in content order.
                if indexed && orderly(state) {
                    let mut listed = buildings
                        .by_state
                        .get(&state)
                        .into_iter()
                        .flatten()
                        .filter(|(_, kind, _)| tokens.contains(kind))
                        .peekable();
                    while let Some(&(province, _, mut count)) = listed.next() {
                        while let Some((_, _, more)) =
                            listed.next_if(|(next, _, _)| *next == province)
                        {
                            count += more;
                        }
                        for _ in 0..count {
                            if !visit(path_source(5, province)) {
                                return Ok(false);
                            }
                        }
                    }
                    continue;
                }
                let provinces = match self.world.state_order(state) {
                    Ok(provinces) => provinces,
                    Err(reason) => {
                        if !visit(Source::unknown(reason)) {
                            return Ok(false);
                        }
                        continue;
                    }
                };
                for &province in provinces {
                    let count: usize = wanted
                        .iter()
                        .filter_map(|provinces| provinces.get(&province))
                        .sum();
                    for _ in 0..count {
                        if !visit(path_source(5, province)) {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        Ok(true)
    }

    /// Our air and missile sites in their native order.
    fn air_sites(&self) -> Result<&Vec<&'a RaidAirSiteRow>, String> {
        lazy(&self.air_sites, || {
            let mut sites: Vec<_> = table::<tables::RaidAirSites>(self.world)?
                .iter()
                .filter(|row| row.canonical_slot == self.ours)
                .collect();
            sites.sort_by_key(|row| row.ordinal);
            Ok(sites)
        })
    }

    /// Each country's naval base provinces, in `NavalBases` order.
    /// Naval base provinces by owner slot, and those whose owner reference
    /// links to no slot (any country's, ours included).
    fn naval_bases(&self) -> Result<&Owned, String> {
        lazy(&self.naval_bases, || {
            let rows = table::<tables::NavalBases>(self.world)?;
            let key = self.tables(&[TableId::NavalBases, TableId::CountryLinks]);
            let indexes = &self.assets.table_indexes;
            indexes.naval_bases.get_or_insert_with(key, || {
                let mut out = Owned::default();
                for base in rows {
                    match self.world.slot(base.country) {
                        Some(country) => out.0.entry(country).or_default().push(base.province),
                        None if base.country != 0 => out.1.push((base.province, base.country)),
                        None => {}
                    }
                }
                Ok(Arc::new(out))
            })
        })
        .map(|owned| &**owned)
    }

    /// Province building counts by type and province, and by type the
    /// content states listing a province that holds one.
    fn province_buildings(&self) -> Result<&ProvinceBuildings, String> {
        lazy(&self.province_buildings, || {
            let rows = table::<tables::ProvinceBuildings>(self.world)?;
            let key = self.tables(&[TableId::ProvinceBuildings]);
            let indexes = &self.assets.table_indexes;
            indexes.province_buildings.get_or_insert_with(key, || {
                let mut out = ProvinceBuildings::default();
                for row in rows {
                    *out.counts
                        .entry(row.building_type)
                        .or_default()
                        .entry(row.location)
                        .or_default() += 1;
                }
                let mut by_province: HashMap<u32, Vec<(u32, usize)>> = HashMap::new();
                for (&kind, provinces) in &out.counts {
                    for (&province, &count) in provinces {
                        by_province.entry(province).or_default().push((kind, count));
                    }
                }
                for (&state, provinces) in self.world.state_provinces.iter() {
                    let listed: Vec<_> = provinces
                        .iter()
                        .flat_map(|&province| {
                            by_province
                                .get(&province)
                                .into_iter()
                                .flatten()
                                .map(move |&(kind, count)| (province, kind, count))
                        })
                        .collect();
                    if !listed.is_empty() {
                        out.by_state.insert(state, listed);
                    }
                }
                Ok(Arc::new(out))
            })
        })
        .map(|buildings| &**buildings)
    }

    /// Each country's owned states, ascending.
    /// States by owner slot in id order, and those whose owner reference
    /// links to no slot (any country's, ours included).
    fn owned_states(&self) -> Result<&Owned, String> {
        lazy(&self.owned_states, || {
            let key = self.tables(&[TableId::States, TableId::CountryLinks]);
            let indexes = &self.assets.table_indexes;
            indexes.owned_states.get_or_insert_with(key, || {
                let mut out = Owned::default();
                // `World::states` iterates in id order.
                for state in self.world.states.values() {
                    match self.world.slot(state.owner) {
                        Some(owner) => out.0.entry(owner).or_default().push(state.id),
                        None if state.owner != 0 => out.1.push((state.id, state.owner)),
                        None => {}
                    }
                }
                Ok(Arc::new(out))
            })
        })
        .map(|owned| &**owned)
    }

    /// The identity of the hour's `tables`.
    fn tables(&self, tables: &[TableId]) -> TableKey {
        TableKey::of(self.world.frame(), tables)
    }

    /// The country's faction members in faction order, when it has a faction.
    fn faction(&self) -> Result<&LaunchMembers, String> {
        lazy(&self.faction, || {
            let members = table::<tables::FactionMembers>(self.world)?;
            // A member row whose country links to no slot may be ours, or a
            // partner whose sources the list would miss.
            let unlinked = |row: &replay::rows::FactionMemberRow| {
                format!(
                    "a FactionMembers row's country {} has no CountryLinks row",
                    row.country
                )
            };
            let Some(faction) = members
                .iter()
                .find(|row| self.ours(row.country))
                .map(|row| row.faction)
            else {
                return match members
                    .iter()
                    .find(|row| row.country != 0 && self.world.slot(row.country).is_none())
                {
                    Some(row) => Err(unlinked(row)),
                    None => Ok(None),
                };
            };
            let mut rows: Vec<_> = members
                .iter()
                .filter(|row| row.faction == faction)
                .collect();
            rows.sort_by_key(|row| row.ordinal);
            Ok(Some(
                rows.iter()
                    .map(|row| {
                        if row.country == 0 {
                            Ok(LaunchCountry::Reserved)
                        } else {
                            self.world
                                .slot(row.country)
                                .map(LaunchCountry::Linked)
                                .ok_or_else(|| unlinked(row))
                        }
                    })
                    .collect(),
            ))
        })
    }

    /// An air site source: its province and validity.
    fn air_source(&self, base: &'a AirBaseRow, capacity: Option<i64>) -> Source<'a> {
        // Use the base province or host ship's task-force location; zero when the
        // ship is gone.
        let province = if base.state.is_some() {
            base.province
                .ok_or_else(|| "an air site has no recorded province".to_owned())
        } else {
            self.ships()
                .map(|ships| ships.get(&base.host).map_or(0, |force| force.province))
        };
        let valid = match self.held() {
            _ if base.state.is_some() => Eval::True,
            Ok((_, busy_ships)) => Eval::from_bool(!busy_ships.contains(&base.host)),
            Err(reason) => Eval::unknown(reason),
        };
        Source {
            province,
            path: None,
            site: Some((base, capacity)),
            valid,
            unknown: None,
        }
    }

    /// A wing reaches a source when already at the site or admitted to its air base.
    fn wing_reaches(&self, wing: &AirWingRow, site: &AirBaseRow, capacity: Option<i64>) -> Eval {
        let bases = match self.bases() {
            Ok(bases) => bases,
            Err(reason) => return Eval::unknown(reason),
        };
        let current = if wing.base_override != 0 && bases.contains_key(&wing.base_override) {
            wing.base_override
        } else {
            wing.air_base
        };
        if current == site.id {
            return Eval::True;
        }
        // Require air-base access for the pool's country.
        let Some(capacity) = capacity else {
            return Eval::False;
        };
        let Some(pool) = bases.get(&wing.air_base) else {
            return Eval::unknown("a wing's pool base is not in AirBases");
        };
        // Refuse a rocket site on either side, a gun site, or the pool's own base.
        if [site.kind, pool.kind]
            .iter()
            .any(|&kind| kind == 1 || kind == 2)
            || pool.id == site.id
        {
            return Eval::False;
        }
        if capacity <= 0 {
            return Eval::False;
        }
        let mut result = Eval::True;
        match site.state {
            None => result = Eval::unknown("carrier capability of a wing's pool is not recorded"),
            Some(state) => match lazy(&self.demilitarized, || {
                Ok(table::<tables::StateFlags>(self.world)?
                    .iter()
                    .filter(|flag| flag.demilitarized != 0)
                    .map(|flag| flag.state)
                    .collect())
            }) {
                Ok(demilitarized) => {
                    if demilitarized.contains(&state) {
                        return Eval::False;
                    }
                }
                Err(reason) => result = Eval::unknown(reason),
            },
        }
        result.and(|| self.foreign_controller(wing, site, pool))
    }

    /// The end of `air_base_admission`: across controllers, only a volunteer
    /// link to the site's controller asks for volunteer capacity.
    fn foreign_controller(&self, wing: &AirWingRow, site: &AirBaseRow, pool: &AirBaseRow) -> Eval {
        let (site_controller, pool_controller) =
            match (self.controller(site), self.controller(pool)) {
                (Ok(site), Ok(pool)) => (site, pool),
                (Err(reason), _) | (_, Err(reason)) => return Eval::unknown(reason),
            };
        if site_controller == pool_controller {
            return Eval::True;
        }
        let raw = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        let volunteers = match lazy(&self.volunteers, || {
            Ok(table::<tables::VolunteerHosts>(self.world)?
                .iter()
                .map(|row| (self.world.slot(row.country), self.world.slot(row.host)))
                .collect())
        }) {
            Ok(volunteers) => volunteers,
            Err(reason) => return Eval::unknown(reason),
        };
        if volunteers.contains(&(self.world.slot(raw), site_controller)) {
            Eval::unknown("remaining volunteer capacity is not reconstructed")
        } else {
            Eval::True
        }
    }

    /// Base province controller, or host ship owner.
    fn controller(&self, base: &AirBaseRow) -> Result<Option<u32>, String> {
        if base.state.is_some() {
            let province = base
                .province
                .ok_or("an air base has no recorded province")?;
            return Ok(self.world.province_controller(province));
        }
        Ok(self
            .ships()?
            .get(&base.host)
            .and_then(|force| self.world.slot(force.roster_owner)))
    }
}

impl air_range::Sources for Context<'_, '_> {
    fn slot(&self, raw: u32) -> Option<u32> {
        self.world.slot(raw)
    }

    fn country(&self, slot: u32) -> Result<&RaidCountryInputRow, String> {
        self.inputs(slot)
    }

    fn command_power(&self, slot: u32) -> Result<&CountryCommandPowerRow, String> {
        self.command_power_row(slot)
    }

    fn wing(&self, handle: u64) -> Result<Option<&AirWingRow>, String> {
        Context::wing(self, handle)
    }

    fn ace(&self, handle: u64) -> Result<Option<&AceRangeInputRow>, String> {
        Context::ace(self, handle)
    }
}

/// Each source's fields, for comparing two source lists.
#[cfg(all(test, debug_assertions))]
fn described(sources: &[Source<'_>]) -> Vec<String> {
    sources
        .iter()
        .map(|source| {
            format!(
                "{:?} {:?} {:?} {:?} {:?}",
                source.province,
                source.path,
                source.site.map(|(base, capacity)| (base.id, capacity)),
                source.valid,
                source.unknown
            )
        })
        .collect()
}

/// A naval-base or building source (kinds 2 and 5).
fn path_source<'a>(kind: u8, province: u32) -> Source<'a> {
    Source {
        province: Ok(province),
        path: Some(PathSource { kind, province }),
        site: None,
        valid: Eval::True,
        unknown: None,
    }
}
