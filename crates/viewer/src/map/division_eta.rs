//! The viewer's one movement-timing source.
//!
//! Derives stored-route timing with [`replay::eta_derive`] from
//! `DivisionEtaInputs` and this hour's province, state and weather rows, and
//! each moving division's first-edge progress from `DivisionMovement`. Edge
//! distances, crossing kinds, terrain operands and the movement defines come
//! from the matching content stack ([`MovementContent`]), never the recording.
//!
//! The counter arrival uses [`CounterQueries`], an on-demand native route
//! query with exact current-hour operands and an explicit viewer route policy.
//! The stored-route pair remains available for aggregate timing and progress.
//! The progress is the movement-arrow fill target for a selected unit of the
//! player's own country; a navy also requires its task force's selection word.
//! The viewer's selection drives the movement-arrow layer. Counter and figure
//! positions use the settled route point in [`super::counter_move`].
//!
//! The derivation refuses a row it cannot rebuild exactly rather than
//! substituting a zero: a refused division has no ETA or no progress.

use super::tables::rows;
mod fresh_search;
#[path = "division_eta/classes/naval.rs"]
mod naval;
#[path = "division_eta/classes/railway.rs"]
mod railway;
#[path = "division_eta/classes/task_force.rs"]
mod task_force;
use installed_content::movement::{
    MovementDefines, MovementMap, NavalMovementDefines, RailMovementDefines,
};
use replay::{
    eta_derive::{self, Defines, ProvinceOperands},
    frame::DecodedFrame,
    rows::tables,
};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Mutex},
};

/// The branch and recorded hour one cached derivation is for.
pub type Key = (replay::BranchId, replay::Hour);

/// How many hours the cache holds: the two `crate::hours` builds ahead, the
/// parts that build in parallel, and slack.
const CAPACITY: usize = 8;

/// The last few hours' ETAs, so no hour is derived twice.
///
/// The window thread builds the counter layer for the playhead while workers
/// build the close-zoom models for the hours ahead, so the hours in flight are
/// not one but several and a single slot thrashes between them. Entries are
/// evicted oldest-inserted first.
#[derive(Default)]
pub struct EtaCache {
    entries: Mutex<VecDeque<(Key, Arc<Etas>)>>,
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
        if let Some((_, existing)) = entries.iter().find(|(at, _)| *at == key) {
            return Ok(Arc::clone(existing));
        }
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        entries.push_back((key, Arc::clone(&etas)));
        Ok(etas)
    }

    fn get(&self, key: Key) -> Option<Arc<Etas>> {
        self.entries
            .lock()
            .expect("ETA cache poisoned")
            .iter()
            .find(|(at, _)| *at == key)
            .map(|(_, etas)| Arc::clone(etas))
    }
}

/// How many ETA rows were derived and which rows were refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub derived: usize,
    /// Rows the derivation refused: an operand this hour does not hold, or
    /// content geometry the route needs.
    pub refused: usize,
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
    /// The dump report's line, or `None` for an hour with no movement at all.
    pub fn line(&self) -> Option<String> {
        (self.derived != 0
            || self.refused != 0
            || self.progress_refused != 0
            || self.routes_unresolved != 0
            || self.route_hops_skipped != 0)
            .then(|| {
                format!(
                    "Movement timing: {} ETA rows derived, {} ETA rows refused, {} progress rows refused, {} routes unplaced, {} route hops skipped",
                    self.derived,
                    self.refused,
                    self.progress_refused,
                    self.routes_unresolved,
                    self.route_hops_skipped
                )
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
    pub naval: Result<NavalMovementDefines, String>,
    pub(crate) task_force_terrain: Result<task_force::TerrainSpeedContent, String>,
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
            },
            rail: defines.rail,
            naval: defines.naval,
            task_force_terrain: Err("fresh task force terrain content is unavailable".into()),
        }
    }

    pub(crate) fn set_task_force_terrain(
        &mut self,
        terrain: Result<Arc<installed_content::terrain::TerrainDefinitions>, String>,
        regions: &BTreeMap<u32, installed_content::regions::StrategicRegion>,
        subunits: Result<crate::table_access::TokenKeys, String>,
    ) {
        self.task_force_terrain =
            terrain.map(|terrain| task_force::TerrainSpeedContent::new(terrain, regions, subunits));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CounterUnit {
    Division(u64),
    TaskForce(u64),
    Railway(u64),
}

#[derive(Debug)]
pub(crate) struct CounterPreview {
    // Independent signed fixed5 result; caller owns ceiling/i32 narrowing/visibility.
    pub(crate) duration: Result<i64, String>,
    // Actual fresh search output, source excluded; no actual-route substitute.
    pub(crate) route: Vec<u32>,
}

impl CounterQueries<'_> {
    pub(crate) fn preview(
        &self,
        unit: CounterUnit,
        target: u32,
        local_option: bool,
    ) -> Result<CounterPreview, String> {
        self.query(|inputs, moves| match unit {
            CounterUnit::Division(handle) => inputs.derive_to(moves, handle, target, local_option),
            CounterUnit::TaskForce(handle) => {
                task_force::derive_to(inputs, moves, handle, target, local_option)
            }
            CounterUnit::Railway(handle) => {
                railway::derive_to(inputs, moves, handle, target, local_option)
            }
        })
    }
}

