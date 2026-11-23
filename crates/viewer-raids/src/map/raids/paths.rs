
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak},
};
use viewer_runtime::debug_fields;

pub(crate) const NAVAL_BASE: u8 = engine_native::craid_source::KIND_NAVAL_BASE as u8;
pub(crate) const BUILDING: u8 = engine_native::craid_source::KIND_BUILDING as u8;

use content_reader_map::{province_geometry::ProvinceGeometry, province_graph::{ProvinceGraph, RaidPath, raid_distance}};
use replay_data::{
    rows::RaidPathDispatcher, rows::RaidPathDistanceRow, rows::RaidScope, rows::tables,
};
use replay_frame::frame::DecodedSource;

use super::{
    RaidAssets,
    script::{Eval, World},
};

type Search = Result<Option<i64>, String>;
type Target = Result<Option<u32>, String>;

type MemoKey = (RaidPathDispatcher, u32, u32);
type Entry = Option<Held>;

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
type StateMembers = (BTreeMap<u32, u32>, BTreeMap<u32, usize>);

type Memo = Mutex<HashMap<Key, Weak<Searched>>>;

pub struct Searched {
    result: Search,
    memo: Option<(Weak<Memo>, Key)>,
}

pub type Held = Arc<Searched>;

impl Searched {
    fn unkept(result: Search) -> Held {
        Arc::new(Self { result, memo: None })
    }
}

impl Drop for Searched {
    fn drop(&mut self) {
        let this: *const Self = self;
        let Some((memo, key)) = &self.memo else {
            return;
        };
        let Some(memo) = memo.upgrade() else {
            return;
        };
        let mut entries = memo.lock().unwrap_or_else(PoisonError::into_inner);
        if entries
            .get(key)
            .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), this))
        {
            entries.remove(key);
        }
    }
}

impl fmt::Debug for Searched {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.result.fmt(f)
    }
}

pub struct PathSearch {
    graph: ProvinceGraph,
    searched: Arc<Memo>,
}
debug_fields!(PathSearch(this) {
    graph
} {

});

impl PathSearch {
    pub fn new(graph: ProvinceGraph) -> Self {
        Self {
            graph,
            searched: Arc::default(),
        }
    }

    pub fn graph(&self) -> &ProvinceGraph {
        &self.graph
    }

    fn memo(&self) -> MutexGuard<'_, HashMap<Key, Weak<Searched>>> {
        self.searched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn search(&self, geometry: &ProvinceGeometry, from: u32, to: u32, path: RaidPath) -> Held {
        let key = (from, to, path);
        if let Some(held) = self.memo().get(&key).and_then(Weak::upgrade) {
            return held;
        }
        let result = raid_distance(&self.graph, geometry, from, to, path);
        if result.is_err() {
            return Searched::unkept(result);
        }
        let mut memo = self.memo();
        if let Some(raced) = memo.get(&key).and_then(Weak::upgrade) {
            return raced;
        }
        let held = Arc::new(Searched {
            result,
            memo: Some((Arc::downgrade(&self.searched), key)),
        });
        memo.insert(key, Arc::downgrade(&held));
        held
    }
}

#[derive(Clone, Debug, Default)]
pub struct AskedPairs(BTreeMap<MemoKey, Entry>);

impl AskedPairs {
    pub fn share_a_pair(&self, other: &Self) -> bool {
        let (small, large) = if self.0.len() <= other.0.len() {
            (self, other)
        } else {
            (other, self)
        };
        small.0.keys().any(|pair| large.0.contains_key(pair))
    }

    pub fn absorb(&mut self, later: Self) {
        self.0.extend(later.0);
    }
}

#[derive(Default)]
pub struct Ledger<'b> {
    base: Option<&'b AskedPairs>,
    own: RefCell<AskedPairs>,
    naval_target: RefCell<Option<(RaidScope, Target)>>,
    used: RefCell<Vec<Held>>,
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

    fn take_held(&self) -> Box<[Held]> {
        let mut used = self.used.take();
        used.sort_unstable_by_key(Arc::as_ptr);
        used.dedup_by(|a, b| Arc::ptr_eq(a, b));
        used.into_boxed_slice()
    }
}

