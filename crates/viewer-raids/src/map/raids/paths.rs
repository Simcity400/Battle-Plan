//! Raid source distances from the content graph and recorded session memo.
//!
//! Air, rocket, carrier and submarine sources measure the wrapped centroid
//! distance; naval bases and buildings run the raid path search
//! (`province_graph::raid_distance`). The game memoises those searches for
//! the session in one table keyed by the province pair only, so a pair's
//! first computed kind answers every later query (`RaidPathDistances`). An
//! hour's evaluation reads that memo and adds its own results in the order
//! it asks, as the game's rebuild does.
//!
//! Ask distance only after every earlier target gate passes and no earlier
//! source is usable. If an earlier input is undecided, whether the pair was
//! asked is unknown. Keep it as maybe asked; decide a later query only when
//! every kind that might have been asked first gives the same answer.
//!
//! Those asked pairs are the only state a target's evaluation leaves for the
//! next. A batch split across threads ([`PartPaths`]) gives each part the
//! pairs asked before the batch and keeps what the part asks in its own
//! [`Ledger`]; the parts' ledgers are then taken in batch order, and a part
//! that asked a pair an earlier part asked is evaluated again after it
//! ([`super::offers::Sweep`]).

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Mutex, MutexGuard, OnceLock},
};
use viewer_foundation::debug_fields;

/// Source kinds (`engine::craid_source`) whose distance uses a path:
/// a naval base measures to the target's naval path target, a building
/// to its province.
pub(crate) const NAVAL_BASE: u8 = engine::craid_source::KIND_NAVAL_BASE as u8;
pub(crate) const BUILDING: u8 = engine::craid_source::KIND_BUILDING as u8;

use installed_content::{
    province_geometry::ProvinceGeometry,
    province_graph::{ProvinceGraph, RaidPath, raid_distance},
};
use replay::{
    frame::DecodedFrame,
    rows::{RaidPathDistanceRow, RaidScope, tables},
};

use super::{
    RaidAssets,
    script::{Eval, World},
};

type Search = Result<Option<i64>, String>;
/// A resolved target province, `None` when there is none.
type Target = Result<Option<u32>, String>;

/// Whether the game certainly asked a distance, so that the session memo
/// holds the pair's first kind from then on, or only may have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    Certain,
    Maybe,
}

/// A pair this evaluation asked, beyond the recorded memo.
#[derive(Clone, Debug)]
enum Entry {
    /// The game asked it first by this kind; the answer once searched.
    Certain(RaidPath, Option<Search>),
    /// The game may have asked it by these kinds (a bit per kind).
    Maybe(u8),
    /// The game asked it, by a kind that is not known and that matters.
    Ambiguous,
}

fn bit(path: RaidPath) -> u8 {
    match path {
        RaidPath::Naval => 1,
        RaidPath::Land => 2,
    }
}

const KINDS: [RaidPath; 2] = [RaidPath::Naval, RaidPath::Land];
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
pub struct AskedPairs(BTreeMap<(u32, u32), Entry>);

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

    fn get(&self, pair: &(u32, u32)) -> Option<Entry> {
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
    frame: &'a DecodedFrame,
    /// The recorded memo, in its key order (source, target): a decoded table
    /// holds unique ascending keys (`replay::frame::validate_rows`).
    recorded: Result<&'a [RaidPathDistanceRow], String>,
    naval_bases: OnceLock<Result<BTreeSet<u32>, String>>,
    /// Each province's recorded state, and each state's province count.
    states: OnceLock<Result<StateMembers, String>>,
    /// The pairs asked this hour, in the order the hour's evaluations
    /// asked them.
    asked: Mutex<Ledger<'static>>,
}

