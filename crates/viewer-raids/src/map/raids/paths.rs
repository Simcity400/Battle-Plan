//! Raid source distances use separate land and naval dispatcher memos.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Mutex, MutexGuard, OnceLock},
};
use viewer_foundation::debug_fields;

/// Source kinds (`engine_native::craid_source`) whose distance uses a path:
/// a naval base measures to the target's naval path target, a building
/// to its province.
pub(crate) const NAVAL_BASE: u8 = engine_native::craid_source::KIND_NAVAL_BASE as u8;
pub(crate) const BUILDING: u8 = engine_native::craid_source::KIND_BUILDING as u8;

use installed_content_core::{
    province_geometry::ProvinceGeometry, province_graph::ProvinceGraph, province_graph::RaidPath,
    province_graph::raid_distance,
};
use replay_data::{
    rows::RaidPathDispatcher, rows::RaidPathDistanceRow, rows::RaidScope, rows::tables,
};
use replay_frame::frame::DecodedSource;

use super::{
    RaidAssets,
    script::{Eval, World},
};

type Search = Result<Option<i64>, String>;
/// A resolved target province, `None` when there is none.
type Target = Result<Option<u32>, String>;

type MemoKey = (RaidPathDispatcher, u32, u32);
type Entry = Option<Search>;

pub(super) fn dispatcher(kind: u8) -> Result<RaidPathDispatcher, String> {
    match kind {
        NAVAL_BASE => Ok(RaidPathDispatcher::Naval),
        BUILDING => Ok(RaidPathDispatcher::Land),
        other => Err(format!("raid source kind {other} has no path dispatcher")),
    }
}

fn path_dispatcher(path: RaidPath) -> RaidPathDispatcher {
    match path {
        RaidPath::Land => RaidPathDispatcher::Land,
        RaidPath::Naval => RaidPathDispatcher::Naval,
    }
}

type Key = (u32, u32, RaidPath);
/// Each province's recorded state, and each state's province count.
type StateMembers = (BTreeMap<u32, u32>, BTreeMap<u32, usize>);

/// The memory the content search cache may hold, entries and error text.
pub const CACHE_BUDGET: usize = 4 << 20;

/// What one cached entry costs beyond its error text: the map slot, the
/// recency index entry and their allocator overhead, rounded up.
const ENTRY_BYTES: usize = 96;

/// Least-recently-used results, bounded by [`CACHE_BUDGET`].
#[derive(Default)]
struct Cache {
    entries: HashMap<Key, (Search, u64)>,
    recency: BTreeMap<u64, Key>,
    tick: u64,
    bytes: usize,
}

fn cost(result: &Search) -> usize {
    ENTRY_BYTES + result.as_ref().err().map_or(0, String::capacity)
}

impl Cache {
    fn get(&mut self, key: &Key) -> Option<Search> {
        let (result, stamp) = self.entries.get_mut(key)?;
        self.recency.remove(stamp);
        self.tick += 1;
        *stamp = self.tick;
        self.recency.insert(self.tick, *key);
        Some(result.clone())
    }

    fn insert(&mut self, key: Key, result: Search, budget: usize) {
        if let Some((old, stamp)) = self.entries.remove(&key) {
            self.recency.remove(&stamp);
            self.bytes -= cost(&old);
        }
        self.tick += 1;
        self.bytes += cost(&result);
        self.recency.insert(self.tick, key);
        self.entries.insert(key, (result, self.tick));
        while self.bytes > budget
            && let Some((_, oldest)) = self.recency.pop_first()
        {
            if let Some((old, _)) = self.entries.remove(&oldest) {
                self.bytes -= cost(&old);
            }
        }
    }
}

/// The content search, shared by every hour and perspective: its result
/// depends on content alone, so it is cached for the session within
/// [`CACHE_BUDGET`], least recently used first out.
pub struct PathSearch {
    graph: ProvinceGraph,
    cache: Mutex<Cache>,
    budget: usize,
}
debug_fields!(PathSearch(this) {
    graph, budget
} {

});

impl PathSearch {
    pub fn new(graph: ProvinceGraph) -> Self {
        Self::with_budget(graph, CACHE_BUDGET)
    }

    fn with_budget(graph: ProvinceGraph, budget: usize) -> Self {
        Self {
            graph,
            cache: Mutex::default(),
            budget,
        }
    }

    pub fn graph(&self) -> &ProvinceGraph {
        &self.graph
    }

