//! Shared movement content, division timing derivation and the one hourly ETA cache.
use crate::{map::tables::rows, table_access::TokenKeys};
use installed_content_core::{
    movement::MovementDefines, movement::MovementMap, movement::RailMovementDefines,
    terrain::TerrainDefinitions,
};
use installed_content_regions::regions::StrategicRegion;
use installed_content_unit_path::unit_path::Fact;
use replay_data::{rows::DivisionRow, rows::tables};
use replay_derive::{
    eta_derive, eta_derive::Defines, eta_derive::OrganisationDefines, eta_derive::ProvinceOperands,
    eta_derive::RailDefines, eta_derive::frame_terms::Geography,
    eta_derive::frame_terms::SubUnitContent,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io,
    sync::{Arc, Mutex, Weak},
};

mod worker;
pub use worker::WorkerEtas;

/// The branch and recorded hour one cached derivation is for.
pub type Key = (replay_data::BranchId, replay_data::Hour);

/// The hours' ETAs, so no hour is derived twice: the latest derived or asked for, and every
/// other while an hour in flight holds it.
///
/// The window thread builds the counter layer for the playhead while workers
/// build the close-zoom models for the hours ahead, so the hours in flight are
/// not one but several and a single slot thrashes between them.
#[derive(Default)]
pub struct EtaCache {
    entries: Mutex<Held>,
}

#[derive(Default)]
struct Held {
    latest: Option<(Key, Arc<Etas>)>,
    earlier: Vec<(Key, Weak<Etas>)>,
}

impl Held {
    /// `key`'s ETAs, then the latest.
    fn get(&mut self, key: Key) -> Option<Arc<Etas>> {
        if let Some((at, etas)) = &self.latest
            && *at == key
        {
            return Some(Arc::clone(etas));
        }
        let etas = self
            .earlier
            .iter()
            .find(|(at, _)| *at == key)
            .and_then(|(_, etas)| etas.upgrade())?;
        self.put(key, &etas);
        Some(etas)
    }

    fn put(&mut self, key: Key, etas: &Arc<Etas>) {
        self.earlier
            .retain(|(at, earlier)| *at != key && earlier.strong_count() > 0);
        if let Some((at, earlier)) = self.latest.replace((key, Arc::clone(etas)))
            && at != key
        {
            self.earlier.push((at, Arc::downgrade(&earlier)));
        }
    }
}

impl EtaCache {
    /// This key's entry, building and inserting it if the cache lacks one.
    ///
    /// `build` runs with **no lock held**, so the window thread never waits on
    /// a worker deriving a different hour. Two threads can therefore build the
    /// same key at once; the first to insert wins and both share its result.
    pub fn get_or_insert_with<E>(
        &self,
        key: Key,
        build: impl FnOnce() -> Result<Etas, E>,
    ) -> Result<Arc<Etas>, E> {
        if let Some(etas) = self.get(key) {
            return Ok(etas);
        }
        let etas = Arc::new(build()?);
        let mut entries = self.entries.lock().expect("ETA cache poisoned");
        if let Some(existing) = entries.get(key) {
            return Ok(existing);
        }
        entries.put(key, &etas);
        Ok(etas)
    }

    /// This frame's ETAs, derived from `movement` if the cache lacks them.
    pub fn etas(
        &self,
        frame: &DecodedFrame,
        movement: Option<&MovementContent>,
    ) -> io::Result<Arc<Etas>> {
        self.get_or_insert_with((frame.branch, frame.hour), || Etas::read(frame, movement))
    }

    fn get(&self, key: Key) -> Option<Arc<Etas>> {
        self.entries.lock().expect("ETA cache poisoned").get(key)
    }
}

/// How many moving divisions have an arrival estimate, and why the others do not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Moving divisions with an arrival estimate.
    pub estimated: usize,
    /// Moving divisions without one, counted by the estimate's refusal.
    pub refused: BTreeMap<Arc<str>, usize>,
    /// Divisions whose first-edge progress (the arrow fill target) cannot be rebuilt.
    pub progress_refused: usize,
    /// Moving divisions whose route the content map cannot place; they stand
    /// on their province.
    pub routes_unresolved: usize,
    /// Later hops of placed routes that the movement polyline skips: no content
    /// neighbour of the province before them, or no anchor.
    pub route_hops_skipped: usize,
}

impl Report {
    /// Moving divisions without an arrival estimate.
    pub fn refused_total(&self) -> usize {
        self.refused.values().sum()
    }