impl<'a> Distances<'a> {
    pub fn new(assets: &'a RaidAssets, frame: &'a DecodedFrame) -> Self {
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

    fn recorded(&self, pair: (u32, u32)) -> Result<Option<Option<i64>>, String> {
        let rows = self.recorded.as_ref().map_err(Clone::clone)?;
        Ok(rows
            .binary_search_by(|row| (row.source, row.target).cmp(&pair))
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
    pub fn distance(&self, kind: u8, from: u32, scope: &RaidScope, asked: Asked) -> Search {
        self.distance_in(&self.asked(), kind, from, scope, asked)
    }

    /// Notes that the game asked, or may have asked, a distance whose
    /// answer this evaluation does not need.
    pub fn note(&self, kind: u8, from: u32, scope: &RaidScope, asked: Asked) {
        self.note_in(&self.asked(), kind, from, scope, asked);
    }

    /// [`Self::distance`] with the pairs `ledger` holds.
    fn distance_in(
        &self,
        ledger: &Ledger<'_>,
        kind: u8,
        from: u32,
        scope: &RaidScope,
        asked: Asked,
    ) -> Search {
        if from == 0 {
            return Ok(Some(0));
        }
        match self.target(ledger, kind, scope)? {
            Some((to, path)) => self
                .path(ledger, from, to, path, asked, true)
                .expect("a needed answer"),
            None => Ok(None),
        }
    }

    /// [`Self::note`] into `ledger`.
    fn note_in(&self, ledger: &Ledger<'_>, kind: u8, from: u32, scope: &RaidScope, asked: Asked) {
        if from != 0
            && let Ok(Some((to, path))) = self.target(ledger, kind, scope)
        {
            self.path(ledger, from, to, path, asked, false);
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
        asked: Asked,
        needed: bool,
    ) -> Option<Search> {
        let pair = (from, to);
        let search = |kind: RaidPath| -> Search {
            let paths = self.assets.paths.as_ref().map_err(Clone::clone)?;
            paths.search(&self.assets.geometry, from, to, kind)
        };
        // One answer whichever of `kinds` the game searched first.
        let agreed = |kinds: u8, reason: &dyn Fn() -> String| -> Search {
            let mut answer = None;
            for kind in KINDS.into_iter().filter(|&kind| kinds & bit(kind) != 0) {
                let result = search(kind)?;
                if answer.is_some_and(|known| known != result) {
                    return Err(reason());
                }
                answer = Some(result);
            }
            Ok(answer.flatten())
        };
        match &self.recorded {
            Ok(_) => {
                if let Ok(Some(result)) = self.recorded(pair) {
                    return needed.then_some(Ok(result));
                }
            }
            // Any pair may already hold either kind.
            Err(reason) => {
                return needed.then(|| {
                    agreed(bit(RaidPath::Naval) | bit(RaidPath::Land), &|| {
                        format!(
                            "raid path {from}->{to} differs by kind and the memo is unknown: {reason}"
                        )
                    })
                });
            }
        }
        // Every pair read here is written back below, so a ledger's own
        // pairs are all it read of the pairs asked before it.
        let (answer, next) = match (ledger.get(&pair), asked) {
            (None, Asked::Certain) => {
                let answer = needed.then(|| search(path));
                (answer.clone(), Entry::Certain(path, answer))
            }
            (None, Asked::Maybe) => (needed.then(|| search(path)), Entry::Maybe(bit(path))),
            (Some(Entry::Certain(kind, known)), _) => {
                let known = match known {
                    Some(known) => Some(known),
                    None if needed => Some(search(kind)),
                    None => None,
                };
                (
                    known.clone().filter(|_| needed),
                    Entry::Certain(kind, known),
                )
            }
            (Some(Entry::Maybe(kinds)), asked) => {
                // Unasked, the game searches this query's own kind.
                let kinds = kinds | bit(path);
                let why = || {
                    format!(
                        "raid path {from}->{to} differs by kind, and an undecided earlier offer may have asked it first"
                    )
                };
                let answer = needed.then(|| agreed(kinds, &why));
                let next = match (&answer, asked) {
                    (Some(Ok(result)), Asked::Certain) => Entry::Certain(path, Some(Ok(*result))),
                    (Some(Err(_)), Asked::Certain) => Entry::Ambiguous,
                    _ => Entry::Maybe(kinds),
                };
                (answer, next)
            }
            (Some(Entry::Ambiguous), _) => (
                needed.then(|| {
                    Err(format!(
                        "raid path {from}->{to} was first asked by a kind an undecided earlier offer leaves unknown"
                    ))
                }),
                Entry::Ambiguous,
            ),
        };
        ledger.own.borrow_mut().0.insert(pair, next);
        answer
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
    pub fn new(assets: &'a RaidAssets, frame: &'a DecodedFrame) -> Self {
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
        asked: Asked,
    ) -> Result<Option<i64>, String> {
        self.distances
            .distance(source.kind, source.province, target, asked)
    }

    fn note(&self, source: &super::launch::PathSource, target: &RaidScope, asked: Asked) {
        self.distances
            .note(source.kind, source.province, target, asked);
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
        asked: Asked,
    ) -> Result<Option<i64>, String> {
        self.hour
            .distances
            .distance_in(&self.ledger, source.kind, source.province, target, asked)
    }

    fn note(&self, source: &super::launch::PathSource, target: &RaidScope, asked: Asked) {
        self.hour
            .distances
            .note_in(&self.ledger, source.kind, source.province, target, asked);
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
}