/// Reached counter queries in one hover or diagnostics batch. Whole-frame
/// indexes are built lazily once; every requested route still runs native A*.
pub(crate) struct CounterQueries<'a> {
    content: &'a crate::document::Content,
    frame: &'a DecodedFrame,
    world: OnceCell<super::raids::script::World<'a>>,
    inputs: OnceCell<fresh_search::Inputs<'a>>,
    cells: super::raids::reach::Cells,
    common: super::raids::reach::CommonCells<'a>,
}

impl<'a> CounterQueries<'a> {
    pub(crate) fn new(content: &'a crate::document::Content, frame: &'a DecodedFrame) -> Self {
        Self {
            content,
            frame,
            world: OnceCell::new(),
            inputs: OnceCell::new(),
            cells: Default::default(),
            common: super::raids::reach::CommonCells::new(frame),
        }
    }

    /// Share this hover's existing World/Cells; no new graph or prospective query.
    pub(crate) fn selected_strait(
        &self,
        through: u32,
        raw_owner: u32,
    ) -> Result<installed_content::unit_path::StraitFleet, String> {
        let world = self.world.get_or_init(|| {
            super::raids::script::World::read(self.frame, self.content.icons.raids.facts())
        });
        let moves = super::raids::reach::Moves::for_common(
            world,
            &self.content.icons.raids,
            &self.cells,
            &self.common,
        )?;
        moves.selected_strait(through, raw_owner)
    }

    /// The same hover's borrowed World, directed owners and text services.
    pub(crate) fn selected_move_message(
        &self,
        unit: CounterUnit,
        target: u32,
        text: &mut String,
    ) -> Result<bool, String> {
        let world = self.world.get_or_init(|| {
            super::raids::script::World::read(self.frame, self.content.icons.raids.facts())
        });
        let moves = super::raids::reach::Moves::for_common(
            world,
            &self.content.icons.raids,
            &self.cells,
            &self.common,
        )?;
        let content = super::raids::reach::CommonText {
            frame: self.frame,
            loc: &self.content.names.localisation,
            recorded_language: &self.content.recorded_language,
            state_keys: &self.content.state_name_keys,
        };
        match unit {
            CounterUnit::Division(handle) => {
                moves.common_army_message(handle, target, text, &content)
            }
            CounterUnit::Railway(handle) => {
                moves.common_railway_message(handle, target, text, &content)
            }
            CounterUnit::TaskForce(_) => Err("selected land movement received a task force".into()),
        }
    }

    pub(crate) fn selected_transport_message(
        &self,
        selected: &[super::unit_selection::Unit],
        target: u32,
    ) -> Result<installed_content::unit_path::selected_transport::Outputs, String> {
        let world = self.world.get_or_init(|| {
            super::raids::script::World::read(self.frame, self.content.icons.raids.facts())
        });
        let moves = super::raids::reach::Moves::for_common(
            world,
            &self.content.icons.raids,
            &self.cells,
            &self.common,
        )?;
        let content = super::raids::reach::CommonText {
            frame: self.frame,
            loc: &self.content.names.localisation,
            recorded_language: &self.content.recorded_language,
            state_keys: &self.content.state_name_keys,
        };
        moves.selected_transport_message(selected, target, &content)
    }

    /// The local route preference is a viewer policy, never a captured operand.
    pub(super) fn remaining(
        &self,
        movement: &replay::rows::DivisionMovementRow,
        ours: bool,
        local_option: bool,
    ) -> Result<i64, String> {
        self.query(|inputs, moves| {
            inputs.derive(
                moves,
                movement,
                if ours {
                    fresh_search::Endpoint::Destination
                } else {
                    fresh_search::Endpoint::Next
                },
                local_option,
            )
        })
    }