    fn search(&self, geometry: &ProvinceGeometry, from: u32, to: u32, path: RaidPath) -> Search {
        let key = (from, to, path);
        if let Some(result) = self.cache.lock().expect("raid path cache").get(&key) {
            return result;
        }
        // Searched outside the lock: builders of other hours may search too.
        let result = raid_distance(&self.graph, geometry, from, to, path);
        self.cache
            .lock()
            .expect("raid path cache")
            .insert(key, result.clone(), self.budget);
        result
    }
}

/// Pairs an evaluation asked beyond the recorded memo, with what each
/// holds.
#[derive(Clone, Debug, Default)]
pub struct AskedPairs(BTreeMap<MemoKey, Entry>);

impl AskedPairs {
    /// Whether both hold some pair.
    pub fn share_a_pair(&self, other: &Self) -> bool {
        let (small, large) = if self.0.len() <= other.0.len() {
            (self, other)
        } else {
            (other, self)
        };
        small.0.keys().any(|pair| large.0.contains_key(pair))
    }

    /// `later`'s pairs replace these.
    pub fn absorb(&mut self, later: Self) {
        self.0.extend(later.0);
    }
}

/// What one view of the hour's distances asked: the pairs asked before it
/// (`base`), those it asked since (`own`, which every pair it reads ends up
/// in) and its last naval target.
#[derive(Default)]
pub struct Ledger<'b> {
    base: Option<&'b AskedPairs>,
    own: RefCell<AskedPairs>,
    /// The last naval target resolved: a target asks it once per source.
    naval_target: RefCell<Option<(RaidScope, Target)>>,
}

impl<'b> Ledger<'b> {
    fn on(base: &'b AskedPairs) -> Self {
        Self {
            base: Some(base),
            ..Self::default()
        }
    }

    fn get(&self, pair: &MemoKey) -> Option<Entry> {
        self.own
            .borrow()
            .0
            .get(pair)
            .or_else(|| self.base.and_then(|base| base.0.get(pair)))
            .cloned()
    }
}

/// One evaluation's distance inputs: the hour's memo, its naval bases and
/// its province-to-state membership, each read on first use, and the pairs
/// asked so far this hour beyond the memo.
pub struct Distances<'a> {
    assets: &'a RaidAssets,
    frame: &'a dyn DecodedSource,
    /// Decoded rows ascend by (dispatcher, source, target).
    recorded: Result<&'a [RaidPathDistanceRow], String>,
    naval_bases: OnceLock<Result<BTreeSet<u32>, String>>,
    /// Each province's recorded state, and each state's province count.
    states: OnceLock<Result<StateMembers, String>>,
    /// The pairs asked this hour, in the order the hour's evaluations
    /// asked them.
    asked: Mutex<Ledger<'static>>,
}

impl<'a> Distances<'a> {
    pub fn new(assets: &'a RaidAssets, frame: &'a dyn DecodedSource) -> Self {
        Self {
            assets,
            frame,
            recorded: frame
                .complete::<tables::RaidPathDistances>()
                .map_err(|error| error.to_string()),
            naval_bases: OnceLock::new(),
            states: OnceLock::new(),
            asked: Mutex::default(),
        }
    }

