//! The viewer's one ETA source.
//!
//! Derives the remaining-time pair with [`replay::eta_derive`] from
//! `DivisionEtaInputs` and this hour's province, state, weather and defines rows.
//!
//! Every consumer reads this map and nothing else: the counter and close-zoom
//! glide take `next_remaining` ([`super::counter_move`]) and the arrival line
//! takes `destination_remaining` ([`super::counter_eta`]).
//!
//! The derivation refuses a row it cannot rebuild exactly rather than
//! substituting a zero: a refused division has no ETA.

use super::tables::rows;
use replay::{
    eta_derive::{self, ProvinceOperands},
    frame::DecodedFrame,
    rows::{MovementDefinesRow, tables},
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
    /// Rows the derivation refused: a route the recorder truncated, or an
    /// operand this hour does not hold.
    pub refused: usize,
}

impl Report {
    /// The dump report's line, or `None` for an hour with no ETA at all.
    pub fn line(&self) -> Option<String> {
        (self.derived != 0 || self.refused != 0).then(|| {
            format!(
                "ETA: {} rows derived from DivisionEtaInputs, {} rows refused",
                self.derived, self.refused
            )
        })
    }
}

/// One hour's `(next_remaining, destination_remaining)` per division handle.
#[derive(Clone, Debug, Default)]
pub struct Etas {
    by_division: Pairs,
    report: Report,
}

impl Etas {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let (by_division, refused) = derive(frame)?;
        let derived = by_division.len();
        Ok(Self {
            by_division,
            report: Report { derived, refused },
        })
    }

    /// This division's `(next_remaining, destination_remaining)`, native signed
    /// fixed5, or `None` where the hour cannot derive it.
    pub fn get(&self, division: u64) -> Option<(i64, i64)> {
        self.by_division.get(&division).copied()
    }

    pub fn report(&self) -> Report {
        self.report
    }
}

/// `(next_remaining, destination_remaining)` per division handle.
type Pairs = BTreeMap<u64, (i64, i64)>;