    pub(super) fn railway_remaining(
        &self,
        gun: &replay::rows::RailwayGunCounterInputRow,
        ours: bool,
        local_option: bool,
    ) -> Result<i64, String> {
        self.query(|inputs, moves| railway::derive(inputs, moves, gun, ours, local_option))
    }

    pub(crate) fn task_force_remaining(
        &self,
        motion: &replay::rows::TaskForceMapMotionRow,
        ours: bool,
        local_option: bool,
    ) -> Result<i64, String> {
        self.query(|inputs, moves| task_force::derive(inputs, moves, motion, ours, local_option))
    }

    fn query<T>(
        &self,
        derive: impl FnOnce(
            &fresh_search::Inputs<'a>,
            &super::raids::reach::Moves<'_, 'a>,
        ) -> Result<T, String>,
    ) -> Result<T, String> {
        let content = self
            .content
            .movement
            .as_deref()
            .ok_or("fresh counter ETA movement content is unavailable")?;
        let inputs = self
            .inputs
            .get_or_init(|| fresh_search::Inputs::new(self.frame, content));
        let world = self.world.get_or_init(|| {
            super::raids::script::World::read(self.frame, self.content.icons.raids.facts())
        });
        let moves = super::raids::reach::Moves::for_counter(
            world,
            &self.content.icons.raids,
            &self.cells,
            inputs,
        );
        derive(inputs, &moves)
    }
}

/// One hour's `(next_remaining, destination_remaining)` and first-edge progress
/// per division handle.
#[derive(Clone, Debug, Default)]
pub struct Etas {
    by_division: Pairs,
    progress: BTreeMap<u64, i64>,
    /// Why a division the hour moves has no progress.
    progress_gaps: BTreeMap<u64, &'static str>,
    report: Report,
}

impl Etas {
    /// `movement` is `None` when the content's movement map failed to load:
    /// every moving division then has neither progress nor an ETA.
    pub fn read(frame: &DecodedFrame, movement: Option<&MovementContent>) -> io::Result<Self> {
        derive(frame, movement)
    }

    /// This division's `(next_remaining, destination_remaining)`, native signed
    /// fixed5, or `None` where the hour cannot derive it.
    pub fn get(&self, division: u64) -> Option<(i64, i64)> {
        self.by_division.get(&division).copied()
    }

    /// First-edge movement fraction in signed fixed5, sentinel and all, or why
    /// the hour cannot rebuild it. Zero for an empty route. It drives the selected
    /// own unit's movement-arrow fill target (see the module docs) and naval
    /// invasion progress in a combat division hover; a counter never reads it.
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

    pub fn report(&self) -> Report {
        self.report
    }
}

/// `(next_remaining, destination_remaining)` per division handle.
type Pairs = BTreeMap<u64, (i64, i64)>;