    /// The dump report's line, or `None` for an hour with no movement at all.
    pub fn line(&self) -> Option<String> {
        let refused = self.refused_total();
        (self.estimated != 0
            || refused != 0
            || self.progress_refused != 0
            || self.routes_unresolved != 0
            || self.route_hops_skipped != 0)
            .then(|| {
                let mut line = format!(
                    "Movement timing: {} arrival estimates, {refused} estimates refused",
                    self.estimated
                );
                for (index, (reason, count)) in self.refused.iter().enumerate() {
                    let open = if index == 0 { " (" } else { "; " };
                    let _ = write!(line, "{open}{count} {reason}");
                }
                if !self.refused.is_empty() {
                    line.push(')');
                }
                let _ = write!(
                    line,
                    ", {} progress rows refused, {} routes unplaced, {} route hops skipped",
                    self.progress_refused, self.routes_unresolved, self.route_hops_skipped
                );
                line
            })
    }
}

/// What the movement timing reads from the recorded content stack, loaded once
/// per document: the movement map and the movement `NDefines`
/// ([`MovementDefines::read`]), so a mod's defines apply.
#[derive(Debug)]
pub struct MovementContent {
    pub map: MovementMap,
    pub defines: Defines,
    pub rail: Result<RailMovementDefines, String>,
    pub task_force_terrain: Result<TerrainSpeedContent, String>,
    /// The land terrain categories' own unit adjuster movement, by one-based terrain
    /// index, and each province's strategic region: the movement speed's terrain and
    /// region weather terms.
    pub land: Result<LandSpeedContent, String>,
}

/// The content the land movement speed reads beyond the movement map.
#[derive(Debug)]
pub struct LandSpeedContent {
    /// Each terrain category's common unit adjuster movement, in database order: entry
    /// `i` is terrain index `i + 1`.
    pub terrain_movement: Vec<i64>,
    /// Each province's strategic region id.
    pub regions: BTreeMap<u32, u32>,
    /// The sub-unit database: the special-forces share and the armour term read it.
    pub sub_units: Result<Arc<installed_content_core::unit_leader::SubUnits>, String>,
}

impl Default for LandSpeedContent {
    fn default() -> Self {
        Self {
            terrain_movement: Vec::new(),
            regions: BTreeMap::new(),
            sub_units: Err("the content's sub-unit database is unavailable".into()),
        }
    }
}

impl MovementContent {
    pub fn new(map: MovementMap, defines: MovementDefines) -> Self {
        Self {
            map,
            defines: Defines {
                embark_distance: defines.embark_distance,
                transport_scale: defines.transport_scale,
                invasion_speed: defines.invasion_speed,
                outer_scale: defines.outer_scale,
                river_crossing_two: defines.river_crossing_two,
                river_crossing_three: defines.river_crossing_three,
                organisation: defines
                    .organisation
                    .ok()
                    .map(|(threshold, penalty)| OrganisationDefines { threshold, penalty }),
                rail: defines.rail.as_ref().ok().map(|rail| RailDefines {
                    speed: rail.speed,
                    maximum_level: rail.maximum_level,
                }),
                noncore_supply: defines
                    .noncore_supply
                    .each_ref()
                    .map(|value| value.as_ref().ok().copied()),
                combat_movement: defines.combat_movement.as_ref().ok().copied(),
            },
            rail: defines.rail,
            task_force_terrain: Err("fresh task force terrain content is unavailable".into()),
            land: Err("the land movement terrain content is unavailable".into()),
        }
    }

    pub fn set_task_force_terrain(
        &mut self,
        terrain: Result<Arc<installed_content_core::terrain::TerrainDefinitions>, String>,
        regions: &BTreeMap<u32, installed_content_regions::regions::StrategicRegion>,
        subunits: Result<TokenKeys, String>,
        sub_units: Result<Arc<installed_content_core::unit_leader::SubUnits>, String>,
    ) {
        self.land = terrain
            .as_ref()
            .map_err(Clone::clone)
            .map(|terrain| LandSpeedContent {
                terrain_movement: terrain
                    .categories
                    .iter()
                    .map(|category| category.common_adjuster.movement)
                    .collect(),
                regions: regions
                    .iter()
                    .flat_map(|(&id, region)| {
                        region.provinces.iter().map(move |&province| (province, id))
                    })
                    .collect(),
                sub_units,
            });
        self.task_force_terrain =
            terrain.map(|terrain| TerrainSpeedContent::new(terrain, regions, subunits));
    }
}

/// Shares the existing typed terrain database. Mission/range defines cannot
/// make a task-force departure adjuster unavailable.
#[derive(Debug)]
pub struct TerrainSpeedContent {
    terrain: Arc<TerrainDefinitions>,
    regions: BTreeMap<u32, Option<usize>>,
    subunits: Fact<TokenKeys>,
}

