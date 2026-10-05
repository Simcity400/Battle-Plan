//! Worker-owned latest operands, loaned across consecutive callbacks.
use super::*;
use std::sync::Arc;

/// Every order and path field the ETA reader takes from the census, the exact
/// ordinary route, and the combat vector header, so entering or leaving combat
/// reads at once. The reader never reads the alternate route. Anything else it
/// reads (organisation, fuel, a combat's progress, modifiers, controllers) is
/// an operand refreshed on the division's five-hour slot.
#[derive(Default, PartialEq, Eq)]
struct Witness {
    pointer: usize,
    owners: [Option<u32>; 3],
    location: Option<usize>,
    template: Option<usize>,
    orders_group: Option<usize>,
    class: Option<u32>,
    stat_block: Option<usize>,
    target: Option<usize>,
    source: Option<usize>,
    transfer: Option<usize>,
    retreat: Option<u8>,
    strategic: Option<u8>,
    combat: Option<[u8; 16]>,
    route: Vec<u32>,
}

impl Witness {
    /// Ok(false) for a division that is not moving.
    fn read(&mut self, memory: &impl Memory, seed: &DivisionSeed) -> Result<bool, &'static str> {
        let route = seed
            .ordinary_route
            .and_then(|header| engine::cpdx_array::Array::from_header(header, 4))
            .ok_or("division ETA witness route invalid")?;
        if route.count == 0 {
            return Ok(false);
        }
        // SetMovementTarget (140bf5730) and AssignUnitMovementRoute (140be87d0)
        // rewrite the route in place, so compare its contents, not its header.
        let count = route.count as usize;
        if count > self.route.len() {
            let additional = count - self.route.len();
            engine::allocation::reserve(&mut self.route, additional)
                .map_err(|_| "collector allocation failed")?;
        }
        self.route.resize(count, 0);
        engine::cpdx_array::copy_u32s(
            memory,
            route,
            &mut self.route,
            "division ETA witness route unreadable",
        )?;
        self.pointer = seed.pointer;
        self.owners = [
            seed.roster_owner,
            seed.logical_owner,
            seed.expeditionary_owner,
        ];
        self.location = seed.location;
        self.template = seed.standard_template;
        self.orders_group = seed.orders_group;
        self.class = seed.class;
        self.stat_block = seed.stat_block;
        self.target = seed.movement_target;
        self.source = seed.movement_source;
        self.transfer = seed.naval_transfer_membership;
        self.retreat = seed.retreat;
        self.strategic = seed.strategic_redeployment;
        self.combat = seed.combat;
        Ok(true)
    }
}

struct Entry {
    witness: Witness,
    row: Option<Arc<DivisionEtaInputsRow>>,
    /// Terrain pairs of the row's read, so a full re-read can be presized.
    terrain: usize,
    seen: i32,
}

/// What a full read of this partition's current moving census needs.
#[derive(Clone, Copy, Default)]
pub(crate) struct Census {
    pub rows: usize,
    pub terrain: usize,
    pub legs: usize,
}

#[derive(Default)]
pub(crate) struct Store {
    entries: Vec<(u64, Entry)>,
    scratch: Witness,
    pub(super) selected: Vec<DivisionSeed>,
    hour: Option<i32>,
    census: Census,
    bytes: usize,
}

impl Store {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    pub(super) fn census(&self) -> Census {
        self.census
    }