/// Every division the hour can rebuild, and how many it refused.
fn derive(frame: &DecodedFrame, movement_content: Option<&MovementContent>) -> io::Result<Etas> {
    let movement: BTreeMap<u64, _> = rows::<tables::DivisionMovement>(frame)?
        .rows()
        .iter()
        .map(|row| (row.division, row))
        .collect();
    let inputs = rows::<tables::DivisionEtaInputs>(frame)?;
    let inputs = inputs.rows();
    let mut etas = Etas::default();
    // A division with no route stands still: progress zero needs no content.
    for row in movement.values().filter(|row| row.path_total == 0) {
        if let Some(value) = eta_derive::movement_progress(
            row,
            None,
            None,
            |_| None,
            |_, _| None,
            &Defines::default(),
        ) {
            etas.progress.insert(row.division, value);
        } else {
            etas.progress_gaps.insert(
                row.division,
                "eta_derive::movement_progress refuses its empty route",
            );
        }
    }
    let Some(content_map) = movement_content else {
        etas.report.refused = inputs.len();
        etas.report.progress_refused = movement.len() - etas.progress.len();
        for division in movement.keys() {
            if !etas.progress.contains_key(division) {
                etas.progress_gaps.entry(*division).or_insert(
                    "the content's movement map (terrain and movement defines) failed to load",
                );
            }
        }
        return Ok(etas);
    };
    let (map, defines) = (&content_map.map, &content_map.defines);
    let location: BTreeMap<u64, u32> = rows::<tables::Divisions>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row.province))
        .collect();
    // Ordinary province and state IDs are dense and small. The index retains
    // direct lookup for those while bounding storage for sparse replay keys.
    // A province without content terrain is absent, never the null terrain.
    let terrain_rows: Vec<(u32, ProvinceOperands)> = rows::<tables::Provinces>(frame)?
        .rows()
        .iter()
        .filter_map(|row| {
            let terrain = map.terrain(row.id)?;
            Some((
                row.id,
                ProvinceOperands {
                    terrain_flags: terrain.flags,
                    terrain_index: terrain.index,
                    fort_positive: row.fort_positive != 0,
                    state: row.state,
                },
            ))
        })
        .collect();
    let provinces = indexed(&terrain_rows, |&row| row);
    let distance = |from, to| map.distance(from, to);
    for row in inputs.iter().filter(|row| row.valid_at(frame.hour)) {
        let Some(movement) = movement.get(&row.division) else {
            continue;
        };
        match eta_derive::movement_progress(
            movement,
            Some(row),
            location.get(&row.division).copied(),
            |id| provinces.get(id),
            distance,
            defines,
        ) {
            Some(value) => {
                etas.progress.insert(row.division, value);
            }
            None => {
                etas.progress_gaps.insert(
                    row.division,
                    "eta_derive::movement_progress refuses its recorded route with the content map",
                );
            }
        }
    }
    for division in movement.keys() {
        if !etas.progress.contains_key(division) {
            etas.progress_gaps
                .entry(*division)
                .or_insert("it has no DivisionEtaInputs row valid this hour");
        }
    }
    etas.report.progress_refused = movement.len() - etas.progress.len();
    let Some(snow) = snow_provinces(frame)? else {
        // No weather census or ceiling: every ETA row is unrebuildable;
        // progress stands.
        etas.report.refused = inputs.len();
        return Ok(etas);
    };
    let infrastructure = indexed(rows::<tables::States>(frame)?.rows(), |row| {
        (row.id, row.movement_infrastructure)
    });
    for row in inputs {
        if !row.valid_at(frame.hour) {
            etas.report.refused += 1;
            continue;
        }
        let pair = movement
            .get(&row.division)
            .zip(location.get(&row.division))
            .and_then(|(movement, &location)| {
                eta_derive::derive(
                    row,
                    movement,
                    location,
                    |id| provinces.get(id),
                    |id| infrastructure.get(id),
                    |id| snow.get(id).unwrap_or(false),
                    distance,
                    |from, to| map.crossing_kind(from, to),
                    defines,
                )
            });
        match pair {
            Some(pair) => {
                etas.by_division.insert(row.division, pair);
            }
            None => etas.report.refused += 1,
        }
    }
    etas.report.derived = etas.by_division.len();
    Ok(etas)
}

/// Ordinary province and state IDs are dense, but replay keys allow all u32
/// values. Keep allocation proportional to recorded rows even for sparse IDs.
enum OperandIndex<V> {
    Dense(Vec<Option<V>>),
    Sparse(BTreeMap<u32, V>),
}

impl<V: Copy> OperandIndex<V> {
    fn get(&self, id: u32) -> Option<V> {
        match self {
            Self::Dense(values) => values.get(id as usize).copied().flatten(),
            Self::Sparse(values) => values.get(&id).copied(),
        }
    }
}

fn indexed<R, V: Copy>(rows: &[R], key: impl Fn(&R) -> (u32, V)) -> OperandIndex<V> {
    let length = rows
        .iter()
        .map(|row| u64::from(key(row).0) + 1)
        .max()
        .unwrap_or(0);
    if length > rows.len().saturating_mul(4) as u64 {
        return OperandIndex::Sparse(rows.iter().map(key).collect());
    }
    let mut out = vec![None; length as usize];
    for row in rows {
        let (id, value) = key(row);
        out[id as usize] = Some(value);
    }
    OperandIndex::Dense(out)
}

/// `cweather_manager::movement_snow` from the recorded words: permanent plus
/// current snow, wrapping as the engine adds, clamped to the ceiling, indexed
/// by province id. `None` where the snow census or the ceiling is missing,
/// which refuses rather than reading a missing operand as no snow.
fn snow_provinces(frame: &DecodedFrame) -> io::Result<Option<OperandIndex<bool>>> {
    let provinces = rows::<tables::WeatherProvinces>(frame)?;
    let regions = rows::<tables::WeatherRegions>(frame)?;
    let Some(ceiling) = regions.rows().first().map(|row| row.max_snow) else {
        return Ok(None);
    };
    if !provinces.is_complete() {
        return Ok(None);
    }
    Ok(Some(indexed(provinces.rows(), |row| {
        (
            row.id,
            row.permanent_snow.wrapping_add(row.snow).min(ceiling) > 0,
        )
    })))
}

#[cfg(test)]
mod tests;