/// Every division the hour can rebuild, and how many it refused.
fn derive(frame: &DecodedFrame) -> io::Result<(Pairs, usize)> {
    let inputs = rows::<tables::DivisionEtaInputs>(frame)?;
    let inputs = inputs.rows();
    if inputs.is_empty() {
        return Ok((BTreeMap::new(), 0));
    }
    let defines: Option<MovementDefinesRow> = rows::<tables::MovementDefines>(frame)?
        .rows()
        .first()
        .cloned();
    let snow = snow_provinces(frame)?;
    let (Some(defines), Some(snow)) = (defines, snow) else {
        // No defines or no weather census: every row is unrebuildable.
        return Ok((BTreeMap::new(), inputs.len()));
    };
    let movement: BTreeMap<u64, _> = rows::<tables::DivisionMovement>(frame)?
        .rows()
        .iter()
        .map(|row| (row.division, row))
        .collect();
    let location: BTreeMap<u64, u32> = rows::<tables::Divisions>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row.province))
        .collect();
    // Ordinary province and state IDs are dense and small. The index retains
    // direct lookup for those while bounding storage for sparse replay keys.
    let provinces = indexed(rows::<tables::Provinces>(frame)?.rows(), |row| {
        (
            row.id,
            ProvinceOperands {
                terrain_flags: row.terrain_flags,
                terrain_index: row.terrain_index,
                fort_positive: row.fort_positive != 0,
                state: row.state,
            },
        )
    });
    let infrastructure = indexed(rows::<tables::States>(frame)?.rows(), |row| {
        (row.id, row.movement_infrastructure)
    });
    let mut out = BTreeMap::new();
    let mut refused = 0;
    for row in inputs {
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
                    &defines,
                )
            });
        match pair {
            Some(pair) => {
                out.insert(row.division, pair);
            }
            None => refused += 1,
        }
    }
    Ok((out, refused))
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
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            DivisionEtaInputsRow, DivisionMovementRow, DivisionRow, ProvinceRow, StateRow, TableId,
            TableRows, WeatherProvinceRow, WeatherRegionRow,
        },
    };

    const DIVISION: u64 = 7;

    /// The one branch with a flat recorded speed, so the expected pair is arithmetic.
    const INVASION_SPEED: i64 = 700_000;

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 60_832_632,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not needed for the ETA source".into(),
            }),
            metrics: Metrics::default(),
        }
    }

    fn put(frame: &mut DecodedFrame, id: TableId, rows: TableRows) {
        frame.tables[id.index()] = TableView::Complete(rows);
    }

    /// A busy hour: `provinces` land provinces, `divisions` movers each on a
    /// `legs`-hop route, all on the flat invasion branch so none is refused.
    fn busy_hour(provinces: u32, divisions: u64, legs: usize) -> DecodedFrame {
        let mut frame = derivable(vec![350_000; legs]);
        let hop = |division: u64, leg: usize| (division as u32 * 7 + leg as u32) % provinces + 1;
        put(
            &mut frame,
            TableId::WeatherProvinces,
            TableRows::WeatherProvinces(
                (1..=provinces)
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
                (1..=provinces)
                    .map(|id| ProvinceRow {
                        id,
                        state: id % 900 + 1,
                        terrain_flags: 1,
                        terrain_index: 1,
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
            TableRows::DivisionEtaInputs(
                (0..divisions)
                    .map(|id| DivisionEtaInputsRow {
                        division: id,
                        invasion: 1,
                        adjacency_complete: 1,
                        current_speed: INVASION_SPEED,
                        terrain_speed: vec![0],
                        leg_distance: vec![350_000; legs],
                        leg_crossing: vec![0; legs],
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        frame
    }

    /// One division at province 1 on the two-leg route 2, 3, all land, no snow,
    /// no fort and no infrastructure, taking the invasion branch.
    fn derivable(legs: Vec<i64>) -> DecodedFrame {
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
            TableId::MovementDefines,
            TableRows::MovementDefines(vec![MovementDefinesRow {
                invasion_speed: INVASION_SPEED,
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
                        terrain_flags: 1,
                        terrain_index: 1,
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
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::DivisionEtaInputs,
            TableRows::DivisionEtaInputs(vec![DivisionEtaInputsRow {
                division: DIVISION,
                invasion: 1,
                adjacency_complete: 1,
                current_speed: INVASION_SPEED,
                terrain_speed: vec![0],
                leg_distance: legs.clone(),
                leg_crossing: vec![0; legs.len()],
                ..Default::default()
            }]),
        );
        frame
    }

    #[test]
    fn an_hour_without_inputs_has_no_eta_and_no_refusal() {
        let etas = Etas::read(&frame()).unwrap();
        assert_eq!(etas.report(), Report::default());
        assert_eq!(etas.get(DIVISION), None);
        assert_eq!(etas.report().line(), None);
    }

    #[test]
    fn an_hour_derives_both_remaining_durations() {
        // 350000 and 700000 distance over speed 700000: half an hour and one hour.
        let etas = Etas::read(&derivable(vec![350_000, 700_000])).unwrap();
        assert_eq!(etas.get(DIVISION), Some((50_000, 150_000)));
        assert_eq!(
            etas.report(),
            Report {
                derived: 1,
                refused: 0,
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
    fn sparse_maximum_ids_derive_with_their_recorded_weather_and_infrastructure() {
        let mut frame = derivable(vec![100_000]);
        let destination = u32::MAX;
        put(
            &mut frame,
            TableId::Provinces,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    terrain_flags: 1,
                    terrain_index: 1,
                    ..Default::default()
                },
                ProvinceRow {
                    id: destination,
                    state: u32::MAX,
                    terrain_flags: 1,
                    terrain_index: 1,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableId::States,
            TableRows::States(vec![StateRow {
                id: u32::MAX,
                movement_infrastructure: 50_000,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::WeatherProvinces,
            TableRows::WeatherProvinces(vec![WeatherProvinceRow {
                id: destination,
                snow: 0,
                permanent_snow: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: DIVISION,
                path: vec![destination],
                path_total: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableId::DivisionEtaInputs,
            TableRows::DivisionEtaInputs(vec![DivisionEtaInputsRow {
                division: DIVISION,
                speed_modifier: 100_000,
                speed_base: 400_000,
                speed_outer_scale: 100_000,
                current_speed: 400_000,
                terrain_speed: vec![0],
                snow_penalty: -100_000,
                leg_distance: vec![100_000],
                leg_crossing: vec![0],
                ..Default::default()
            }]),
        );
        // (1 - 1 snow + 0.5 infrastructure) * 400000 = 200000 speed: the
        // destination's permanent snow alone makes it snowy.
        let etas = Etas::read(&frame).unwrap();
        assert_eq!(etas.get(DIVISION), Some((50_000, 50_000)));
        assert_eq!(etas.report().refused, 0);
    }

    #[test]
    fn a_missing_operand_refuses_the_row_instead_of_deriving_without_it() {
        // The snow census and its ceiling are part of the edge speed, so losing
        // either refuses.
        for table in [TableId::WeatherProvinces, TableId::WeatherRegions] {
            let mut frame = derivable(vec![350_000, 700_000]);
            frame.tables[table.index()] = TableView::Unavailable {
                reason: "dropped for the test".into(),
            };
            let etas = Etas::read(&frame).unwrap();
            assert_eq!(etas.get(DIVISION), None);
            assert_eq!((etas.report().derived, etas.report().refused), (0, 1));
        }

        // An incomplete adjacency walk is a diagnostic, not a refusal: the leg
        // distances hold the native fallback, so the pair derives as recorded.
        let mut frame = derivable(vec![350_000, 700_000]);
        let row = match &frame.tables[TableId::DivisionEtaInputs.index()] {
            TableView::Complete(TableRows::DivisionEtaInputs(rows)) => rows[0].clone(),
            _ => unreachable!(),
        };
        put(
            &mut frame,
            TableId::DivisionEtaInputs,
            TableRows::DivisionEtaInputs(vec![DivisionEtaInputsRow {
                adjacency_complete: 0,
                ..row
            }]),
        );
        let etas = Etas::read(&frame).unwrap();
        assert_eq!(etas.get(DIVISION), Some((50_000, 150_000)));
        assert_eq!(etas.report().refused, 0);
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
        let frame = busy_hour(13_000, 1_000, 30);
        let etas = Etas::read(&frame).unwrap();
        assert_eq!(etas.report().derived, 1_000);
        assert_eq!(etas.report().refused, 0);
        assert_eq!(etas.by_division.len(), 1_000);
        assert_eq!(etas.get(0).map(|pair| pair.1), Some(1_500_000));
    }
}