    pub(crate) fn invalidate(&mut self) {
        self.hour = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn select(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        force_full: bool,
        partition: (usize, usize),
        seeds: &[DivisionSeed],
        outcome: Outcome,
    ) -> Result<(), &'static str> {
        if partition.1 == 0 || partition.0 >= partition.1 {
            return Err("division ETA partition invalid");
        }
        if force_full || self.hour.and_then(|last| last.checked_add(1)) != Some(hour) {
            self.entries.clear();
        }
        self.hour = None; // Only a complete acquisition commits the witnesses.
        self.selected.clear();
        match outcome {
            Outcome::Rows { .. } => {}
            Outcome::Unavailable { reason } => return Err(reason),
            Outcome::Truncated { .. } => return Err("division ETA witness census truncated"),
        }
        let expected = base
            .checked_add(carmy::VTABLE_RVA)
            .ok_or("division ETA witness module overflow")? as u64;
        // Existing keys need no node storage. Count only new moving keys in
        // this partition; the seed headers are already owned by this pass.
        let missing = seeds
            .iter()
            .filter(|seed| {
                (seed.division ^ (seed.division >> 32)) % partition.1 as u64 == partition.0 as u64
                    && seed
                        .ordinary_route
                        .and_then(|header| engine::cpdx_array::Array::from_header(header, 4))
                        .is_some_and(|route| route.count != 0)
                    && self
                        .entries
                        .binary_search_by_key(&seed.division, |entry| entry.0)
                        .is_err()
            })
            .count();
        engine::allocation::reserve(&mut self.entries, missing)
            .map_err(|_| "collector allocation failed")?;
        let mut selected = 0_usize;
        for seed in seeds {
            let owner = (seed.division ^ (seed.division >> 32)) % partition.1 as u64;
            if owner != partition.0 as u64 {
                continue;
            }
            if seed.pointer == 0
                || seed.division == 0
                || seed.primary_vtable != Some(expected)
                || seed.handle != Some(seed.division)
            {
                return Err("division ETA witness identity invalid");
            }
            if !self.scratch.read(memory, seed)? {
                continue;
            }
            let index = match self
                .entries
                .binary_search_by_key(&seed.division, |entry| entry.0)
            {
                Ok(index) => index,
                Err(index) => {
                    // The census reserve covers every possible new entry this pass.
                    self.entries.insert(
                        index,
                        (
                            seed.division,
                            Entry {
                                witness: Witness::default(),
                                row: None,
                                terrain: 0,
                                seen: hour.wrapping_sub(1),
                            },
                        ),
                    );
                    index
                }
            };
            let entry = &mut self.entries[index].1;
            if entry.seen == hour {
                return Err("division ETA witness duplicate division");
            }
            entry.seen = hour;
            let changed = entry.witness != self.scratch;
            let due = entry.row.as_ref().is_none_or(|row| !row.valid_at(hour))
                || seed.division % 5 == hour.rem_euclid(5) as u64;
            if changed || due {
                std::mem::swap(&mut entry.witness, &mut self.scratch);
                entry.row = None;
                selected += 1;
            }
        }
        self.entries.retain(|(_, entry)| entry.seen == hour);
        engine::allocation::reserve_push_batch(&mut self.selected, selected)
            .map_err(|_| "collector allocation failed")?;
        // A missing row marks a selected witness. Preserve native census order
        // without another native read or another copy of a selected seed.
        for seed in seeds {
            if (seed.division ^ (seed.division >> 32)) % partition.1 as u64 == partition.0 as u64
                && let Ok(index) = self
                    .entries
                    .binary_search_by_key(&seed.division, |entry| entry.0)
                && self.entries[index].1.row.is_none()
            {
                self.selected.push(*seed);
            }
        }
        Ok(())
    }

    /// Commit the selected reads, each with its terrain pair count, and return
    /// every moving division's latest row.
    pub(super) fn finish(
        &mut self,
        hour: i32,
        rows: impl IntoIterator<Item = (DivisionEtaInputsRow, usize)>,
    ) -> Result<Vec<Arc<DivisionEtaInputsRow>>, &'static str> {
        for (row, terrain) in rows {
            let index = self
                .entries
                .binary_search_by_key(&row.division, |entry| entry.0)
                .map_err(|_| "division ETA result missing witness")?;
            let entry = &mut self.entries[index].1;
            entry.row = Some(Arc::new(row));
            entry.terrain = terrain;
        }
        let mut output = Vec::new();
        engine::allocation::reserve_push_batch(&mut output, self.entries.len())
            .map_err(|_| "collector allocation failed")?;
        for (_, entry) in &self.entries {
            output.push(Arc::clone(
                entry
                    .row
                    .as_ref()
                    .ok_or("division ETA selected row missing")?,
            ));
        }
        self.hour = Some(hour);
        // A full read needs room for the whole census, not just this hour's
        // selection: route admission reserves every stored leg.
        self.census = Census {
            rows: self.entries.len(),
            terrain: self
                .entries
                .iter()
                .map(|(_, entry)| entry)
                .map(|entry| entry.terrain)
                .sum(),
            legs: self
                .entries
                .iter()
                .map(|(_, entry)| entry)
                .map(|entry| entry.witness.route.len())
                .sum(),
        };
        // Keep the recorded logical-entry estimate used before checked storage.
        // It is a compatibility metric, not the retained slab's actual capacity.
        self.bytes = self.selected.capacity() * size_of::<DivisionSeed>()
            + self.scratch.route.capacity() * 4
            + self
                .entries
                .iter()
                .map(|(_, entry)| entry)
                .map(|entry| {
                    let row = entry.row.as_ref().unwrap();
                    size_of::<Entry>()
                        + size_of::<DivisionEtaInputsRow>()
                        + 64
                        + entry.witness.route.capacity() * 4
                        + row.terrain_speed.capacity() * size_of::<TerrainSpeed>()
                        + (row.leg_river_combined.capacity()
                            + row.leg_transport_speed.capacity()
                            + row.leg_rail_speed.capacity())
                            * 8
                })
                .sum::<usize>();
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;

