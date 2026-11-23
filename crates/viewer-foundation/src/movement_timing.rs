use crate::tables::rows;
use dashboard_text::table_access::TokenKeys;
use content_reader_map::movement::{MovementDefines, MovementMap, RailMovementDefines};
use content_reader_key_names::terrain::TerrainDefinitions;
use content_reader_regions::regions::StrategicRegion;
use content_reader_unit_path::unit_path::Fact;
use replay_data::{rows::DivisionMovementRow, rows::DivisionRow, rows::tables};
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

pub type Key = (replay_data::BranchId, replay_data::Hour);

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub estimated: usize,
    pub refused: BTreeMap<Arc<str>, usize>,
    pub progress_refused: usize,
    pub routes_unresolved: usize,
    pub route_hops_skipped: usize,
}

impl Report {
    pub fn refused_total(&self) -> usize {
        self.refused.values().sum()
    }

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

#[derive(Debug)]
pub struct MovementContent {
    pub map: MovementMap,
    pub defines: Defines,
    pub rail: Result<RailMovementDefines, String>,
    pub task_force_terrain: Result<TerrainSpeedContent, String>,
    pub land: Result<LandSpeedContent, String>,
}

#[derive(Debug)]
pub struct LandSpeedContent {
    pub terrain_movement: Vec<i64>,
    pub regions: BTreeMap<u32, u32>,
    pub sub_units: Result<Arc<content_reader_script::sub_units::SubUnits>, String>,
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
        terrain: Result<Arc<content_reader_key_names::terrain::TerrainDefinitions>, String>,
        regions: &BTreeMap<u32, content_reader_regions::regions::StrategicRegion>,
        subunits: Result<TokenKeys, String>,
        sub_units: Result<Arc<content_reader_script::sub_units::SubUnits>, String>,
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
        Ok(common.wrapping_add(own))
    }
}

pub fn operands(map: &MovementMap, province: u32) -> Option<ProvinceOperands> {
    map.terrain(province).map(|terrain| ProvinceOperands {
        terrain_flags: terrain.flags,
        terrain_index: terrain.index,
        ..Default::default()
    })
}

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

#[derive(Clone, Debug, Default)]
pub struct Etas {
    by_division: Pairs,
    estimate_gaps: BTreeMap<u64, Arc<str>>,
    progress: BTreeMap<u64, i64>,
    progress_gaps: BTreeMap<u64, &'static str>,
    report: Report,
}

const NO_ROUTE: &str = "it has no ordinary route this hour";
const NO_CONTENT: &str = "the content's movement map (terrain and movement defines) failed to load";
const NO_LOCATION: &str = "it has no Divisions row this hour";

impl Etas {
    pub fn read(frame: &DecodedFrame, movement: Option<&MovementContent>) -> io::Result<Self> {
        derive(frame, movement)
    }

    pub fn get(&self, division: u64) -> Option<(i64, i64)> {
        self.by_division.get(&division).copied()
    }

    pub fn estimate(&self, division: u64) -> Result<(i64, i64), &str> {
        match self.by_division.get(&division) {
            Some(pair) => Ok(*pair),
            None => Err(self
                .estimate_gaps
                .get(&division)
                .map_or(NO_ROUTE, |reason| reason)),
        }
    }

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

    fn refuse(&mut self, division: u64, reason: &str) {
        let reason = match self.report.refused.get_key_value(reason) {
            Some((reason, _)) => Arc::clone(reason),
            None => Arc::from(reason),
        };
        *self.report.refused.entry(Arc::clone(&reason)).or_default() += 1;
        self.estimate_gaps.insert(division, reason);
    }
}

type Pairs = BTreeMap<u64, (i64, i64)>;

fn derive(frame: &DecodedFrame, content: Option<&MovementContent>) -> io::Result<Etas> {
    let movement = rows::<tables::DivisionMovement>(frame)?;
    let mut etas = Etas::default();
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
        match estimate_with(row, division, &terms, content) {
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

fn estimate_with(row: &DivisionMovementRow, division: &DivisionRow, terms: &HourTerms<'_>, content: &MovementContent) -> Result<(i64, i64), String> {
    let (map, defines) = (&content.map, &content.defines);
    let province = |id| operands(map, id);
    let distance = |from, to| map.distance(from, to);
    terms.with_terms(division, row, content, defines, |terms| {
        eta_derive::estimate(row, division.province, province, distance, terms, defines)
    })
}

pub fn estimate_one(row: &DivisionMovementRow, division: Option<&DivisionRow>, terms: &HourTerms<'_>, content: Option<&MovementContent>) -> Option<(i64, i64)> {
    if row.path_total == 0 {
        return None;
    }
    estimate_with(row, division?, terms, content?).ok()
}

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