pub struct Distances<'a> {
    assets: &'a RaidAssets,
    frame: &'a dyn DecodedSource,
    recorded: Result<&'a [RaidPathDistanceRow], String>,
    naval_bases: OnceLock<Result<BTreeSet<u32>, String>>,
    states: OnceLock<Result<StateMembers, String>>,
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

    pub fn distance(&self, kind: u8, from: u32, scope: &RaidScope) -> Search {
        self.distance_in(&self.asked(), kind, from, scope)
    }

    pub fn note(&self, kind: u8, from: u32, scope: &RaidScope) {
        self.note_in(&self.asked(), kind, from, scope);
    }

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

    fn note_in(&self, ledger: &Ledger<'_>, kind: u8, from: u32, scope: &RaidScope) {
        if from != 0
            && let Ok(Some((to, path))) = self.target(ledger, kind, scope)
        {
            self.path(ledger, from, to, path, false);
        }
    }

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
        if sizes.get(&state) != Some(&members.len())
            || members.iter().any(|id| states.get(id) != Some(&state))
        {
            return Err(format!(
                "state {state}'s provinces differ from the content, so their native order is unknown"
            ));
        }
        Ok(members.iter().copied().find(|id| bases.contains(id)))
    }

    fn path(
        &self,
        ledger: &Ledger<'_>,
        from: u32,
        to: u32,
        path: RaidPath,
        needed: bool,
    ) -> Option<Search> {
        let pair = (path_dispatcher(path), from, to);
        let search = || -> Held {
            match &self.assets.paths {
                Ok(paths) => paths.search(&self.assets.geometry, from, to, path),
                Err(error) => Searched::unkept(Err(error.clone())),
            }
        };
        if let Ok(Some(result)) = self.recorded(pair) {
            return needed.then_some(Ok(result));
        }
        let known = ledger.get(&pair).flatten();
        let answer = if needed {
            let held = known.unwrap_or_else(search);
            ledger.used.borrow_mut().push(Arc::clone(&held));
            Some(held)
        } else {
            known
        };
        let result = answer
            .as_ref()
            .filter(|_| needed)
            .map(|held| held.result.clone());
        ledger.own.borrow_mut().0.insert(pair, answer);
        result
    }

    pub fn take_held(&self) -> Box<[Held]> {
        self.asked().take_held()
    }
}

pub struct HourPaths<'a> {
    assets: &'a RaidAssets,
    distances: Distances<'a>,
    reach: viewer_unit_reads::movement_reads::Cells,
}

impl<'a> HourPaths<'a> {
    pub fn new(assets: &'a RaidAssets, frame: &'a dyn DecodedSource) -> Self {
        Self {
            assets,
            distances: Distances::new(assets, frame),
            reach: viewer_unit_reads::movement_reads::Cells::default(),
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
        viewer_unit_reads::movement_reads::Moves::new(
            world,
            self.assets.movement_reads(),
            &self.reach,
        )
        .reaches(division, source_province)
    }

    fn hour(&self) -> Option<&HourPaths<'_>> {
        Some(self)
    }

    fn take_held(&self) -> Box<[Held]> {
        self.distances.take_held()
    }
}

impl HourPaths<'_> {
    pub fn asked(&self) -> AskedSoFar<'_> {
        AskedSoFar(self.distances.asked())
    }

    pub fn moves<'s, 'w>(
        &'s self,
        world: &'s World<'w>,
    ) -> viewer_unit_reads::movement_reads::Moves<'s, 'w> {
        viewer_unit_reads::movement_reads::Moves::new(
            world,
            self.assets.movement_reads(),
            &self.reach,
        )
    }
}

pub struct AskedSoFar<'h>(MutexGuard<'h, Ledger<'static>>);

impl AskedSoFar<'_> {
    pub fn pairs(&mut self) -> &AskedPairs {
        self.0.own.get_mut()
    }

    pub fn absorb(&mut self, part: AskedPairs) {
        self.0.own.get_mut().absorb(part);
    }
}

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
        viewer_unit_reads::movement_reads::Moves::new(
            world,
            self.hour.assets.movement_reads(),
            &self.hour.reach,
        )
        .reaches(division, source_province)
    }

    fn take_held(&self) -> Box<[Held]> {
        self.ledger.take_held()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        land.0.insert(
            (RaidPathDispatcher::Land, 7, 9),
            Some(Searched::unkept(Ok(Some(900_000)))),
        );
        naval.0.insert(
            (RaidPathDispatcher::Naval, 7, 9),
            Some(Searched::unkept(Ok(Some(500_000)))),
        );
        assert!(!land.share_a_pair(&naval));
        assert!(land.share_a_pair(&land.clone()));
        land.absorb(naval);
        assert_eq!(land.0.len(), 2);
    }
}