    fn seed(id: u64) -> DivisionSeed {
        let mut header = [0; 16];
        header[..8].copy_from_slice(&0x1000_u64.to_le_bytes());
        header[8..12].copy_from_slice(&2_u32.to_le_bytes());
        header[12..].copy_from_slice(&2_u32.to_le_bytes());
        DivisionSeed {
            division: id,
            pointer: 0x2000,
            primary_vtable: Some(carmy::VTABLE_RVA as u64),
            handle: Some(id),
            ordinary_route: Some(header),
            alternate_route: Some([0; 16]),
            movement_target: Some(0),
            movement_source: Some(0),
            location: Some(0),
            retreat: Some(0),
            strategic_redeployment: Some(0),
            naval_transfer_membership: Some(0),
            combat: Some(combat(0)),
            ..Default::default()
        }
    }

    fn combat(count: u32) -> [u8; 16] {
        let mut header = [0; 16];
        header[..8].copy_from_slice(&0x3000_u64.to_le_bytes());
        header[8..12].copy_from_slice(&count.to_le_bytes());
        header[12..].copy_from_slice(&4_u32.to_le_bytes());
        header
    }

    fn read(
        store: &mut Store,
        hour: i32,
        seeds: &[DivisionSeed],
        path: &[u8],
        force: bool,
    ) -> Vec<Arc<DivisionEtaInputsRow>> {
        store
            .select(
                &SliceMemory {
                    base: 0x1000,
                    bytes: path,
                },
                0,
                hour,
                force,
                (0, 1),
                seeds,
                Outcome::Rows { len: seeds.len() },
            )
            .unwrap();
        let rows: Vec<_> = store
            .selected
            .iter()
            .map(|seed| {
                let row = DivisionEtaInputsRow {
                    division: seed.division,
                    sample_hour: hour,
                    ..Default::default()
                };
                (row, 3)
            })
            .collect();
        store.finish(hour, rows).unwrap()
    }

    #[test]
    fn full_first_read_then_one_slot_and_explicit_provenance() {
        let mut store = Store::default();
        let seeds: Vec<_> = (1..=50).map(seed).collect();
        let path = [0; 8];
        let rows = read(&mut store, -7, &seeds, &path, false);
        assert_eq!(store.selected.len(), 50);
        assert!(rows.iter().all(|row| row.sample_hour == -7));
        for hour in -6..9 {
            let rows = read(&mut store, hour, &seeds, &path, false);
            assert_eq!(store.selected.len(), 10);
            for row in rows {
                assert!(row.valid_at(hour));
                assert_eq!(
                    row.sample_hour == hour,
                    row.division % 5 == hour.rem_euclid(5) as u64
                );
            }
        }
        // A full re-read is sized from the whole moving census, not the slot.
        let census = store.census();
        assert_eq!((census.rows, census.terrain, census.legs), (50, 150, 100));
    }

    #[test]
    fn every_order_field_and_combat_entry_refresh_off_slot_but_operands_are_carried() {
        let mut store = Store::default();
        let mut unit = seed(4); // Its slot is hour % 5 == 4.
        let mut path = [0; 8];
        let first = read(&mut store, 0, &[unit], &path, false).remove(0);
        unit.movement_numerator = Some(50);
        unit.organisation = Some(100);
        unit.strength = Some(25);
        unit.alternate_route = Some([2; 16]);
        let mut hour = 1;
        for change in 0..11 {
            // An unchanged hour carries the row; the change then reads it off-slot.
            for fresh in [false, true] {
                if hour % 5 == 4 {
                    let row = read(&mut store, hour, &[unit], &path, false).remove(0);
                    assert_eq!(row.sample_hour, hour);
                    hour += 1;
                }
                if fresh {
                    match change {
                        // SetMovementTarget can refill the same buffer at the same length.
                        0 => path[4] += 1,
                        1 => unit.movement_target = Some(7),
                        2 => unit.location = Some(8),
                        3 => unit.orders_group = Some(9),
                        4 => unit.pointer += 1,
                        5 => unit.logical_owner = Some(10),
                        6 => unit.retreat = Some(1),
                        7 => unit.strategic_redeployment = Some(1),
                        8 => unit.naval_transfer_membership = Some(11),
                        // Entering and leaving combat change its speed branch.
                        9 => unit.combat = Some(combat(1)),
                        _ => unit.combat = Some(combat(0)),
                    }
                }
                let row = read(&mut store, hour, &[unit], &path, false).remove(0);
                assert_eq!(
                    row.sample_hour == hour,
                    fresh,
                    "change {change} hour {hour}"
                );
                if hour == 1 {
                    assert!(Arc::ptr_eq(&first, &row));
                }
                hour += 1;
            }
        }
    }

