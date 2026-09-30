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
mod tests {
    use super::*;
    use installed_content::movement::{Neighbour, ProvinceTerrain};
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            DivisionEtaInputsRow, DivisionMovementRow, DivisionRow, ProvinceRow, StateRow, TableId,
            TableRows, WeatherProvinceRow, WeatherRegionRow,
        },
    };

    const DIVISION: u64 = 7;
    /// A normal-speed fixture whose expected pair is arithmetic.
    const SPEED: i64 = 700_000;
    const LAND: ProvinceTerrain = ProvinceTerrain { flags: 1, index: 1 };

    fn content(map: MovementMap) -> MovementContent {
        MovementContent {
            map,
            defines: Defines {
                outer_scale: 100_000,
                ..Default::default()
            },
        }
    }

    /// Land provinces 1, 2 and 3 with the route edges 1-2 (350000) and 2-3 (700000).
    fn two_leg_map() -> MovementMap {
        let edge = |to, distance| Neighbour {
            to,
            distance: Some(distance),
            ..Default::default()
        };
        MovementMap::from_parts(&[
            (1, LAND, vec![edge(2, 350_000)]),
            (2, LAND, vec![edge(1, 350_000), edge(3, 700_000)]),
            (3, LAND, vec![edge(2, 700_000)]),
        ])
    }

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 60_832_632,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not needed for the ETA source".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        }
    }

    fn put(frame: &mut DecodedFrame, id: TableId, rows: TableRows) {
        *frame.tables[id.index()] = TableView::Complete(rows);
    }

    fn inputs(division: u64) -> DivisionEtaInputsRow {
        DivisionEtaInputsRow {
            division,
            sample_hour: frame().hour,
            speed_modifier: 100_000,
            speed_base: SPEED,
            current_speed: SPEED,
            terrain_speed: vec![replay::rows::TerrainSpeed {
                terrain: 1,
                modifier: 0,
            }],
            ..Default::default()
        }
    }

    /// A busy hour: `provinces` land provinces with no authored neighbours, so
    /// every hop takes the native 100000 fallback, and `divisions` movers each
    /// on a `legs`-hop route with complete normal-speed operands.
    fn busy_hour(provinces: u32, divisions: u64, legs: usize) -> (DecodedFrame, MovementMap) {
        let mut frame = derivable();
        let hop = |division: u64, leg: usize| (division as u32 * 7 + leg as u32) % provinces + 1;
        let ids = 1..=provinces;
        put(
            &mut frame,
            TableId::WeatherProvinces,
            TableRows::WeatherProvinces(
                ids.clone()
                    .map(|id| WeatherProvinceRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableId::Provinces,
            TableRows::Provinces(
                ids.clone()
                    .map(|id| ProvinceRow {
                        id,
                        state: id % 900 + 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableId::States,
            TableRows::States(
                (1..=901)
                    .map(|id| StateRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableId::Divisions,
            TableRows::Divisions(
                (0..divisions)
                    .map(|id| DivisionRow {
                        id,
                        province: hop(id, 0),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(
                (0..divisions)
                    .map(|id| DivisionMovementRow {
                        division: id,
                        path: (1..=legs).map(|leg| hop(id, leg)).collect(),
                        path_total: legs as u32,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableId::DivisionEtaInputs,
            TableRows::DivisionEtaInputs((0..divisions).map(inputs).collect()),
        );
        let map =
            MovementMap::from_parts(&ids.map(|id| (id, LAND, Vec::new())).collect::<Vec<_>>());
        (frame, map)
    }

    /// One division at province 1 on the two-leg route 2, 3, all land, no snow,
    /// no fort and no infrastructure, taking the normal-speed branch.
    fn derivable() -> DecodedFrame {
        let mut frame = frame();
        put(
            &mut frame,
            TableId::WeatherRegions,
            TableRows::WeatherRegions(vec![WeatherRegionRow {
                max_snow: 20_000,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::WeatherProvinces,
            TableRows::WeatherProvinces(
                (1..=3)
                    .map(|id| WeatherProvinceRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableId::Provinces,
            TableRows::Provinces(
                (1..=3)
                    .map(|id| ProvinceRow {
                        id,
                        state: 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableId::States,
            TableRows::States(vec![StateRow {
                id: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::Divisions,
            TableRows::Divisions(vec![DivisionRow {
                id: DIVISION,
                province: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: DIVISION,
                path: vec![2, 3],
                path_total: 2,
                numerator: 175_000,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::DivisionEtaInputs,
            TableRows::DivisionEtaInputs(vec![inputs(DIVISION)]),
        );
        frame
    }

    #[test]
    fn an_hour_without_inputs_has_no_eta_and_no_refusal() {
        let map = content(two_leg_map());
        let etas = Etas::read(&frame(), Some(&map)).unwrap();
        assert_eq!(etas.report(), Report::default());
        assert_eq!(etas.get(DIVISION), None);
        assert_eq!(etas.report().line(), None);
        let skipped = Report {
            route_hops_skipped: 2,
            ..Report::default()
        };
        assert!(
            skipped
                .line()
                .is_some_and(|line| line.ends_with("0 routes unplaced, 2 route hops skipped"))
        );
    }

    #[test]
    fn latest_operand_rows_survive_seek_but_future_and_expired_rows_are_refused() {
        let map = content(two_leg_map());
        for age in [-1, 0, 1, 4, 5] {
            let mut frame = derivable();
            let mut row = inputs(DIVISION);
            row.sample_hour = frame.hour - age;
            put(
                &mut frame,
                TableId::DivisionEtaInputs,
                TableRows::DivisionEtaInputs(vec![row]),
            );
            let etas = Etas::read(&frame, Some(&map)).unwrap();
            assert_eq!(etas.get(DIVISION).is_some(), (0..5).contains(&age));
            assert_eq!(etas.report().refused, usize::from(!(0..5).contains(&age)));
        }
    }

    #[test]
    fn an_hour_derives_progress_and_both_remaining_durations() {
        // 350000 and 700000 distance over speed 700000: half an hour and one
        // hour, less the 175000 numerator already moved (a quarter hour).
        let map = content(two_leg_map());
        let etas = Etas::read(&derivable(), Some(&map)).unwrap();
        assert_eq!(etas.get(DIVISION), Some((25_000, 125_000)));
        // Progress divides the numerator by the first edge's content distance.
        assert_eq!(etas.progress(DIVISION), Ok(50_000));
        assert_eq!(
            etas.report(),
            Report {
                derived: 1,
                ..Report::default()
            }
        );
    }

    #[test]
    fn operand_indices_bound_storage_by_rows_and_preserve_sparse_keys() {
        let dense = indexed(&[(0, 3), (2, 7)], |&row| row);
        assert!(matches!(&dense, OperandIndex::Dense(values) if values.len() == 3));
        assert_eq!(dense.get(0), Some(3));
        assert_eq!(dense.get(1), None);
        let sparse = indexed(&[(1, 3), (u32::MAX, 7)], |&row| row);
        assert!(matches!(&sparse, OperandIndex::Sparse(values) if values.len() == 2));
        assert_eq!(sparse.get(1), Some(3));
        assert_eq!(sparse.get(u32::MAX), Some(7));
        assert_eq!(sparse.get(u32::MAX - 1), None);
    }

    #[test]
    fn permanent_snow_and_infrastructure_enter_the_normal_edge() {
        let mut frame = derivable();
        put(
            &mut frame,
            TableId::States,
            TableRows::States(vec![StateRow {
                id: 1,
                movement_infrastructure: 50_000,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: DIVISION,
                path: vec![2],
                path_total: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::DivisionEtaInputs,
            TableRows::DivisionEtaInputs(vec![DivisionEtaInputsRow {
                speed_base: 400_000,
                current_speed: 400_000,
                snow_penalty: -100_000,
                ..inputs(DIVISION)
            }]),
        );
        // (1 - 1 snow + 0.5 infrastructure) * 400000 = 200000 speed over the
        // 350000 edge: the destination's permanent snow alone makes it snowy.
        put(
            &mut frame,
            TableId::WeatherProvinces,
            TableRows::WeatherProvinces(
                (1..=3)
                    .map(|id| WeatherProvinceRow {
                        id,
                        permanent_snow: i64::from(id == 2),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let map = content(two_leg_map());
        let etas = Etas::read(&frame, Some(&map)).unwrap();
        assert_eq!(etas.get(DIVISION), Some((175_000, 175_000)));
        assert_eq!(etas.report().refused, 0);
    }

    #[test]
    fn a_missing_operand_refuses_the_row_instead_of_deriving_without_it() {
        let map = content(two_leg_map());
        // The snow census and its ceiling are part of the edge speed, so
        // losing either refuses the ETA; first-edge progress needs no weather
        // and stands.
        for table in [TableId::WeatherProvinces, TableId::WeatherRegions] {
            let mut frame = derivable();
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "dropped for the test".into(),
            };
            let etas = Etas::read(&frame, Some(&map)).unwrap();
            assert_eq!(etas.get(DIVISION), None);
            assert_eq!(etas.progress(DIVISION), Ok(50_000));
            assert_eq!((etas.report().derived, etas.report().refused), (0, 1));
        }

        // Without the ETA row there is neither progress nor a claimed zero.
        let mut frame = derivable();
        *frame.tables[TableId::DivisionEtaInputs.index()] = TableView::Unavailable {
            reason: "dropped for the test".into(),
        };
        let etas = Etas::read(&frame, Some(&map)).unwrap();
        assert_eq!(
            etas.progress(DIVISION),
            Err("it has no DivisionEtaInputs row valid this hour")
        );
        assert_eq!(etas.report().progress_refused, 1);

        // A province absent from the content map cannot silently become the
        // null terrain or an ordinary land province.
        let partial = content(MovementMap::from_parts(&[(1, LAND, Vec::new())]));
        let etas = Etas::read(&derivable(), Some(&partial)).unwrap();
        assert_eq!(etas.get(DIVISION), None);
        assert_eq!(
            etas.progress(DIVISION),
            Err("eta_derive::movement_progress refuses its recorded route with the content map")
        );

        // Content that failed to load refuses every moving division, but a
        // division without a route still stands at progress zero.
        let mut frame = derivable();
        put(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![
                DivisionMovementRow {
                    division: DIVISION,
                    path: vec![2],
                    path_total: 1,
                    ..Default::default()
                },
                DivisionMovementRow {
                    division: DIVISION + 1,
                    ..Default::default()
                },
            ]),
        );
        let etas = Etas::read(&frame, None).unwrap();
        assert_eq!(etas.get(DIVISION), None);
        assert_eq!(
            etas.progress(DIVISION),
            Err("the content's movement map (terrain and movement defines) failed to load")
        );
        assert_eq!(etas.progress(DIVISION + 1), Ok(0));
        assert_eq!(
            (etas.report().refused, etas.report().progress_refused),
            (1, 1)
        );
    }

    #[test]
    fn the_cache_holds_the_hours_in_flight_and_evicts_the_oldest() {
        let cache = EtaCache::default();
        let built = std::cell::Cell::new(0);
        let fill = |hour: replay::Hour| {
            cache
                .get_or_insert_with((1, hour), || {
                    built.set(built.get() + 1);
                    Ok::<_, io::Error>(Etas::default())
                })
                .expect("the build cannot fail")
        };
        // A worker builds the hours ahead; the window then arrives at them.
        for hour in 1..=3 {
            fill(hour);
        }
        assert_eq!(built.get(), 3);
        fill(1);
        fill(2);
        assert_eq!(built.get(), 3, "the look-ahead must still be cached");
        // The ninth distinct key evicts the first.
        for hour in 4..=9 {
            fill(hour);
        }
        assert_eq!(built.get(), 9);
        fill(9);
        assert_eq!(built.get(), 9);
        fill(1);
        assert_eq!(built.get(), 10, "hour 1 should have been evicted");
    }

    /// Exercise a large synthetic hour without tying correctness to the speed
    /// or load of the machine running the test.
    #[test]
    fn a_busy_hour_derives_every_route() {
        let (frame, map) = busy_hour(13_000, 1_000, 30);
        let map = content(map);
        let etas = Etas::read(&frame, Some(&map)).unwrap();
        assert_eq!(etas.report().derived, 1_000);
        assert_eq!(etas.report().refused, 0);
        assert_eq!(etas.report().progress_refused, 0);
        // Thirty known non-neighbour hops at the native 100000 distance, each
        // truncated to 14285 at speed 700000.
        assert_eq!(etas.get(0).map(|pair| pair.1), Some(30 * 14_285));
    }
}