impl TerrainSpeedContent {
    pub fn new(
        terrain: Arc<TerrainDefinitions>,
        regions: &BTreeMap<u32, StrategicRegion>,
        subunits: Fact<TokenKeys>,
    ) -> Self {
        // The generic terrain-validity byte is 1 for real categories.
        // Unknown or absent authored terrain is null and uses the five-byte fallback key.
        // The authored naval_terrain boolean is independent of that validity byte.
        let fallback = terrain.category(b"ocean");
        let regions = regions
            .iter()
            .map(|(&id, region)| {
                let selected = region
                    .naval_terrain
                    .as_ref()
                    .and_then(|name| terrain.category(name.as_bytes()))
                    .or(fallback);
                (id, selected)
            })
            .collect();
        Self {
            terrain,
            regions,
            subunits,
        }
    }

    pub fn movement<'a>(
        &self,
        region: u32,
        class_name: impl FnOnce() -> Fact<Option<&'a str>>,
    ) -> Fact<i64> {
        let selected = self
            .regions
            .get(&region)
            .ok_or_else(|| format!("fresh naval ETA region {region} has no content template"))?;
        let Some(at) = *selected else {
            // The terrain database supplies a null terrain; its common unit adjuster
            // initializes all three values to zero.
            return Ok(0);
        };
        let terrain = &self.terrain.categories[at];
        let common = terrain.common_adjuster.movement;
        if terrain.classes.is_empty() {
            return Ok(common);
        }
        let Some(name) = class_name()? else {
            return Ok(common);
        };
        let Some(at) = terrain
            .classes
            .iter()
            .position(|(key, _)| key.eq_ignore_ascii_case(name.as_bytes()))
        else {
            return Ok(common);
        };
        // Skip an authored class whose sub-unit resolves to null.
        // Registered repeated class tokens keep their first bucket.
        if self
            .subunits
            .as_ref()
            .map_err(Clone::clone)?
            .resolve(name)?
            .is_none()
        {
            return Ok(common);
        }
        let own = terrain
            .class_adjusters
            .get(at)
            .ok_or("fresh naval ETA terrain class adjuster is unavailable")?
            .movement;
        // Merge common values into each class after load.
        Ok(common.wrapping_add(own))
    }
}

/// The province operands an estimate reads: the content terrain, whose flags
/// decide the embark edge. `None` for a province the content
/// map lacks, which an estimate refuses as missing geometry.
pub fn operands(map: &MovementMap, province: u32) -> Option<ProvinceOperands> {
    map.terrain(province).map(|terrain| ProvinceOperands {
        terrain_flags: terrain.flags,
        terrain_index: terrain.index,
        ..Default::default()
    })
}

/// The hour's recorded speed-term operands ([`replay_derive::eta_derive::frame_terms`]).
pub use replay_derive::eta_derive::frame_terms::FrameTerms as HourTerms;

impl SubUnitContent for MovementContent {
    fn special_forces(&self, name: &[u8]) -> Result<bool, String> {
        let land = self.land.as_ref().map_err(Clone::clone)?;
        land.sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .special_forces(name)
    }

    fn sub_unit_types(&self, name: &[u8]) -> Result<u64, String> {
        let land = self.land.as_ref().map_err(Clone::clone)?;
        land.sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .category(name)
    }
}

impl Geography for MovementContent {
    fn terrain_index(&self, province: u32) -> Option<i32> {
        self.map.terrain(province).map(|terrain| terrain.index)
    }

    fn terrain_flags(&self, province: u32) -> Option<u8> {
        self.map.terrain(province).map(|terrain| terrain.flags)
    }

    fn terrain_movement(&self, index: i32) -> Option<i64> {
        let land = self.land.as_ref().ok()?;
        usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|at| land.terrain_movement.get(at))
            .copied()
    }

    fn crossing_kind(&self, from: u32, to: u32) -> Option<u8> {
        self.map.crossing_kind(from, to)
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.land.as_ref().ok()?.regions.get(&province).copied()
    }
}

/// One hour's arrival estimates and first-edge progress per division handle.
#[derive(Clone, Debug, Default)]
pub struct Etas {
    by_division: Pairs,
    /// Why a division the hour moves has no estimate.
    estimate_gaps: BTreeMap<u64, Arc<str>>,
    progress: BTreeMap<u64, i64>,
    /// Why a division the hour moves has no progress.
    progress_gaps: BTreeMap<u64, &'static str>,
    report: Report,
}

/// Why a division without a moving route has no estimate.
const NO_ROUTE: &str = "it has no ordinary route this hour";
/// Why no moving division has an estimate or progress without content.
const NO_CONTENT: &str = "the content's movement map (terrain and movement defines) failed to load";
/// Why a moving division absent from the hour's census has neither.
const NO_LOCATION: &str = "it has no Divisions row this hour";

impl Etas {
    /// `movement` is `None` when the content's movement map failed to load:
    /// every moving division then has neither progress nor an estimate.
    pub fn read(frame: &DecodedFrame, movement: Option<&MovementContent>) -> io::Result<Self> {
        derive(frame, movement)
    }