    #[test]
    fn stops_deletions_gaps_forced_refresh_and_failed_reads_do_not_reuse_rows() {
        let mut store = Store::default();
        let path = [0; 8];
        let unit = seed(4);
        read(&mut store, 0, &[unit], &path, false);
        let mut stopped = unit;
        stopped.ordinary_route = Some([0; 16]);
        assert!(read(&mut store, 1, &[stopped], &path, false).is_empty());
        assert_eq!(read(&mut store, 2, &[unit], &path, false)[0].sample_hour, 2);
        assert!(read(&mut store, 3, &[], &path, false).is_empty());
        assert_eq!(read(&mut store, 5, &[unit], &path, false)[0].sample_hour, 5);
        assert_eq!(read(&mut store, 6, &[unit], &path, true)[0].sample_hour, 6);
        assert!(
            store
                .select(
                    &SliceMemory {
                        base: 0,
                        bytes: &[]
                    },
                    0,
                    7,
                    false,
                    (0, 1),
                    &[unit],
                    Outcome::Rows { len: 1 }
                )
                .is_err()
        );
        assert_eq!(read(&mut store, 8, &[unit], &path, false)[0].sample_hour, 8);
        store.invalidate(); // accepted worker failure / unpublished acquisition
        assert_eq!(read(&mut store, 9, &[unit], &path, false)[0].sample_hour, 9);
        assert_eq!(read(&mut store, 0, &[unit], &path, false)[0].sample_hour, 0);
    }

