//! The viewer's one movement-timing source.
//!
//! Each moving division's first-edge progress follows route integration over its
//! recorded `DivisionMovement` route ([`eta_derive::division_progress`]). Its
//! arrival times are estimates ([`eta_derive::estimate`]): the native walk
//! over the recorded route from `Divisions.province`, every edge
//! at the division movement speed given by the recorded
//! stat-block base speed (`DivisionMovement.base_speed`) alone, floored at the
//! native 1 km/h and scaled by the content's movement scale. The game also
//! applies each edge's terrain, infrastructure, weather, river, supply, air,
//! organisation and fuel terms, which the recording does not hold, so an
//! estimate drifts from the game wherever those apply or change mid-route.
//! Edge distances, terrain flags and the movement defines come from the
//! matching content stack ([`MovementContent`]), never the recording.
//!
//! The progress is the game's movement-arrow fill target (`+0x28`), and
//! The game draws that arrow only for a unit of the player's own country
//! that is selected (`unit+0xC`; for a navy also its task force's `+0x24`
//! word). The viewer's selection drives the movement-arrow layer. Counter
//! and figure positions use the settled route point in [`super::counter_move`].
//!
//! Counter hovers ([`CounterQueries`]) word a division's estimate, a railway
//! gun's fresh native route query and a task force's stored route. A row the
//! hour cannot measure has no estimate or no progress rather than a
//! substituted zero, and keeps its reason.

use super::tables::rows;
mod fresh_search;
#[path = "division_eta/classes/naval.rs"]
mod naval;
#[path = "division_eta/classes/railway.rs"]
mod railway;
#[path = "division_eta/classes/task_force.rs"]
mod task_force;
use installed_content::movement::{MovementDefines, MovementMap, RailMovementDefines};
use replay::{
    eta_derive::{self, Defines, ProvinceOperands},
    frame::DecodedFrame,
    rows::tables,
};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, VecDeque},
    fmt::Write as _,
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
    /// A prospective route to `target`: the native route search
    /// for divisions and railway guns. A task force's search needs range,
    /// theatre and nearest-base admission inputs the recording does not hold,
    /// so its preview refuses by name.
    pub(crate) fn preview(
        &self,
        unit: CounterUnit,
        target: u32,
        local_option: bool,
    ) -> Result<CounterPreview, String> {
        match unit {
            CounterUnit::Division(handle) => {
                self.query(|inputs, moves| inputs.derive_to(moves, handle, target, local_option))
            }
            CounterUnit::TaskForce(_) => Err(task_force::ADMISSION_UNRECORDED.into()),
            CounterUnit::Railway(handle) => self.query(|inputs, moves| {
                railway::derive_to(inputs, moves, handle, target, local_option)
            }),
        }
    }
}

/// Reached counter queries in one hover or diagnostics batch. Whole-frame
/// indexes are built lazily once; every requested route still runs native A*.
pub(crate) struct CounterQueries<'a> {
    content: &'a crate::document::Content,
    frame: &'a DecodedFrame,
    world: OnceCell<super::raids::script::World<'a>>,
    inputs: OnceCell<fresh_search::Inputs<'a>>,
    etas: OnceCell<Result<Arc<Etas>, String>>,
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
            etas: OnceCell::new(),
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

    /// A division's arrival duration from this hour's shared estimates
    /// ([`Hour::etas`](crate::document::Hour::etas)); see [`arrival`].
    pub(super) fn division_remaining(&self, division: u64, ours: bool) -> Result<i64, String> {
        let etas = self.etas.get_or_init(|| {
            crate::document::Hour::new(self.content, self.frame, 0.0)
                .etas()
                .map_err(|error| error.to_string())
        });
        arrival(etas.as_deref().map_err(Clone::clone)?, division, ours)
    }

    pub(super) fn railway_remaining(
        &self,
        gun: &replay::rows::RailwayGunCounterInputRow,
        ours: bool,
        local_option: bool,
    ) -> Result<i64, String> {
        self.query(|inputs, moves| railway::derive(inputs, moves, gun, ours, local_option))
    }

    /// A task force's arrival over its stored ordinary route
    /// (`TaskForceMapMotion.path`), searching no route.
    pub(crate) fn task_force_remaining(
        &self,
        motion: &replay::rows::TaskForceMapMotionRow,
        ours: bool,
    ) -> Result<i64, String> {
        let content = self
            .content
            .movement
            .as_deref()
            .ok_or("task force arrival movement content is unavailable")?;
        let inputs = self
            .inputs
            .get_or_init(|| fresh_search::Inputs::new(self.frame, content));
        task_force::derive(inputs, motion, ours, |from| {
            self.content.icons.raids.selected_region(from)
        })
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

/// The counter arrival line's duration for a division: the
/// estimate to its destination for the viewing country's own unit and to its
/// next province otherwise, or the estimate's refusal by name.
pub(super) fn arrival(etas: &Etas, division: u64, ours: bool) -> Result<i64, String> {
    let (next, destination) = etas.estimate(division).map_err(str::to_owned)?;
    Ok(if ours { destination } else { next })
}

/// The province operands an estimate reads: the content terrain, whose flags
/// decide the embark edge. `None` for a province the content
/// map lacks, which an estimate refuses as missing geometry.
fn operands(map: &MovementMap, province: u32) -> Option<ProvinceOperands> {
    map.terrain(province).map(|terrain| ProvinceOperands {
        terrain_flags: terrain.flags,
        terrain_index: terrain.index,
        ..Default::default()
    })
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

    /// Native first-edge movement fraction in signed fixed5, sentinel and all,
    /// or why the hour cannot rebuild it. Zero for an empty route. In the game
    /// it drives the movement arrow's fill target (`+0x28`) of a selected own
    /// unit (see the module docs) and the naval invasion progress in a
    /// combat's division hover; a counter never reads
    /// it.
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
    let location: BTreeMap<u64, u32> = rows::<tables::Divisions>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row.province))
        .collect();
    let province = |id| operands(map, id);
    let distance = |from, to| map.distance(from, to);
    for row in moving {
        let Some(&at) = location.get(&row.division) else {
            etas.progress_gaps.insert(row.division, NO_LOCATION);
            etas.refuse(row.division, NO_LOCATION);
            continue;
        };
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
        match eta_derive::estimate(row, at, province, distance, defines) {
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

#[cfg(test)]
mod tests;
