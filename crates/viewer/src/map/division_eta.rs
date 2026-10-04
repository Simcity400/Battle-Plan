//! The viewer's one movement-timing source.
//!
//! Derives the remaining-time pair with [`replay::eta_derive`] from
//! `DivisionEtaInputs` and this hour's province, state and weather rows, and
//! each moving division's first-edge progress from `DivisionMovement`. Edge
//! distances, crossing kinds, terrain operands and the movement defines come
//! from the matching content stack ([`MovementContent`]), never the recording.
//!
//! The arrival line takes `destination_remaining` ([`super::counter_eta`]).
//! The progress is the game's movement-arrow fill target (`+0x28`), and
//! `141632130` draws that arrow only for a unit of the player's own country
//! that is selected (`CUnit+0xC`; for a navy also its task force's `+0x24`
//! word), a selection the viewer has no equivalent of, so it is not drawn. No
//! counter or figure position reads it ([`super::counter_move`] draws the
//! settled route point).
//!
//! The derivation refuses a row it cannot rebuild exactly rather than
//! substituting a zero: a refused division has no ETA or no progress.

use super::tables::rows;
use installed_content::movement::{MovementDefines, MovementMap};
use replay::{
    eta_derive::{self, Defines, ProvinceOperands},
    frame::DecodedFrame,
    rows::tables,
};
use std::{
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
    /// Later hops of placed routes that the movement polyline skips
    /// (`BuildUnitMovementAnchorPath@1416307F0`): no content neighbour of the
    /// province before them, or no anchor.
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
        }
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

    /// Native first-edge movement fraction in signed fixed5, sentinel and all,
    /// or why the hour cannot rebuild it. Zero for an empty route. In the game
    /// it drives the movement arrow's fill target (`+0x28`) of a selected own
    /// unit (see the module docs) and the naval invasion progress in a
    /// combat's division hover (140BEC9A0 in 1415BF4B0); a counter never reads
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