    #[test]
    fn reordering_and_unrelated_arrivals_do_not_refresh_off_slot_divisions() {
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[0; 8],
        };
        let mut stores = [Store::default(), Store::default()];
        for hour in 0..5 {
            let mut seeds: Vec<_> = (1..=10).map(|id| seed((id << 32) | 9000)).collect();
            if hour != 0 {
                seeds.reverse();
                seeds.insert(3, seed((11 << 32) | 9000));
            }
            let mut seen = std::collections::BTreeSet::new();
            for (part, store) in stores.iter_mut().enumerate() {
                store
                    .select(
                        &memory,
                        0,
                        hour,
                        false,
                        (part, 2),
                        &seeds,
                        Outcome::Rows { len: seeds.len() },
                    )
                    .unwrap();
                let rows: Vec<_> = store
                    .selected
                    .iter()
                    .map(|unit| {
                        let row = DivisionEtaInputsRow {
                            division: unit.division,
                            sample_hour: hour,
                            ..Default::default()
                        };
                        (row, 0)
                    })
                    .collect();
                for row in store.finish(hour, rows).unwrap() {
                    assert!(seen.insert(row.division));
                    let new = hour == 0 || (hour == 1 && row.division == ((11 << 32) | 9000));
                    assert_eq!(
                        row.sample_hour == hour,
                        new || row.division % 5 == hour as u64 % 5
                    );
                }
            }
            assert_eq!(seen.len(), seeds.len());
        }
    }

    #[test]
    fn incomplete_selected_operands_and_census_are_never_committed() {
        let mut store = Store::default();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[0; 8],
        };
        let unit = seed(4);
        store
            .select(
                &memory,
                0,
                0,
                false,
                (0, 1),
                &[unit],
                Outcome::Rows { len: 1 },
            )
            .unwrap();
        assert!(store.finish(0, vec![]).is_err());
        assert_eq!(
            read(&mut store, 1, &[unit], &[0; 8], false)[0].sample_hour,
            1
        );
        assert!(
            store
                .select(
                    &memory,
                    0,
                    2,
                    false,
                    (0, 1),
                    &[unit],
                    Outcome::Truncated {
                        len: 1,
                        unread_rows: 1
                    }
                )
                .is_err()
        );
        assert_eq!(
            read(&mut store, 3, &[unit], &[0; 8], false)[0].sample_hour,
            3
        );
    }

    #[test]
    fn retained_capacities_and_metric_match_pinned_btree_behavior() {
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[0; 8],
        };
        let mut store = Store::default();
        let mut previous_selected = Vec::<DivisionSeed>::new();
        let mut previous_entries = std::collections::BTreeMap::<u64, Entry>::new();
        let mut previous_scratch = Witness::default();
        let mut previous_hour: Option<i32> = None;
        for (hour, count, force) in [
            (0, 50, false),
            (1, 50, false),
            (2, 50, false),
            (3, 5, false),
            (4, 70, false),
            (5, 0, false),
            (6, 2, false),
            (10, 7, false),
            (11, 7, true),
        ] {
            let mut seeds: Vec<_> = (1..=count).map(seed).collect();
            if hour % 2 != 0 {
                seeds.reverse();
            }
            let rows = read(&mut store, hour, &seeds, &[0; 8], force);
            // Pinned e46's BTreeMap witness selection, push growth, optional
            // row collection and recorded logical-entry estimate.
            if force || previous_hour.and_then(|last| last.checked_add(1)) != Some(hour) {
                previous_entries.clear();
            }
            previous_selected.clear();
            for seed in &seeds {
                if !previous_scratch.read(&memory, seed).unwrap() {
                    continue;
                }
                let entry = previous_entries
                    .entry(seed.division)
                    .or_insert_with(|| Entry {
                        witness: Witness::default(),
                        row: None,
                        terrain: 0,
                        seen: hour.wrapping_sub(1),
                    });
                entry.seen = hour;
                let changed = entry.witness != previous_scratch;
                let due = entry.row.as_ref().is_none_or(|row| !row.valid_at(hour))
                    || seed.division % 5 == hour.rem_euclid(5) as u64;
                if changed || due {
                    std::mem::swap(&mut entry.witness, &mut previous_scratch);
                    entry.row = None;
                    previous_selected.push(*seed);
                }
            }
            previous_entries.retain(|_, entry| entry.seen == hour);
            for seed in &previous_selected {
                let entry = previous_entries.get_mut(&seed.division).unwrap();
                entry.row = Some(Arc::new(DivisionEtaInputsRow {
                    division: seed.division,
                    sample_hour: hour,
                    ..Default::default()
                }));
                entry.terrain = 3;
            }
            let previous_output: Option<Vec<_>> = previous_entries
                .values()
                .map(|entry| entry.row.clone())
                .collect();
            let previous_output = previous_output.unwrap();
            previous_hour = Some(hour);
            let previous_bytes = previous_selected.capacity() * size_of::<DivisionSeed>()
                + previous_scratch.route.capacity() * 4
                + previous_entries
                    .values()
                    .map(|entry| {
                        let row = entry.row.as_ref().unwrap();
                        size_of::<Entry>()
                            + size_of::<DivisionEtaInputsRow>()
                            + 64
                            + entry.witness.route.capacity() * 4
                            + row.terrain_speed.capacity() * size_of::<TerrainSpeed>()
                            + (row.leg_river_combined.capacity()
                                + row.leg_transport_speed.capacity()
                                + row.leg_rail_speed.capacity())
                                * 8
                    })
                    .sum::<usize>();
            assert_eq!(
                store.selected.capacity(),
                previous_selected.capacity(),
                "selected hour {hour}"
            );
            assert_eq!(
                rows.capacity(),
                previous_output.capacity(),
                "carried hour {hour}"
            );
            assert_eq!(
                store.allocated_bytes(),
                previous_bytes,
                "metric hour {hour}"
            );
            assert!(
                store
                    .selected
                    .iter()
                    .zip(&previous_selected)
                    .all(|(seed, previous)| seed.division == previous.division)
            );
            assert!(
                rows.iter()
                    .zip(&previous_output)
                    .all(|(row, previous)| row.division == previous.division
                        && row.sample_hour == previous.sample_hour)
            );
        }
    }

    #[test]
    fn deletion_releases_rows_but_retains_checked_entry_slab() {
        let mut store = Store::default();
        let seeds: Vec<_> = (1..=50).map(seed).collect();
        let rows = read(&mut store, 0, &seeds, &[0; 8], false);
        let owner = Arc::downgrade(&rows[0]);
        drop(rows);
        let capacity = store.entries.capacity();
        let bytes = store.allocated_bytes();
        assert!(read(&mut store, 1, &[], &[0; 8], false).is_empty());
        assert!(owner.upgrade().is_none());
        assert!(store.entries.is_empty());
        assert_eq!(store.entries.capacity(), capacity);
        assert!(store.allocated_bytes() < bytes);
        let retained_slab_bytes = capacity * size_of::<(u64, Entry)>();
        assert!(retained_slab_bytes != 0);
        // The compatibility estimate excludes vacant entry storage; its named
        // rule does not certify actual retained allocation after deletions.
        assert_eq!(
            store.allocated_bytes(),
            store.selected.capacity() * size_of::<DivisionSeed>()
                + store.scratch.route.capacity() * 4
        );
    }
}