    /// The pairs asked so far this hour.
    fn asked(&self) -> MutexGuard<'_, Ledger<'static>> {
        self.asked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn recorded(&self, pair: MemoKey) -> Result<Option<Option<i64>>, String> {
        let rows = self.recorded.as_ref().map_err(Clone::clone)?;
        Ok(rows
            .binary_search_by(|row| (row.dispatcher, row.source, row.target).cmp(&pair))
            .ok()
            .map(|at| (rows[at].ok != 0).then_some(rows[at].distance)))
    }

    fn naval_bases(&self) -> Result<&BTreeSet<u32>, String> {
        self.naval_bases
            .get_or_init(|| {
                self.frame
                    .complete::<tables::NavalBases>()
                    .map(|rows| rows.iter().map(|row| row.province).collect())
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn states(&self) -> Result<&StateMembers, String> {
        self.states
            .get_or_init(|| {
                self.frame
                    .complete::<tables::Provinces>()
                    .map(|rows| {
                        let mut sizes = BTreeMap::new();
                        for row in rows {
                            *sizes.entry(row.state).or_insert(0) += 1;
                        }
                        (rows.iter().map(|row| (row.id, row.state)).collect(), sizes)
                    })
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Distance for a naval-base (kind 2) or building (kind 5) source:
    /// `Ok(None)` is an unsuccessful search. A null source province measures
    /// zero and passes.
    pub fn distance(&self, kind: u8, from: u32, scope: &RaidScope) -> Search {
        self.distance_in(&self.asked(), kind, from, scope)
    }

    /// Notes that the game asked, or may have asked, a distance whose
    /// answer this evaluation does not need.
    pub fn note(&self, kind: u8, from: u32, scope: &RaidScope) {
        self.note_in(&self.asked(), kind, from, scope);
    }

    /// [`Self::distance`] with the pairs `ledger` holds.
    fn distance_in(&self, ledger: &Ledger<'_>, kind: u8, from: u32, scope: &RaidScope) -> Search {
        if from == 0 {
            return Ok(Some(0));
        }
        match self.target(ledger, kind, scope)? {
            Some((to, path)) => self
                .path(ledger, from, to, path, true)
                .expect("a needed answer"),
            None => Ok(None),
        }
    }

    /// [`Self::note`] into `ledger`.
    fn note_in(&self, ledger: &Ledger<'_>, kind: u8, from: u32, scope: &RaidScope) {
        if from != 0
            && let Ok(Some((to, path))) = self.target(ledger, kind, scope)
        {
            self.path(ledger, from, to, path, false);
        }
    }

    /// The province a source of `kind` measures to, and its search.
    fn target(
        &self,
        ledger: &Ledger<'_>,
        kind: u8,
        scope: &RaidScope,
    ) -> Result<Option<(u32, RaidPath)>, String> {
        Ok(match kind {
            NAVAL_BASE => self
                .cached_naval_target(ledger, scope)?
                .map(|to| (to, RaidPath::Naval)),
            BUILDING => self.assets.province(scope).map(|to| (to, RaidPath::Land)),
            other => return Err(format!("raid source kind {other} has no path search")),
        })
    }

    fn cached_naval_target(
        &self,
        ledger: &Ledger<'_>,
        scope: &RaidScope,
    ) -> Result<Option<u32>, String> {
        if let Some((last, target)) = &*ledger.naval_target.borrow()
            && last == scope
        {
            return target.clone();
        }
        let target = self.naval_target(scope);
        *ledger.naval_target.borrow_mut() = Some((scope.clone(), target.clone()));
        target
    }

    /// Use the target province when it has a naval base, else the first province
    /// of its state with one, through the recorded `NavalBases` keys.
    fn naval_target(&self, scope: &RaidScope) -> Result<Option<u32>, String> {
        let Some(province) = self.assets.province(scope) else {
            return Ok(None);
        };
        let bases = self.naval_bases()?;
        if bases.contains(&province) {
            return Ok(Some(province));
        }
        let (states, sizes) = self.states()?;
        let state = states
            .get(&province)
            .copied()
            .ok_or_else(|| format!("province {province} has no recorded state"))?;
        let members = self
            .assets
            .trigger_content
            .state_provinces
            .get(&state)
            .ok_or_else(|| format!("state {state} is not in the content"))?;
        // The runtime list keeps content order while membership is unchanged.
        if sizes.get(&state) != Some(&members.len())
            || members.iter().any(|id| states.get(id) != Some(&state))
        {
            return Err(format!(
                "state {state}'s provinces differ from the content, so their native order is unknown"
            ));
        }
        Ok(members.iter().copied().find(|id| bases.contains(id)))
    }

    /// The memo's answer, else this evaluation's, else a new search that
    /// this evaluation keeps for its later queries of the same pair. `None`
    /// when the answer is not `needed`.
    fn path(
        &self,
        ledger: &Ledger<'_>,
        from: u32,
        to: u32,
        path: RaidPath,
        needed: bool,
    ) -> Option<Search> {
        let pair = (path_dispatcher(path), from, to);
        let search = || -> Search {
            let paths = self.assets.paths.as_ref().map_err(Clone::clone)?;
            paths.search(&self.assets.geometry, from, to, path)
        };
        if let Ok(Some(result)) = self.recorded(pair) {
            return needed.then_some(Ok(result));
        }
        let known = ledger.get(&pair).flatten();
        let answer = if needed {
            Some(known.unwrap_or_else(search))
        } else {
            known
        };
        ledger.own.borrow_mut().0.insert(pair, answer.clone());
        answer.filter(|_| needed)
    }
}

/// One hour's path searches for the launch gate: distances through the
/// content search and the recorded memo, and army reach through
/// `unit_path`, each answered once per hour.
pub struct HourPaths<'a> {
    assets: &'a RaidAssets,
    distances: Distances<'a>,
    reach: viewer_foundation::map::movement_reads::Cells,
}

impl<'a> HourPaths<'a> {
    pub fn new(assets: &'a RaidAssets, frame: &'a dyn DecodedSource) -> Self {
        Self {
            assets,
            distances: Distances::new(assets, frame),
            reach: viewer_foundation::map::movement_reads::Cells::default(),
        }
    }
}

impl super::launch::RaidPaths for HourPaths<'_> {
    fn distance(
        &self,
        _: &World<'_>,
        source: &super::launch::PathSource,
        target: &RaidScope,
    ) -> Result<Option<i64>, String> {
        self.distances
            .distance(source.kind, source.province, target)
    }

    fn note(&self, source: &super::launch::PathSource, target: &RaidScope) {
        self.distances.note(source.kind, source.province, target);
    }

    fn recorded_rows(&self) -> Option<&[RaidPathDistanceRow]> {
        self.distances.recorded.as_ref().ok().copied()
    }

    fn path_target(&self, kind: u8, target: &RaidScope) -> Result<Option<u32>, String> {
        let ledger = self.distances.asked();
        Ok(self
            .distances
            .target(&ledger, kind, target)?
            .map(|(to, _)| to))
    }

    fn army_reaches(&self, world: &World<'_>, division: u64, source_province: u32) -> Eval {
        viewer_foundation::map::movement_reads::Moves::new(
            world,
            self.assets.movement_reads(),
            &self.reach,
        )
        .reaches(division, source_province)
    }

    fn hour(&self) -> Option<&HourPaths<'_>> {
        Some(self)
    }
}

impl HourPaths<'_> {
    /// The pairs asked so far this hour, held while a batch is evaluated
    /// in parts and until the parts' pairs are taken in.
    pub fn asked(&self) -> AskedSoFar<'_> {
        AskedSoFar(self.distances.asked())
    }

    /// The army search the hour's queries share ([`viewer_foundation::map::movement_reads::Moves`]).
    pub fn moves<'s, 'w>(
        &'s self,
        world: &'s World<'w>,
    ) -> viewer_foundation::map::movement_reads::Moves<'s, 'w> {
        viewer_foundation::map::movement_reads::Moves::new(
            world,
            self.assets.movement_reads(),
            &self.reach,
        )
    }
}

/// The hour's asked pairs, held for one batch.
pub struct AskedSoFar<'h>(MutexGuard<'h, Ledger<'static>>);

impl AskedSoFar<'_> {
    pub fn pairs(&mut self) -> &AskedPairs {
        self.0.own.get_mut()
    }

    /// A part's pairs, taken in after every earlier part's.
    pub fn absorb(&mut self, part: AskedPairs) {
        self.0.own.get_mut().absorb(part);
    }
}

/// The hour's path searches for one part of a batch: the pairs asked before
/// the part (`base`), and its own since.
pub struct PartPaths<'p, 'a> {
    hour: &'p HourPaths<'a>,
    ledger: Ledger<'p>,
}

impl<'p, 'a> PartPaths<'p, 'a> {
    pub fn new(hour: &'p HourPaths<'a>, base: &'p AskedPairs) -> Self {
        Self {
            hour,
            ledger: Ledger::on(base),
        }
    }