    /// This division's estimated `(next_remaining, destination_remaining)`,
    /// native signed fixed5 hours, or `None` where the hour has none.
    pub fn get(&self, division: u64) -> Option<(i64, i64)> {
        self.by_division.get(&division).copied()
    }

    /// The same estimate, or why the hour has none: for a moving division the
    /// replay estimate's own refusal (for example a recording that holds no
    /// `DivisionMovement.base_speed`).
    pub fn estimate(&self, division: u64) -> Result<(i64, i64), &str> {
        match self.by_division.get(&division) {
            Some(pair) => Ok(*pair),
            None => Err(self
                .estimate_gaps
                .get(&division)
                .map_or(NO_ROUTE, |reason| reason)),
        }
    }

    /// Native first-edge movement fraction in signed fixed5, sentinel and all, or why the hour
    /// cannot rebuild it. Zero for an empty route. In the game it drives the movement arrow's fill
    /// target of a selected own unit (see the module docs) and the naval invasion progress in a
    /// combat's division hover; a counter never reads it.
    pub fn progress(&self, division: u64) -> Result<i64, &'static str> {
        match self.progress.get(&division) {
            Some(progress) => Ok(*progress),
            None => Err(self
                .progress_gaps
                .get(&division)
                .copied()
                .unwrap_or("it has no DivisionMovement row this hour")),
        }
    }

    pub fn report(&self) -> &Report {
        &self.report
    }

    /// Records why `division` has no estimate; equal reasons share one string.
    fn refuse(&mut self, division: u64, reason: &str) {
        let reason = match self.report.refused.get_key_value(reason) {
            Some((reason, _)) => Arc::clone(reason),
            None => Arc::from(reason),
        };
        *self.report.refused.entry(Arc::clone(&reason)).or_default() += 1;
        self.estimate_gaps.insert(division, reason);
    }
}

/// `(next_remaining, destination_remaining)` per division handle.
type Pairs = BTreeMap<u64, (i64, i64)>;

/// Every moving division's progress and estimate, or why it has none.
fn derive(frame: &DecodedFrame, content: Option<&MovementContent>) -> io::Result<Etas> {
    let movement = rows::<tables::DivisionMovement>(frame)?;
    let mut etas = Etas::default();
    // A division with no route stands still: progress zero needs no content.
    for row in movement.rows().iter().filter(|row| row.path_total == 0) {
        match eta_derive::division_progress(row, None, |_| None, |_, _| None, &Defines::default()) {
            Some(value) => {
                etas.progress.insert(row.division, value);
            }
            None => {
                etas.progress_gaps.insert(
                    row.division,
                    "eta_derive::division_progress refuses its empty route",
                );
            }
        }
    }
    let moving = movement.rows().iter().filter(|row| row.path_total != 0);
    let Some(content) = content else {
        for row in moving {
            etas.progress_gaps.insert(row.division, NO_CONTENT);
            etas.refuse(row.division, NO_CONTENT);
        }
        etas.report.progress_refused = etas.progress_gaps.len();
        return Ok(etas);
    };
    let (map, defines) = (&content.map, &content.defines);
    let divisions: BTreeMap<u64, &DivisionRow> = rows::<tables::Divisions>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let terms = HourTerms::new(frame);
    let province = |id| operands(map, id);
    let distance = |from, to| map.distance(from, to);
    for row in moving {
        let Some(&division) = divisions.get(&row.division) else {
            etas.progress_gaps.insert(row.division, NO_LOCATION);
            etas.refuse(row.division, NO_LOCATION);
            continue;
        };
        let at = division.province;
        match eta_derive::division_progress(row, Some(at), province, distance, defines) {
            Some(value) => {
                etas.progress.insert(row.division, value);
            }
            None => {
                etas.progress_gaps.insert(
                    row.division,
                    "eta_derive::division_progress refuses its recorded route with the content map",
                );
            }
        }
        match terms.with_terms(division, row, content, defines, |terms| {
            eta_derive::estimate(row, at, province, distance, terms, defines)
        }) {
            Ok(pair) => {
                etas.by_division.insert(row.division, pair);
            }
            Err(reason) => etas.refuse(row.division, &reason),
        }
    }
    etas.report.estimated = etas.by_division.len();
    etas.report.progress_refused = etas.progress_gaps.len();
    Ok(etas)
}

/// The counter arrival line's duration for a division: the
/// estimate to its destination for the viewing country's own unit and to its
/// next province otherwise, or the estimate's refusal by name.
pub fn arrival(etas: &Etas, division: u64, ours: bool) -> Result<i64, String> {
    let (next, destination) = etas.estimate(division).map_err(str::to_owned)?;
    Ok(if ours { destination } else { next })
}
#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "movement_timing/test_support.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "movement_timing/terrain_tests.rs"]
mod terrain_tests;