    /// The pairs this part asked, each with what it holds after the part.
    pub fn into_asked(self) -> AskedPairs {
        self.ledger.own.into_inner()
    }
}

impl super::launch::RaidPaths for PartPaths<'_, '_> {
    fn distance(
        &self,
        _: &World<'_>,
        source: &super::launch::PathSource,
        target: &RaidScope,
    ) -> Result<Option<i64>, String> {
        self.hour
            .distances
            .distance_in(&self.ledger, source.kind, source.province, target)
    }

    fn note(&self, source: &super::launch::PathSource, target: &RaidScope) {
        self.hour
            .distances
            .note_in(&self.ledger, source.kind, source.province, target);
    }

    fn recorded_rows(&self) -> Option<&[RaidPathDistanceRow]> {
        self.hour.distances.recorded.as_ref().ok().copied()
    }

    fn path_target(&self, kind: u8, target: &RaidScope) -> Result<Option<u32>, String> {
        Ok(self
            .hour
            .distances
            .target(&self.ledger, kind, target)?
            .map(|(to, _)| to))
    }

    fn army_reaches(&self, world: &World<'_>, division: u64, source_province: u32) -> Eval {
        viewer_foundation::map::movement_reads::Moves::new(
            world,
            self.hour.assets.movement_reads(),
            &self.hour.reach,
        )
        .reaches(division, source_province)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_evicts_the_least_recently_used_within_its_budget() {
        let mut cache = Cache::default();
        let key = |id| (id, id, RaidPath::Land);
        let budget = 3 * ENTRY_BYTES;
        for id in 1..=3 {
            cache.insert(key(id), Ok(Some(i64::from(id))), budget);
        }
        assert_eq!(cache.get(&key(1)), Some(Ok(Some(1))));
        cache.insert(key(4), Ok(None), budget);
        assert_eq!(cache.get(&key(2)), None, "least recent is gone");
        assert!(cache.get(&key(1)).is_some() && cache.get(&key(3)).is_some());
        cache.insert(key(5), Err("x".repeat(ENTRY_BYTES)), budget);
        assert!(cache.bytes <= budget);
        assert_eq!(cache.entries.len(), cache.recency.len());
    }

    #[test]
    fn recorded_dispatchers_keep_distances_and_failures_apart_without_a_graph() {
        use super::super::test_support::{frame, put, unnamed_assets};
        let assets = unnamed_assets();
        let scope = RaidScope {
            province: 3,
            ..Default::default()
        };
        for (land_ok, naval_ok) in [(1, 1), (0, 1), (1, 0)] {
            let mut frame = frame();
            put::<tables::NavalBases>(
                &mut frame,
                vec![replay_data::rows::NavalBaseRow {
                    province: 3,
                    ..Default::default()
                }],
            );
            put::<tables::RaidPathDistances>(
                &mut frame,
                vec![
                    RaidPathDistanceRow {
                        dispatcher: RaidPathDispatcher::Land,
                        source: 1,
                        target: 3,
                        distance: 900_000,
                        ok: land_ok,
                    },
                    RaidPathDistanceRow {
                        dispatcher: RaidPathDispatcher::Naval,
                        source: 1,
                        target: 3,
                        distance: 500_000,
                        ok: naval_ok,
                    },
                ],
            );
            let distances = Distances::new(&assets, &frame);
            assert_eq!(
                distances.distance(BUILDING, 1, &scope),
                Ok((land_ok != 0).then_some(900_000))
            );
            assert_eq!(
                distances.distance(NAVAL_BASE, 1, &scope),
                Ok((naval_ok != 0).then_some(500_000))
            );
        }
    }

    #[test]
    fn an_unrecorded_dispatcher_never_borrows_the_other_memos_pair() {
        use super::super::test_support::{frame, put, unnamed_assets};
        let assets = unnamed_assets();
        let mut frame = frame();
        put::<tables::NavalBases>(
            &mut frame,
            vec![replay_data::rows::NavalBaseRow {
                province: 3,
                ..Default::default()
            }],
        );
        put::<tables::RaidPathDistances>(
            &mut frame,
            vec![RaidPathDistanceRow {
                dispatcher: RaidPathDispatcher::Land,
                source: 1,
                target: 3,
                distance: 900_000,
                ok: 1,
            }],
        );
        let distances = Distances::new(&assets, &frame);
        let scope = RaidScope {
            province: 3,
            ..Default::default()
        };
        assert_eq!(distances.distance(BUILDING, 1, &scope), Ok(Some(900_000)));
        assert!(distances.distance(NAVAL_BASE, 1, &scope).is_err());
    }

    #[test]
    fn batches_share_pairs_only_within_the_same_dispatcher() {
        let mut land = AskedPairs::default();
        let mut naval = AskedPairs::default();
        land.0
            .insert((RaidPathDispatcher::Land, 7, 9), Some(Ok(Some(900_000))));
        naval
            .0
            .insert((RaidPathDispatcher::Naval, 7, 9), Some(Ok(Some(500_000))));
        assert!(!land.share_a_pair(&naval));
        assert!(land.share_a_pair(&land.clone()));
        land.absorb(naval);
        assert_eq!(land.0.len(), 2);
    }
}
